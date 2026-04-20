//! Run each package's build commands against the bumped workspace.
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
        let has_any = ctx.config.packages.iter().any(|p| !p.build.is_empty());
        if !has_any {
            return Ok(());
        }

        if ctx.dry_run {
            for pkg in &ctx.config.packages {
                for cmd in &pkg.build {
                    eprintln!("[dry-run] Would run build hook ({}): {cmd}", pkg.path);
                }
            }
            return Ok(());
        }

        for pkg in &ctx.config.packages {
            if pkg.build.is_empty() {
                continue;
            }
            crate::hooks::run_build(&pkg.build, ctx.hooks_env)?;
        }
        Ok(())
    }
}
