//! Upload every declared artifact as a release asset.
//!
//! Users list each artifact as a literal path in `packages[].artifacts`.
//! No glob expansion — what you see is what gets uploaded.
//!
//! Idempotent: before uploading, checks which basenames are already present
//! on the release and skips those. Repeated runs are safe.

use std::collections::HashSet;
use std::path::Path;

use super::{Stage, StageContext};
use crate::error::ReleaseError;
use crate::release::resolve_paths;

pub struct UploadArtifacts;

impl UploadArtifacts {
    /// Compute (files_to_upload, files_to_skip) by diffing declared paths
    /// against the set of asset basenames already on the release.
    fn partition<'a>(
        resolved: &'a [String],
        existing: &HashSet<String>,
    ) -> (Vec<&'a str>, Vec<&'a str>) {
        let mut to_upload = Vec::new();
        let mut to_skip = Vec::new();
        for path in resolved {
            if existing.contains(basename(path)) {
                to_skip.push(path.as_str());
            } else {
                to_upload.push(path.as_str());
            }
        }
        (to_upload, to_skip)
    }
}

impl Stage for UploadArtifacts {
    fn name(&self) -> &'static str {
        "upload_artifacts"
    }

    /// Converged when every declared artifact is already attached to the
    /// release as an asset. Reconciler contract: read actual state (the
    /// release's asset list), compare to desired (declared paths, by
    /// basename), noop when they match. Local files are not consulted: a
    /// release that already carries every asset is complete whether or not
    /// the build output is still on disk.
    fn is_complete(&self, ctx: &StageContext<'_>) -> Result<bool, ReleaseError> {
        if ctx.dry_run {
            return Ok(false);
        }
        Ok(missing_from_release(ctx)?.is_empty())
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        let declared = ctx.config.all_artifacts();
        if declared.is_empty() {
            return Ok(());
        }

        let existing: HashSet<String> = ctx
            .vcs
            .list_assets(&ctx.plan.tag_name)?
            .into_iter()
            .collect();

        let (to_upload, to_skip) = Self::partition(&declared, &existing);

        // Literal-path resolution: every file still to upload must exist on
        // disk. Assets already on the release need no local copy.
        let to_upload: Vec<String> = to_upload.into_iter().map(String::from).collect();
        let resolved = resolve_paths(&to_upload).map_err(ReleaseError::Vcs)?;

        if ctx.dry_run {
            eprintln!("[dry-run] Would upload {} artifact(s):", resolved.len());
            for f in &resolved {
                eprintln!("[dry-run]   {f}");
            }
            return Ok(());
        }

        for skipped in &to_skip {
            eprintln!("skipping {} (already uploaded)", basename(skipped));
        }

        if !resolved.is_empty() {
            let files: Vec<&str> = resolved.iter().map(String::as_str).collect();
            ctx.vcs.upload_assets(&ctx.plan.tag_name, &files)?;
            eprintln!(
                "Uploaded {} artifact(s) to {}",
                files.len(),
                ctx.plan.tag_name
            );
        }
        Ok(())
    }
}

fn basename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

/// Declared artifacts whose basename is not yet an asset on the release.
pub(crate) fn missing_from_release(ctx: &StageContext<'_>) -> Result<Vec<String>, ReleaseError> {
    let declared = ctx.config.all_artifacts();
    if declared.is_empty() {
        return Ok(declared);
    }
    let on_release: HashSet<String> = ctx
        .vcs
        .list_assets(&ctx.plan.tag_name)?
        .into_iter()
        .collect();
    Ok(declared
        .into_iter()
        .filter(|p| !on_release.contains(basename(p)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_splits_known_and_missing() {
        let resolved = vec![
            "/tmp/out/app.tar.gz".to_string(),
            "/tmp/out/app.zip".to_string(),
            "/tmp/out/manual.json".to_string(),
        ];
        let mut existing = HashSet::new();
        existing.insert("app.tar.gz".to_string());

        let (to_upload, to_skip) = UploadArtifacts::partition(&resolved, &existing);
        assert_eq!(to_skip, vec!["/tmp/out/app.tar.gz"]);
        assert_eq!(to_upload, vec!["/tmp/out/app.zip", "/tmp/out/manual.json"]);
    }

    #[test]
    fn partition_all_existing_yields_empty_upload() {
        let resolved = vec!["/x/a.txt".into(), "/x/b.txt".into()];
        let existing: HashSet<String> = ["a.txt".to_string(), "b.txt".to_string()]
            .into_iter()
            .collect();
        let (to_upload, to_skip) = UploadArtifacts::partition(&resolved, &existing);
        assert!(to_upload.is_empty());
        assert_eq!(to_skip.len(), 2);
    }
}
