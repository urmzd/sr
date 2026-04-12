//! Hook execution for sr lifecycle events.
//!
//! Runs configured shell commands at sr lifecycle boundaries (pre/post for
//! each command). Hook context is passed as JSON via stdin so commands can
//! act on structured data (event name, version, files, etc).

use crate::config::{HookEvent, HooksConfig};
use crate::error::ReleaseError;

/// Context passed to hook commands as JSON on stdin.
#[derive(Debug, serde::Serialize)]
pub struct HookContext<'a> {
    /// The lifecycle event being fired.
    pub event: &'a str,
    /// Environment variables set for this hook (flattened as key-value pairs).
    #[serde(flatten)]
    pub env: std::collections::BTreeMap<&'a str, &'a str>,
}

/// Run all commands for a lifecycle event.
///
/// Each command receives:
/// - JSON context on stdin (event name + env vars as structured data)
/// - Environment variables (e.g. SR_VERSION, SR_TAG for release hooks)
pub fn run_event(
    config: &HooksConfig,
    event: HookEvent,
    env: &[(&str, &str)],
) -> Result<(), ReleaseError> {
    let commands = match config.hooks.get(&event) {
        Some(cmds) if !cmds.is_empty() => cmds,
        _ => return Ok(()),
    };

    let label = format!("{event:?}");

    // Build JSON context for stdin
    let mut env_map = std::collections::BTreeMap::new();
    for &(k, v) in env {
        env_map.insert(k, v);
    }
    let context = HookContext {
        event: &label,
        env: env_map,
    };
    let json = serde_json::to_string(&context)
        .map_err(|e| ReleaseError::Hook(format!("failed to serialize hook context: {e}")))?;

    for cmd in commands {
        eprintln!("hook [{label}]: {cmd}");
        run_shell(cmd, Some(&json), env)?;
    }

    Ok(())
}

/// Run a shell command (`sh -c`), optionally piping data to stdin and/or
/// injecting environment variables. Returns an error if the command exits
/// non-zero.
pub fn run_shell(
    cmd: &str,
    stdin_data: Option<&str>,
    env: &[(&str, &str)],
) -> Result<(), ReleaseError> {
    let mut child = {
        let mut builder = std::process::Command::new("sh");
        builder.args(["-c", cmd]);
        for &(k, v) in env {
            builder.env(k, v);
        }
        if stdin_data.is_some() {
            builder.stdin(std::process::Stdio::piped());
        } else {
            builder.stdin(std::process::Stdio::inherit());
        }
        builder
            .spawn()
            .map_err(|e| ReleaseError::Hook(format!("{cmd}: {e}")))?
    };

    if let Some(data) = stdin_data
        && let Some(ref mut stdin) = child.stdin
    {
        use std::io::Write;
        let _ = stdin.write_all(data.as_bytes());
    }

    let status = child
        .wait()
        .map_err(|e| ReleaseError::Hook(format!("{cmd}: {e}")))?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        return Err(ReleaseError::Hook(format!("{cmd} exited with code {code}")));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_shell_success() {
        run_shell("true", None, &[]).unwrap();
    }

    #[test]
    fn run_shell_failure() {
        let result = run_shell("false", None, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn run_shell_with_env() {
        run_shell("test \"$MY_VAR\" = hello", None, &[("MY_VAR", "hello")]).unwrap();
    }

    #[test]
    fn run_event_empty_config() {
        let config = HooksConfig::default();
        run_event(&config, HookEvent::PreRelease, &[]).unwrap();
    }

    #[test]
    fn run_event_simple_command() {
        use std::collections::BTreeMap;
        let mut hooks = BTreeMap::new();
        hooks.insert(HookEvent::PreRelease, vec!["true".to_string()]);
        let config = HooksConfig { hooks };
        run_event(&config, HookEvent::PreRelease, &[]).unwrap();
    }

    #[test]
    fn run_event_failure_aborts() {
        use std::collections::BTreeMap;
        let mut hooks = BTreeMap::new();
        hooks.insert(HookEvent::PreRelease, vec!["false".to_string()]);
        let config = HooksConfig { hooks };
        let result = run_event(&config, HookEvent::PreRelease, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn run_event_passes_env() {
        use std::collections::BTreeMap;
        let mut hooks = BTreeMap::new();
        hooks.insert(
            HookEvent::PostRelease,
            vec!["test \"$SR_VERSION\" = 1.2.3".to_string()],
        );
        let config = HooksConfig { hooks };
        run_event(&config, HookEvent::PostRelease, &[("SR_VERSION", "1.2.3")]).unwrap();
    }

    #[test]
    fn run_event_passes_json_stdin() {
        use std::collections::BTreeMap;
        let mut hooks = BTreeMap::new();
        // Read stdin JSON and verify it contains the event name
        hooks.insert(
            HookEvent::PreCommit,
            vec!["cat | grep -q PreCommit".to_string()],
        );
        let config = HooksConfig { hooks };
        run_event(&config, HookEvent::PreCommit, &[]).unwrap();
    }
}
