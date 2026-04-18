//! Stage bumped files + extra glob-matched paths, then commit.

use super::{Stage, StageContext};
use crate::error::ReleaseError;
use crate::release::resolve_globs;

pub struct Commit;

impl Stage for Commit {
    fn name(&self) -> &'static str {
        "commit"
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            return Ok(());
        }

        let mut paths_to_stage: Vec<String> = ctx.bumped_files.clone();
        if !ctx.active_package.stage_files.is_empty() {
            let extra =
                resolve_globs(&ctx.active_package.stage_files).map_err(ReleaseError::Config)?;
            paths_to_stage.extend(extra);
        }

        if !paths_to_stage.is_empty() {
            let refs: Vec<&str> = paths_to_stage.iter().map(|s| s.as_str()).collect();
            let commit_msg = format!("chore(release): {} [skip ci]", ctx.plan.tag_name);
            ctx.git.stage_and_commit(&refs, &commit_msg)?;
        }
        Ok(())
    }
}
