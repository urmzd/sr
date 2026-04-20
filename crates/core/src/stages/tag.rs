//! Tag creation stages: local annotated tag and floating major-version tag.

use super::{Stage, StageContext};
use crate::error::ReleaseError;

/// Create the annotated release tag locally. Skips if it already exists.
pub struct LocalTag;

impl Stage for LocalTag {
    fn name(&self) -> &'static str {
        "local_tag"
    }

    fn is_complete(&self, ctx: &StageContext<'_>) -> Result<bool, ReleaseError> {
        if ctx.dry_run {
            return Ok(false);
        }
        ctx.git.tag_exists(&ctx.plan.tag_name)
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            let sign_label = if ctx.sign_tags { " (signed)" } else { "" };
            eprintln!(
                "[dry-run] Would create tag: {}{sign_label}",
                ctx.plan.tag_name
            );
            eprintln!("[dry-run] Would push commit and tag: {}", ctx.plan.tag_name);
            if let Some(ref floating) = ctx.plan.floating_tag_name {
                eprintln!("[dry-run] Would create/update floating tag: {floating}");
                eprintln!("[dry-run] Would force-push floating tag: {floating}");
            }
            return Ok(());
        }
        let tag_message = format!("{}\n\n{}", ctx.plan.tag_name, ctx.changelog_body);
        ctx.git
            .create_tag(&ctx.plan.tag_name, &tag_message, ctx.sign_tags)?;
        Ok(())
    }
}

/// Force-create and force-push the floating major-version tag (e.g. `v3`).
/// Always runs when configured — floating tags are meant to move.
///
/// Intentionally overrides no `is_complete`: unlike `LocalTag` (immutable,
/// idempotent after first write), the floating tag must move to the new
/// release commit on every run. On a partial-failure re-run this means the
/// floating tag advances before downstream stages (`vcs_release`, `upload`,
/// `publish`) finish; a subsequent re-run will noop the floating tag move
/// itself but complete the remaining stages. Ordering is safe because the
/// floating tag's only consumer is `@latest`-style action references, which
/// pin to the underlying commit — never to in-progress asset state.
pub struct FloatingTag;

impl Stage for FloatingTag {
    fn name(&self) -> &'static str {
        "floating_tag"
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        if ctx.dry_run {
            return Ok(());
        }
        if let Some(ref floating) = ctx.plan.floating_tag_name {
            ctx.git.force_create_tag(floating)?;
            ctx.git.force_push_tag(floating)?;
        }
        Ok(())
    }
}
