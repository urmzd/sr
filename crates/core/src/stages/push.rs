//! Push commit to origin; push tag to origin.

use super::{Stage, StageContext};
use crate::error::ReleaseError;
use crate::git::PushOutcome;

/// Push the release commit to origin. Always safe to re-run (idempotent
/// when up to date).
///
/// Pushes the exact release SHA — never `HEAD` — so commits that landed on
/// the branch after the release commit are not swept along. A non-fast-
/// forward rejection (the remote branch advanced past the base, e.g. queued
/// releases on a busy trunk) is a warning, not a failure: the release stays
/// locked to its commit, which remains reachable through the tag pushed by
/// the next stage.
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
        if ctx.git.push(&ctx.release_sha)? == PushOutcome::Rejected {
            eprintln!(
                "warning: branch advanced past {}; skipping branch push — \
                 the release commit remains reachable via tag {}",
                &ctx.plan.base_sha[..ctx.plan.base_sha.len().min(12)],
                ctx.plan.tag_name
            );
        }
        Ok(())
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
