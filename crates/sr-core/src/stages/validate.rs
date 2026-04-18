//! Enforce the "declared artifacts exist" contract before tagging.
//!
//! If `hooks.build` is configured, every pattern in `artifacts` must resolve
//! to ≥1 file on disk. A mismatch aborts the pipeline before tag creation,
//! guaranteeing that tag-on-remote implies all declared artifacts were built.

use super::{Stage, StageContext};
use crate::error::ReleaseError;
use crate::release::resolve_globs;

pub struct ValidateArtifacts;

impl Stage for ValidateArtifacts {
    fn name(&self) -> &'static str {
        "validate_artifacts"
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            return Ok(());
        }

        // Contract only applies when the user opted into build hooks.
        let has_build_hooks = ctx
            .active_package
            .hooks
            .as_ref()
            .map(|h| !h.build.is_empty())
            .unwrap_or(false);
        if !has_build_hooks {
            return Ok(());
        }

        let declared = ctx.config.all_artifacts();
        if declared.is_empty() {
            return Ok(());
        }

        let mut missing: Vec<&String> = Vec::new();
        for pattern in &declared {
            let slice = std::slice::from_ref(pattern);
            let resolved = resolve_globs(slice).map_err(ReleaseError::Vcs)?;
            if resolved.is_empty() {
                missing.push(pattern);
            }
        }

        if !missing.is_empty() {
            return Err(ReleaseError::Vcs(format!(
                "build completed but declared artifact patterns matched no files: {}. \
                 Fix the build or remove the patterns from sr.yaml.",
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }

        Ok(())
    }
}
