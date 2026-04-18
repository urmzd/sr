//! Bump version files and write changelog to disk.
//!
//! Populates `ctx.bumped_files` so [`super::commit::Commit`] can stage them.

use std::fs;
use std::path::Path;

use super::{Stage, StageContext};
use crate::error::ReleaseError;
use crate::version_files::{bump_version_file, discover_lock_files, is_supported_version_file};

pub struct Bump;

impl Stage for Bump {
    fn name(&self) -> &'static str {
        "bump"
    }

    fn run(&self, ctx: &mut StageContext<'_>) -> Result<(), ReleaseError> {
        let version_files = ctx.config.version_files_for(ctx.active_package);
        let version_files_strict = ctx.active_package.version_files_strict;
        let changelog_file = ctx.config.changelog_for(ctx.active_package).file.clone();

        if ctx.dry_run {
            for file in &version_files {
                let filename = Path::new(file)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if is_supported_version_file(filename) {
                    eprintln!("[dry-run] Would bump version in: {file}");
                } else if version_files_strict {
                    return Err(ReleaseError::VersionBump(format!(
                        "unsupported version file: {filename}"
                    )));
                } else {
                    eprintln!("[dry-run] warning: unsupported version file, would skip: {file}");
                }
            }
            if !ctx.active_package.stage_files.is_empty() {
                eprintln!(
                    "[dry-run] Would stage additional files: {}",
                    ctx.active_package.stage_files.join(", ")
                );
            }
            return Ok(());
        }

        let mut files_to_stage: Vec<String> = Vec::new();
        for file in &version_files {
            match bump_version_file(Path::new(file), ctx.version_str) {
                Ok(extra) => {
                    files_to_stage.push(file.clone());
                    for extra_path in extra {
                        files_to_stage.push(extra_path.to_string_lossy().into_owned());
                    }
                }
                Err(e) if !version_files_strict => {
                    eprintln!("warning: {e} — skipping {file}");
                }
                Err(e) => return Err(e),
            }
        }

        // Auto-discover and stage lock files associated with bumped manifests
        for lock_file in discover_lock_files(&files_to_stage) {
            let lock_str = lock_file.to_string_lossy().into_owned();
            if !files_to_stage.contains(&lock_str) {
                files_to_stage.push(lock_str);
            }
        }

        // Write changelog file if configured. Path goes into bumped_files
        // so Commit stages it alongside the version files.
        if let Some(cf) = &changelog_file {
            let path = Path::new(cf);
            let existing = if path.exists() {
                fs::read_to_string(path).map_err(|e| ReleaseError::Changelog(e.to_string()))?
            } else {
                String::new()
            };
            let new_content = if existing.is_empty() {
                format!("# Changelog\n\n{}\n", ctx.changelog_body)
            } else {
                match existing.find("\n\n") {
                    Some(pos) => {
                        let (header, rest) = existing.split_at(pos);
                        format!("{header}\n\n{}\n{rest}", ctx.changelog_body)
                    }
                    None => format!("{existing}\n\n{}\n", ctx.changelog_body),
                }
            };
            fs::write(path, new_content).map_err(|e| ReleaseError::Changelog(e.to_string()))?;
        }

        // Changelog file is staged first so tests and diffs see a stable order.
        let mut ordered: Vec<String> = Vec::new();
        if let Some(cf) = changelog_file {
            ordered.push(cf);
        }
        ordered.extend(files_to_stage);
        ctx.bumped_files = ordered;
        Ok(())
    }
}
