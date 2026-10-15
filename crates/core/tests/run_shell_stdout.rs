//! `run_shell` must never write to sr's stdout.
//!
//! sr's stdout carries only the JSON result, and the GitHub Action parses it
//! with `jq`. Publishers (`npm publish`, `cargo publish`, a `publish: custom`
//! command) print progress to stdout, so `run_shell` sends the child's stdout
//! to sr's stderr instead.
//!
//! The check needs real file descriptors, so the test re-runs this test
//! binary as a child process with its stdout and stderr captured
//! separately. In the child, `child_mode` calls `run_shell` with a command
//! that echoes to stdout. The parent then checks which stream the echo
//! reached.

use std::process::Command;

const CHILD_ENV: &str = "SR_RUN_SHELL_STDOUT_CHILD";
const MARKER: &str = "publisher-noise-7f3a";

#[test]
fn child_mode() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    sr_core::hooks::run_shell(&format!("echo {MARKER}"), None, &[]).unwrap();
}

#[test]
fn run_shell_child_stdout_goes_to_stderr() {
    let exe = std::env::current_exe().unwrap();
    let out = Command::new(exe)
        .args(["--exact", "child_mode", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, "1")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "child failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stdout.contains(MARKER),
        "publisher output leaked to stdout:\n{stdout}"
    );
    assert!(
        stderr.contains(MARKER),
        "publisher output missing from stderr:\n{stderr}"
    );
}
