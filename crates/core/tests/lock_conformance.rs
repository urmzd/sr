//! Round-trip conformance: does sr's offline lock rewrite reproduce exactly
//! what the ecosystem's own resolver writes?
//!
//! sr edits `Cargo.lock`, `uv.lock`, `poetry.lock` and `package-lock.json` as
//! data so a release never needs a network resolve. That is only sound while
//! sr's output matches the real tool's byte for byte. The unit tests can't
//! prove that — they assert sr's edits against sr's own expectations, and would
//! keep passing after a format change. These tests can: each one builds a
//! fixture, locks it with the real tool, bumps it with sr, then re-locks and
//! demands the files be identical.
//!
//! They need the ecosystem tools on `PATH` and (for registry dependencies) a
//! network, so they're `#[ignore]`d by default:
//!
//! ```text
//! just conformance          # or: cargo test -p sr-core --test lock_conformance -- --ignored
//! ```
//!
//! A test whose tool is missing skips loudly rather than failing, so a partial
//! toolchain still reports on what it can check.
//!
//! When one of these fails, the fix is *not* to loosen the assertion: the
//! format moved, so re-verify the rewrite and update the matching
//! `*_LOCK_MAX_VERSION` constant in `version_files.rs`.

use std::fs;
use std::path::Path;
use std::process::Command;

use sr_core::version_files::{bump_version_file, sync_lock_files};

/// Run a command in `dir`, returning stdout+stderr on failure.
fn run(dir: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("failed to spawn {program}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "{program} {} failed:\n{}\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

fn tool_missing(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Bump every manifest, then sync locks once — the same order the bump stage
/// uses.
fn sr_bump(manifests: &[&Path], version: &str) {
    let mut staged = Vec::new();
    let mut names = Vec::new();
    for m in manifests {
        let outcome = bump_version_file(m, version).unwrap();
        staged.push(m.to_string_lossy().into_owned());
        for extra in outcome.extra_files {
            staged.push(extra.to_string_lossy().into_owned());
        }
        names.extend(outcome.package_names);
    }
    sync_lock_files(&staged, version, &names).unwrap();
}

/// Assert sr's edit matches what `relock` produces from the same manifests.
fn assert_matches_real_tool(lock: &Path, relock: impl FnOnce() -> Result<(), String>) {
    let sr_output = fs::read_to_string(lock).unwrap();
    relock().unwrap();
    let tool_output = fs::read_to_string(lock).unwrap();
    assert_eq!(
        sr_output,
        tool_output,
        "\nsr's offline rewrite of {} diverged from the real tool's output.\n\
         The lock format has moved: re-verify the rewrite in version_files.rs \
         and update the matching *_LOCK_MAX_VERSION constant.\n",
        lock.display()
    );
}

#[test]
#[ignore = "requires cargo and a network-reachable registry"]
fn cargo_lock_matches_cargo() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    write(
        &root.join("Cargo.toml"),
        r#"[workspace]
members = ["crates/*"]
resolver = "2"

[workspace.package]
version = "1.0.0"
edition = "2021"
"#,
    );
    write(
        &root.join("crates/core/Cargo.toml"),
        r#"[package]
name = "conformance-core"
version = "1.0.0"
edition = "2021"

[dependencies]
"#,
    );
    write(&root.join("crates/core/src/lib.rs"), "");
    write(
        &root.join("crates/cli/Cargo.toml"),
        r#"[package]
name = "conformance-cli"
version = "1.0.0"
edition = "2021"

[dependencies]
conformance-core = { path = "../core", version = "1.0.0" }
"#,
    );
    write(&root.join("crates/cli/src/lib.rs"), "");

    if tool_missing("cargo") {
        eprintln!("SKIP cargo_lock_matches_cargo: cargo not on PATH");
        return;
    }
    run(root, "cargo", &["generate-lockfile", "--offline"]).unwrap();

    sr_bump(&[&root.join("Cargo.toml")], "2.0.0");

    assert_matches_real_tool(&root.join("Cargo.lock"), || {
        run(root, "cargo", &["generate-lockfile", "--offline"])
    });
}

#[test]
#[ignore = "requires uv and a network-reachable registry"]
fn uv_lock_matches_uv() {
    if tool_missing("uv") {
        eprintln!("SKIP uv_lock_matches_uv: uv not on PATH");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    write(
        &root.join("pyproject.toml"),
        r#"[project]
name = "conformance-root"
version = "1.0.0"
requires-python = ">=3.9"

[tool.uv.workspace]
members = ["packages/*"]
"#,
    );
    // Registry dependencies and an environment marker — the parts of the lock
    // most likely to be reshaped by a format change.
    write(
        &root.join("packages/core/pyproject.toml"),
        r#"[project]
name = "conformance-core"
version = "1.0.0"
requires-python = ">=3.9"
dependencies = ["idna>=3", "certifi"]
"#,
    );
    write(
        &root.join("packages/api/pyproject.toml"),
        r#"[project]
name = "conformance-api"
version = "1.0.0"
requires-python = ">=3.9"
dependencies = [
    "conformance-core",
    "typing-extensions; python_version < '3.11'",
]

[tool.uv.sources]
conformance-core = { workspace = true }
"#,
    );

    run(root, "uv", &["lock"]).unwrap();

    sr_bump(&[&root.join("pyproject.toml")], "2.0.0");

    // uv must also accept the hand-edited lock as already current.
    run(root, "uv", &["lock", "--check", "--offline"])
        .expect("uv rejected sr's rewrite as out of date");

    assert_matches_real_tool(&root.join("uv.lock"), || {
        run(root, "uv", &["lock", "--offline"])
    });
}

#[test]
#[ignore = "requires poetry and a network-reachable registry"]
fn poetry_lock_matches_poetry() {
    if tool_missing("poetry") {
        eprintln!("SKIP poetry_lock_matches_poetry: poetry not on PATH");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    write(
        &root.join("packages/core/pyproject.toml"),
        r#"[project]
name = "conformance-core"
version = "1.0.0"
requires-python = ">=3.9"
"#,
    );
    write(
        &root.join("packages/api/pyproject.toml"),
        r#"[tool.poetry]
name = "conformance-api"
version = "1.0.0"
description = ""
authors = []

[tool.poetry.dependencies]
python = "^3.9"
conformance-core = { path = "../core", develop = true }

[build-system]
requires = ["poetry-core"]
build-backend = "poetry.core.masonry.api"
"#,
    );

    let api_dir = root.join("packages/api");
    run(&api_dir, "poetry", &["lock"]).unwrap();

    // The stale entry is the sibling's, recorded in the dependent's lock.
    sr_bump(
        &[
            &root.join("packages/api/pyproject.toml"),
            &root.join("packages/core/pyproject.toml"),
        ],
        "2.0.0",
    );

    assert_matches_real_tool(&api_dir.join("poetry.lock"), || {
        run(&api_dir, "poetry", &["lock"])
    });
}

#[test]
#[ignore = "requires npm"]
fn package_lock_matches_npm() {
    if tool_missing("npm") {
        eprintln!("SKIP package_lock_matches_npm: npm not on PATH");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    write(
        &root.join("package.json"),
        r#"{
  "name": "conformance-root",
  "version": "1.0.0",
  "workspaces": ["packages/*"]
}
"#,
    );
    write(
        &root.join("packages/core/package.json"),
        r#"{
  "name": "@conformance/core",
  "version": "1.0.0"
}
"#,
    );
    write(
        &root.join("packages/api/package.json"),
        r#"{
  "name": "@conformance/api",
  "version": "1.0.0",
  "dependencies": { "@conformance/core": "^1.0.0" }
}
"#,
    );

    run(root, "npm", &["install", "--package-lock-only"]).unwrap();

    sr_bump(&[&root.join("package.json")], "2.0.0");

    assert_matches_real_tool(&root.join("package-lock.json"), || {
        run(root, "npm", &["install", "--package-lock-only"])
    });
}
