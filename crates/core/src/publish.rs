//! Per-package publish orchestration.
//!
//! Dispatches each package's `publish` config to a typed `Publisher` that
//! knows how to query its registry (state check) and shell out to the
//! publishing tool (run). Stateless: the registry is the source of truth,
//! not sr. Every invocation asks every configured package "are you already
//! at this version?" and runs when the answer is no.

use crate::config::PackageConfig;
use crate::publishers::{PublishCtx, PublishState, publisher_for};

/// Outcome of a single package's publish attempt. Reported to callers
/// (the Publish stage, or CLI `sr publish`) for aggregate reporting.
#[derive(Debug, Clone)]
pub enum PublishOutcome {
    /// Package has no `publish` config — nothing to do.
    NotConfigured { path: String },
    /// Registry already has the target version — skipped.
    AlreadyPublished { path: String, publisher: String },
    /// Command ran to completion.
    Succeeded { path: String, publisher: String },
    /// Command failed. `message` carries the error text.
    Failed {
        path: String,
        publisher: String,
        message: String,
    },
}

impl PublishOutcome {
    pub fn path(&self) -> &str {
        match self {
            Self::NotConfigured { path }
            | Self::AlreadyPublished { path, .. }
            | Self::Succeeded { path, .. }
            | Self::Failed { path, .. } => path,
        }
    }

    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Run publish for a single package. Dispatches on the package's
/// `publish` config:
///   - `None` → [`PublishOutcome::NotConfigured`]
///   - `Some(cfg)` → build the matching `Publisher`, call `check`, and
///     either skip (Completed) or run (Needed/Unknown).
pub fn run_package_publish(
    package: &PackageConfig,
    version: &str,
    tag: &str,
    dry_run: bool,
    env: &[(&str, &str)],
) -> PublishOutcome {
    let Some(cfg) = package.publish.as_ref() else {
        return PublishOutcome::NotConfigured {
            path: package.path.clone(),
        };
    };

    let publisher = publisher_for(cfg);
    let pub_name = publisher.name().to_string();
    let ctx = PublishCtx {
        package,
        version,
        tag,
        dry_run,
        env,
    };

    match publisher.check(&ctx) {
        Ok(PublishState::Completed) => {
            eprintln!(
                "{pub_name} ({}): already at {version} — skipping",
                package.path
            );
            return PublishOutcome::AlreadyPublished {
                path: package.path.clone(),
                publisher: pub_name,
            };
        }
        Ok(PublishState::Needed) => {}
        Ok(PublishState::Unknown(reason)) => {
            eprintln!(
                "{pub_name} ({}): state check inconclusive ({reason}) — attempting publish",
                package.path
            );
        }
        Err(e) => {
            return PublishOutcome::Failed {
                path: package.path.clone(),
                publisher: pub_name,
                message: format!("check failed: {e}"),
            };
        }
    }

    match publisher.run(&ctx) {
        Ok(()) => PublishOutcome::Succeeded {
            path: package.path.clone(),
            publisher: pub_name,
        },
        Err(e) => PublishOutcome::Failed {
            path: package.path.clone(),
            publisher: pub_name,
            message: e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PackageConfig, PublishConfig};

    #[test]
    fn not_configured_when_publish_is_none() {
        let pkg = PackageConfig {
            path: ".".into(),
            ..Default::default()
        };
        let outcome = run_package_publish(&pkg, "1.0.0", "v1.0.0", false, &[]);
        assert!(matches!(outcome, PublishOutcome::NotConfigured { .. }));
    }

    #[test]
    fn custom_run_succeeds() {
        let pkg = PackageConfig {
            path: ".".into(),
            publish: Some(PublishConfig::Custom {
                command: "true".into(),
                check: None,
                cwd: Some(".".into()),
            }),
            ..Default::default()
        };
        let outcome = run_package_publish(&pkg, "1.0.0", "v1.0.0", false, &[]);
        assert!(matches!(outcome, PublishOutcome::Succeeded { .. }));
    }

    #[test]
    fn custom_run_failure_captured() {
        let pkg = PackageConfig {
            path: ".".into(),
            publish: Some(PublishConfig::Custom {
                command: "false".into(),
                check: None,
                cwd: Some(".".into()),
            }),
            ..Default::default()
        };
        let outcome = run_package_publish(&pkg, "1.0.0", "v1.0.0", false, &[]);
        assert!(matches!(outcome, PublishOutcome::Failed { .. }));
    }

    #[test]
    fn custom_check_completed_short_circuits_run() {
        // check: "true" → Completed → skip run (would fail).
        let pkg = PackageConfig {
            path: ".".into(),
            publish: Some(PublishConfig::Custom {
                command: "false".into(),
                check: Some("true".into()),
                cwd: Some(".".into()),
            }),
            ..Default::default()
        };
        let outcome = run_package_publish(&pkg, "1.0.0", "v1.0.0", false, &[]);
        assert!(matches!(outcome, PublishOutcome::AlreadyPublished { .. }));
    }

    #[test]
    fn go_always_completed() {
        let pkg = PackageConfig {
            path: ".".into(),
            publish: Some(PublishConfig::Go),
            ..Default::default()
        };
        let outcome = run_package_publish(&pkg, "1.0.0", "v1.0.0", false, &[]);
        assert!(matches!(outcome, PublishOutcome::AlreadyPublished { .. }));
    }

    #[test]
    fn unknown_check_still_runs() {
        // `custom` with no `check` returns PublishState::Unknown — the
        // same state any built-in publisher returns for a registry 5xx,
        // rate-limit, or auth failure. Verify the dispatcher does NOT
        // skip: Unknown → proceed to run(), relying on the publish tool's
        // own idempotency to handle "already published" cleanly.
        let pkg = PackageConfig {
            path: ".".into(),
            publish: Some(PublishConfig::Custom {
                command: "true".into(),
                check: None,
                cwd: Some(".".into()),
            }),
            ..Default::default()
        };
        let outcome = run_package_publish(&pkg, "1.0.0", "v1.0.0", false, &[]);
        assert!(
            matches!(outcome, PublishOutcome::Succeeded { .. }),
            "Unknown state should proceed to run, got {outcome:?}"
        );
    }

    #[test]
    fn dry_run_skips_execution() {
        // Command would fail, but dry-run returns a synthetic Succeeded.
        let pkg = PackageConfig {
            path: ".".into(),
            publish: Some(PublishConfig::Custom {
                command: "false".into(),
                check: None,
                cwd: Some(".".into()),
            }),
            ..Default::default()
        };
        let outcome = run_package_publish(&pkg, "1.0.0", "v1.0.0", true, &[]);
        // Dry-run of Custom.run returns Ok(()), so outcome is Succeeded.
        assert!(matches!(outcome, PublishOutcome::Succeeded { .. }));
    }
}
