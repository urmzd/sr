//! Stage bumped files + explicit `stage_files` from every package, then commit.

use super::{Stage, StageContext};
use crate::error::ReleaseError;
use crate::release::resolve_paths;

pub struct Commit;

impl Stage for Commit {
    fn name(&self) -> &'static str {
        "commit"
    }

    /// Idempotent recovery: if the release tag already exists, the release
    /// commit has already been made (local tags only exist at a commit).
    /// Skip to avoid re-attempting to stage files that are already committed.
    fn is_complete(&self, ctx: &StageContext<'_>) -> Result<bool, ReleaseError> {
        if ctx.dry_run {
            return Ok(false);
        }
        ctx.git.tag_exists(&ctx.plan.tag_name)
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            return Ok(());
        }

        let mut paths_to_stage: Vec<String> = ctx.bumped_files.clone();
        for pkg in &ctx.config.packages {
            if pkg.stage_files.is_empty() {
                continue;
            }
            let extra = resolve_paths(&pkg.stage_files).map_err(ReleaseError::Config)?;
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
