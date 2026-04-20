//! Enforce the "declared artifacts exist" contract before tagging.
//!
//! If any package has `build` commands configured, every literal path in
//! that package's `artifacts` must exist on disk. A mismatch aborts the
//! pipeline before tag creation, guaranteeing that tag-on-remote implies
//! all declared artifacts were built.

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

        let has_build_commands = ctx.config.packages.iter().any(|p| !p.build.is_empty());
        if !has_build_commands {
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
                "build completed but declared artifacts are missing on disk: {}. \
                 Fix the build or remove the entries from sr.yaml.",
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
