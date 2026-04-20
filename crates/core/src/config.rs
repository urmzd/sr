use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::commit::CommitType;
use crate::error::ReleaseError;
use crate::version::BumpLevel;
use crate::version_files::detect_version_files;

/// Preferred config file name.
pub const DEFAULT_CONFIG_FILE: &str = "sr.yaml";

/// Config file candidates, checked in priority order.
pub const CONFIG_CANDIDATES: &[&str] = &["sr.yaml", "sr.yml"];

// ---------------------------------------------------------------------------
// Top-level config
// ---------------------------------------------------------------------------

/// Root configuration. Six top-level concerns:
/// - `git` — tag prefix, floating tags, signing
/// - `commit` — type→bump classification
/// - `changelog` — file, template, groups
/// - `channels` — branch→release mapping
/// - `vcs` — provider-specific config
/// - `packages` — version files, artifacts, publish targets
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub git: GitConfig,
    pub commit: CommitConfig,
    pub changelog: ChangelogConfig,
    pub channels: ChannelsConfig,
    pub vcs: VcsConfig,
    #[serde(default = "default_packages")]
    pub packages: Vec<PackageConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            git: GitConfig::default(),
            commit: CommitConfig::default(),
            changelog: ChangelogConfig::default(),
            channels: ChannelsConfig::default(),
            vcs: VcsConfig::default(),
            packages: default_packages(),
        }
    }
}

fn default_packages() -> Vec<PackageConfig> {
    vec![PackageConfig {
        path: ".".into(),
        ..Default::default()
    }]
}

// ---------------------------------------------------------------------------
// Git config
// ---------------------------------------------------------------------------

/// Git-level settings — tags, signing, identity, commit filtering.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GitConfig {
    /// Prefix for git tags (e.g. "v" → "v1.2.0").
    pub tag_prefix: String,
    /// Create floating major version tags (e.g. "v3" → latest v3.x.x).
    pub floating_tag: bool,
    /// Sign tags with GPG/SSH.
    pub sign_tags: bool,
    /// Prevent breaking changes from bumping 0.x.y to 1.0.0.
    /// When true, major bumps at v0 are downshifted to minor.
    pub v0_protection: bool,
    /// Override the git identity used for release commits and tags.
    /// When unset, sr leaves it to the repo's git config / environment.
    pub user: GitUserConfig,
    /// Substrings that, when present in a commit message, exclude that
    /// commit from release planning and changelog. Matched against the full
    /// commit message. `chore(release):` is always filtered regardless.
    pub skip_patterns: Vec<String>,
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            tag_prefix: "v".into(),
            floating_tag: true,
            sign_tags: false,
            v0_protection: true,
            user: GitUserConfig::default(),
            skip_patterns: default_skip_patterns(),
        }
    }
}

/// Git author/committer identity for release operations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GitUserConfig {
    /// Author/committer name. None = inherit from git config / env.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Author/committer email. None = inherit from git config / env.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// Default skip tokens. `[skip release]` and `[skip sr]` are recognized out
/// of the box so users can opt a commit out of the release without touching
/// config.
pub fn default_skip_patterns() -> Vec<String> {
    vec!["[skip release]".into(), "[skip sr]".into()]
}

// ---------------------------------------------------------------------------
// Commit config
// ---------------------------------------------------------------------------

/// How commits are classified by semver bump level.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CommitConfig {
    /// Commit types grouped by bump level.
    pub types: CommitTypesConfig,
}

/// Commit type names grouped by the semver bump level they trigger.
/// Breaking changes always bump major regardless of configured level.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CommitTypesConfig {
    /// Types that trigger a minor version bump.
    pub minor: Vec<String>,
    /// Types that trigger a patch version bump.
    pub patch: Vec<String>,
    /// Types that do not trigger a release on their own.
    pub none: Vec<String>,
}

impl Default for CommitTypesConfig {
    fn default() -> Self {
        Self {
            minor: vec!["feat".into()],
            patch: vec!["fix".into(), "perf".into(), "refactor".into()],
            none: vec![
                "docs".into(),
                "revert".into(),
                "chore".into(),
                "ci".into(),
                "test".into(),
                "build".into(),
                "style".into(),
            ],
        }
    }
}

impl CommitTypesConfig {
    /// All type names across all bump levels.
    pub fn all_type_names(&self) -> Vec<&str> {
        self.minor
            .iter()
            .chain(self.patch.iter())
            .chain(self.none.iter())
            .map(|s| s.as_str())
            .collect()
    }

    /// Convert to internal `Vec<CommitType>` representation.
    pub fn into_commit_types(&self) -> Vec<CommitType> {
        let mut types = Vec::new();
        for name in &self.minor {
            types.push(CommitType {
                name: name.clone(),
                bump: Some(BumpLevel::Minor),
            });
        }
        for name in &self.patch {
            types.push(CommitType {
                name: name.clone(),
                bump: Some(BumpLevel::Patch),
            });
        }
        for name in &self.none {
            types.push(CommitType {
                name: name.clone(),
                bump: None,
            });
        }
        types
    }
}

// ---------------------------------------------------------------------------
// Changelog config
// ---------------------------------------------------------------------------

/// Changelog generation — file, template, and commit grouping.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChangelogConfig {
    /// Path to the changelog file. None = skip changelog generation.
    pub file: Option<String>,
    /// Jinja template — path to file or inline string. None = built-in default.
    pub template: Option<String>,
    /// Ordered groups for organizing commits in the changelog.
    pub groups: Vec<ChangelogGroup>,
}

impl Default for ChangelogConfig {
    fn default() -> Self {
        Self {
            file: Some("CHANGELOG.md".into()),
            template: None,
            groups: default_changelog_groups(),
        }
    }
}

/// A named group of commit types for changelog rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangelogGroup {
    /// Machine-readable name (e.g. "breaking", "features").
    pub name: String,
    /// Commit types included in this group (e.g. ["feat"]).
    pub content: Vec<String>,
}

pub fn default_changelog_groups() -> Vec<ChangelogGroup> {
    vec![
        ChangelogGroup {
            name: "breaking".into(),
            content: vec!["breaking".into()],
        },
        ChangelogGroup {
            name: "features".into(),
            content: vec!["feat".into()],
        },
        ChangelogGroup {
            name: "bug-fixes".into(),
            content: vec!["fix".into()],
        },
        ChangelogGroup {
            name: "performance".into(),
            content: vec!["perf".into()],
        },
        ChangelogGroup {
            name: "refactoring".into(),
            content: vec!["refactor".into()],
        },
        ChangelogGroup {
            name: "misc".into(),
            content: vec![
                "docs".into(),
                "revert".into(),
                "chore".into(),
                "ci".into(),
                "test".into(),
                "build".into(),
                "style".into(),
            ],
        },
    ]
}

// ---------------------------------------------------------------------------
// Channels config
// ---------------------------------------------------------------------------

/// Release channels for trunk-based promotion.
/// All channels release from the same branch — channels control the release
/// strategy (stable vs prerelease vs draft), not the branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelsConfig {
    /// Default channel when no --channel flag given.
    pub default: String,
    /// The trunk branch that triggers releases.
    pub branch: String,
    /// Channel definitions.
    pub content: Vec<ChannelConfig>,
}

impl Default for ChannelsConfig {
    fn default() -> Self {
        Self {
            default: "stable".into(),
            branch: "main".into(),
            content: vec![ChannelConfig {
                name: "stable".into(),
                prerelease: None,
                draft: false,
            }],
        }
    }
}

/// A named release channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConfig {
    /// Channel name (e.g. "stable", "rc", "canary").
    pub name: String,
    /// Pre-release identifier (e.g. "rc", "canary"). None = stable release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerelease: Option<String>,
    /// Create GitHub release as a draft.
    #[serde(default)]
    pub draft: bool,
}

// ---------------------------------------------------------------------------
// VCS config
// ---------------------------------------------------------------------------

/// VCS provider-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VcsConfig {
    pub github: GitHubConfig,
}

/// GitHub-specific release settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct GitHubConfig {
    /// Minijinja template for the GitHub release name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_name_template: Option<String>,
}

// ---------------------------------------------------------------------------
// Package config
// ---------------------------------------------------------------------------

/// A releasable package — version files, artifacts, build/publish targets.
///
/// All packages share one global release tag (`git.tag_prefix` + semver) and
/// one version line. Each package's `version_files` are bumped to that same
/// global version on every release; `packages[]` controls *where to write
/// versions and how to publish*, not *how to version*.
///
/// sr does not run user shell commands. Artifact builds happen in CI
/// between `sr prepare` and `sr release`; sr uploads whatever paths in
/// `artifacts` point to at tag time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PackageConfig {
    /// Directory path relative to repo root. Used for organizing changelog
    /// sections and as the working directory for typed publishers.
    pub path: String,
    /// Manifest files to bump with the global release version. Literal
    /// paths only — no glob expansion.
    pub version_files: Vec<String>,
    /// Fail on unsupported version file formats.
    pub version_files_strict: bool,
    /// Additional files to stage in the release commit. Literal paths only.
    pub stage_files: Vec<String>,
    /// Artifact files to upload as release assets. Literal paths only —
    /// every entry must exist on disk before the tag is created.
    pub artifacts: Vec<String>,
    /// Changelog config override for this package.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changelog: Option<ChangelogConfig>,
    /// Per-package publish target (invoked during `sr release`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish: Option<PublishConfig>,
}

impl Default for PackageConfig {
    fn default() -> Self {
        Self {
            path: ".".into(),
            version_files: vec![],
            version_files_strict: false,
            stage_files: vec![],
            artifacts: vec![],
            changelog: None,
            publish: None,
        }
    }
}

/// Per-package publish configuration. Typed enum — the user picks a
/// known publisher (`cargo`, `npm`, `docker`, `pypi`, `go`) and sr handles
/// both the "is it already published?" check (registry API call) and the
/// actual publish command (shelled out). The `custom` variant is the escape
/// hatch for arbitrary shell commands + a user-supplied state check.
///
/// Deserializes as an internally-tagged enum: `{ type: <publisher>, ... }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PublishConfig {
    /// `cargo publish` to crates.io (or a custom registry).
    Cargo {
        /// Feature flags forwarded to `cargo publish --features ...`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        features: Vec<String>,
        /// Cargo registry name (defined in ~/.cargo/config.toml). None =
        /// default registry (crates.io).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        registry: Option<String>,
        /// When true, publish every `[workspace].members` crate, not just
        /// the one at `path`. Check aggregates across members (completed
        /// iff every member is already on the registry).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        workspace: bool,
    },
    /// `npm publish` to registry.npmjs.org (or a custom registry).
    /// Auto-detects pnpm / yarn / npm from lockfiles at `path`.
    Npm {
        /// Registry URL. None = default (https://registry.npmjs.org/).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        registry: Option<String>,
        /// Access level for scoped packages: "public" or "restricted".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access: Option<String>,
        /// When true, publish every workspace member (from `package.json`
        /// `workspaces` or `pnpm-workspace.yaml`). Uses the tool's native
        /// recursive publish where available.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        workspace: bool,
    },
    /// Push a container image to a registry using `docker buildx build --push`.
    Docker {
        /// Fully-qualified image name (e.g. `ghcr.io/owner/repo`).
        image: String,
        /// Target platforms for buildx (e.g. `["linux/amd64", "linux/arm64"]`).
        /// When empty, buildx picks the default.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        platforms: Vec<String>,
        /// Path to Dockerfile relative to the package dir. Default: `Dockerfile`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dockerfile: Option<String>,
    },
    /// Publish to PyPI via `twine upload` or `uv publish` (auto-detected).
    ///
    /// Artifacts are resolved per-member from `<package_path>/<dist_dir>` by
    /// filename prefix (PEP 625 stem + version) — matching `uv build --all`'s
    /// workspace-root dist layout rather than assuming per-member dist dirs.
    Pypi {
        /// Repository name (matches `[tool.twine.repository]` or env).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repository: Option<String>,
        /// When true, publish every uv workspace member
        /// (`[tool.uv.workspace].members`). Check aggregates across members.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        workspace: bool,
        /// Directory (relative to the package path) where built wheels/sdists
        /// live. Defaults to `dist` — matches `uv build --all` output.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dist_dir: Option<String>,
    },
    /// Go modules publish by git-tag; sr already cuts the tag, so this is
    /// effectively a noop documenting the package's presence in the manifest.
    Go,
    /// Arbitrary publish command with a user-supplied state check.
    /// The only place sr shells out for a user-provided command; limited to
    /// registries that don't have a built-in publisher.
    Custom {
        /// Shell command that performs the publish.
        command: String,
        /// Shell command that returns exit 0 iff the package is already
        /// published at the current version. Optional — when absent, the
        /// publisher always runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        check: Option<String>,
        /// Working directory. Defaults to the package path.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Config methods
// ---------------------------------------------------------------------------

impl Config {
    /// Find the first config file that exists in the given directory.
    pub fn find_config(dir: &Path) -> Option<std::path::PathBuf> {
        for &candidate in CONFIG_CANDIDATES {
            let path = dir.join(candidate);
            if path.exists() {
                return Some(path);
            }
        }
        None
    }

    /// Load config from a YAML file. Falls back to defaults if the file doesn't exist.
    pub fn load(path: &Path) -> Result<Self, ReleaseError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents =
            std::fs::read_to_string(path).map_err(|e| ReleaseError::Config(e.to_string()))?;
        let config: Self =
            serde_yaml_ng::from_str(&contents).map_err(|e| ReleaseError::Config(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Validate config consistency.
    fn validate(&self) -> Result<(), ReleaseError> {
        // Check for duplicate type names across bump levels.
        let mut seen = std::collections::HashSet::new();
        for name in self.commit.types.all_type_names() {
            if !seen.insert(name) {
                return Err(ReleaseError::Config(format!(
                    "duplicate commit type: {name}"
                )));
            }
        }

        // Need at least one type with a bump level.
        if self.commit.types.minor.is_empty() && self.commit.types.patch.is_empty() {
            return Err(ReleaseError::Config(
                "commit.types must have at least one minor or patch type".into(),
            ));
        }

        // Check for duplicate channel names.
        let mut channel_names = std::collections::HashSet::new();
        for ch in &self.channels.content {
            if !channel_names.insert(&ch.name) {
                return Err(ReleaseError::Config(format!(
                    "duplicate channel name: {}",
                    ch.name
                )));
            }
        }

        Ok(())
    }

    /// Resolve a named release channel, returning the channel config.
    pub fn resolve_channel(&self, name: &str) -> Result<&ChannelConfig, ReleaseError> {
        self.channels
            .content
            .iter()
            .find(|ch| ch.name == name)
            .ok_or_else(|| {
                let available: Vec<&str> = self
                    .channels
                    .content
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect();
                ReleaseError::Config(format!(
                    "channel '{name}' not found. Available: {}",
                    if available.is_empty() {
                        "(none)".to_string()
                    } else {
                        available.join(", ")
                    }
                ))
            })
    }

    /// Resolve the default channel.
    pub fn default_channel(&self) -> Result<&ChannelConfig, ReleaseError> {
        self.resolve_channel(&self.channels.default)
    }

    /// Find a package by path.
    pub fn find_package(&self, path: &str) -> Result<&PackageConfig, ReleaseError> {
        self.packages
            .iter()
            .find(|p| p.path == path)
            .ok_or_else(|| {
                let available: Vec<&str> = self.packages.iter().map(|p| p.path.as_str()).collect();
                ReleaseError::Config(format!(
                    "package '{path}' not found. Available: {}",
                    if available.is_empty() {
                        "(none)".to_string()
                    } else {
                        available.join(", ")
                    }
                ))
            })
    }

    /// Find a package by name (last component of path).
    pub fn find_package_by_name(&self, name: &str) -> Result<&PackageConfig, ReleaseError> {
        self.packages
            .iter()
            .find(|p| p.path.rsplit('/').next().unwrap_or(&p.path) == name)
            .ok_or_else(|| {
                let available: Vec<&str> = self
                    .packages
                    .iter()
                    .map(|p| p.path.rsplit('/').next().unwrap_or(&p.path))
                    .collect();
                ReleaseError::Config(format!(
                    "package '{name}' not found. Available: {}",
                    if available.is_empty() {
                        "(none)".to_string()
                    } else {
                        available.join(", ")
                    }
                ))
            })
    }

    /// Resolve effective changelog config for a package.
    pub fn changelog_for<'a>(&'a self, pkg: &'a PackageConfig) -> &'a ChangelogConfig {
        pkg.changelog.as_ref().unwrap_or(&self.changelog)
    }

    /// Resolve effective version files for a package, with auto-detection.
    pub fn version_files_for(&self, pkg: &PackageConfig) -> Vec<String> {
        if !pkg.version_files.is_empty() {
            return pkg.version_files.clone();
        }
        let detected = detect_version_files(Path::new(&pkg.path));
        if pkg.path == "." {
            detected
        } else {
            detected
                .into_iter()
                .map(|f| format!("{}/{f}", pkg.path))
                .collect()
        }
    }

    /// Collect all artifacts glob patterns from all packages.
    pub fn all_artifacts(&self) -> Vec<String> {
        self.packages
            .iter()
            .flat_map(|p| p.artifacts.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Template generation
// ---------------------------------------------------------------------------

pub fn default_config_template(version_files: &[String]) -> String {
    let vf = if version_files.is_empty() {
        "    version_files: []\n".to_string()
    } else {
        let mut s = "    version_files:\n".to_string();
        for f in version_files {
            s.push_str(&format!("      - {f}\n"));
        }
        s
    };

    format!(
        r#"# sr configuration
# Full reference: https://github.com/urmzd/sr#configuration

git:
  tag_prefix: "v"
  floating_tag: true
  sign_tags: false
  v0_protection: true
  # user:
  #   name: "sr-releaser[bot]"
  #   email: "sr-releaser[bot]@users.noreply.github.com"
  # Commits whose message contains any of these substrings are excluded from
  # the release plan and changelog. chore(release): is always filtered.
  skip_patterns:
    - "[skip release]"
    - "[skip sr]"

commit:
  types:
    minor:
      - feat
    patch:
      - fix
      - perf
      - refactor
    none:
      - docs
      - revert
      - chore
      - ci
      - test
      - build
      - style

changelog:
  file: CHANGELOG.md
  # template: changelog.md.j2
  groups:
    - name: breaking
      content:
        - breaking
    - name: features
      content:
        - feat
    - name: bug-fixes
      content:
        - fix
    - name: performance
      content:
        - perf
    - name: misc
      content:
        - chore
        - ci
        - test
        - build
        - style

channels:
  default: stable
  branch: main
  content:
    - name: stable
  # - name: rc
  #   prerelease: rc
  #   draft: true
  # - name: canary
  #   branch: develop
  #   prerelease: canary

# vcs:
#   github:
#     release_name_template: "{{{{ tag_name }}}}"

# Repo-wide lifecycle hooks. Run once per release.
# hooks:
#   # Runs before any mutation: tests, lints. May abort the release.
#   pre_release:
#     - cargo test
#   # Runs after tag + GitHub release.
#   post_release:
#     - echo "released $SR_VERSION"

packages:
  - path: .
{vf}    # version_files_strict: false
    # stage_files: []
    # artifacts: []
    # # Build commands produce this package's declared `artifacts`.
    # # Runs after version bump, before commit.
    # build:
    #   - cargo build --release
    # # Per-package publish target for `sr publish`. Idempotent.
    # publish:
    #   command: cargo publish
"#
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let config = Config::default();
        assert_eq!(config.git.tag_prefix, "v");
        assert!(config.git.floating_tag);
        assert!(!config.git.sign_tags);
        assert_eq!(config.commit.types.minor, vec!["feat"]);
        assert!(config.commit.types.patch.contains(&"fix".to_string()));
        assert!(config.commit.types.none.contains(&"chore".to_string()));
        assert_eq!(config.changelog.file.as_deref(), Some("CHANGELOG.md"));
        assert!(!config.changelog.groups.is_empty());
        assert_eq!(config.channels.default, "stable");
        assert_eq!(config.channels.content.len(), 1);
        assert_eq!(config.channels.content[0].name, "stable");
        assert_eq!(config.channels.branch, "main");
        assert_eq!(config.packages.len(), 1);
        assert_eq!(config.packages[0].path, ".");
    }

    #[test]
    fn load_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.yml");
        let config = Config::load(&path).unwrap();
        assert_eq!(config.git.tag_prefix, "v");
    }

    #[test]
    fn load_partial_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yml");
        std::fs::write(&path, "git:\n  tag_prefix: rel-\n").unwrap();

        let config = Config::load(&path).unwrap();
        assert_eq!(config.git.tag_prefix, "rel-");
        assert_eq!(config.channels.default, "stable");
    }

    #[test]
    fn load_yaml_with_packages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yml");
        std::fs::write(
            &path,
            "packages:\n  - path: crates/core\n    version_files:\n      - crates/core/Cargo.toml\n",
        )
        .unwrap();

        let config = Config::load(&path).unwrap();
        assert_eq!(config.packages.len(), 1);
        assert_eq!(config.packages[0].path, "crates/core");
    }

    #[test]
    fn commit_types_conversion() {
        let types = CommitTypesConfig::default();
        let commit_types = types.into_commit_types();
        let feat = commit_types.iter().find(|t| t.name == "feat").unwrap();
        assert_eq!(feat.bump, Some(BumpLevel::Minor));
        let fix = commit_types.iter().find(|t| t.name == "fix").unwrap();
        assert_eq!(fix.bump, Some(BumpLevel::Patch));
        let chore = commit_types.iter().find(|t| t.name == "chore").unwrap();
        assert_eq!(chore.bump, None);
    }

    #[test]
    fn all_type_names() {
        let types = CommitTypesConfig::default();
        let names = types.all_type_names();
        assert!(names.contains(&"feat"));
        assert!(names.contains(&"fix"));
        assert!(names.contains(&"chore"));
    }

    #[test]
    fn resolve_channel() {
        let config = Config::default();
        let channel = config.resolve_channel("stable").unwrap();
        assert!(channel.prerelease.is_none());
    }

    #[test]
    fn resolve_channel_not_found() {
        let config = Config::default();
        assert!(config.resolve_channel("missing").is_err());
    }

    #[test]
    fn validate_duplicate_types() {
        let config = Config {
            commit: CommitConfig {
                types: CommitTypesConfig {
                    minor: vec!["feat".into()],
                    patch: vec!["feat".into()],
                    none: vec![],
                },
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_no_bump_types() {
        let config = Config {
            commit: CommitConfig {
                types: CommitTypesConfig {
                    minor: vec![],
                    patch: vec![],
                    none: vec!["chore".into()],
                },
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_duplicate_channels() {
        let config = Config {
            channels: ChannelsConfig {
                default: "stable".into(),
                branch: "main".into(),
                content: vec![
                    ChannelConfig {
                        name: "stable".into(),
                        prerelease: None,
                        draft: false,
                    },
                    ChannelConfig {
                        name: "stable".into(),
                        prerelease: None,
                        draft: false,
                    },
                ],
            },
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn default_template_parses() {
        let template = default_config_template(&[]);
        let config: Config = serde_yaml_ng::from_str(&template).unwrap();
        assert_eq!(config.git.tag_prefix, "v");
        assert!(config.git.floating_tag);
        assert_eq!(config.channels.default, "stable");
        assert!(
            config
                .git
                .skip_patterns
                .iter()
                .any(|p| p == "[skip release]")
        );
    }

    #[test]
    fn default_skip_patterns_present() {
        let config = Config::default();
        assert_eq!(
            config.git.skip_patterns,
            vec!["[skip release]".to_string(), "[skip sr]".to_string()]
        );
    }

    #[test]
    fn git_user_defaults_to_none() {
        let config = Config::default();
        assert!(config.git.user.name.is_none());
        assert!(config.git.user.email.is_none());
    }

    #[test]
    fn git_user_loads_from_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yml");
        std::fs::write(
            &path,
            "git:\n  user:\n    name: \"Bot\"\n    email: \"bot@example.com\"\n",
        )
        .unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(config.git.user.name.as_deref(), Some("Bot"));
        assert_eq!(config.git.user.email.as_deref(), Some("bot@example.com"));
    }

    #[test]
    fn default_template_with_version_files() {
        let template = default_config_template(&["Cargo.toml".into(), "package.json".into()]);
        let config: Config = serde_yaml_ng::from_str(&template).unwrap();
        assert_eq!(
            config.packages[0].version_files,
            vec!["Cargo.toml", "package.json"]
        );
    }

    #[test]
    fn find_package_by_name_works() {
        let config = Config {
            packages: vec![
                PackageConfig {
                    path: "crates/core".into(),
                    ..Default::default()
                },
                PackageConfig {
                    path: "crates/cli".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let pkg = config.find_package_by_name("core").unwrap();
        assert_eq!(pkg.path, "crates/core");
    }

    #[test]
    fn collect_all_artifacts() {
        let config = Config {
            packages: vec![
                PackageConfig {
                    path: "crates/core".into(),
                    artifacts: vec!["core-*".into()],
                    ..Default::default()
                },
                PackageConfig {
                    path: "crates/cli".into(),
                    artifacts: vec!["cli-*".into()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let artifacts = config.all_artifacts();
        assert_eq!(artifacts, vec!["core-*", "cli-*"]);
    }
}
