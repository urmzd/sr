//! Cargo publisher: crates.io (or a custom registry).
//!
//! - `check`: for each crate under consideration, GET
//!   `https://crates.io/api/v1/crates/<name>/<version>`.
//!   200 → published; 404 → not published.
//!   In workspace mode, aggregates across all members: Completed iff every
//!   member is already on the registry.
//! - `run`: `cargo publish -p <name>` per crate. crates.io's index can lag
//!   30–60s between publishes; cargo retries internally. We publish members in
//!   intra-workspace dependency order (a crate is published after every other
//!   member it depends on), so `members = ["crates/*"]` works regardless of how
//!   the glob happens to sort.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

fn resolve_targets(pkg_path: &str, workspace: bool) -> Vec<PathBuf> {
    if workspace {
        order_by_dependencies(discover_cargo_members(Path::new(pkg_path)))
    } else {
        vec![Path::new(pkg_path).join("Cargo.toml")]
    }
}

/// Order workspace member manifests so each crate is published after the other
/// members it depends on. crates.io rejects a publish whose intra-workspace
/// dependency requirement is not yet on the index, so glob / declaration order
/// (e.g. `crates/*` → `oag-cli` before `oag-core`) is not safe. We build the
/// intra-workspace dependency graph and emit a stable topological order
/// (dependencies first), preserving the original order as a tiebreak and for
/// any crate whose manifest can't be parsed.
fn order_by_dependencies(targets: Vec<PathBuf>) -> Vec<PathBuf> {
    let n = targets.len();
    if n < 2 {
        return targets;
    }

    // Map each member's package name to its index. First declaration wins.
    let mut index_of: HashMap<String, usize> = HashMap::new();
    for (i, manifest) in targets.iter().enumerate() {
        if let Ok(name) = read_cargo_package_name(manifest) {
            index_of.entry(name).or_insert(i);
        }
    }

    // deps[i] = sorted indices of other members that member i depends on.
    let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, manifest) in targets.iter().enumerate() {
        for dep in read_intra_workspace_dep_names(manifest) {
            if let Some(&j) = index_of.get(&dep)
                && j != i
                && !deps[i].contains(&j)
            {
                deps[i].push(j);
            }
        }
        deps[i].sort_unstable();
    }

    // Stable DFS post-order: a node is emitted after its dependencies. Marking
    // a node visited before recursing makes any cycle terminate (a real cargo
    // dependency cycle is unpublishable anyway, so best-effort is fine).
    let mut visited = vec![false; n];
    let mut ordered = Vec::with_capacity(n);
    for start in 0..n {
        dfs_post_order(start, &deps, &mut visited, &mut ordered);
    }
    ordered.into_iter().map(|i| targets[i].clone()).collect()
}

fn dfs_post_order(i: usize, deps: &[Vec<usize>], visited: &mut [bool], out: &mut Vec<usize>) {
    if visited[i] {
        return;
    }
    visited[i] = true;
    for &j in &deps[i] {
        dfs_post_order(j, deps, visited, out);
    }
    out.push(i);
}

/// Collect the crate names a manifest depends on via `[dependencies]` and
/// `[build-dependencies]`. Honors renamed deps (`alias = { package = "real" }`
/// → `real`). Dev-dependencies are intentionally excluded: they are not part
/// of the published verification build and can introduce false cycles (e.g. a
/// core crate dev-depending on a CLI that depends on it).
fn read_intra_workspace_dep_names(manifest: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(manifest) else {
        return Vec::new();
    };
    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for table in ["dependencies", "build-dependencies"] {
        let Some(tbl) = doc.get(table).and_then(|t| t.as_table_like()) else {
            continue;
        };
        for (key, val) in tbl.iter() {
            // Renamed dependency: `alias = { package = "real-name" }`.
            let real = val
                .as_table_like()
                .and_then(|t| t.get("package"))
                .and_then(|p| p.as_str())
                .unwrap_or(key);
            names.push(real.to_string());
        }
    }
    names
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

    /// The oag scenario: `crates/*` globs `oag-cli` (package `oag`) before
    /// `oag-core` alphabetically, but `oag` depends on `oag-core`. The core
    /// crate must be published first.
    #[test]
    fn workspace_members_ordered_by_dependency() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("crates/oag-cli")).unwrap();
        std::fs::write(
            dir.path().join("crates/oag-cli/Cargo.toml"),
            "[package]\nname = \"oag\"\nversion = \"0.1.0\"\n\
             [dependencies]\noag-core = { path = \"../oag-core\", version = \"0.1.0\" }\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("crates/oag-core")).unwrap();
        std::fs::write(
            dir.path().join("crates/oag-core/Cargo.toml"),
            "[package]\nname = \"oag-core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let ws = resolve_targets(dir.path().to_str().unwrap(), true);
        let order: Vec<String> = ws
            .iter()
            .map(|p| read_cargo_package_name(p).unwrap())
            .collect();
        assert_eq!(order, vec!["oag-core".to_string(), "oag".to_string()]);
    }

    /// A renamed dependency (`alias = { package = "real" }`) still creates the
    /// ordering edge against the real crate name.
    #[test]
    fn ordering_honors_renamed_dependency() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"a-cli\", \"z-core\"]\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("a-cli")).unwrap();
        std::fs::write(
            dir.path().join("a-cli/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n\
             [dependencies]\ncore = { package = \"z-core\", path = \"../z-core\", version = \"0.1.0\" }\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("z-core")).unwrap();
        std::fs::write(
            dir.path().join("z-core/Cargo.toml"),
            "[package]\nname = \"z-core\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let ws = resolve_targets(dir.path().to_str().unwrap(), true);
        let order: Vec<String> = ws
            .iter()
            .map(|p| read_cargo_package_name(p).unwrap())
            .collect();
        assert_eq!(order, vec!["z-core".to_string(), "a".to_string()]);
    }

    /// Independent members keep a deterministic, stable order.
    #[test]
    fn ordering_is_stable_without_deps() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();
        for name in ["alpha", "bravo", "charlie"] {
            std::fs::create_dir_all(dir.path().join(format!("crates/{name}"))).unwrap();
            std::fs::write(
                dir.path().join(format!("crates/{name}/Cargo.toml")),
                format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n"),
            )
            .unwrap();
        }
        let ws = resolve_targets(dir.path().to_str().unwrap(), true);
        assert_eq!(ws.len(), 3);
    }

    /// A dependency cycle must terminate (best-effort order, no hang).
    #[test]
    fn ordering_tolerates_cycle() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"a\", \"b\"]\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        std::fs::write(
            dir.path().join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n\
             [dependencies]\nb = { path = \"../b\", version = \"0.1.0\" }\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("b")).unwrap();
        std::fs::write(
            dir.path().join("b/Cargo.toml"),
            "[package]\nname = \"b\"\nversion = \"0.1.0\"\n\
             [dependencies]\na = { path = \"../a\", version = \"0.1.0\" }\n",
        )
        .unwrap();
        let ws = resolve_targets(dir.path().to_str().unwrap(), true);
        assert_eq!(ws.len(), 2);
    }
}
