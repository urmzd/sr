//! Create (or update-in-place) the GitHub release for the pushed tag.
//!
//! Reconciler contract: `run()` uses PATCH semantics when the release
//! already exists, which is idempotent — repeated updates with the same
//! body produce no observable change beyond `updated_at`. We could skip
//! the update entirely when the body matches, but that would require a
//! body-fetch round-trip; the current trade-off favors one write over two
//! reads + conditional write. The stage is still safe to re-run.

use super::{Stage, StageContext};
use crate::error::ReleaseError;

pub struct CreateOrUpdateRelease;

impl Stage for CreateOrUpdateRelease {
    fn name(&self) -> &'static str {
        "create_or_update_release"
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            let draft_label = if ctx.draft { " (draft)" } else { "" };
            eprintln!(
                "[dry-run] Would create GitHub release \"{}\" for {}{draft_label}",
                ctx.release_name, ctx.plan.tag_name
            );
            return Ok(());
        }

        if ctx.vcs.release_exists(&ctx.plan.tag_name)? {
            ctx.vcs.update_release(
                &ctx.plan.tag_name,
                ctx.release_name,
                ctx.changelog_body,
                ctx.plan.prerelease,
                ctx.draft,
            )?;
        } else {
            ctx.vcs.create_release(
                &ctx.plan.tag_name,
                ctx.release_name,
                ctx.changelog_body,
                ctx.plan.prerelease,
                ctx.draft,
            )?;
        }
        Ok(())
    }
}
