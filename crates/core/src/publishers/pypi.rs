//! PyPI publisher: push wheel+sdist artifacts to pypi.org (or a configured repo).
//!
//! - `check`: `GET https://pypi.org/pypi/<name>/<version>/json` per target
//!   package (PEP 503 normalized). Completed iff every target is already
//!   on the registry. Custom `repository` → Unknown (fall through to run()).
//! - `run`: auto-detect `uv` → `uv publish <files>`, else `twine upload <files>`.
//!   Both tools accept explicit file arguments, so sr resolves each member's
//!   artifacts by filename prefix (PEP 625 stem + version) under a shared
//!   `<package_path>/<dist_dir>` — matching `uv build --all`'s output layout
//!   (one workspace-root `dist/`) rather than assuming per-member dist dirs.
//!
//! Assumes the build step (`uv build --all`, `poetry build`, `python -m build`)
//! has populated `dist/` before sr runs. sr does not drive wheel/sdist builds.

use std::path::{Path, PathBuf};

use super::{PublishCtx, PublishState, Publisher};
use crate::error::ReleaseError;
use crate::hooks::run_shell;
use crate::workspaces::discover_uv_members;

pub struct PypiPublisher {
    pub repository: Option<String>,
    pub workspace: bool,
    pub dist_dir: Option<String>,
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

        let dist_dir = self.dist_dir.as_deref().unwrap_or("dist");
        let dist_root = Path::new(&ctx.package.path).join(dist_dir);

        for manifest in &targets {
            let name =
                read_pyproject_name(manifest).map_err(|e| ReleaseError::Config(format!("pypi publish: {e}")))?;
            let stem = filename_stem(&name);
            let artifacts = find_artifacts(&dist_root, &stem, ctx.version)
                .map_err(|e| ReleaseError::Config(format!("pypi publish: {e}")))?;

            if artifacts.is_empty() {
                return Err(ReleaseError::Config(format!(
                    "pypi publish: no artifacts for {name} {} in {} (expected `{stem}-{}*.whl` or `{stem}-{}.tar.gz`)",
                    ctx.version,
                    dist_root.display(),
                    ctx.version,
                    ctx.version,
                )));
            }

            let cmd = build_cmd(uv_available, &self.repository, &artifacts);

            if ctx.dry_run {
                eprintln!("[dry-run] pypi ({name}): {cmd}");
                continue;
            }

            eprintln!("pypi ({name}): {cmd}");
            run_shell(&cmd, None, ctx.env)?;
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

fn build_cmd(uv_available: bool, repository: &Option<String>, files: &[PathBuf]) -> String {
    let files_str = files
        .iter()
        .map(|p| shell_word(&p.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    if uv_available {
        match repository {
            Some(r) => format!("uv publish --publish-url {} {files_str}", shell_word(r)),
            None => format!("uv publish {files_str}"),
        }
    } else {
        match repository {
            Some(r) => format!("twine upload --repository {} {files_str}", shell_word(r)),
            None => format!("twine upload {files_str}"),
        }
    }
}

/// Find wheels + sdists for a package in `dist_root`, matched by filename stem
/// and exact version. Guards against version-prefix collisions (e.g. `1.0` vs
/// `1.0.1`) by requiring the character after `<stem>-<version>` be `-` (wheel)
/// or the suffix to be exactly `.tar.gz` (sdist).
fn find_artifacts(dist_root: &Path, stem: &str, version: &str) -> Result<Vec<PathBuf>, String> {
    let entries = match std::fs::read_dir(dist_root) {
        Ok(e) => e,
        Err(e) => return Err(format!("read {}: {e}", dist_root.display())),
    };
    let prefix = format!("{stem}-{version}");
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let name = fname.to_string_lossy();
        if !name.starts_with(&prefix) {
            continue;
        }
        let rest = &name[prefix.len()..];
        let is_wheel = rest.starts_with('-') && name.ends_with(".whl");
        let is_sdist = rest == ".tar.gz";
        if is_wheel || is_sdist {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
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
/// Used for PyPI URL paths (`/pypi/<name>/<version>/json`).
fn normalize_pypi_name(name: &str) -> String {
    collapse_seps(name, '-')
}

/// PEP 625 filename stem — lowercase + collapse runs of [._-] into '_'.
/// Used to match built wheel/sdist filenames, which use underscore separators.
fn filename_stem(name: &str) -> String {
    collapse_seps(name, '_')
}

fn collapse_seps(name: &str, sep: char) -> String {
    let lower = name.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut last_sep = false;
    for ch in lower.chars() {
        if ch == '.' || ch == '_' || ch == '-' {
            if !last_sep {
                out.push(sep);
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
    fn filename_stem_uses_underscore() {
        // PEP 625 / PEP 427: filenames use `_` as the separator, not `-`.
        assert_eq!(filename_stem("my-pkg"), "my_pkg");
        assert_eq!(filename_stem("My.Package"), "my_package");
        assert_eq!(filename_stem("my_pkg"), "my_pkg");
        assert_eq!(filename_stem("Already_Normal"), "already_normal");
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
    fn build_cmd_uv_default() {
        let files = vec![PathBuf::from("/w/dist/foo-1.0.0.tar.gz")];
        assert_eq!(
            build_cmd(true, &None, &files),
            "uv publish '/w/dist/foo-1.0.0.tar.gz'"
        );
    }

    #[test]
    fn build_cmd_uv_with_repo() {
        let files = vec![PathBuf::from("/w/dist/foo-1.0.0-py3-none-any.whl")];
        let cmd = build_cmd(true, &Some("https://private".into()), &files);
        assert!(cmd.starts_with("uv publish --publish-url 'https://private'"));
        assert!(cmd.ends_with("'/w/dist/foo-1.0.0-py3-none-any.whl'"));
    }

    #[test]
    fn build_cmd_twine_fallback() {
        let files = vec![PathBuf::from("/w/dist/foo-1.0.0.tar.gz")];
        assert!(build_cmd(false, &None, &files).starts_with("twine upload "));
    }

    #[test]
    fn find_artifacts_matches_wheel_and_sdist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("my_pkg-1.0.0.tar.gz"), "").unwrap();
        std::fs::write(dir.path().join("my_pkg-1.0.0-py3-none-any.whl"), "").unwrap();
        std::fs::write(dir.path().join("other_pkg-1.0.0.tar.gz"), "").unwrap();
        std::fs::write(dir.path().join("README.md"), "").unwrap();

        let found = find_artifacts(dir.path(), "my_pkg", "1.0.0").unwrap();
        assert_eq!(found.len(), 2);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"my_pkg-1.0.0.tar.gz".into()));
        assert!(names.contains(&"my_pkg-1.0.0-py3-none-any.whl".into()));
    }

    #[test]
    fn find_artifacts_rejects_version_prefix_collision() {
        // `foo-1.0` must NOT match `foo-1.0.1-py3...whl` or `foo-1.0.1.tar.gz`.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("foo-1.0.1.tar.gz"), "").unwrap();
        std::fs::write(dir.path().join("foo-1.0.1-py3-none-any.whl"), "").unwrap();
        std::fs::write(dir.path().join("foo-1.0.tar.gz"), "").unwrap();

        let found = find_artifacts(dir.path(), "foo", "1.0").unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("foo-1.0.tar.gz"));
    }

    #[test]
    fn find_artifacts_empty_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let found = find_artifacts(dir.path(), "foo", "1.0.0").unwrap();
        assert!(found.is_empty());
    }
}
