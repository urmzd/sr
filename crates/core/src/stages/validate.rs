//! Enforce the "declared artifacts exist" contract before tagging.
//!
//! Every literal path in `packages[].artifacts` must exist on disk before
//! the tag is created. A mismatch aborts the pipeline so a tag-on-remote
//! always implies every declared artifact is present.
//!
//! sr never builds artifacts itself; the user's CI produces them between
//! `sr prepare` and `sr release`. When `artifacts` is empty, this stage
//! is a no-op.

use std::path::Path;

use super::{Stage, StageContext};
use crate::error::ReleaseError;

pub struct ValidateArtifacts;

impl Stage for ValidateArtifacts {
    fn name(&self) -> &'static str {
        "validate_artifacts"
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            return Ok(());
        }

        let declared = ctx.config.all_artifacts();
        if declared.is_empty() {
            return Ok(());
        }

        let mut missing: Vec<&String> = Vec::new();
        for path in &declared {
            if !Path::new(path).is_file() {
                missing.push(path);
            }
        }

        if !missing.is_empty() {
            return Err(ReleaseError::Vcs(format!(
                "declared artifacts are missing on disk: {}. \
                 Build them in CI between `sr prepare` and `sr release`, \
                 or remove the entries from sr.yaml.",
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
