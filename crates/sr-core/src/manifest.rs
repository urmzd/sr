//! `sr-manifest.json` — the per-release completion record.
//!
//! sr uploads this file as the final asset on every release, **after** all
//! other stages (including `post_release` hooks) have succeeded. Presence of
//! the manifest on a tag's release is proof the pipeline finished; absence
//! means either (a) sr never cut this release (legacy/manual tag) or (b) sr
//! died mid-pipeline. sr can't distinguish those two remotely, so it warns
//! rather than blocks.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::error::ReleaseError;
use crate::release::VcsProvider;

/// File name of the manifest asset uploaded to every release.
pub const MANIFEST_ASSET_NAME: &str = "sr-manifest.json";

/// Completion record for a single release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// sr version that produced this release.
    pub sr_version: String,
    /// Tag name (e.g. "v1.2.3").
    pub tag: String,
    /// Full SHA of the commit the tag points to.
    pub commit_sha: String,
    /// Resolved asset basenames that were uploaded to the release.
    /// Reconciliation compares this against the release's actual asset list
    /// to detect partial uploads on re-runs.
    pub artifacts: Vec<String>,
    /// UTC RFC-3339 timestamp when the manifest was written.
    pub completed_at: String,
}

/// Verdict produced by the reconciler against a tag's remote state.
#[derive(Debug, Clone)]
pub enum ReleaseStatus {
    /// Manifest present — release is proven complete.
    Complete(Manifest),
    /// Manifest present but the declared artifacts don't match what's on the
    /// release (e.g. the manifest lists `sr-linux.tar.gz` but the release has
    /// no asset with that name). Indicates a partial re-upload.
    Incomplete {
        manifest: Manifest,
        missing_artifacts: Vec<String>,
    },
    /// No manifest asset on this release. Could be legacy (predates sr-manifest)
    /// or could be an sr release that died before the manifest was written.
    /// Reconciler can't distinguish these; warns instead of blocking.
    Unknown,
}

impl ReleaseStatus {
    pub fn is_complete(&self) -> bool {
        matches!(self, ReleaseStatus::Complete(_))
    }
}

/// Inspect the release for `tag` and classify its completion status.
///
/// - Manifest present + all declared artifacts on the release → `Complete`.
/// - Manifest present + some declared artifact missing → `Incomplete`.
/// - Manifest absent → `Unknown` (legacy release or sr died before uploading).
pub fn check_release_status<V: VcsProvider + ?Sized>(
    vcs: &V,
    tag: &str,
) -> Result<ReleaseStatus, ReleaseError> {
    let bytes = match vcs.fetch_asset(tag, MANIFEST_ASSET_NAME)? {
        Some(b) => b,
        None => return Ok(ReleaseStatus::Unknown),
    };
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| {
        ReleaseError::Vcs(format!(
            "failed to parse {MANIFEST_ASSET_NAME} on release {tag}: {e}"
        ))
    })?;

    let assets: HashSet<String> = vcs.list_assets(tag)?.into_iter().collect();
    let missing: Vec<String> = manifest
        .artifacts
        .iter()
        .filter(|a| !assets.contains(a.as_str()))
        .cloned()
        .collect();

    if missing.is_empty() {
        Ok(ReleaseStatus::Complete(manifest))
    } else {
        Ok(ReleaseStatus::Incomplete {
            manifest,
            missing_artifacts: missing,
        })
    }
}

/// Produce a UTC RFC-3339 timestamp without pulling in `chrono`.
pub(crate) fn utc_rfc3339_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // civil_from_days (Howard Hinnant) — same as release::today_string.
    let z = secs / 86400 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    let sod = secs.rem_euclid(86400);
    let h = sod / 3600;
    let min = (sod % 3600) / 60;
    let s = sod % 60;

    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trip_json() {
        let m = Manifest {
            sr_version: "7.1.0".into(),
            tag: "v1.2.3".into(),
            commit_sha: "a".repeat(40),
            artifacts: vec!["sr-linux.tar.gz".into(), "sr-macos.tar.gz".into()],
            completed_at: "2026-04-18T12:34:56Z".into(),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tag, "v1.2.3");
        assert_eq!(back.artifacts.len(), 2);
    }

    #[test]
    fn release_status_complete_is_complete() {
        let m = Manifest {
            sr_version: "7.1.0".into(),
            tag: "v1.0.0".into(),
            commit_sha: "abc".into(),
            artifacts: vec!["a".into()],
            completed_at: "t".into(),
        };
        assert!(ReleaseStatus::Complete(m).is_complete());
        assert!(!ReleaseStatus::Unknown.is_complete());
    }

    #[test]
    fn utc_rfc3339_now_is_well_formed() {
        let s = utc_rfc3339_now();
        assert_eq!(s.len(), 20, "got {s}");
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], "T");
    }
}
