//! PyPI publisher: push a wheel/sdist to pypi.org (or a configured repo).
//!
//! - `check`: `GET https://pypi.org/pypi/<name>/<version>/json` per target
//!   package (PEP 503 normalized). Completed iff every target is already
//!   on the registry. Custom `repository` → Unknown (fall through to run()).
//! - `run`: auto-detect `uv` → `uv publish`, else `twine upload dist/*`.
//!   In workspace mode, iterates `[tool.uv.workspace].members` and runs
//!   the publish command in each (uv publishes the current project, so
//!   we cd per-member rather than using a single recursive command).
//!
//! Assumes the build has populated `dist/` in each target (user's
//! responsibility via a build hook). We don't drive wheel/sdist builds.

use std::path::Path;

use super::{PublishCtx, PublishState, Publisher};
use crate::error::ReleaseError;
use crate::hooks::run_shell;
use crate::workspaces::discover_uv_members;

pub struct PypiPublisher {
    pub repository: Option<String>,
    pub workspace: bool,
}

impl Publisher for PypiPublisher {
    fn name(&self) -> &'static str {
        "pypi"
    }

    fn check(&self, ctx: &PublishCtx<'_>) -> Result<PublishState, ReleaseError> {
        if self.repository.is_some() {
            return Ok(PublishState::Unknown(
                "custom PyPI repository — skipping API probe".into(),
            ));
        }

        let targets = resolve_targets(&ctx.package.path, self.workspace);
        if targets.is_empty() {
            return Ok(PublishState::Unknown(
                "no pyproject.toml manifests found to check".into(),
            ));
        }

        let mut any_missing = false;
        for manifest in &targets {
            let name = match read_pyproject_name(manifest) {
                Ok(n) => n,
                Err(e) => return Ok(PublishState::Unknown(e)),
            };
            let normalized = normalize_pypi_name(&name);
            match probe_pypi(&normalized, ctx.version) {
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
        let uv_available = which_exists("uv");
        let targets = resolve_targets(&ctx.package.path, self.workspace);
        if targets.is_empty() {
            return Err(ReleaseError::Config(
                "pypi publish: no pyproject.toml manifests found".into(),
            ));
        }

        for manifest in &targets {
            // Publish is run from the member directory (where dist/ lives).
            let pkg_dir = manifest
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| ctx.package.path.clone());
            let cmd = build_cmd(uv_available, &self.repository);

            if ctx.dry_run {
                eprintln!("[dry-run] pypi ({pkg_dir}): {cmd}");
                continue;
            }

            eprintln!("pypi ({pkg_dir}): {cmd}");
            let wrapped = format!("cd {} && {cmd}", shell_word(&pkg_dir));
            run_shell(&wrapped, None, ctx.env)?;
        }
        Ok(())
    }
}

fn resolve_targets(pkg_path: &str, workspace: bool) -> Vec<std::path::PathBuf> {
    if workspace {
        discover_uv_members(Path::new(pkg_path))
    } else {
        vec![Path::new(pkg_path).join("pyproject.toml")]
    }
}

fn build_cmd(uv_available: bool, repository: &Option<String>) -> String {
    if uv_available {
        match repository {
            Some(r) => format!("uv publish --publish-url {}", shell_word(r)),
            None => "uv publish".to_string(),
        }
    } else {
        match repository {
            Some(r) => format!("twine upload --repository {} dist/*", shell_word(r)),
            None => "twine upload dist/*".to_string(),
        }
    }
}

fn probe_pypi(normalized_name: &str, version: &str) -> Result<bool, String> {
    let url = format!("https://pypi.org/pypi/{normalized_name}/{version}/json");
    match ureq::get(&url)
        .header("User-Agent", "sr (+https://github.com/urmzd/sr)")
        .header("Accept", "application/json")
        .call()
    {
        Ok(resp) if resp.status() == 200 => Ok(true),
        Ok(_) => Ok(false),
        Err(ureq::Error::StatusCode(404)) => Ok(false),
        Err(e) => Err(format!("pypi check failed for {normalized_name}: {e}")),
    }
}

fn read_pyproject_name(manifest: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(manifest)
        .map_err(|e| format!("read {}: {e}", manifest.display()))?;
    let doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("parse {}: {e}", manifest.display()))?;

    // [project].name (PEP 621) takes precedence over legacy [tool.poetry].name.
    if let Some(n) = doc
        .get("project")
        .and_then(|p| p.as_table_like())
        .and_then(|t| t.get("name"))
        .and_then(|v| v.as_str())
    {
        return Ok(n.to_string());
    }
    if let Some(n) = doc
        .get("tool")
        .and_then(|t| t.as_table_like())
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.as_table_like())
        .and_then(|t| t.get("name"))
        .and_then(|v| v.as_str())
    {
        return Ok(n.to_string());
    }
    Err(format!("no project.name in {}", manifest.display()))
}

/// PEP 503 name normalization — lowercase + collapse runs of [._-] into '-'.
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

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-c", &format!("command -v {cmd} >/dev/null 2>&1")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
    fn read_pep621_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"my-pkg\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let name = read_pyproject_name(&dir.path().join("pyproject.toml")).unwrap();
        assert_eq!(name, "my-pkg");
    }

    #[test]
    fn read_poetry_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[tool.poetry]\nname = \"poetry-pkg\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let name = read_pyproject_name(&dir.path().join("pyproject.toml")).unwrap();
        assert_eq!(name, "poetry-pkg");
    }

    #[test]
    fn pep621_wins_over_poetry() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            r#"[project]
name = "pep-621-name"
version = "0.1.0"

[tool.poetry]
name = "poetry-name"
"#,
        )
        .unwrap();
        let name = read_pyproject_name(&dir.path().join("pyproject.toml")).unwrap();
        assert_eq!(name, "pep-621-name");
    }

    #[test]
    fn normalize_examples() {
        assert_eq!(normalize_pypi_name("My.Package"), "my-package");
        assert_eq!(normalize_pypi_name("my_pkg"), "my-pkg");
        assert_eq!(normalize_pypi_name("My--Pkg"), "my-pkg");
        assert_eq!(normalize_pypi_name("Already-Normal"), "already-normal");
    }

    #[test]
    fn resolve_targets_uv_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[tool.uv.workspace]\nmembers = [\"packages/*\"]\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("packages/core")).unwrap();
        std::fs::write(
            dir.path().join("packages/core/pyproject.toml"),
            "[project]\nname = \"core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let ws = resolve_targets(dir.path().to_str().unwrap(), true);
        assert_eq!(ws.len(), 1);

        let single = resolve_targets(dir.path().to_str().unwrap(), false);
        assert_eq!(single.len(), 1);
        assert!(single[0].ends_with("pyproject.toml"));
    }

    #[test]
    fn build_cmd_shapes() {
        assert_eq!(build_cmd(true, &None), "uv publish");
        assert!(build_cmd(false, &None).starts_with("twine upload"));
        assert!(build_cmd(true, &Some("private".into())).contains("--publish-url"));
    }
}
