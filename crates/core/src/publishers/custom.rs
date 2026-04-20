//! Custom publisher: user-supplied shell command + optional state check.
//!
//! - check: run `check` command (if provided). Exit 0 → Completed, nonzero
//!   → Needed. When absent, returns Unknown so `run` always executes.
//! - run: run `command` in `cwd` (defaults to package path).

use super::{PublishCtx, PublishState, Publisher};
use crate::error::ReleaseError;
use crate::hooks::run_shell;

pub struct CustomPublisher {
    pub command: String,
    pub check: Option<String>,
    pub cwd: Option<String>,
}

impl Publisher for CustomPublisher {
    fn name(&self) -> &'static str {
        "custom"
    }

    fn check(&self, ctx: &PublishCtx<'_>) -> Result<PublishState, ReleaseError> {
        let Some(check_cmd) = &self.check else {
            return Ok(PublishState::Unknown(
                "no `check` command configured".into(),
            ));
        };

        let cwd = self.cwd.clone().unwrap_or_else(|| ctx.package.path.clone());
        let wrapped = format!("cd {} && {}", shell_word(&cwd), check_cmd);

        // Run without propagating stderr to user unless it fails; exit 0 =
        // already published, anything else = needed. Transient failures
        // (e.g. network) would be reported as "needed" here — acceptable,
        // since the publish command itself will fail cleanly.
        match run_shell(&wrapped, None, ctx.env) {
            Ok(()) => Ok(PublishState::Completed),
            Err(_) => Ok(PublishState::Needed),
        }
    }

    fn run(&self, ctx: &PublishCtx<'_>) -> Result<(), ReleaseError> {
        let cwd = self.cwd.clone().unwrap_or_else(|| ctx.package.path.clone());
        let wrapped = format!("cd {} && {}", shell_word(&cwd), self.command);

        if ctx.dry_run {
            eprintln!("[dry-run] custom ({cwd}): {}", self.command);
            return Ok(());
        }

        eprintln!("custom ({cwd}): {}", self.command);
        run_shell(&wrapped, None, ctx.env)
    }
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
