use semver::Version;
use serde::Serialize;

use crate::changelog::{ChangelogEntry, ChangelogFormatter};
use crate::commit::{CommitParser, ConventionalCommit, DefaultCommitClassifier};
use crate::config::{Config, PackageConfig};
use crate::error::ReleaseError;
use crate::git::GitRepository;
use crate::stages::{StageContext, default_pipeline};
use crate::version::{BumpLevel, apply_bump, apply_prerelease_bump, determine_bump};

/// The computed plan for a release, before execution.
#[derive(Debug, Serialize)]
pub struct ReleasePlan {
    pub current_version: Option<Version>,
    pub next_version: Version,
    pub bump: BumpLevel,
    pub commits: Vec<ConventionalCommit>,
    pub tag_name: String,
    pub floating_tag_name: Option<String>,
    pub prerelease: bool,
}

/// Orchestrates the release flow.
pub trait ReleaseStrategy: Send + Sync {
    /// Plan the release without executing it.
    fn plan(&self) -> Result<ReleasePlan, ReleaseError>;

    /// Execute the release.
    fn execute(&self, plan: &ReleasePlan, dry_run: bool) -> Result<(), ReleaseError>;
}

/// Abstraction over a remote VCS provider (e.g. GitHub, GitLab).
pub trait VcsProvider: Send + Sync {
    /// Create a release on the remote VCS.
    fn create_release(
        &self,
        tag: &str,
        name: &str,
        body: &str,
        prerelease: bool,
        draft: bool,
    ) -> Result<String, ReleaseError>;

    /// Generate a compare URL between two refs.
    fn compare_url(&self, base: &str, head: &str) -> Result<String, ReleaseError>;

    /// Check if a release already exists for the given tag.
    fn release_exists(&self, tag: &str) -> Result<bool, ReleaseError>;

    /// Delete a release by tag.
    fn delete_release(&self, tag: &str) -> Result<(), ReleaseError>;

    /// Return the base URL of the repository (e.g. `https://github.com/owner/repo`).
    fn repo_url(&self) -> Option<String> {
        None
    }

    /// Update an existing release (name and body) using PATCH semantics,
    /// preserving any previously uploaded assets.
    fn update_release(
        &self,
        _tag: &str,
        _name: &str,
        _body: &str,
        _prerelease: bool,
        _draft: bool,
    ) -> Result<String, ReleaseError> {
        Err(ReleaseError::Vcs(
            "update_release not implemented for this provider".into(),
        ))
    }

    /// Upload asset files to an existing release identified by tag.
    fn upload_assets(&self, _tag: &str, _files: &[&str]) -> Result<(), ReleaseError> {
        Ok(())
    }

    /// List the basenames of assets currently attached to the release for `tag`.
    /// Returns `Ok(vec![])` for providers that don't support asset listing.
    /// Used by the idempotent upload path to skip assets already present.
    fn list_assets(&self, _tag: &str) -> Result<Vec<String>, ReleaseError> {
        Ok(Vec::new())
    }

    /// Fetch the content of a named asset on the release for `tag`.
    /// Returns `Ok(None)` if the asset doesn't exist or the provider doesn't
    /// support it. Used by the reconciler to read `sr-manifest.json`.
    fn fetch_asset(&self, _tag: &str, _name: &str) -> Result<Option<Vec<u8>>, ReleaseError> {
        Ok(None)
    }

    /// Verify that a release exists and is in the expected state after creation.
    fn verify_release(&self, _tag: &str) -> Result<(), ReleaseError> {
        Ok(())
    }
}

/// A no-op VcsProvider that silently succeeds. Used when no remote VCS
/// (e.g. GitHub) is configured.
pub struct NoopVcsProvider;

impl VcsProvider for NoopVcsProvider {
    fn create_release(
        &self,
        _tag: &str,
        _name: &str,
        _body: &str,
        _prerelease: bool,
        _draft: bool,
    ) -> Result<String, ReleaseError> {
        Ok(String::new())
    }

    fn compare_url(&self, _base: &str, _head: &str) -> Result<String, ReleaseError> {
        Ok(String::new())
    }

    fn release_exists(&self, _tag: &str) -> Result<bool, ReleaseError> {
        Ok(false)
    }

    fn delete_release(&self, _tag: &str) -> Result<(), ReleaseError> {
        Ok(())
    }
}

/// Concrete release strategy implementing the trunk-based release flow.
pub struct TrunkReleaseStrategy<G, V, C, F> {
    pub git: G,
    pub vcs: V,
    pub parser: C,
    pub formatter: F,
    pub config: Config,
    /// When true, re-release the current tag if HEAD is at the latest tag.
    pub force: bool,
    /// Pre-release identifier resolved from the active channel (None = stable).
    pub prerelease_id: Option<String>,
    /// Whether the GitHub release should be created as a draft.
    pub draft: bool,
}

impl<G, V, C, F> TrunkReleaseStrategy<G, V, C, F>
where
    G: GitRepository,
    V: VcsProvider,
    C: CommitParser,
    F: ChangelogFormatter,
{
    fn format_changelog(&self, plan: &ReleasePlan) -> Result<String, ReleaseError> {
        let today = today_string();
        let compare_url = match &plan.current_version {
            Some(v) => {
                let base = format!("{}{v}", self.config.git.tag_prefix);
                self.vcs
                    .compare_url(&base, &plan.tag_name)
                    .ok()
                    .filter(|s| !s.is_empty())
            }
            None => None,
        };
        let entry = ChangelogEntry {
            version: plan.next_version.to_string(),
            date: today,
            commits: plan.commits.clone(),
            compare_url,
            repo_url: self.vcs.repo_url(),
        };
        self.formatter.format(&[entry])
    }

    /// Render the release name from the configured template, or fall back to the tag name.
    fn release_name(&self, plan: &ReleasePlan) -> String {
        if let Some(ref template_str) = self.config.vcs.github.release_name_template {
            let mut env = minijinja::Environment::new();
            if env.add_template("release_name", template_str).is_ok()
                && let Ok(tmpl) = env.get_template("release_name")
                && let Ok(rendered) = tmpl.render(minijinja::context! {
                    version => plan.next_version.to_string(),
                    tag_name => &plan.tag_name,
                    tag_prefix => &self.config.git.tag_prefix,
                })
            {
                return rendered;
            }
            eprintln!("warning: invalid release_name_template, falling back to tag name");
        }
        plan.tag_name.clone()
    }

    /// Return the active package for a single-package release (the root package or the only one).
    /// Returns the root package (".") if present, otherwise the first package.
    fn active_package(&self) -> Option<&PackageConfig> {
        self.config
            .packages
            .iter()
            .find(|p| p.path == ".")
            .or_else(|| self.config.packages.first())
    }
}

impl<G, V, C, F> ReleaseStrategy for TrunkReleaseStrategy<G, V, C, F>
where
    G: GitRepository,
    V: VcsProvider,
    C: CommitParser,
    F: ChangelogFormatter,
{
    fn plan(&self) -> Result<ReleasePlan, ReleaseError> {
        let is_prerelease = self.prerelease_id.is_some();

        // For stable releases, find the latest stable tag (skip pre-release tags).
        // For pre-releases, find the latest tag of any kind to determine commits since.
        let all_tags = self.git.all_tags(&self.config.git.tag_prefix)?;
        let latest_stable = all_tags.iter().rev().find(|t| t.version.pre.is_empty());
        let latest_any = all_tags.last();

        // Reconciliation check: don't cut a new release on top of an incomplete
        // one. `--force` bypasses (it's explicit user intent to re-run the
        // pipeline, which itself heals). Unknown status (legacy release, no
        // manifest) passes silently — we can't distinguish "old release" from
        // "sr died before upload" remotely.
        if !self.force
            && let Some(latest) = all_tags.last()
        {
            match crate::manifest::check_release_status(&self.vcs, &latest.name)? {
                crate::manifest::ReleaseStatus::Incomplete {
                    missing_artifacts, ..
                } => {
                    return Err(ReleaseError::Vcs(format!(
                        "previous release {tag} is incomplete: {} declared asset(s) missing ({}). \
                         Heal it first — `git checkout {tag} && sr release --force` — \
                         then re-run sr release.",
                        missing_artifacts.len(),
                        missing_artifacts.join(", "),
                        tag = latest.name,
                    )));
                }
                crate::manifest::ReleaseStatus::Complete(_)
                | crate::manifest::ReleaseStatus::Unknown => {}
            }
        }

        // Use the latest tag (any kind) for commit range, but the latest stable for base version
        let tag_info = if is_prerelease {
            latest_any
        } else {
            latest_stable.or(latest_any)
        };

        let (current_version, from_sha) = match tag_info {
            Some(info) => (Some(info.version.clone()), Some(info.sha.as_str())),
            None => (None, None),
        };

        let default_pkg = PackageConfig::default();
        let pkg = self.active_package().unwrap_or(&default_pkg);
        let path_filter = if pkg.path != "." {
            Some(pkg.path.as_str())
        } else {
            None
        };

        let raw_commits = if let Some(path) = path_filter {
            self.git.commits_since_in_path(from_sha, path)?
        } else {
            self.git.commits_since(from_sha)?
        };

        if raw_commits.is_empty() {
            // Force mode: re-release if HEAD is exactly at the latest tag
            if self.force
                && let Some(info) = tag_info
            {
                let head = self.git.head_sha()?;
                if head == info.sha {
                    let floating_tag_name = if self.config.git.floating_tag {
                        Some(format!(
                            "{}{}",
                            self.config.git.tag_prefix, info.version.major
                        ))
                    } else {
                        None
                    };
                    return Ok(ReleasePlan {
                        current_version: Some(info.version.clone()),
                        next_version: info.version.clone(),
                        bump: BumpLevel::Patch,
                        commits: vec![],
                        tag_name: info.name.clone(),
                        floating_tag_name,
                        prerelease: is_prerelease,
                    });
                }
            }
            let (tag, sha) = match tag_info {
                Some(info) => (info.name.clone(), info.sha.clone()),
                None => ("(none)".into(), "(none)".into()),
            };
            return Err(ReleaseError::NoCommits { tag, sha });
        }

        let skip_patterns = &self.config.git.skip_patterns;
        let conventional_commits: Vec<ConventionalCommit> = raw_commits
            .iter()
            .filter(|c| !c.message.starts_with("chore(release):"))
            .filter(|c| !skip_patterns.iter().any(|p| c.message.contains(p.as_str())))
            .filter_map(|c| self.parser.parse(c).ok())
            .collect();

        let classifier = DefaultCommitClassifier::new(self.config.commit.types.into_commit_types());
        let tag_for_err = tag_info
            .map(|i| i.name.clone())
            .unwrap_or_else(|| "(none)".into());
        let commit_count = conventional_commits.len();
        let bump = match determine_bump(&conventional_commits, &classifier) {
            Some(b) => b,
            None if self.force => BumpLevel::Patch,
            None => {
                return Err(ReleaseError::NoBump {
                    tag: tag_for_err,
                    commit_count,
                });
            }
        };

        // For pre-releases, base the version on the latest *stable* tag
        let base_version = if is_prerelease {
            latest_stable
                .map(|t| t.version.clone())
                .or(current_version.clone())
                .unwrap_or(Version::new(0, 0, 0))
        } else {
            current_version.clone().unwrap_or(Version::new(0, 0, 0))
        };

        // v0 protection: downshift Major → Minor when version is 0.x.y
        // to prevent accidentally bumping to v1. Disable with git.v0_protection: false.
        let bump =
            if base_version.major == 0 && bump == BumpLevel::Major && self.config.git.v0_protection
            {
                eprintln!(
                    "v0 protection: breaking change detected at v{base_version}, \
                     downshifting major → minor (set git.v0_protection: false to bump to v1)"
                );
                BumpLevel::Minor
            } else {
                bump
            };

        let next_version = if let Some(ref prerelease_id) = self.prerelease_id {
            let existing_versions: Vec<Version> =
                all_tags.iter().map(|t| t.version.clone()).collect();
            apply_prerelease_bump(&base_version, bump, prerelease_id, &existing_versions)
        } else {
            apply_bump(&base_version, bump)
        };

        let tag_name = format!("{}{next_version}", self.config.git.tag_prefix);

        // Don't update floating tags for pre-releases
        let floating_tag_name = if self.config.git.floating_tag && !is_prerelease {
            Some(format!(
                "{}{}",
                self.config.git.tag_prefix, next_version.major
            ))
        } else {
            None
        };

        Ok(ReleasePlan {
            current_version,
            next_version,
            bump,
            commits: conventional_commits,
            tag_name,
            floating_tag_name,
            prerelease: is_prerelease,
        })
    }

    fn execute(&self, plan: &ReleasePlan, dry_run: bool) -> Result<(), ReleaseError> {
        let version_str = plan.next_version.to_string();
        let changelog_body = self.format_changelog(plan)?;
        let release_name = self.release_name(plan);

        let env = release_env(&version_str, &plan.tag_name);
        let env_refs: Vec<(&str, &str)> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        let default_pkg = PackageConfig::default();
        let active_package = self.active_package().unwrap_or(&default_pkg);

        let mut ctx = StageContext {
            plan,
            config: &self.config,
            git: &self.git,
            vcs: &self.vcs,
            active_package,
            changelog_body: &changelog_body,
            release_name: &release_name,
            version_str: &version_str,
            hooks_env: &env_refs,
            dry_run,
            sign_tags: self.config.git.sign_tags,
            draft: self.draft,
            bumped_files: Vec::new(),
        };

        for stage in default_pipeline() {
            if !stage.is_complete(&ctx)? {
                stage.run(&mut ctx)?;
            }
        }

        if dry_run {
            eprintln!("[dry-run] Changelog:\n{changelog_body}");
        } else {
            eprintln!("Released {}", plan.tag_name);
        }
        Ok(())
    }
}

/// Build release env vars as owned strings.
fn release_env(version: &str, tag: &str) -> Vec<(String, String)> {
    vec![
        ("SR_VERSION".into(), version.into()),
        ("SR_TAG".into(), tag.into()),
    ]
}

/// Resolve glob patterns into a deduplicated, sorted list of file paths.
pub(crate) fn resolve_globs(patterns: &[String]) -> Result<Vec<String>, String> {
    let mut files = std::collections::BTreeSet::new();
    for pattern in patterns {
        let paths =
            glob::glob(pattern).map_err(|e| format!("invalid glob pattern '{pattern}': {e}"))?;
        for entry in paths {
            match entry {
                Ok(path) if path.is_file() => {
                    files.insert(path.to_string_lossy().into_owned());
                }
                Ok(_) => {}
                Err(e) => {
                    return Err(format!("glob error for pattern '{pattern}': {e}"));
                }
            }
        }
    }
    Ok(files.into_iter().collect())
}

pub fn today_string() -> String {
    // Portable date calculation from UNIX epoch (no external deps or subprocess).
    // Uses Howard Hinnant's civil_from_days algorithm.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let z = secs / 86400 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::changelog::DefaultChangelogFormatter;
    use crate::commit::{Commit, TypedCommitParser};
    use crate::config::{
        ChangelogConfig, Config, GitConfig, HooksConfig, PackageConfig, default_changelog_groups,
    };
    use crate::git::{GitRepository, TagInfo};

    // --- Fakes ---

    struct FakeGit {
        tags: Vec<TagInfo>,
        commits: Vec<Commit>,
        /// Commits returned when path filtering is active (None = fall back to `commits`).
        path_commits: Option<Vec<Commit>>,
        head: String,
        created_tags: Mutex<Vec<String>>,
        pushed_tags: Mutex<Vec<String>>,
        committed: Mutex<Vec<(Vec<String>, String)>>,
        push_count: Mutex<u32>,
        force_created_tags: Mutex<Vec<String>>,
        force_pushed_tags: Mutex<Vec<String>>,
    }

    impl FakeGit {
        fn new(tags: Vec<TagInfo>, commits: Vec<Commit>) -> Self {
            let head = tags
                .last()
                .map(|t| t.sha.clone())
                .unwrap_or_else(|| "0".repeat(40));
            Self {
                tags,
                commits,
                path_commits: None,
                head,
                created_tags: Mutex::new(Vec::new()),
                pushed_tags: Mutex::new(Vec::new()),
                committed: Mutex::new(Vec::new()),
                push_count: Mutex::new(0),
                force_created_tags: Mutex::new(Vec::new()),
                force_pushed_tags: Mutex::new(Vec::new()),
            }
        }
    }

    impl GitRepository for FakeGit {
        fn latest_tag(&self, _prefix: &str) -> Result<Option<TagInfo>, ReleaseError> {
            Ok(self.tags.last().cloned())
        }

        fn commits_since(&self, _from: Option<&str>) -> Result<Vec<Commit>, ReleaseError> {
            Ok(self.commits.clone())
        }

        fn create_tag(&self, name: &str, _message: &str, _sign: bool) -> Result<(), ReleaseError> {
            self.created_tags.lock().unwrap().push(name.to_string());
            Ok(())
        }

        fn push_tag(&self, name: &str) -> Result<(), ReleaseError> {
            self.pushed_tags.lock().unwrap().push(name.to_string());
            Ok(())
        }

        fn stage_and_commit(&self, paths: &[&str], message: &str) -> Result<bool, ReleaseError> {
            self.committed.lock().unwrap().push((
                paths.iter().map(|s| s.to_string()).collect(),
                message.to_string(),
            ));
            Ok(true)
        }

        fn push(&self) -> Result<(), ReleaseError> {
            *self.push_count.lock().unwrap() += 1;
            Ok(())
        }

        fn tag_exists(&self, name: &str) -> Result<bool, ReleaseError> {
            Ok(self
                .created_tags
                .lock()
                .unwrap()
                .contains(&name.to_string()))
        }

        fn remote_tag_exists(&self, name: &str) -> Result<bool, ReleaseError> {
            Ok(self.pushed_tags.lock().unwrap().contains(&name.to_string()))
        }

        fn all_tags(&self, _prefix: &str) -> Result<Vec<TagInfo>, ReleaseError> {
            Ok(self.tags.clone())
        }

        fn commits_between(
            &self,
            _from: Option<&str>,
            _to: &str,
        ) -> Result<Vec<Commit>, ReleaseError> {
            Ok(self.commits.clone())
        }

        fn tag_date(&self, _tag_name: &str) -> Result<String, ReleaseError> {
            Ok("2026-01-01".into())
        }

        fn force_create_tag(&self, name: &str) -> Result<(), ReleaseError> {
            self.force_created_tags
                .lock()
                .unwrap()
                .push(name.to_string());
            Ok(())
        }

        fn force_push_tag(&self, name: &str) -> Result<(), ReleaseError> {
            self.force_pushed_tags
                .lock()
                .unwrap()
                .push(name.to_string());
            Ok(())
        }

        fn head_sha(&self) -> Result<String, ReleaseError> {
            Ok(self.head.clone())
        }

        fn commits_since_in_path(
            &self,
            _from: Option<&str>,
            _path: &str,
        ) -> Result<Vec<Commit>, ReleaseError> {
            Ok(self
                .path_commits
                .clone()
                .unwrap_or_else(|| self.commits.clone()))
        }
    }

    struct FakeVcs {
        releases: Mutex<Vec<(String, String)>>,
        deleted_releases: Mutex<Vec<String>>,
        uploaded_assets: Mutex<Vec<(String, Vec<String>)>>,
        /// (tag, basename) → bytes. Populated by upload_assets when the file
        /// on disk is readable; consumed by fetch_asset and list_assets to
        /// simulate GitHub's release-assets view.
        stored_assets: Mutex<Vec<(String, String, Vec<u8>)>>,
    }

    impl FakeVcs {
        fn new() -> Self {
            Self {
                releases: Mutex::new(Vec::new()),
                deleted_releases: Mutex::new(Vec::new()),
                uploaded_assets: Mutex::new(Vec::new()),
                stored_assets: Mutex::new(Vec::new()),
            }
        }

        /// Pre-seed an asset on a release — for reconciliation tests where the
        /// starting state already has a manifest.
        fn seed_asset(&self, tag: &str, name: &str, content: Vec<u8>) {
            self.stored_assets
                .lock()
                .unwrap()
                .push((tag.to_string(), name.to_string(), content));
        }
    }

    impl VcsProvider for FakeVcs {
        fn create_release(
            &self,
            tag: &str,
            _name: &str,
            body: &str,
            _prerelease: bool,
            _draft: bool,
        ) -> Result<String, ReleaseError> {
            self.releases
                .lock()
                .unwrap()
                .push((tag.to_string(), body.to_string()));
            Ok(format!("https://github.com/test/release/{tag}"))
        }

        fn compare_url(&self, base: &str, head: &str) -> Result<String, ReleaseError> {
            Ok(format!("https://github.com/test/compare/{base}...{head}"))
        }

        fn release_exists(&self, tag: &str) -> Result<bool, ReleaseError> {
            Ok(self.releases.lock().unwrap().iter().any(|(t, _)| t == tag))
        }

        fn delete_release(&self, tag: &str) -> Result<(), ReleaseError> {
            self.deleted_releases.lock().unwrap().push(tag.to_string());
            self.releases.lock().unwrap().retain(|(t, _)| t != tag);
            Ok(())
        }

        fn update_release(
            &self,
            tag: &str,
            _name: &str,
            body: &str,
            _prerelease: bool,
            _draft: bool,
        ) -> Result<String, ReleaseError> {
            let mut releases = self.releases.lock().unwrap();
            if let Some(entry) = releases.iter_mut().find(|(t, _)| t == tag) {
                entry.1 = body.to_string();
            }
            Ok(format!("https://github.com/test/release/{tag}"))
        }

        fn upload_assets(&self, tag: &str, files: &[&str]) -> Result<(), ReleaseError> {
            self.uploaded_assets.lock().unwrap().push((
                tag.to_string(),
                files.iter().map(|s| s.to_string()).collect(),
            ));
            // Mirror into stored_assets so list/fetch see what was uploaded.
            for path in files {
                let basename = std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path)
                    .to_string();
                let content = std::fs::read(path).unwrap_or_default();
                self.stored_assets
                    .lock()
                    .unwrap()
                    .push((tag.to_string(), basename, content));
            }
            Ok(())
        }

        fn list_assets(&self, tag: &str) -> Result<Vec<String>, ReleaseError> {
            Ok(self
                .stored_assets
                .lock()
                .unwrap()
                .iter()
                .filter(|(t, _, _)| t == tag)
                .map(|(_, n, _)| n.clone())
                .collect())
        }

        fn fetch_asset(&self, tag: &str, name: &str) -> Result<Option<Vec<u8>>, ReleaseError> {
            Ok(self
                .stored_assets
                .lock()
                .unwrap()
                .iter()
                .find(|(t, n, _)| t == tag && n == name)
                .map(|(_, _, b)| b.clone()))
        }

        fn repo_url(&self) -> Option<String> {
            Some("https://github.com/test/repo".into())
        }
    }

    // --- Helpers ---

    type TestStrategy =
        TrunkReleaseStrategy<FakeGit, FakeVcs, TypedCommitParser, DefaultChangelogFormatter>;

    /// Build a Config with changelog file disabled and a dummy version file,
    /// so tests don't pollute the real CHANGELOG.md or auto-detect and bump
    /// the actual Cargo.toml of whichever crate is running the tests.
    fn test_config() -> Config {
        Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec!["__sr_test_dummy_no_bump__".into()],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// Build a Config with custom git settings (still isolated from real files).
    fn config_with_git(git: GitConfig) -> Config {
        Config {
            git,
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec!["__sr_test_dummy_no_bump__".into()],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn make_strategy(tags: Vec<TagInfo>, commits: Vec<Commit>, config: Config) -> TestStrategy {
        TrunkReleaseStrategy {
            git: FakeGit::new(tags, commits),
            vcs: FakeVcs::new(),
            parser: TypedCommitParser::default(),
            formatter: DefaultChangelogFormatter::new(None, default_changelog_groups()),
            config,
            force: false,
            prerelease_id: None,
            draft: false,
        }
    }

    fn raw_commit(msg: &str) -> Commit {
        Commit {
            sha: "a".repeat(40),
            message: msg.into(),
        }
    }

    // --- plan() tests ---

    #[test]
    fn plan_no_commits_returns_error() {
        let s = make_strategy(vec![], vec![], Config::default());
        let err = s.plan().unwrap_err();
        assert!(matches!(err, ReleaseError::NoCommits { .. }));
    }

    #[test]
    fn plan_no_releasable_returns_error() {
        let s = make_strategy(
            vec![],
            vec![raw_commit("chore: tidy up")],
            Config::default(),
        );
        let err = s.plan().unwrap_err();
        assert!(matches!(err, ReleaseError::NoBump { .. }));
    }

    #[test]
    fn force_releases_patch_when_no_releasable_commits() {
        let tag = TagInfo {
            name: "v1.2.3".into(),
            version: Version::new(1, 2, 3),
            sha: "d".repeat(40),
        };
        let mut s = make_strategy(
            vec![tag],
            vec![raw_commit("chore: rename package")],
            Config::default(),
        );
        s.force = true;
        let plan = s.plan().unwrap();
        assert_eq!(plan.next_version, Version::new(1, 2, 4));
        assert_eq!(plan.bump, BumpLevel::Patch);
    }

    #[test]
    fn plan_first_release() {
        let s = make_strategy(
            vec![],
            vec![raw_commit("feat: initial feature")],
            Config::default(),
        );
        let plan = s.plan().unwrap();
        assert_eq!(plan.next_version, Version::new(0, 1, 0));
        assert_eq!(plan.tag_name, "v0.1.0");
        assert!(plan.current_version.is_none());
    }

    #[test]
    fn plan_skips_commits_matching_skip_patterns() {
        let s = make_strategy(
            vec![],
            vec![
                raw_commit("feat: real feature"),
                raw_commit("feat: noisy experiment [skip release]"),
                raw_commit("fix: swallowed fix [skip sr]"),
            ],
            test_config(),
        );
        let plan = s.plan().unwrap();
        assert_eq!(plan.commits.len(), 1);
        assert_eq!(plan.commits[0].description, "real feature");
    }

    #[test]
    fn plan_custom_skip_patterns_override_defaults() {
        let git = GitConfig {
            skip_patterns: vec!["DO-NOT-RELEASE".into()],
            ..Default::default()
        };
        let s = make_strategy(
            vec![],
            vec![
                raw_commit("feat: shipped"),
                raw_commit("feat: DO-NOT-RELEASE internal"),
                // default patterns no longer active → this commit counts
                raw_commit("feat: still here [skip release]"),
            ],
            config_with_git(git),
        );
        let plan = s.plan().unwrap();
        assert_eq!(plan.commits.len(), 2);
    }

    #[test]
    fn plan_increments_existing() {
        let tag = TagInfo {
            name: "v1.2.3".into(),
            version: Version::new(1, 2, 3),
            sha: "b".repeat(40),
        };
        let s = make_strategy(
            vec![tag],
            vec![raw_commit("fix: patch bug")],
            Config::default(),
        );
        let plan = s.plan().unwrap();
        assert_eq!(plan.next_version, Version::new(1, 2, 4));
    }

    #[test]
    fn plan_breaking_bump() {
        let tag = TagInfo {
            name: "v1.2.3".into(),
            version: Version::new(1, 2, 3),
            sha: "c".repeat(40),
        };
        let s = make_strategy(
            vec![tag],
            vec![raw_commit("feat!: breaking change")],
            Config::default(),
        );
        let plan = s.plan().unwrap();
        assert_eq!(plan.next_version, Version::new(2, 0, 0));
    }

    #[test]
    fn plan_v0_breaking_downshifts_to_minor() {
        let tag = TagInfo {
            name: "v0.5.0".into(),
            version: Version::new(0, 5, 0),
            sha: "c".repeat(40),
        };
        let s = make_strategy(
            vec![tag],
            vec![raw_commit("feat!: breaking change")],
            Config::default(),
        );
        let plan = s.plan().unwrap();
        // v0 protection: Major → Minor, so 0.5.0 → 0.6.0 (not 1.0.0)
        assert_eq!(plan.next_version, Version::new(0, 6, 0));
        assert_eq!(plan.bump, BumpLevel::Minor);
    }

    #[test]
    fn plan_v0_breaking_with_protection_disabled_bumps_major() {
        let tag = TagInfo {
            name: "v0.5.0".into(),
            version: Version::new(0, 5, 0),
            sha: "c".repeat(40),
        };
        let mut config = Config::default();
        config.git.v0_protection = false;
        let s = make_strategy(
            vec![tag],
            vec![raw_commit("feat!: breaking change")],
            config,
        );
        let plan = s.plan().unwrap();
        // v0_protection: false allows bumping to v1
        assert_eq!(plan.next_version, Version::new(1, 0, 0));
        assert_eq!(plan.bump, BumpLevel::Major);
    }

    #[test]
    fn plan_v0_feat_stays_minor() {
        let tag = TagInfo {
            name: "v0.5.0".into(),
            version: Version::new(0, 5, 0),
            sha: "c".repeat(40),
        };
        let s = make_strategy(
            vec![tag],
            vec![raw_commit("feat: new feature")],
            Config::default(),
        );
        let plan = s.plan().unwrap();
        // Non-breaking feat in v0 stays as minor bump
        assert_eq!(plan.next_version, Version::new(0, 6, 0));
        assert_eq!(plan.bump, BumpLevel::Minor);
    }

    #[test]
    fn plan_v0_fix_stays_patch() {
        let tag = TagInfo {
            name: "v0.5.0".into(),
            version: Version::new(0, 5, 0),
            sha: "c".repeat(40),
        };
        let s = make_strategy(
            vec![tag],
            vec![raw_commit("fix: bug fix")],
            Config::default(),
        );
        let plan = s.plan().unwrap();
        // Fix in v0 stays as patch
        assert_eq!(plan.next_version, Version::new(0, 5, 1));
        assert_eq!(plan.bump, BumpLevel::Patch);
    }

    // --- execute() tests ---

    #[test]
    fn execute_dry_run_no_side_effects() {
        let s = make_strategy(vec![], vec![raw_commit("feat: something")], test_config());
        let plan = s.plan().unwrap();
        s.execute(&plan, true).unwrap();

        assert!(s.git.created_tags.lock().unwrap().is_empty());
        assert!(s.git.pushed_tags.lock().unwrap().is_empty());
    }

    #[test]
    fn execute_creates_and_pushes_tag() {
        let s = make_strategy(vec![], vec![raw_commit("feat: something")], test_config());
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        assert_eq!(*s.git.created_tags.lock().unwrap(), vec!["v0.1.0"]);
        assert_eq!(*s.git.pushed_tags.lock().unwrap(), vec!["v0.1.0"]);
    }

    #[test]
    fn execute_calls_vcs_create_release() {
        let s = make_strategy(vec![], vec![raw_commit("feat: something")], test_config());
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        let releases = s.vcs.releases.lock().unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].0, "v0.1.0");
        assert!(!releases[0].1.is_empty());
    }

    #[test]
    fn execute_commits_changelog_before_tag() {
        let dir = tempfile::tempdir().unwrap();
        let changelog_path = dir.path().join("CHANGELOG.md");

        // Use the temp dir as the package path so auto-detection finds no version files.
        let config = Config {
            changelog: ChangelogConfig {
                file: Some(changelog_path.to_str().unwrap().to_string()),
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: dir.path().to_str().unwrap().to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        // Verify changelog was committed
        let committed = s.git.committed.lock().unwrap();
        assert_eq!(committed.len(), 1);
        assert_eq!(
            committed[0].0,
            vec![changelog_path.to_str().unwrap().to_string()]
        );
        assert!(committed[0].1.contains("chore(release): v0.1.0"));

        // Verify tag was created after commit
        assert_eq!(*s.git.created_tags.lock().unwrap(), vec!["v0.1.0"]);
    }

    #[test]
    fn execute_skips_existing_tag() {
        let s = make_strategy(vec![], vec![raw_commit("feat: something")], test_config());
        let plan = s.plan().unwrap();

        // Pre-populate the tag to simulate it already existing
        s.git
            .created_tags
            .lock()
            .unwrap()
            .push("v0.1.0".to_string());

        s.execute(&plan, false).unwrap();

        // Tag should not be created again (still only the one we pre-populated)
        assert_eq!(s.git.created_tags.lock().unwrap().len(), 1);
    }

    #[test]
    fn execute_skips_existing_release() {
        let s = make_strategy(vec![], vec![raw_commit("feat: something")], test_config());
        let plan = s.plan().unwrap();

        // Pre-populate a release to simulate it already existing
        s.vcs
            .releases
            .lock()
            .unwrap()
            .push(("v0.1.0".to_string(), "old notes".to_string()));

        s.execute(&plan, false).unwrap();

        // Should have updated in place without deleting
        let deleted = s.vcs.deleted_releases.lock().unwrap();
        assert!(deleted.is_empty(), "update should not delete");

        let releases = s.vcs.releases.lock().unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].0, "v0.1.0");
        assert_ne!(releases[0].1, "old notes");
    }

    #[test]
    fn execute_idempotent_rerun() {
        let s = make_strategy(vec![], vec![raw_commit("feat: something")], test_config());
        let plan = s.plan().unwrap();

        // First run
        s.execute(&plan, false).unwrap();

        // Second run should also succeed (idempotent)
        s.execute(&plan, false).unwrap();

        // Tag should only have been created once (second run skips because tag_exists)
        assert_eq!(s.git.created_tags.lock().unwrap().len(), 1);

        // Tag push should only happen once (second run skips because remote_tag_exists)
        assert_eq!(s.git.pushed_tags.lock().unwrap().len(), 1);

        // Push (commit) should happen twice (always safe)
        assert_eq!(*s.git.push_count.lock().unwrap(), 2);

        // Release should be updated in place on second run (no delete)
        let deleted = s.vcs.deleted_releases.lock().unwrap();
        assert!(deleted.is_empty(), "update should not delete");

        let releases = s.vcs.releases.lock().unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].0, "v0.1.0");
    }

    #[test]
    fn execute_bumps_version_files() {
        let dir = tempfile::tempdir().unwrap();
        let cargo_path = dir.path().join("Cargo.toml");
        std::fs::write(
            &cargo_path,
            "[package]\nname = \"test\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();

        let config = Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec![cargo_path.to_str().unwrap().to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        // Verify the file was bumped
        let contents = std::fs::read_to_string(&cargo_path).unwrap();
        assert!(contents.contains("version = \"0.1.0\""));

        // Verify it was staged alongside the commit
        let committed = s.git.committed.lock().unwrap();
        assert_eq!(committed.len(), 1);
        assert!(
            committed[0]
                .0
                .contains(&cargo_path.to_str().unwrap().to_string())
        );
    }

    #[test]
    fn execute_stages_changelog_and_version_files_together() {
        let dir = tempfile::tempdir().unwrap();
        let cargo_path = dir.path().join("Cargo.toml");
        std::fs::write(
            &cargo_path,
            "[package]\nname = \"test\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();

        let changelog_path = dir.path().join("CHANGELOG.md");

        let config = Config {
            changelog: ChangelogConfig {
                file: Some(changelog_path.to_str().unwrap().to_string()),
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec![cargo_path.to_str().unwrap().to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        // Both changelog and version file should be staged in a single commit
        let committed = s.git.committed.lock().unwrap();
        assert_eq!(committed.len(), 1);
        assert!(
            committed[0]
                .0
                .contains(&changelog_path.to_str().unwrap().to_string())
        );
        assert!(
            committed[0]
                .0
                .contains(&cargo_path.to_str().unwrap().to_string())
        );
    }

    // --- artifact upload tests ---

    #[test]
    fn execute_uploads_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.tar.gz"), "fake tarball").unwrap();
        std::fs::write(dir.path().join("app.zip"), "fake zip").unwrap();

        let config = Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec!["__sr_test_dummy_no_bump__".into()],
                artifacts: vec![
                    dir.path().join("*.tar.gz").to_str().unwrap().to_string(),
                    dir.path().join("*.zip").to_str().unwrap().to_string(),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        let uploaded = s.vcs.uploaded_assets.lock().unwrap();
        // UploadArtifacts call + UploadManifest call
        assert_eq!(uploaded.len(), 2);
        let artifact_call = uploaded
            .iter()
            .find(|(_tag, files)| files.iter().any(|f| f.ends_with("app.tar.gz")))
            .expect("expected an upload call containing user artifacts");
        assert_eq!(artifact_call.0, "v0.1.0");
        assert_eq!(artifact_call.1.len(), 2);
        assert!(artifact_call.1.iter().any(|f| f.ends_with("app.tar.gz")));
        assert!(artifact_call.1.iter().any(|f| f.ends_with("app.zip")));
    }

    #[test]
    fn execute_dry_run_shows_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.tar.gz"), "fake tarball").unwrap();

        let config = Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec!["__sr_test_dummy_no_bump__".into()],
                artifacts: vec![dir.path().join("*.tar.gz").to_str().unwrap().to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        s.execute(&plan, true).unwrap();

        // No uploads should happen during dry-run
        let uploaded = s.vcs.uploaded_assets.lock().unwrap();
        assert!(uploaded.is_empty());
    }

    #[test]
    fn execute_no_artifacts_skips_upload() {
        let s = make_strategy(vec![], vec![raw_commit("feat: something")], test_config());
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        // No user-declared artifacts → no user-artifact upload call. The
        // manifest stage still uploads sr-manifest.json.
        let uploaded = s.vcs.uploaded_assets.lock().unwrap();
        let user_uploads: Vec<_> = uploaded
            .iter()
            .filter(|(_tag, files)| {
                !files
                    .iter()
                    .all(|f| f.ends_with(crate::manifest::MANIFEST_ASSET_NAME))
            })
            .collect();
        assert!(
            user_uploads.is_empty(),
            "unexpected non-manifest uploads: {user_uploads:?}"
        );
    }

    #[test]
    fn resolve_globs_basic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join("b.txt"), "b").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let pattern = dir.path().join("*.txt").to_str().unwrap().to_string();
        let result = resolve_globs(&[pattern]).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|f: &String| f.ends_with("a.txt")));
        assert!(result.iter().any(|f: &String| f.ends_with("b.txt")));
    }

    #[test]
    fn resolve_globs_deduplicates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), "data").unwrap();

        let pattern = dir.path().join("*.txt").to_str().unwrap().to_string();
        // Same pattern twice should not produce duplicates
        let result = resolve_globs(&[pattern.clone(), pattern]).unwrap();
        assert_eq!(result.len(), 1);
    }

    // --- floating tags tests ---

    #[test]
    fn plan_floating_tag_when_enabled() {
        let tag = TagInfo {
            name: "v3.2.0".into(),
            version: Version::new(3, 2, 0),
            sha: "d".repeat(40),
        };
        let config = config_with_git(GitConfig {
            floating_tag: true,
            ..Default::default()
        });

        let s = make_strategy(vec![tag], vec![raw_commit("fix: patch")], config);
        let plan = s.plan().unwrap();
        assert_eq!(plan.next_version, Version::new(3, 2, 1));
        assert_eq!(plan.floating_tag_name.as_deref(), Some("v3"));
    }

    #[test]
    fn plan_no_floating_tag_when_disabled() {
        let s = make_strategy(
            vec![],
            vec![raw_commit("feat: something")],
            config_with_git(GitConfig {
                floating_tag: false,
                ..Default::default()
            }),
        );
        let plan = s.plan().unwrap();
        assert!(plan.floating_tag_name.is_none());
    }

    #[test]
    fn plan_floating_tag_custom_prefix() {
        let tag = TagInfo {
            name: "release-2.5.0".into(),
            version: Version::new(2, 5, 0),
            sha: "e".repeat(40),
        };
        let config = config_with_git(GitConfig {
            floating_tag: true,
            tag_prefix: "release-".into(),
            ..Default::default()
        });

        let s = make_strategy(vec![tag], vec![raw_commit("fix: patch")], config);
        let plan = s.plan().unwrap();
        assert_eq!(plan.floating_tag_name.as_deref(), Some("release-2"));
    }

    #[test]
    fn execute_floating_tags_force_create_and_push() {
        let config = config_with_git(GitConfig {
            floating_tag: true,
            ..Default::default()
        });

        let tag = TagInfo {
            name: "v1.2.3".into(),
            version: Version::new(1, 2, 3),
            sha: "f".repeat(40),
        };
        let s = make_strategy(vec![tag], vec![raw_commit("fix: a bug")], config);
        let plan = s.plan().unwrap();
        assert_eq!(plan.floating_tag_name.as_deref(), Some("v1"));

        s.execute(&plan, false).unwrap();

        assert_eq!(*s.git.force_created_tags.lock().unwrap(), vec!["v1"]);
        assert_eq!(*s.git.force_pushed_tags.lock().unwrap(), vec!["v1"]);
    }

    #[test]
    fn execute_no_floating_tags_when_disabled() {
        let s = make_strategy(
            vec![],
            vec![raw_commit("feat: something")],
            config_with_git(GitConfig {
                floating_tag: false,
                ..Default::default()
            }),
        );
        let plan = s.plan().unwrap();
        assert!(plan.floating_tag_name.is_none());

        s.execute(&plan, false).unwrap();

        assert!(s.git.force_created_tags.lock().unwrap().is_empty());
        assert!(s.git.force_pushed_tags.lock().unwrap().is_empty());
    }

    #[test]
    fn execute_floating_tags_dry_run_no_side_effects() {
        let config = config_with_git(GitConfig {
            floating_tag: true,
            ..Default::default()
        });

        let tag = TagInfo {
            name: "v2.0.0".into(),
            version: Version::new(2, 0, 0),
            sha: "a".repeat(40),
        };
        let s = make_strategy(vec![tag], vec![raw_commit("fix: something")], config);
        let plan = s.plan().unwrap();
        assert_eq!(plan.floating_tag_name.as_deref(), Some("v2"));

        s.execute(&plan, true).unwrap();

        assert!(s.git.force_created_tags.lock().unwrap().is_empty());
        assert!(s.git.force_pushed_tags.lock().unwrap().is_empty());
    }

    #[test]
    fn execute_floating_tags_idempotent() {
        let config = config_with_git(GitConfig {
            floating_tag: true,
            ..Default::default()
        });

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        assert_eq!(plan.floating_tag_name.as_deref(), Some("v0"));

        // Run twice
        s.execute(&plan, false).unwrap();
        s.execute(&plan, false).unwrap();

        // Force ops run every time (correct for floating tags)
        assert_eq!(s.git.force_created_tags.lock().unwrap().len(), 2);
        assert_eq!(s.git.force_pushed_tags.lock().unwrap().len(), 2);
    }

    // --- force mode tests ---

    #[test]
    fn force_rerelease_when_tag_at_head() {
        let tag = TagInfo {
            name: "v1.2.3".into(),
            version: Version::new(1, 2, 3),
            sha: "a".repeat(40),
        };
        let mut s = make_strategy(vec![tag], vec![], Config::default());
        // HEAD == tag SHA, and no new commits
        s.git.head = "a".repeat(40);
        s.force = true;

        let plan = s.plan().unwrap();
        assert_eq!(plan.next_version, Version::new(1, 2, 3));
        assert_eq!(plan.tag_name, "v1.2.3");
        assert!(plan.commits.is_empty());
        assert_eq!(plan.current_version, Some(Version::new(1, 2, 3)));
    }

    #[test]
    fn force_fails_when_tag_not_at_head() {
        let tag = TagInfo {
            name: "v1.2.3".into(),
            version: Version::new(1, 2, 3),
            sha: "a".repeat(40),
        };
        let mut s = make_strategy(vec![tag], vec![], Config::default());
        // HEAD != tag SHA
        s.git.head = "b".repeat(40);
        s.force = true;

        let err = s.plan().unwrap_err();
        assert!(matches!(err, ReleaseError::NoCommits { .. }));
    }

    // --- build hooks + artifact validation tests ---

    /// Build hooks receive SR_VERSION set to the bumped version.
    #[test]
    fn execute_runs_build_hook_with_version_env() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("saw_version.txt");
        let cmd = format!("echo \"$SR_VERSION\" > {}", marker.display());

        let config = Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec!["__sr_test_dummy_no_bump__".into()],
                hooks: Some(HooksConfig {
                    build: vec![cmd],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        let content = std::fs::read_to_string(&marker).unwrap();
        assert_eq!(content.trim(), "0.1.0");
    }

    /// Build hooks run AFTER version bump — the manifest on disk contains the
    /// new version when the build executes.
    #[test]
    fn execute_build_sees_bumped_version_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let cargo_path = dir.path().join("Cargo.toml");
        std::fs::write(
            &cargo_path,
            "[package]\nname = \"test\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();

        let marker = dir.path().join("observed_version.txt");
        // Build hook reads whatever version is currently in Cargo.toml.
        let cmd = format!(
            "grep '^version' {} > {}",
            cargo_path.display(),
            marker.display()
        );

        let config = Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec![cargo_path.to_str().unwrap().to_string()],
                hooks: Some(HooksConfig {
                    build: vec![cmd],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        let content = std::fs::read_to_string(&marker).unwrap();
        assert!(
            content.contains("0.1.0"),
            "build should see bumped version on disk, got: {content}"
        );
    }

    /// A failing build aborts before tag/commit/release — preserves the invariant
    /// that a tag on remote implies a successful build.
    #[test]
    fn execute_build_failure_leaves_no_tag_or_commit() {
        let config = Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec!["__sr_test_dummy_no_bump__".into()],
                hooks: Some(HooksConfig {
                    build: vec!["false".into()],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        let err = s.execute(&plan, false).unwrap_err();
        assert!(matches!(err, ReleaseError::Hook(_)), "got {err:?}");

        assert!(s.git.created_tags.lock().unwrap().is_empty());
        assert!(s.git.pushed_tags.lock().unwrap().is_empty());
        assert!(s.git.committed.lock().unwrap().is_empty());
        assert!(s.vcs.releases.lock().unwrap().is_empty());
    }

    /// When `hooks.build` is set, every declared artifact glob must match ≥1 file
    /// or the pipeline aborts before tagging.
    #[test]
    fn execute_validation_fails_when_declared_artifact_missing() {
        let config = Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec!["__sr_test_dummy_no_bump__".into()],
                artifacts: vec!["/definitely/not/here/*.tar.gz".into()],
                hooks: Some(HooksConfig {
                    build: vec!["true".into()],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        let err = s.execute(&plan, false).unwrap_err();

        match err {
            ReleaseError::Vcs(ref msg) => {
                assert!(
                    msg.contains("matched no files"),
                    "expected validation error, got: {msg}"
                );
            }
            other => panic!("expected Vcs error, got {other:?}"),
        }

        assert!(s.git.created_tags.lock().unwrap().is_empty());
        assert!(s.git.pushed_tags.lock().unwrap().is_empty());
        assert!(s.vcs.releases.lock().unwrap().is_empty());
    }

    /// Validation passes when every declared glob resolves to ≥1 file.
    #[test]
    fn execute_validation_passes_when_all_artifacts_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.tar.gz"), "fake").unwrap();

        let config = Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec!["__sr_test_dummy_no_bump__".into()],
                artifacts: vec![dir.path().join("*.tar.gz").to_str().unwrap().to_string()],
                hooks: Some(HooksConfig {
                    build: vec!["true".into()],
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        assert_eq!(*s.git.created_tags.lock().unwrap(), vec!["v0.1.0"]);
    }

    /// No `hooks.build` means no contract — missing declared artifacts do NOT
    /// fail the pipeline. Preserves today's behavior for users building
    /// outside sr.
    #[test]
    fn execute_validation_skipped_without_build_hooks() {
        let config = Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec!["__sr_test_dummy_no_bump__".into()],
                artifacts: vec!["/still/not/here/*.tar.gz".into()],
                // No hooks.build — today's behavior.
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        assert_eq!(*s.git.created_tags.lock().unwrap(), vec!["v0.1.0"]);
    }

    // --- manifest + reconciliation tests ---

    /// sr-manifest.json is uploaded on every successful release.
    #[test]
    fn execute_uploads_manifest_as_final_asset() {
        let s = make_strategy(vec![], vec![raw_commit("feat: something")], test_config());
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        let assets = s.vcs.list_assets("v0.1.0").unwrap();
        assert!(
            assets.contains(&crate::manifest::MANIFEST_ASSET_NAME.to_string()),
            "manifest should be uploaded; got {assets:?}"
        );
    }

    /// Manifest records the tag, the commit sha at HEAD, and (when declared)
    /// the resolved artifact basenames.
    #[test]
    fn execute_manifest_contains_tag_and_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.tar.gz"), "fake").unwrap();

        let config = Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec!["__sr_test_dummy_no_bump__".into()],
                artifacts: vec![dir.path().join("*.tar.gz").to_str().unwrap().to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();
        s.execute(&plan, false).unwrap();

        let manifest_bytes = s
            .vcs
            .fetch_asset("v0.1.0", crate::manifest::MANIFEST_ASSET_NAME)
            .unwrap()
            .expect("manifest should be present");
        let manifest: crate::manifest::Manifest = serde_json::from_slice(&manifest_bytes).unwrap();

        assert_eq!(manifest.tag, "v0.1.0");
        assert!(manifest.artifacts.iter().any(|a| a == "app.tar.gz"));
        assert!(!manifest.commit_sha.is_empty());
        assert!(!manifest.sr_version.is_empty());
    }

    /// Second run against a release that already has all declared artifacts
    /// uploaded is a no-op for upload — no duplicate asset errors.
    #[test]
    fn execute_skips_already_uploaded_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.tar.gz"), "fake").unwrap();

        let config = Config {
            changelog: ChangelogConfig {
                file: None,
                ..Default::default()
            },
            packages: vec![PackageConfig {
                path: ".".into(),
                version_files: vec!["__sr_test_dummy_no_bump__".into()],
                artifacts: vec![dir.path().join("*.tar.gz").to_str().unwrap().to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let s = make_strategy(vec![], vec![raw_commit("feat: something")], config);
        let plan = s.plan().unwrap();

        // First run uploads the artifact + manifest.
        s.execute(&plan, false).unwrap();
        let uploads_after_first = s.vcs.uploaded_assets.lock().unwrap().len();

        // Second run should skip both — no new uploads.
        s.execute(&plan, false).unwrap();
        let uploads_after_second = s.vcs.uploaded_assets.lock().unwrap().len();

        assert_eq!(
            uploads_after_first, uploads_after_second,
            "idempotent re-run should not re-upload existing assets"
        );
    }

    /// plan() refuses to cut a new version when the latest remote tag has a
    /// manifest declaring artifacts that aren't present on the release.
    #[test]
    fn plan_blocks_when_previous_release_incomplete() {
        let prev_tag = TagInfo {
            name: "v1.0.0".into(),
            version: Version::new(1, 0, 0),
            sha: "a".repeat(40),
        };
        let s = make_strategy(
            vec![prev_tag],
            vec![raw_commit("feat: new thing")],
            test_config(),
        );

        // Seed an incomplete manifest on the remote: declares an asset that
        // isn't in list_assets.
        let incomplete = crate::manifest::Manifest {
            sr_version: "7.1.0".into(),
            tag: "v1.0.0".into(),
            commit_sha: "a".repeat(40),
            artifacts: vec!["missing-binary.tar.gz".into()],
            completed_at: "2026-04-18T00:00:00Z".into(),
        };
        s.vcs.seed_asset(
            "v1.0.0",
            crate::manifest::MANIFEST_ASSET_NAME,
            serde_json::to_vec(&incomplete).unwrap(),
        );

        let err = s.plan().unwrap_err();
        match err {
            ReleaseError::Vcs(ref msg) => {
                assert!(
                    msg.contains("incomplete") && msg.contains("missing-binary.tar.gz"),
                    "unexpected error: {msg}"
                );
            }
            other => panic!("expected Vcs error, got {other:?}"),
        }
    }

    /// Complete manifest on the previous release → plan proceeds.
    #[test]
    fn plan_passes_when_previous_release_complete() {
        let prev_tag = TagInfo {
            name: "v1.0.0".into(),
            version: Version::new(1, 0, 0),
            sha: "a".repeat(40),
        };
        let s = make_strategy(
            vec![prev_tag],
            vec![raw_commit("feat: next thing")],
            test_config(),
        );

        let complete = crate::manifest::Manifest {
            sr_version: "7.1.0".into(),
            tag: "v1.0.0".into(),
            commit_sha: "a".repeat(40),
            artifacts: vec!["ok.tar.gz".into()],
            completed_at: "2026-04-18T00:00:00Z".into(),
        };
        s.vcs.seed_asset(
            "v1.0.0",
            crate::manifest::MANIFEST_ASSET_NAME,
            serde_json::to_vec(&complete).unwrap(),
        );
        s.vcs.seed_asset("v1.0.0", "ok.tar.gz", b"bin".to_vec());

        let plan = s.plan().unwrap();
        assert_eq!(plan.next_version, Version::new(1, 1, 0));
    }

    /// No manifest on the previous release (legacy/pre-sr) → plan proceeds
    /// without blocking. We can't distinguish legacy from aborted remotely.
    #[test]
    fn plan_passes_when_previous_release_has_no_manifest() {
        let prev_tag = TagInfo {
            name: "v1.0.0".into(),
            version: Version::new(1, 0, 0),
            sha: "a".repeat(40),
        };
        let s = make_strategy(
            vec![prev_tag],
            vec![raw_commit("feat: legacy compat")],
            test_config(),
        );
        // Intentionally no seed — fetch_asset returns None → Unknown status.

        let plan = s.plan().unwrap();
        assert_eq!(plan.next_version, Version::new(1, 1, 0));
    }

    /// --force bypasses the reconciliation check so a broken tag can be healed
    /// by re-running the pipeline against it.
    #[test]
    fn plan_with_force_bypasses_reconciliation_block() {
        let prev_tag = TagInfo {
            name: "v1.0.0".into(),
            version: Version::new(1, 0, 0),
            sha: "a".repeat(40),
        };
        let mut s = make_strategy(vec![prev_tag], vec![], test_config());
        s.git.head = "a".repeat(40); // HEAD at tag → force-rerelease path
        s.force = true;

        let incomplete = crate::manifest::Manifest {
            sr_version: "7.1.0".into(),
            tag: "v1.0.0".into(),
            commit_sha: "a".repeat(40),
            artifacts: vec!["missing.tar.gz".into()],
            completed_at: "2026-04-18T00:00:00Z".into(),
        };
        s.vcs.seed_asset(
            "v1.0.0",
            crate::manifest::MANIFEST_ASSET_NAME,
            serde_json::to_vec(&incomplete).unwrap(),
        );

        // Should NOT error — --force bypasses the reconciliation check.
        let plan = s.plan().unwrap();
        assert_eq!(plan.next_version, Version::new(1, 0, 0));
    }
}
