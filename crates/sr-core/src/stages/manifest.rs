//! Upload `sr-manifest.json` as the final asset on the release.
//!
//! Presence of this asset is proof the pipeline completed, including
//! post-release hooks. The reconciler reads it on subsequent runs to decide
//! whether the tag's release needs healing.

use std::path::Path;

use super::{Stage, StageContext};
use crate::error::ReleaseError;
use crate::manifest::{MANIFEST_ASSET_NAME, Manifest, utc_rfc3339_now};
use crate::release::resolve_globs;

pub struct UploadManifest;

impl Stage for UploadManifest {
    fn name(&self) -> &'static str {
        "upload_manifest"
    }

    fn is_complete(&self, ctx: &StageContext<'_>) -> Result<bool, ReleaseError> {
        if ctx.dry_run {
            return Ok(false);
        }
        let assets = ctx.vcs.list_assets(&ctx.plan.tag_name)?;
        Ok(assets.iter().any(|a| a == MANIFEST_ASSET_NAME))
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            eprintln!("[dry-run] Would upload {MANIFEST_ASSET_NAME} to {}", ctx.plan.tag_name);
            return Ok(());
        }

        // Resolve artifacts to their basenames — what was actually uploaded.
        let all_artifacts = ctx.config.all_artifacts();
        let resolved = resolve_globs(&all_artifacts).map_err(ReleaseError::Vcs)?;
        let artifact_basenames: Vec<String> = resolved
            .iter()
            .filter_map(|p| {
                Path::new(p)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_string())
            })
            .collect();

        let commit_sha = ctx.git.head_sha()?;

        let manifest = Manifest {
            sr_version: env!("CARGO_PKG_VERSION").to_string(),
            tag: ctx.plan.tag_name.clone(),
            commit_sha,
            artifacts: artifact_basenames,
            completed_at: utc_rfc3339_now(),
        };

        let json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| ReleaseError::Vcs(format!("failed to serialize manifest: {e}")))?;

        // GitHub derives the asset name from the URL parameter, but sr's
        // upload path derives it from the file's basename on disk — so the
        // file must be named sr-manifest.json. Use a unique temp dir
        // (tempfile handles concurrency safely) and write under the
        // canonical name inside it.
        let tmp_dir = tempfile::tempdir()
            .map_err(|e| ReleaseError::Vcs(format!("failed to create temp dir: {e}")))?;
        let final_path = tmp_dir.path().join(MANIFEST_ASSET_NAME);
        std::fs::write(&final_path, &json)
            .map_err(|e| ReleaseError::Vcs(format!("failed to write manifest: {e}")))?;

        let path_str = final_path.to_string_lossy().into_owned();
        ctx.vcs
            .upload_assets(&ctx.plan.tag_name, &[path_str.as_str()])?;
        // TempDir drop cleans up the directory automatically.

        eprintln!("Uploaded {MANIFEST_ASSET_NAME} to {}", ctx.plan.tag_name);
        Ok(())
    }
}
