//! Cargo publisher: crates.io (or a custom registry).
//!
//! - `check`: for each crate under consideration, GET
//!   `https://crates.io/api/v1/crates/<name>/<version>`.
//!   200 → published; 404 → not published.
//!   In workspace mode, aggregates across all members: Completed iff every
//!   member is already on the registry.
//! - `run`: `cargo publish -p <name>` per crate. crates.io's index can lag
//!   30–60s between publishes; cargo retries internally. We iterate in
//!   `[workspace].members` order (user's responsibility to list deps first).

use std::path::Path;

use super::{PublishCtx, PublishState, Publisher};
use crate::error::ReleaseError;
use crate::hooks::run_shell;
use crate::workspaces::discover_cargo_members;

pub struct CargoPublisher {
    pub features: Vec<String>,
    pub registry: Option<String>,
    pub workspace: bool,
}

impl Publisher for CargoPublisher {
    fn name(&self) -> &'static str {
        "cargo"
    }

    fn check(&self, ctx: &PublishCtx<'_>) -> Result<PublishState, ReleaseError> {
        if self.registry.is_some() {
            return Ok(PublishState::Unknown(
                "custom cargo registry — skipping API probe".into(),
            ));
        }

        let targets = resolve_targets(&ctx.package.path, self.workspace);
        if targets.is_empty() {
            return Ok(PublishState::Unknown(
                "no crate manifests found to check".into(),
            ));
        }

        let mut any_missing = false;
        for manifest in &targets {
            let name = match read_cargo_package_name(manifest) {
                Ok(n) => n,
                Err(e) => return Ok(PublishState::Unknown(e)),
            };
            match probe_crates_io(&name, ctx.version) {
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
        let targets = resolve_targets(&ctx.package.path, self.workspace);
        if targets.is_empty() {
            return Err(ReleaseError::Config(
                "cargo publish: no crate manifests found".into(),
            ));
        }

        for manifest in &targets {
            let name = read_cargo_package_name(manifest)
                .map_err(|e| ReleaseError::Config(format!("cargo publish: {e}")))?;

            let mut cmd = format!("cargo publish -p {}", shell_word(&name));
            if !self.features.is_empty() {
                cmd.push_str(" --features ");
                cmd.push_str(&shell_word(&self.features.join(",")));
            }
            if let Some(reg) = &self.registry {
                cmd.push_str(" --registry ");
                cmd.push_str(&shell_word(reg));
            }

            if ctx.dry_run {
                eprintln!("[dry-run] cargo ({}): {cmd}", ctx.package.path);
                continue;
            }

            eprintln!("cargo ({}): {cmd}", ctx.package.path);
            let wrapped = format!("cd {} && {cmd}", shell_word(&ctx.package.path));
            run_shell(&wrapped, None, ctx.env)?;
        }
        Ok(())
    }
}

fn resolve_targets(pkg_path: &str, workspace: bool) -> Vec<std::path::PathBuf> {
    if workspace {
        discover_cargo_members(Path::new(pkg_path))
    } else {
        vec![Path::new(pkg_path).join("Cargo.toml")]
    }
}

fn probe_crates_io(name: &str, version: &str) -> Result<bool, String> {
    let url = format!("https://crates.io/api/v1/crates/{name}/{version}");
    match ureq::get(&url)
        .header("User-Agent", "sr (+https://github.com/urmzd/sr)")
        .header("Accept", "application/json")
        .call()
    {
        Ok(resp) if resp.status() == 200 => Ok(true),
        Ok(_) => Ok(false),
        Err(ureq::Error::StatusCode(404)) => Ok(false),
        Err(e) => Err(format!("crates.io check failed for {name}: {e}")),
    }
}

fn read_cargo_package_name(manifest: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(manifest)
        .map_err(|e| format!("read {}: {e}", manifest.display()))?;
    let doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("parse {}: {e}", manifest.display()))?;
    doc.get("package")
        .and_then(|p| p.as_table_like())
        .and_then(|t| t.get("name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("no package.name in {}", manifest.display()))
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
    fn read_name_from_real_cargo_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let name = read_cargo_package_name(&dir.path().join("Cargo.toml")).unwrap();
        assert_eq!(name, "my-crate");
    }

    #[test]
    fn read_name_missing_cargo_toml_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = read_cargo_package_name(&dir.path().join("Cargo.toml")).unwrap_err();
        assert!(err.contains("read"));
    }

    #[test]
    fn read_name_missing_name_field_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let err = read_cargo_package_name(&dir.path().join("Cargo.toml")).unwrap_err();
        assert!(err.contains("no package.name"));
    }

    #[test]
    fn resolve_targets_single_vs_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("crates/core")).unwrap();
        std::fs::write(
            dir.path().join("crates/core/Cargo.toml"),
            "[package]\nname = \"c\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let single = resolve_targets(dir.path().to_str().unwrap(), false);
        assert_eq!(single.len(), 1);
        assert!(single[0].ends_with("Cargo.toml"));

        let ws = resolve_targets(dir.path().to_str().unwrap(), true);
        assert_eq!(ws.len(), 1);
        assert!(ws[0].to_string_lossy().contains("crates/core"));
    }
}
