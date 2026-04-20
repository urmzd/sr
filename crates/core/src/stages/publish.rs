//! Per-package publish stage.
//!
//! Iterates `config.packages[]`. For each package with a `publish` config,
//! dispatches to the typed publisher (cargo/npm/docker/pypi/go/custom),
//! which answers "already published?" via its registry API and runs the
//! tool's publish command if not.
//!
//! Stage `is_complete` returns false unconditionally — per-package state
//! is determined per-publisher at run time. The stage aggregates outcomes:
//! it stops the pipeline only if at least one configured package failed.
//! `AlreadyPublished` and `NotConfigured` never fail.

use super::{Stage, StageContext};
use crate::error::ReleaseError;
use crate::publish::{PublishOutcome, run_package_publish};

pub struct Publish;

impl Stage for Publish {
    fn name(&self) -> &'static str {
        "publish"
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        let has_any = ctx.config.packages.iter().any(|p| p.publish.is_some());
        if !has_any {
            return Ok(());
        }

        let mut outcomes: Vec<PublishOutcome> = Vec::with_capacity(ctx.config.packages.len());
        for pkg in &ctx.config.packages {
            let outcome = run_package_publish(
                pkg,
                ctx.version_str,
                &ctx.plan.tag_name,
                ctx.dry_run,
                ctx.hooks_env,
            );
            outcomes.push(outcome);
        }

        let failures: Vec<&PublishOutcome> = outcomes.iter().filter(|o| o.is_failure()).collect();

        if ctx.dry_run {
            return Ok(());
        }

        if failures.is_empty() {
            Ok(())
        } else {
            let messages: Vec<String> = failures
                .iter()
                .map(|o| match o {
                    PublishOutcome::Failed {
                        path,
                        publisher,
                        message,
                    } => format!("  [{publisher}] {path}: {message}"),
                    _ => String::new(),
                })
                .collect();
            Err(ReleaseError::Hook(format!(
                "{} package(s) failed to publish:\n{}",
                failures.len(),
                messages.join("\n")
            )))
        }
    }
}
