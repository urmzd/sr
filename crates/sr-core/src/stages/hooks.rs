//! Pre- and post-release hook stages.

use super::{Stage, StageContext};
use crate::error::ReleaseError;

pub struct PreReleaseHooks;

impl Stage for PreReleaseHooks {
    fn name(&self) -> &'static str {
        "pre_release_hooks"
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            return Ok(());
        }
        if let Some(ref hooks) = ctx.active_package.hooks {
            crate::hooks::run_pre_release(hooks, ctx.hooks_env)?;
        }
        Ok(())
    }
}

pub struct PostReleaseHooks;

impl Stage for PostReleaseHooks {
    fn name(&self) -> &'static str {
        "post_release_hooks"
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            return Ok(());
        }
        if let Some(ref hooks) = ctx.active_package.hooks {
            crate::hooks::run_post_release(hooks, ctx.hooks_env)?;
        }
        Ok(())
    }
}
