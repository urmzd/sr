//! Workspace member discovery for Cargo, npm/pnpm/yarn, and uv.
//!
//! Given a workspace root path, returns absolute paths to the manifest file
//! of every published member. Used by the workspace-aware publishers to:
//! 1. Aggregate registry checks (Completed iff every member is already published).
//! 2. Drive per-member publish commands for ecosystems without a native
//!    `publish --workspace` flag (cargo, uv).
//!
//! Non-workspace roots return an empty list.

use std::path::{Path, PathBuf};

/// Discover Cargo workspace member manifests (Cargo.toml paths).
///
/// Reads `[workspace].members` globs from the root `Cargo.toml` and resolves
/// each to a directory containing a `Cargo.toml`. Returns empty if the root
/// is not a workspace or if the file is missing/unreadable.
pub fn discover_cargo_members(root: &Path) -> Vec<PathBuf> {
    let manifest = root.join("Cargo.toml");
    let text = match std::fs::read_to_string(&manifest) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let doc: toml_edit::DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let members = toml_string_array(&doc, &["workspace", "members"]);
    resolve_member_globs(root, &members, "Cargo.toml")
}

/// Discover npm/pnpm/yarn workspace member manifests (package.json paths).
///
/// Reads `workspaces` from the root `package.json` — supports both the
/// array form (`"workspaces": ["packages/*"]`) and the object form
/// (`"workspaces": {"packages": [...]}`). For pnpm-only repos, falls back
/// to reading `pnpm-workspace.yaml` (`packages:` list).
pub fn discover_npm_members(root: &Path) -> Vec<PathBuf> {
    // Prefer package.json workspaces.
    if let Ok(text) = std::fs::read_to_string(root.join("package.json"))
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&text)
    {
        let patterns = match val.get("workspaces") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>(),
            Some(serde_json::Value::Object(obj)) => obj
                .get("packages")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if !patterns.is_empty() {
            return resolve_member_globs(root, &patterns, "package.json");
        }
    }

    // Fall back to pnpm-workspace.yaml.
    if let Ok(text) = std::fs::read_to_string(root.join("pnpm-workspace.yaml"))
        && let Ok(val) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text)
    {
        let patterns: Vec<String> = val
            .get("packages")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if !patterns.is_empty() {
            return resolve_member_globs(root, &patterns, "package.json");
        }
    }

    Vec::new()
}

/// Discover uv workspace member manifests (pyproject.toml paths).
///
/// Reads `[tool.uv.workspace].members` globs from the root `pyproject.toml`.
/// Returns empty for a non-workspace pyproject.
pub fn discover_uv_members(root: &Path) -> Vec<PathBuf> {
    let manifest = root.join("pyproject.toml");
    let text = match std::fs::read_to_string(&manifest) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let doc: toml_edit::DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let members = toml_string_array(&doc, &["tool", "uv", "workspace", "members"]);
    resolve_member_globs(root, &members, "pyproject.toml")
}

/// Detect which npm-compatible tool is in use at `root`. Checks lockfiles
/// in priority order: pnpm > yarn > npm. Returns `"npm"` as the safe default.
pub fn detect_npm_tool(root: &Path) -> &'static str {
    if root.join("pnpm-lock.yaml").exists() || root.join("pnpm-workspace.yaml").exists() {
        "pnpm"
    } else if root.join("yarn.lock").exists() {
        "yarn"
    } else {
        "npm"
    }
}

/// Resolve glob patterns into a list of manifest paths. Each glob is
/// resolved relative to `root`, and `manifest_name` is appended to each
/// matched directory. Nonexistent manifests are filtered out.
fn resolve_member_globs(root: &Path, patterns: &[String], manifest_name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for pattern in patterns {
        let full = root.join(pattern).to_string_lossy().into_owned();
        let Ok(entries) = glob::glob(&full) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.is_dir() {
                continue;
            }
            let manifest = entry.join(manifest_name);
            if manifest.exists() {
                out.push(manifest);
            }
        }
    }
    out
}

fn toml_string_array(doc: &toml_edit::DocumentMut, keys: &[&str]) -> Vec<String> {
    let mut item: Option<&toml_edit::Item> = None;
    for key in keys {
        item = match item {
            None => doc.get(key),
            Some(parent) => parent.get(key),
        };
        if item.is_none() {
            return Vec::new();
        }
    }
    item.and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn discover_cargo_members_basic() {
        let dir = tempdir();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("crates/core")).unwrap();
        fs::write(
            dir.path().join("crates/core/Cargo.toml"),
            "[package]\nname = \"core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("crates/cli")).unwrap();
        fs::write(
            dir.path().join("crates/cli/Cargo.toml"),
            "[package]\nname = \"cli\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let members = discover_cargo_members(dir.path());
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn discover_cargo_members_no_workspace_returns_empty() {
        let dir = tempdir();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"p\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert!(discover_cargo_members(dir.path()).is_empty());
    }

    #[test]
    fn discover_npm_members_array_form() {
        let dir = tempdir();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name": "root", "private": true, "workspaces": ["packages/*"]}"#,
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("packages/a")).unwrap();
        fs::write(
            dir.path().join("packages/a/package.json"),
            r#"{"name": "a", "version": "0.1.0"}"#,
        )
        .unwrap();

        let members = discover_npm_members(dir.path());
        assert_eq!(members.len(), 1);
    }

    #[test]
    fn discover_npm_members_object_form() {
        let dir = tempdir();
        fs::write(
            dir.path().join("package.json"),
            r#"{"workspaces": {"packages": ["pkgs/*"]}}"#,
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("pkgs/a")).unwrap();
        fs::write(
            dir.path().join("pkgs/a/package.json"),
            r#"{"name": "a", "version": "0.1.0"}"#,
        )
        .unwrap();
        assert_eq!(discover_npm_members(dir.path()).len(), 1);
    }

    #[test]
    fn discover_npm_members_pnpm_workspace_yaml() {
        let dir = tempdir();
        // No workspaces in package.json — should fall back to pnpm-workspace.yaml.
        fs::write(dir.path().join("package.json"), r#"{"name": "root"}"#).unwrap();
        fs::write(
            dir.path().join("pnpm-workspace.yaml"),
            "packages:\n  - packages/*\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("packages/x")).unwrap();
        fs::write(
            dir.path().join("packages/x/package.json"),
            r#"{"name": "x", "version": "0.1.0"}"#,
        )
        .unwrap();
        assert_eq!(discover_npm_members(dir.path()).len(), 1);
    }

    #[test]
    fn discover_uv_members_basic() {
        let dir = tempdir();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[tool.uv.workspace]\nmembers = [\"packages/*\"]\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("packages/core")).unwrap();
        fs::write(
            dir.path().join("packages/core/pyproject.toml"),
            "[project]\nname = \"core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        assert_eq!(discover_uv_members(dir.path()).len(), 1);
    }

    #[test]
    fn detect_npm_tool_priority() {
        let dir = tempdir();
        assert_eq!(detect_npm_tool(dir.path()), "npm");

        fs::write(dir.path().join("yarn.lock"), "").unwrap();
        assert_eq!(detect_npm_tool(dir.path()), "yarn");

        fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        assert_eq!(detect_npm_tool(dir.path()), "pnpm");
    }
}
