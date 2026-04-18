//! Post-release verification. Warnings only — never fails the pipeline.

use super::{Stage, StageContext};
use crate::error::ReleaseError;

pub struct VerifyRelease;

impl Stage for VerifyRelease {
    fn name(&self) -> &'static str {
        "verify_release"
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            eprintln!("[dry-run] Would verify release: {}", ctx.plan.tag_name);
            return Ok(());
        }

        if let Err(e) = ctx.vcs.verify_release(&ctx.plan.tag_name) {
            eprintln!("warning: post-release verification failed: {e}");
            eprintln!(
                "  The tag {} was pushed but the GitHub release may be incomplete.",
                ctx.plan.tag_name
            );
            eprintln!("  Re-run with --force to retry.");
        }
        Ok(())
    }
}
