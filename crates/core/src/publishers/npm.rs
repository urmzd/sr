//! npm publisher: registry.npmjs.org (or a custom registry).
//!
//! Auto-detects the tool in use at the package path (pnpm / yarn / npm)
//! from lockfile presence, so workspace publishes use the right native
//! recursive command without the user picking manually.
//!
//! - `check`: `GET <registry>/<name>/<version>` for each target package
//!   (single package, or every workspace member when `workspace: true`).
//!   Completed iff every target is already on the registry. Private
//!   packages (`"private": true`) are skipped — they can't be published.
//! - `run`:
//!   - pnpm workspace → `pnpm publish -r` (filters already-published).
//!   - npm workspace  → `npm publish --workspaces`.
//!   - yarn workspace → `yarn workspaces foreach -A npm publish` (yarn v3+).
//!   - non-workspace  → `npm publish` in the package dir.

use std::path::Path;

use super::{PublishCtx, PublishState, Publisher};
use crate::error::ReleaseError;
use crate::hooks::run_shell;
use crate::workspaces::{detect_npm_tool, discover_npm_members};

pub struct NpmPublisher {
    pub registry: Option<String>,
    pub access: Option<String>,
    pub workspace: bool,
}

impl Publisher for NpmPublisher {
    fn name(&self) -> &'static str {
        "npm"
    }

    fn check(&self, ctx: &PublishCtx<'_>) -> Result<PublishState, ReleaseError> {
        let targets = resolve_targets(&ctx.package.path, self.workspace);
        if targets.is_empty() {
            return Ok(PublishState::Unknown(
                "no package.json manifests found to check".into(),
            ));
        }

        let registry = self
            .registry
            .as_deref()
            .unwrap_or("https://registry.npmjs.org")
            .trim_end_matches('/');

        let mut any_missing = false;
        for manifest in &targets {
            // Private packages can't be published — skip silently.
            if read_package_json_private(manifest).unwrap_or(false) {
                continue;
            }
            let name = match read_package_json_name(manifest) {
                Ok(n) => n,
                Err(e) => return Ok(PublishState::Unknown(e)),
            };
            match probe_npm(registry, &name, ctx.version) {
                Ok(true) => {}
                Ok(false) => any_missing = true,
                Err(e) => return Ok(PublishState::Unknown(e)),
            }
        }

        if any_missing {
            Ok(PublishState::Needed)
        } else {
            Ok(PublishState::Completed)
        }
    }

    fn run(&self, ctx: &PublishCtx<'_>) -> Result<(), ReleaseError> {
        let pkg_path = Path::new(&ctx.package.path);
        let cmd = if self.workspace {
            match detect_npm_tool(pkg_path) {
                "pnpm" => build_pnpm_workspace_cmd(&self.registry, &self.access),
                "yarn" => build_yarn_workspace_cmd(&self.access),
                _ => build_npm_workspace_cmd(&self.registry, &self.access),
            }
        } else {
            build_single_cmd(&self.registry, &self.access)
        };

        if ctx.dry_run {
            eprintln!("[dry-run] npm ({}): {cmd}", ctx.package.path);
            return Ok(());
        }

        eprintln!("npm ({}): {cmd}", ctx.package.path);
        let wrapped = format!("cd {} && {cmd}", shell_word(&ctx.package.path));
        run_shell(&wrapped, None, ctx.env)
    }
}

fn resolve_targets(pkg_path: &str, workspace: bool) -> Vec<std::path::PathBuf> {
    if workspace {
        discover_npm_members(Path::new(pkg_path))
    } else {
        vec![Path::new(pkg_path).join("package.json")]
    }
}

fn build_single_cmd(registry: &Option<String>, access: &Option<String>) -> String {
    let mut c = String::from("npm publish");
    if let Some(reg) = registry {
        c.push_str(" --registry ");
        c.push_str(&shell_word(reg));
    }
    if let Some(access) = access {
        c.push_str(" --access ");
        c.push_str(&shell_word(access));
    }
    c
}

fn build_pnpm_workspace_cmd(registry: &Option<String>, access: &Option<String>) -> String {
    let mut c = String::from("pnpm publish -r --no-git-checks");
    if let Some(reg) = registry {
        c.push_str(" --registry ");
        c.push_str(&shell_word(reg));
    }
    if let Some(access) = access {
        c.push_str(" --access ");
        c.push_str(&shell_word(access));
    }
    c
}

fn build_yarn_workspace_cmd(access: &Option<String>) -> String {
    // yarn v3+ workspaces. Users on yarn v1 classic should use `publish: custom`.
    let mut c = String::from("yarn workspaces foreach -A --no-private npm publish");
    if let Some(access) = access {
        c.push_str(" --access ");
        c.push_str(&shell_word(access));
    }
    c
}

fn build_npm_workspace_cmd(registry: &Option<String>, access: &Option<String>) -> String {
    let mut c = String::from("npm publish --workspaces");
    if let Some(reg) = registry {
        c.push_str(" --registry ");
        c.push_str(&shell_word(reg));
    }
    if let Some(access) = access {
        c.push_str(" --access ");
        c.push_str(&shell_word(access));
    }
    c
}

fn probe_npm(registry: &str, name: &str, version: &str) -> Result<bool, String> {
    let encoded_name = name.replacen('/', "%2F", 1);
    let url = format!("{registry}/{encoded_name}/{version}");
    match ureq::get(&url)
        .header("User-Agent", "sr (+https://github.com/urmzd/sr)")
        .header("Accept", "application/json")
        .call()
    {
        Ok(resp) if resp.status() == 200 => Ok(true),
        Ok(_) => Ok(false),
        Err(ureq::Error::StatusCode(404)) => Ok(false),
        Err(e) => Err(format!("npm registry check failed for {name}: {e}")),
    }
}

fn read_package_json_name(manifest: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(manifest)
        .map_err(|e| format!("read {}: {e}", manifest.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("parse {}: {e}", manifest.display()))?;
    value
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("no `name` in {}", manifest.display()))
}

fn read_package_json_private(manifest: &Path) -> Result<bool, String> {
    let text = std::fs::read_to_string(manifest)
        .map_err(|e| format!("read {}: {e}", manifest.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("parse {}: {e}", manifest.display()))?;
    Ok(value
        .get("private")
        .and_then(|v| v.as_bool())
        .unwrap_or(false))
}

fn shell_word(s: &str) -> String {
    let mut out = String::from("'");
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_name_from_package_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name": "@scope/pkg", "version": "0.1.0"}"#,
        )
        .unwrap();
        let name = read_package_json_name(&dir.path().join("package.json")).unwrap();
        assert_eq!(name, "@scope/pkg");
    }

    #[test]
    fn private_flag_parsed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name": "priv", "private": true}"#,
        )
        .unwrap();
        let p = read_package_json_private(&dir.path().join("package.json")).unwrap();
        assert!(p);
    }

    #[test]
    fn missing_package_json_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_package_json_name(&dir.path().join("package.json")).unwrap_err();
        assert!(err.contains("read"));
    }

    #[test]
    fn resolve_targets_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name": "root", "private": true, "workspaces": ["packages/*"]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("packages/a")).unwrap();
        std::fs::write(
            dir.path().join("packages/a/package.json"),
            r#"{"name": "a", "version": "0.1.0"}"#,
        )
        .unwrap();

        let ws = resolve_targets(dir.path().to_str().unwrap(), true);
        assert_eq!(ws.len(), 1);
        assert!(ws[0].to_string_lossy().contains("packages/a"));

        let single = resolve_targets(dir.path().to_str().unwrap(), false);
        assert_eq!(single.len(), 1);
        assert!(single[0].ends_with("package.json"));
    }

    #[test]
    fn command_shapes() {
        assert!(build_single_cmd(&None, &None).contains("npm publish"));
        assert!(!build_single_cmd(&None, &None).contains("--workspaces"));
        assert!(build_pnpm_workspace_cmd(&None, &None).starts_with("pnpm publish -r"));
        assert!(build_yarn_workspace_cmd(&None).starts_with("yarn workspaces foreach"));
        assert!(build_npm_workspace_cmd(&None, &None).contains("--workspaces"));
    }
}
