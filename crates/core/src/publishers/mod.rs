//! Publishers — typed adapters that know how to check + push per registry.
//!
//! The `Publisher` trait carries two operations: `check` queries the target
//! registry for the desired version (answering "is work needed?") and `run`
//! invokes the publishing tool. Built-in publishers (`cargo`, `npm`,
//! `docker`, `pypi`, `go`) know their registry's API and the command they
//! shell out to. `Custom` is the escape hatch for arbitrary commands.
//!
//! Publishers do not fetch from files or manage working directories beyond
//! the package path; the dispatcher reads package metadata (e.g. the
//! Cargo.toml package name) and hands it to the publisher.

use crate::config::{PackageConfig, PublishConfig};
use crate::error::ReleaseError;

pub mod cargo;
pub mod custom;
pub mod docker;
pub mod go;
pub mod npm;
pub mod pypi;

/// The reconciler verdict from a publisher's state check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishState {
    /// Registry already has this version — no work needed.
    Completed,
    /// Version is absent from the registry — run() should execute.
    Needed,
    /// Could not determine (network error, auth failure, registry 5xx,
    /// rate limit, unsupported). `run()` should still execute: the publish
    /// command itself is the authoritative check, and every built-in
    /// publish tool (`cargo publish`, `npm publish`, `uv publish`,
    /// `docker push`) is idempotent against an already-published version
    /// and exits non-zero with a distinct "already exists" message that
    /// surfaces in the job log. That trade-off is deliberate: a transient
    /// crates.io 503 should not block a release.
    Unknown(String),
}

/// Context handed to publishers for check + run. Everything a publisher
/// needs to decide "is work needed?" and "what command should I run?".
pub struct PublishCtx<'a> {
    /// Package config — publishers may read `path` and `version_files` to
    /// derive a package name or dockerfile.
    pub package: &'a PackageConfig,
    /// Version string being released (e.g. "1.2.3").
    pub version: &'a str,
    /// Full tag name (e.g. "v1.2.3").
    pub tag: &'a str,
    /// Dry run — publishers must not mutate state when true. `check` is
    /// always safe; `run` becomes a log-only preview.
    pub dry_run: bool,
    /// Env vars forwarded to any shell invocation (SR_VERSION, SR_TAG, etc.).
    pub env: &'a [(&'a str, &'a str)],
}

pub trait Publisher {
    /// Machine-readable name, for logs.
    fn name(&self) -> &'static str;

    /// Query the registry for the current state.
    fn check(&self, ctx: &PublishCtx<'_>) -> Result<PublishState, ReleaseError>;

    /// Perform the publish. Callers must have checked `check()` first;
    /// a `Completed` result should short-circuit before calling `run`.
    fn run(&self, ctx: &PublishCtx<'_>) -> Result<(), ReleaseError>;
}

/// Dispatch `PublishConfig` → boxed `Publisher`. Returns a trait object so
/// the Publish stage can call `check`/`run` uniformly.
pub fn publisher_for(cfg: &PublishConfig) -> Box<dyn Publisher> {
    match cfg {
        PublishConfig::Cargo {
            features,
            registry,
            workspace,
        } => Box::new(cargo::CargoPublisher {
            features: features.clone(),
            registry: registry.clone(),
            workspace: *workspace,
        }),
        PublishConfig::Npm {
            registry,
            access,
            workspace,
        } => Box::new(npm::NpmPublisher {
            registry: registry.clone(),
            access: access.clone(),
            workspace: *workspace,
        }),
        PublishConfig::Docker {
            image,
            platforms,
            dockerfile,
        } => Box::new(docker::DockerPublisher {
            image: image.clone(),
            platforms: platforms.clone(),
            dockerfile: dockerfile.clone(),
        }),
        PublishConfig::Pypi {
            repository,
            workspace,
        } => Box::new(pypi::PypiPublisher {
            repository: repository.clone(),
            workspace: *workspace,
        }),
        PublishConfig::Go => Box::new(go::GoPublisher),
        PublishConfig::Custom {
            command,
            check,
            cwd,
        } => Box::new(custom::CustomPublisher {
            command: command.clone(),
            check: check.clone(),
            cwd: cwd.clone(),
        }),
    }
}
