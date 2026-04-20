//! Go publisher: noop.
//!
//! Go modules are "published" by pushing a semver-prefixed git tag
//! (`v1.2.3` for root modules, `<path>/v1.2.3` for submodules). sr already
//! cuts the tag as part of the release pipeline, so by the time a package
//! with `publish: go` is evaluated, the work is done.
//!
//! `check` always returns `Completed`. `run` is a no-op.

use super::{PublishCtx, PublishState, Publisher};
use crate::error::ReleaseError;

pub struct GoPublisher;

impl Publisher for GoPublisher {
    fn name(&self) -> &'static str {
        "go"
    }

    fn check(&self, _ctx: &PublishCtx<'_>) -> Result<PublishState, ReleaseError> {
        Ok(PublishState::Completed)
    }

    fn run(&self, _ctx: &PublishCtx<'_>) -> Result<(), ReleaseError> {
        Ok(())
    }
}
