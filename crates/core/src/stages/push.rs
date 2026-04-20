//! Push commit to origin; push tag to origin.

use super::{Stage, StageContext};
use crate::error::ReleaseError;

/// Push HEAD to origin. Always safe to re-run (idempotent when up to date).
pub struct PushCommit;

impl Stage for PushCommit {
    fn name(&self) -> &'static str {
        "push_commit"
    }

    /// Idempotent recovery: if the tag is already on the remote, the commit
    /// it points to must also be there (tags require their target commit).
    fn is_complete(&self, ctx: &StageContext<'_>) -> Result<bool, ReleaseError> {
        if ctx.dry_run {
            return Ok(false);
        }
        ctx.git.remote_tag_exists(&ctx.plan.tag_name)
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            return Ok(());
        }
        ctx.git.push()
    }
}

/// Push the release tag to origin. Skips if the tag is already on the remote.
pub struct PushTag;

impl Stage for PushTag {
    fn name(&self) -> &'static str {
        "push_tag"
    }

    fn is_complete(&self, ctx: &StageContext<'_>) -> Result<bool, ReleaseError> {
        if ctx.dry_run {
            return Ok(false);
        }
        ctx.git.remote_tag_exists(&ctx.plan.tag_name)
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            return Ok(());
        }
        ctx.git.push_tag(&ctx.plan.tag_name)
    }
}
