//! Run build hooks against the bumped workspace.
//!
//! Runs after [`super::bump::Bump`] writes new versions to disk and before
//! [`super::commit::Commit`] records them. Build failures leave the workspace
//! dirty but abort the pipeline before any tag/commit/push — `git checkout .`
//! heals it.

use super::{Stage, StageContext};
use crate::error::ReleaseError;

pub struct Build;

impl Stage for Build {
    fn name(&self) -> &'static str {
        "build"
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        let hooks = match &ctx.active_package.hooks {
            Some(h) if !h.build.is_empty() => h,
            _ => return Ok(()),
        };

        if ctx.dry_run {
            for cmd in &hooks.build {
                eprintln!("[dry-run] Would run build hook: {cmd}");
            }
            return Ok(());
        }

        crate::hooks::run_build(hooks, ctx.hooks_env)
    }
}
