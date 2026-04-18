//! Hook execution for sr lifecycle events.
//!
//! Runs configured shell commands at sr lifecycle boundaries.
//! Hook context is passed as JSON via stdin so commands can
//! act on structured data.

use crate::config::HooksConfig;
use crate::error::ReleaseError;

/// Context passed to hook commands as JSON on stdin.
#[derive(Debug, serde::Serialize)]
pub struct HookContext<'a> {
    pub event: &'a str,
    #[serde(flatten)]
    pub env: std::collections::BTreeMap<&'a str, &'a str>,
}

/// Run a list of shell commands with environment variables.
pub fn run_commands(
    label: &str,
    commands: &[String],
    env: &[(&str, &str)],
) -> Result<(), ReleaseError> {
    if commands.is_empty() {
        return Ok(());
    }

    let mut env_map = std::collections::BTreeMap::new();
    for &(k, v) in env {
        env_map.insert(k, v);
    }
    let context = HookContext {
        event: label,
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

/// Run pre_release hooks from a HooksConfig.
pub fn run_pre_release(config: &HooksConfig, env: &[(&str, &str)]) -> Result<(), ReleaseError> {
    run_commands("pre_release", &config.pre_release, env)
}

/// Run build hooks from a HooksConfig.
/// Runs after version bump, before commit — output artifacts must match
/// the declared `artifacts` globs (sr validates this before tagging).
pub fn run_build(config: &HooksConfig, env: &[(&str, &str)]) -> Result<(), ReleaseError> {
    run_commands("build", &config.build, env)
}

/// Run post_release hooks from a HooksConfig.
pub fn run_post_release(config: &HooksConfig, env: &[(&str, &str)]) -> Result<(), ReleaseError> {
    run_commands("post_release", &config.post_release, env)
}

/// Run a shell command (`sh -c`), optionally piping data to stdin and/or
/// injecting environment variables.
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
    fn run_commands_empty() {
        run_commands("test", &[], &[]).unwrap();
    }

    #[test]
    fn run_commands_success() {
        run_commands("test", &["true".into()], &[]).unwrap();
    }

    #[test]
    fn run_commands_failure_aborts() {
        let result = run_commands("test", &["false".into()], &[]);
        assert!(result.is_err());
    }

    #[test]
    fn run_commands_passes_env() {
        run_commands(
            "test",
            &["test \"$SR_VERSION\" = 1.2.3".into()],
            &[("SR_VERSION", "1.2.3")],
        )
        .unwrap();
    }

    #[test]
    fn run_pre_release_empty() {
        let config = HooksConfig::default();
        run_pre_release(&config, &[]).unwrap();
    }

    #[test]
    fn run_post_release_empty() {
        let config = HooksConfig::default();
        run_post_release(&config, &[]).unwrap();
    }
}
