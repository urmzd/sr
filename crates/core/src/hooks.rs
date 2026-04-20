//! Shell execution helper.
//!
//! The only thing in sr that runs user-visible shell commands is the
//! `Publish` stage — it shells out to `cargo publish` / `npm publish` /
//! `docker buildx build` / `uv publish` / `twine` when a typed publisher
//! needs to hit a registry, or to a `publish: custom` command.
//!
//! sr intentionally does not run user "hooks" (pre_release, post_release,
//! build). Those belong in the CI workflow around `sr plan` / `sr prepare` /
//! `sr release`, not inside sr.

use crate::error::ReleaseError;

/// Run a shell command (`sh -c`). Inherits stdio unless `stdin_data` is
/// provided (in which case stdin is piped). `env` is injected into the
/// child process; `RELSTATE_*` / `SR_VERSION` etc. flow through here.
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
}
