//! Release pipeline stages.
//!
//! Each stage exposes [`Stage::is_complete`] (has the work already been done,
//! against externally-observable state) and [`Stage::run`] (do the work).
//! The orchestrator in `release.rs` walks the pipeline: for each stage, if
//! it is not complete, run it.
//!
//! sr runs only its own operations — no user shell commands. Artifact
//! builds are the user's responsibility between `sr prepare` and
//! `sr release`. The `Publish` stage shells out only to known tools
//! (`cargo publish`, `npm publish`, `docker buildx`, `uv publish`) or to
//! a user-configured `publish: custom` command.

use crate::config::Config;
use crate::error::ReleaseError;
use crate::git::GitRepository;
use crate::release::{ReleasePlan, VcsProvider};

pub mod bump;
pub mod commit;
pub mod publish;
pub mod push;
pub mod tag;
pub mod upload;
pub mod validate;
pub mod vcs_release;
pub mod verify;

/// Mutable state threaded through the pipeline.
///
/// Release stages loop `plan.packages` internally where per-package work is
/// needed (Bump, UploadArtifacts, Publish). There is no "active package" —
/// one version, one tag, one release event applies to every package at once.
/// `bumped_files` is populated across all packages by [`bump::Bump`] and
/// consumed by [`commit::Commit`] as a single staged set.
pub struct StageContext<'a> {
    pub plan: &'a ReleasePlan,
    pub config: &'a Config,
    pub git: &'a dyn GitRepository,
    pub vcs: &'a dyn VcsProvider,
    pub changelog_body: &'a str,
    pub release_name: &'a str,
    pub version_str: &'a str,
    pub hooks_env: &'a [(&'a str, &'a str)],
    pub dry_run: bool,
    pub sign_tags: bool,
    pub draft: bool,
    /// Files produced by [`bump::Bump`] across every package that
    /// [`commit::Commit`] must stage in a single commit.
    pub bumped_files: Vec<String>,
}

/// A single step in the release pipeline.
pub trait Stage {
    /// Short identifier for logs.
    fn name(&self) -> &'static str;

    /// Whether the stage's work is already reflected in external state.
    /// Default: always run. Override for stages with an observable
    /// completion marker (e.g. "tag exists on remote").
    fn is_complete(&self, _ctx: &StageContext<'_>) -> Result<bool, ReleaseError> {
        Ok(false)
    }

    /// Perform the stage's work.
    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError>;
}

/// Build the default trunk-release pipeline in execution order.
///
/// State model: the VCS is the only source of truth. Tag present = version
/// exists, release object present = sr got far enough to create it, assets
/// present = sr got far enough to upload them, registry contains package =
/// publish ran. sr writes no state asset of its own.
pub fn default_pipeline() -> Vec<Box<dyn Stage>> {
    vec![
        Box::new(bump::Bump),
        Box::new(validate::ValidateArtifacts),
        Box::new(commit::Commit),
        Box::new(tag::LocalTag),
        Box::new(push::PushCommit),
        Box::new(push::PushTag),
        Box::new(tag::FloatingTag),
        Box::new(vcs_release::CreateOrUpdateRelease),
        Box::new(upload::UploadArtifacts),
        Box::new(verify::VerifyRelease),
        Box::new(publish::Publish),
    ]
}
