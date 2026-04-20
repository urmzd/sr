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
use crate::release::{partition_paths, resolve_paths};

pub struct UploadArtifacts;

impl UploadArtifacts {
    /// Compute (files_to_upload, files_to_skip) by diffing resolved local
    /// paths against the set of asset basenames already on the release.
    fn partition<'a>(
        resolved: &'a [String],
        existing: &HashSet<String>,
    ) -> (Vec<&'a str>, Vec<&'a str>) {
        let mut to_upload = Vec::new();
        let mut to_skip = Vec::new();
        for path in resolved {
            let basename = Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path.as_str());
            if existing.contains(basename) {
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
    /// basename), noop when they match.
    fn is_complete(&self, ctx: &StageContext<'_>) -> Result<bool, ReleaseError> {
        if ctx.dry_run {
            return Ok(false);
        }
        let declared = ctx.config.all_artifacts();
        if declared.is_empty() {
            return Ok(true);
        }
        let (existing_on_disk, missing_on_disk) = partition_paths(&declared);
        if !missing_on_disk.is_empty() {
            // Files declared but not yet built — not complete.
            return Ok(false);
        }
        let existing_on_release: HashSet<String> = ctx
            .vcs
            .list_assets(&ctx.plan.tag_name)?
            .into_iter()
            .collect();
        let all_present = existing_on_disk.iter().all(|p| {
            let basename = Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(p.as_str());
            existing_on_release.contains(basename)
        });
        Ok(all_present)
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        let declared = ctx.config.all_artifacts();
        if declared.is_empty() {
            return Ok(());
        }

        // Literal-path resolution: every declared file must exist on disk.
        let resolved = resolve_paths(&declared).map_err(ReleaseError::Vcs)?;

        if ctx.dry_run {
            eprintln!("[dry-run] Would upload {} artifact(s):", resolved.len());
            for f in &resolved {
                eprintln!("[dry-run]   {f}");
            }
            return Ok(());
        }

        let existing: HashSet<String> = ctx
            .vcs
            .list_assets(&ctx.plan.tag_name)?
            .into_iter()
            .collect();

        let (to_upload, to_skip) = Self::partition(&resolved, &existing);

        for skipped in &to_skip {
            let basename = Path::new(skipped)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(skipped);
            eprintln!("skipping {basename} (already uploaded)");
        }

        if !to_upload.is_empty() {
            ctx.vcs.upload_assets(&ctx.plan.tag_name, &to_upload)?;
            eprintln!(
                "Uploaded {} artifact(s) to {}",
                to_upload.len(),
                ctx.plan.tag_name
            );
        }
        Ok(())
    }
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
