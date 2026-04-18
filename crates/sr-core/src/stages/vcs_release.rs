//! Create (or update-in-place) the GitHub release for the pushed tag.

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
