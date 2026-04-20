//! Repo-wide pre-/post-release hook stages.
//!
//! Hooks run once per release, not per package. Per-package work belongs in
//! `packages[].build` (build stage) or `packages[].publish` (`sr publish`).

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
        if let Some(ref hooks) = ctx.config.hooks {
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
        if let Some(ref hooks) = ctx.config.hooks {
            crate::hooks::run_post_release(hooks, ctx.hooks_env)?;
        }
        Ok(())
    }
}
