//! Structured diff between desired and actual release state.
//!
//! `sr plan` computes a [`ReleaseDiff`] and renders it for the user.
//! Every resource the reconciler would touch appears as one [`ResourceDiff`]
//! entry showing its current observable state, its desired state, and the
//! `Action` that would converge the two.
//!
//! This is Terraform-style: the diff *describes* what would happen, not
//! what commit-message was parsed. The changelog is still computed (it's
//! the body of the Release resource) but the user-facing plan output is
//! resource-by-resource.

use std::path::Path;

use serde::Serialize;

use crate::config::{Config, PackageConfig, PublishConfig};
use crate::error::ReleaseError;
use crate::git::GitRepository;
use crate::publishers::{PublishCtx, PublishState, publisher_for};
use crate::release::{ReleasePlan, VcsProvider, partition_paths};
use crate::workspaces::{discover_cargo_members, discover_npm_members, discover_uv_members};

/// Kind of resource under reconciliation. Determines the row's prefix
/// label in the human-readable diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Tag,
    FloatingTag,
    Release,
    Asset,
    VersionFile,
    Publish,
}

impl ResourceKind {
    fn label(self) -> &'static str {
        match self {
            Self::Tag => "tag",
            Self::FloatingTag => "floating-tag",
            Self::Release => "release",
            Self::Asset => "asset",
            Self::VersionFile => "version-file",
            Self::Publish => "publish",
        }
    }
}

/// What's there vs. what we want there.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ResourceState {
    Absent,
    Present { value: String },
    /// Present but its shape is opaque to sr (e.g. a remote release
    /// exists; we haven't fetched the body to compare).
    PresentOpaque,
    /// State query failed or unsupported. Carries a reason for the user.
    Unknown { reason: String },
}

impl ResourceState {
    /// Borrow the inner value if the state is `Present`. Used by renderers
    /// that want to compare current vs. desired values directly.
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Present { value } => Some(value),
            _ => None,
        }
    }
}

/// What the reconciler would do to converge actual → desired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Actual is absent; reconciler will create.
    Create,
    /// Actual is present but differs from desired; reconciler will update.
    Update,
    /// Actual already matches desired; reconciler will skip.
    NoChange,
    /// State is unknown; reconciler will attempt the work and rely on the
    /// underlying tool's idempotency.
    Uncertain,
}

impl Action {
    fn symbol(self) -> &'static str {
        match self {
            Self::Create => "+",
            Self::Update => "~",
            Self::NoChange => "=",
            Self::Uncertain => "?",
        }
    }
}

/// One row in the plan output.
#[derive(Debug, Clone, Serialize)]
pub struct ResourceDiff {
    pub kind: ResourceKind,
    /// Unique, human-readable id (e.g. "v8.0.0", "crates/core/Cargo.toml#version").
    pub id: String,
    pub current: ResourceState,
    pub desired: ResourceState,
    pub action: Action,
}

/// Aggregate plan diff.
#[derive(Debug, Clone, Serialize)]
pub struct ReleaseDiff {
    pub tag_name: String,
    pub current_version: Option<String>,
    pub next_version: String,
    pub resources: Vec<ResourceDiff>,
}

impl ReleaseDiff {
    /// Summary counts by action.
    pub fn summary(&self) -> DiffSummary {
        let mut s = DiffSummary::default();
        for r in &self.resources {
            match r.action {
                Action::Create => s.create += 1,
                Action::Update => s.update += 1,
                Action::NoChange => s.no_change += 1,
                Action::Uncertain => s.uncertain += 1,
            }
        }
        s
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct DiffSummary {
    pub create: usize,
    pub update: usize,
    pub no_change: usize,
    pub uncertain: usize,
}

/// Build the diff by querying actual state for every resource the plan
/// would touch.
///
/// `env` is forwarded to publishers (they run `check()` which may shell out
/// — for built-in publishers like cargo/npm, `check` is a pure HTTP call,
/// but `custom` runs the user's check command).
pub fn build_diff<G: GitRepository, V: VcsProvider + ?Sized>(
    plan: &ReleasePlan,
    git: &G,
    vcs: &V,
    config: &Config,
    env: &[(&str, &str)],
) -> Result<ReleaseDiff, ReleaseError> {
    let mut resources: Vec<ResourceDiff> = Vec::new();

    // Tag.
    let tag_exists = git.tag_exists(&plan.tag_name)?;
    resources.push(ResourceDiff {
        kind: ResourceKind::Tag,
        id: plan.tag_name.clone(),
        current: if tag_exists {
            ResourceState::Present {
                value: plan.tag_name.clone(),
            }
        } else {
            ResourceState::Absent
        },
        desired: ResourceState::Present {
            value: plan.tag_name.clone(),
        },
        action: if tag_exists {
            Action::NoChange
        } else {
            Action::Create
        },
    });

    // Floating tag (optional).
    if let Some(floating) = &plan.floating_tag_name {
        let floating_exists = git.tag_exists(floating)?;
        resources.push(ResourceDiff {
            kind: ResourceKind::FloatingTag,
            id: floating.clone(),
            // Floating tags always move — we don't try to inspect their
            // current pointee; just record existence.
            current: if floating_exists {
                ResourceState::PresentOpaque
            } else {
                ResourceState::Absent
            },
            desired: ResourceState::Present {
                value: format!("{floating} → {}", plan.tag_name),
            },
            action: Action::Update,
        });
    }

    // Version files — read current value from each file.
    let mut seen_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    for pkg_plan in &plan.packages {
        for file in &pkg_plan.version_files {
            if !seen_files.insert(file.clone()) {
                continue;
            }
            let current = read_current_version(Path::new(file));
            let desired_value = plan.next_version.to_string();
            let action = match &current {
                ResourceState::Present { value } if value == &desired_value => Action::NoChange,
                ResourceState::Present { .. } => Action::Update,
                ResourceState::Absent => Action::Create,
                ResourceState::PresentOpaque | ResourceState::Unknown { .. } => Action::Uncertain,
            };
            resources.push(ResourceDiff {
                kind: ResourceKind::VersionFile,
                id: file.clone(),
                current,
                desired: ResourceState::Present {
                    value: desired_value,
                },
                action,
            });
        }
    }

    // Release object. We don't fetch the body (cost), so it's opaque-present
    // or absent. Updates are implicit every run — report as Update when
    // present to signal "body may be rewritten".
    let release_exists = vcs.release_exists(&plan.tag_name)?;
    resources.push(ResourceDiff {
        kind: ResourceKind::Release,
        id: plan.tag_name.clone(),
        current: if release_exists {
            ResourceState::PresentOpaque
        } else {
            ResourceState::Absent
        },
        desired: ResourceState::Present {
            value: format!("release {}", plan.tag_name),
        },
        action: if release_exists {
            Action::Update
        } else {
            Action::Create
        },
    });

    // Assets — one row per declared artifact (literal path, not globbed).
    let declared_artifacts = config.all_artifacts();
    if !declared_artifacts.is_empty() {
        let existing_assets: std::collections::HashSet<String> = if release_exists {
            vcs.list_assets(&plan.tag_name)?.into_iter().collect()
        } else {
            std::collections::HashSet::new()
        };
        let (on_disk, missing_on_disk) = partition_paths(&declared_artifacts);

        // Rows for files missing on disk → "build hasn't produced them yet".
        for path in &missing_on_disk {
            resources.push(ResourceDiff {
                kind: ResourceKind::Asset,
                id: path.clone(),
                current: ResourceState::Absent,
                desired: ResourceState::Unknown {
                    reason: "declared artifact not present on disk (build pending?)".into(),
                },
                action: Action::Uncertain,
            });
        }

        // Rows for files on disk → either already uploaded or needs upload.
        {
            for path in &on_disk {
                let basename = Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path.as_str())
                    .to_string();
                let already = existing_assets.contains(&basename);
                resources.push(ResourceDiff {
                    kind: ResourceKind::Asset,
                    id: format!("{}/{basename}", plan.tag_name),
                    current: if already {
                        ResourceState::Present {
                            value: basename.clone(),
                        }
                    } else {
                        ResourceState::Absent
                    },
                    desired: ResourceState::Present { value: basename },
                    action: if already {
                        Action::NoChange
                    } else {
                        Action::Create
                    },
                });
            }
        }
    }

    // Publish resources — one per workspace member per publish target. Empty
    // for packages without a publish config.
    for pkg in &config.packages {
        let Some(cfg) = pkg.publish.as_ref() else {
            continue;
        };
        let rows =
            publish_diff_rows(pkg, cfg, &plan.next_version.to_string(), &plan.tag_name, env)?;
        resources.extend(rows);
    }

    Ok(ReleaseDiff {
        tag_name: plan.tag_name.clone(),
        current_version: plan.current_version.as_ref().map(|v| v.to_string()),
        next_version: plan.next_version.to_string(),
        resources,
    })
}

/// Enumerate publish resource rows for one package. For workspace mode,
/// produces one row per member; otherwise one row.
fn publish_diff_rows(
    pkg: &PackageConfig,
    cfg: &PublishConfig,
    version: &str,
    tag: &str,
    env: &[(&str, &str)],
) -> Result<Vec<ResourceDiff>, ReleaseError> {
    let targets: Vec<PublishTarget> = match cfg {
        PublishConfig::Cargo { workspace, .. } => {
            if *workspace {
                discover_cargo_members(Path::new(&pkg.path))
                    .iter()
                    .filter_map(|m| read_cargo_name(m).map(|n| PublishTarget::Cargo { name: n }))
                    .collect()
            } else {
                read_cargo_name(&Path::new(&pkg.path).join("Cargo.toml"))
                    .map(|n| vec![PublishTarget::Cargo { name: n }])
                    .unwrap_or_default()
            }
        }
        PublishConfig::Npm { workspace, .. } => {
            if *workspace {
                discover_npm_members(Path::new(&pkg.path))
                    .iter()
                    .filter_map(|m| read_npm_name(m).map(|n| PublishTarget::Npm { name: n }))
                    .collect()
            } else {
                read_npm_name(&Path::new(&pkg.path).join("package.json"))
                    .map(|n| vec![PublishTarget::Npm { name: n }])
                    .unwrap_or_default()
            }
        }
        PublishConfig::Pypi { workspace, .. } => {
            if *workspace {
                discover_uv_members(Path::new(&pkg.path))
                    .iter()
                    .filter_map(|m| read_pyproject_name(m).map(|n| PublishTarget::Pypi { name: n }))
                    .collect()
            } else {
                read_pyproject_name(&Path::new(&pkg.path).join("pyproject.toml"))
                    .map(|n| vec![PublishTarget::Pypi { name: n }])
                    .unwrap_or_default()
            }
        }
        PublishConfig::Docker { image, .. } => {
            vec![PublishTarget::Docker {
                image: image.clone(),
            }]
        }
        PublishConfig::Go => vec![PublishTarget::Go {
            path: pkg.path.clone(),
        }],
        PublishConfig::Custom { command, .. } => vec![PublishTarget::Custom {
            label: command.clone(),
        }],
    };

    // If we couldn't identify anything concrete, fall back to a single
    // publisher-level row using `check()`.
    if targets.is_empty() {
        let publisher = publisher_for(cfg);
        let ctx = PublishCtx {
            package: pkg,
            version,
            tag,
            dry_run: false,
            env,
        };
        let (current, action) = match publisher.check(&ctx) {
            Ok(PublishState::Completed) => (
                ResourceState::Present {
                    value: version.to_string(),
                },
                Action::NoChange,
            ),
            Ok(PublishState::Needed) => (ResourceState::Absent, Action::Create),
            Ok(PublishState::Unknown(r)) => (ResourceState::Unknown { reason: r }, Action::Uncertain),
            Err(e) => (
                ResourceState::Unknown {
                    reason: e.to_string(),
                },
                Action::Uncertain,
            ),
        };
        return Ok(vec![ResourceDiff {
            kind: ResourceKind::Publish,
            id: format!("{}/{}", publisher.name(), pkg.path),
            current,
            desired: ResourceState::Present {
                value: version.to_string(),
            },
            action,
        }]);
    }

    // Per-target rows. Each uses a single-scope check (not the aggregated
    // workspace check) so the user sees per-member state.
    let mut rows = Vec::new();
    for target in targets {
        let (kind_label, id, action, current) = match &target {
            PublishTarget::Cargo { name } => {
                let present = probe_registry(&format!(
                    "https://crates.io/api/v1/crates/{name}/{version}"
                ))
                .unwrap_or(None);
                state_from_probe("cargo", name, version, present)
            }
            PublishTarget::Npm { name } => {
                let encoded = name.replacen('/', "%2F", 1);
                let present = probe_registry(&format!(
                    "https://registry.npmjs.org/{encoded}/{version}"
                ))
                .unwrap_or(None);
                state_from_probe("npm", name, version, present)
            }
            PublishTarget::Pypi { name } => {
                let norm = normalize_pypi_name(name);
                let present = probe_registry(&format!(
                    "https://pypi.org/pypi/{norm}/{version}/json"
                ))
                .unwrap_or(None);
                state_from_probe("pypi", name, version, present)
            }
            PublishTarget::Docker { image } => {
                // Skip HEAD request for docker in the diff — too much auth
                // complexity for a preview. Show as Uncertain.
                (
                    "docker".to_string(),
                    format!("{image}:{version}"),
                    Action::Uncertain,
                    ResourceState::Unknown {
                        reason: "docker state check deferred to publish".into(),
                    },
                )
            }
            PublishTarget::Go { path } => (
                "go".to_string(),
                format!("{path} (via tag)"),
                Action::NoChange,
                ResourceState::Present {
                    value: tag.to_string(),
                },
            ),
            PublishTarget::Custom { label } => (
                "custom".to_string(),
                label.clone(),
                Action::Uncertain,
                ResourceState::Unknown {
                    reason: "custom check deferred to publish".into(),
                },
            ),
        };
        rows.push(ResourceDiff {
            kind: ResourceKind::Publish,
            id: format!("{kind_label}:{id}"),
            current,
            desired: ResourceState::Present {
                value: version.to_string(),
            },
            action,
        });
    }
    Ok(rows)
}

enum PublishTarget {
    Cargo { name: String },
    Npm { name: String },
    Pypi { name: String },
    Docker { image: String },
    Go { path: String },
    Custom { label: String },
}

fn state_from_probe(
    publisher: &str,
    name: &str,
    version: &str,
    probe: Option<bool>,
) -> (String, String, Action, ResourceState) {
    let id = format!("{name}@{version}");
    match probe {
        Some(true) => (
            publisher.to_string(),
            id,
            Action::NoChange,
            ResourceState::Present {
                value: version.to_string(),
            },
        ),
        Some(false) => (publisher.to_string(), id, Action::Create, ResourceState::Absent),
        None => (
            publisher.to_string(),
            id,
            Action::Uncertain,
            ResourceState::Unknown {
                reason: "registry probe failed".into(),
            },
        ),
    }
}

fn probe_registry(url: &str) -> Result<Option<bool>, ()> {
    match ureq::get(url)
        .header("User-Agent", "sr (+https://github.com/urmzd/sr)")
        .header("Accept", "application/json")
        .call()
    {
        Ok(resp) if resp.status() == 200 => Ok(Some(true)),
        Ok(_) => Ok(Some(false)),
        Err(ureq::Error::StatusCode(404)) => Ok(Some(false)),
        Err(_) => Ok(None),
    }
}

fn read_cargo_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let doc: toml_edit::DocumentMut = text.parse().ok()?;
    doc.get("package")
        .and_then(|p| p.as_table_like())
        .and_then(|t| t.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn read_npm_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())
}

fn read_pyproject_name(manifest: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let doc: toml_edit::DocumentMut = text.parse().ok()?;
    if let Some(n) = doc
        .get("project")
        .and_then(|p| p.as_table_like())
        .and_then(|t| t.get("name"))
        .and_then(|v| v.as_str())
    {
        return Some(n.to_string());
    }
    doc.get("tool")
        .and_then(|t| t.as_table_like())
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.as_table_like())
        .and_then(|t| t.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn normalize_pypi_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut last_sep = false;
    for ch in lower.chars() {
        if ch == '.' || ch == '_' || ch == '-' {
            if !last_sep {
                out.push('-');
                last_sep = true;
            }
        } else {
            out.push(ch);
            last_sep = false;
        }
    }
    out
}

/// Read the `version` field from a manifest file. Format-aware.
fn read_current_version(path: &Path) -> ResourceState {
    if !path.exists() {
        return ResourceState::Absent;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            return ResourceState::Unknown {
                reason: format!("read {}: {e}", path.display()),
            };
        }
    };
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if (filename == "Cargo.toml" || filename == "pyproject.toml")
        && let Ok(doc) = text.parse::<toml_edit::DocumentMut>()
    {
        // Cargo: package.version or workspace.package.version
        if filename == "Cargo.toml"
            && let Some(v) = doc
                .get("package")
                .and_then(|p| p.as_table_like())
                .and_then(|t| t.get("version"))
                .and_then(|v| v.as_str())
        {
            return ResourceState::Present {
                value: v.to_string(),
            };
        }
        if filename == "Cargo.toml"
            && let Some(v) = doc
                .get("workspace")
                .and_then(|w| w.as_table_like())
                .and_then(|w| w.get("package"))
                .and_then(|p| p.as_table_like())
                .and_then(|t| t.get("version"))
                .and_then(|v| v.as_str())
        {
            return ResourceState::Present {
                value: v.to_string(),
            };
        }
        if filename == "pyproject.toml"
            && let Some(v) = doc
                .get("project")
                .and_then(|p| p.as_table_like())
                .and_then(|t| t.get("version"))
                .and_then(|v| v.as_str())
        {
            return ResourceState::Present {
                value: v.to_string(),
            };
        }
    }

    if filename == "package.json"
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(version) = v.get("version").and_then(|n| n.as_str())
    {
        return ResourceState::Present {
            value: version.to_string(),
        };
    }

    // Regex fallback for Gradle / pom.xml / Go.
    // Gradle: version = '...' or version = "..."
    let gradle = regex::Regex::new(r#"(?m)^\s*version\s*=\s*["']([^"']+)["']"#).unwrap();
    if let Some(cap) = gradle.captures(&text)
        && let Some(m) = cap.get(1)
    {
        return ResourceState::Present {
            value: m.as_str().to_string(),
        };
    }
    // pom.xml: <version>...</version> (first one, skipping <parent>)
    let pom = regex::Regex::new(r#"<version>([^<]+)</version>"#).unwrap();
    if let Some(cap) = pom.captures(&text)
        && let Some(m) = cap.get(1)
    {
        return ResourceState::Present {
            value: m.as_str().to_string(),
        };
    }
    // Go: var/const Version = "..."
    let go = regex::Regex::new(r#"(?:var|const)\s+Version\s*(?:string\s*)?=\s*"([^"]+)""#).unwrap();
    if let Some(cap) = go.captures(&text)
        && let Some(m) = cap.get(1)
    {
        return ResourceState::Present {
            value: m.as_str().to_string(),
        };
    }

    ResourceState::Unknown {
        reason: format!("unsupported format: {}", path.display()),
    }
}

/// Human-friendly rendering of the diff (Terraform-style).
pub fn render_human(diff: &ReleaseDiff) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "Plan: {} → {}\n\n",
        diff.current_version
            .as_deref()
            .unwrap_or("(initial release)"),
        diff.next_version
    ));

    if diff.resources.is_empty() {
        out.push_str("  (no resources)\n");
        return out;
    }

    // Group by kind for readability.
    let kinds = [
        ResourceKind::Tag,
        ResourceKind::FloatingTag,
        ResourceKind::VersionFile,
        ResourceKind::Release,
        ResourceKind::Asset,
        ResourceKind::Publish,
    ];
    for kind in kinds {
        let rows: Vec<&ResourceDiff> = diff.resources.iter().filter(|r| r.kind == kind).collect();
        if rows.is_empty() {
            continue;
        }
        out.push_str(&format!("{}\n", kind.label()));
        for r in rows {
            let detail = match (&r.current, &r.desired) {
                (ResourceState::Present { value: a }, ResourceState::Present { value: b })
                    if a == b =>
                {
                    format!("({a})")
                }
                (ResourceState::Present { value: a }, ResourceState::Present { value: b }) => {
                    format!("{a} → {b}")
                }
                (ResourceState::Absent, ResourceState::Present { value: b }) => b.clone(),
                (ResourceState::PresentOpaque, _) => "exists → will update".into(),
                (ResourceState::Unknown { reason }, _) => format!("? ({reason})"),
                _ => String::new(),
            };
            out.push_str(&format!(
                "  {} {:<40}  {}\n",
                r.action.symbol(),
                r.id,
                detail
            ));
        }
        out.push('\n');
    }

    let s = diff.summary();
    out.push_str(&format!(
        "Summary: {} to create, {} to update, {} unchanged, {} uncertain.\n",
        s.create, s.update, s.no_change, s.uncertain
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_symbols() {
        assert_eq!(Action::Create.symbol(), "+");
        assert_eq!(Action::Update.symbol(), "~");
        assert_eq!(Action::NoChange.symbol(), "=");
        assert_eq!(Action::Uncertain.symbol(), "?");
    }

    #[test]
    fn read_version_from_cargo_toml() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Cargo.toml");
        std::fs::write(&p, "[package]\nname = \"x\"\nversion = \"1.2.3\"\n").unwrap();
        match read_current_version(&p) {
            ResourceState::Present { value } => assert_eq!(value, "1.2.3"),
            other => panic!("expected present, got {other:?}"),
        }
    }

    #[test]
    fn read_version_from_package_json() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("package.json");
        std::fs::write(&p, r#"{"name": "x", "version": "2.0.0"}"#).unwrap();
        match read_current_version(&p) {
            ResourceState::Present { value } => assert_eq!(value, "2.0.0"),
            other => panic!("expected present, got {other:?}"),
        }
    }

    #[test]
    fn read_version_absent() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("no-such.toml");
        assert!(matches!(read_current_version(&p), ResourceState::Absent));
    }

    #[test]
    fn summary_counts() {
        let diff = ReleaseDiff {
            tag_name: "v1".into(),
            current_version: None,
            next_version: "1.0.0".into(),
            resources: vec![
                ResourceDiff {
                    kind: ResourceKind::Tag,
                    id: "v1".into(),
                    current: ResourceState::Absent,
                    desired: ResourceState::Present {
                        value: "v1".into(),
                    },
                    action: Action::Create,
                },
                ResourceDiff {
                    kind: ResourceKind::Release,
                    id: "v1".into(),
                    current: ResourceState::PresentOpaque,
                    desired: ResourceState::Present {
                        value: "v1".into(),
                    },
                    action: Action::Update,
                },
            ],
        };
        let s = diff.summary();
        assert_eq!(s.create, 1);
        assert_eq!(s.update, 1);
    }
}
