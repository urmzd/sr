use crate::ai::{AiRequest, BackendConfig, resolve_backend};
use crate::cache::{CacheLookup, CacheManager};
use crate::git::GitRepo;
use crate::ui;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitPlan {
    pub commits: Vec<PlannedCommit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedCommit {
    pub order: Option<u32>,
    pub message: String,
    pub body: Option<String>,
    pub footer: Option<String>,
    pub files: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub struct CommitArgs {
    /// Only analyze staged changes
    #[arg(short, long)]
    pub staged: bool,

    /// Additional context or instructions for commit generation
    #[arg(short = 'M', long)]
    pub message: Option<String>,

    /// Display plan without executing
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Skip confirmation prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Bypass cache (always call AI)
    #[arg(long)]
    pub no_cache: bool,
}

const COMMIT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {
        "commits": {
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "order": { "type": "integer" },
                    "message": { "type": "string", "description": "Header: type(scope): subject — imperative, lowercase, no period, max 72 chars" },
                    "body": { "type": "string", "description": "Body: explain WHY the change was made, wrap at 72 chars" },
                    "footer": { "type": "string", "description": "Footer: BREAKING CHANGE notes, Closes/Fixes/Refs #issue, etc." },
                    "files": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["order", "message", "body", "files"]
            }
        }
    },
    "required": ["commits"]
}"#;

const SYSTEM_PROMPT: &str = r#"You are an expert at analyzing git diffs and creating atomic, well-organized commits following the Angular Conventional Commits standard.

HEADER ("message" field):
- Must match this regex: (?s)(build|bump|chore|ci|docs|feat|fix|perf|refactor|revert|style|test)(\(\S+\))?!?: ([^\n\r]+)((\n\n.*)|(\\s*))?$
- Format: type(scope): subject
- Valid types ONLY: build, bump, chore, ci, docs, feat, fix, perf, refactor, revert, style, test
- NEVER invent types. Words like db, auth, api, etc. are scopes, not types. Use the semantically correct type for the change (e.g. feat(db): add user cache migration, fix(auth): resolve token expiry)
- scope is optional but recommended when applicable
- subject: imperative mood, lowercase first letter, no period at end, max 72 chars

BODY ("body" field — required):
- Explain WHY the change was made, not what changed (the diff shows that)
- Use imperative tense ("add" not "added")
- Wrap at 72 characters

FOOTER ("footer" field — optional):
- BREAKING CHANGE: description of what breaks and migration path
- Closes #N, Fixes #N, Refs #N for issue references
- Only include when relevant

COMMIT ORGANIZATION:
- Each commit must be atomic: one logical change per commit
- Every changed file must appear in exactly one commit
- CRITICAL: A file must NEVER appear in more than one commit. The execution engine stages entire files, not individual hunks. Splitting one file across commits will fail.
- If one file contains multiple logical changes, place it in the most fitting commit and note the secondary changes in that commit's body.
- Order: infrastructure/config -> core library -> features -> tests -> docs
- File paths must be relative to the repository root and match exactly as git reports them"#;

enum CacheStatus {
    /// No cache used (--no-cache, or cache unavailable)
    None,
    /// Exact cache hit
    Cached,
    /// Incremental hit with delta info
    Incremental { cached: usize, reanalyzed: usize },
}

pub async fn run(args: &CommitArgs, backend_config: &BackendConfig) -> Result<()> {
    let repo = GitRepo::discover()?;

    // Check for changes
    let has_changes = if args.staged {
        repo.has_staged_changes()?
    } else {
        repo.has_any_changes()?
    };

    if !has_changes {
        bail!(crate::error::SrAiError::NoChanges);
    }

    // Resolve AI backend
    let backend = resolve_backend(backend_config).await?;
    let backend_name = backend.name().to_string();
    let model_name = backend_config
        .model
        .as_deref()
        .unwrap_or("default")
        .to_string();

    // Build cache manager (may be None if cache dir unavailable)
    let cache = if args.no_cache {
        None
    } else {
        CacheManager::new(
            repo.root(),
            args.staged,
            args.message.as_deref(),
            &backend_name,
            &model_name,
        )
    };

    // Cache lookup
    let (mut plan, cache_status) = match cache.as_ref().map(|c| c.lookup()) {
        Some(CacheLookup::ExactHit(cached_plan)) => (cached_plan, CacheStatus::Cached),
        Some(CacheLookup::IncrementalHit {
            previous_plan,
            delta_summary,
        }) => {
            let cached_count = previous_plan.commits.len();

            let spinner = ui::spinner(&format!(
                "Analyzing changes with {} (incremental)...",
                backend_name
            ));

            let user_prompt =
                build_incremental_prompt(args, &repo, &previous_plan, &delta_summary)?;

            let request = AiRequest {
                system_prompt: SYSTEM_PROMPT.to_string(),
                user_prompt,
                json_schema: Some(COMMIT_SCHEMA.to_string()),
                working_dir: repo.root().to_string_lossy().to_string(),
            };

            let response = backend.request(&request).await?;
            spinner.finish_and_clear();

            let p: CommitPlan = serde_json::from_str(&response.text)
                .context("failed to parse commit plan from AI response")?;

            (
                p,
                CacheStatus::Incremental {
                    cached: cached_count,
                    reanalyzed: 1, // at least one AI call was made
                },
            )
        }
        _ => {
            // Miss or no cache
            let spinner = ui::spinner(&format!("Analyzing changes with {}...", backend_name));

            let user_prompt = build_user_prompt(args, &repo)?;

            let request = AiRequest {
                system_prompt: SYSTEM_PROMPT.to_string(),
                user_prompt,
                json_schema: Some(COMMIT_SCHEMA.to_string()),
                working_dir: repo.root().to_string_lossy().to_string(),
            };

            let response = backend.request(&request).await?;
            spinner.finish_and_clear();

            let p: CommitPlan = serde_json::from_str(&response.text)
                .context("failed to parse commit plan from AI response")?;

            (p, CacheStatus::None)
        }
    };

    if plan.commits.is_empty() {
        bail!(crate::error::SrAiError::EmptyPlan);
    }

    // Validate: merge commits with shared files
    plan = validate_plan(plan);

    // Store in cache (before display/execute so dry-runs populate cache too)
    if let Some(cache) = &cache {
        cache.store(&plan, &backend_name, &model_name);
    }

    // Display plan with cache status indicator
    match &cache_status {
        CacheStatus::Cached => println!("[cached]"),
        CacheStatus::Incremental { cached, reanalyzed } => {
            println!("[incremental: {cached} cached, {reanalyzed} re-analyzed]")
        }
        CacheStatus::None => {}
    }
    ui::display_plan(&plan);

    if args.dry_run {
        println!();
        println!("(dry run - no commits created)");
        return Ok(());
    }

    // Confirm
    if !args.yes {
        println!();
        if !ui::confirm("Execute this plan? [y/N]")? {
            bail!(crate::error::SrAiError::Cancelled);
        }
    }

    // Execute
    execute_plan(&repo, &plan)?;

    Ok(())
}

fn build_user_prompt(args: &CommitArgs, repo: &GitRepo) -> Result<String> {
    let git_root = repo.root().to_string_lossy();

    let mut prompt = if args.staged {
        "Analyze the staged git changes and group them into atomic commits.\n\
         Use `git diff --cached` and `git diff --cached --stat` to inspect what's staged."
            .to_string()
    } else {
        "Analyze all git changes (staged, unstaged, and untracked) and group them into atomic commits.\n\
         Use `git diff HEAD`, `git diff --cached`, `git diff`, `git status --porcelain`, and \
         `git ls-files --others --exclude-standard` to inspect changes."
            .to_string()
    };

    prompt.push_str(&format!("\nThe git repository root is: {git_root}"));

    if let Some(msg) = &args.message {
        prompt.push_str(&format!("\n\nAdditional context from the user:\n{msg}"));
    }

    Ok(prompt)
}

fn build_incremental_prompt(
    args: &CommitArgs,
    repo: &GitRepo,
    previous_plan: &CommitPlan,
    delta_summary: &str,
) -> Result<String> {
    let mut prompt = build_user_prompt(args, repo)?;

    let previous_json =
        serde_json::to_string_pretty(previous_plan).unwrap_or_else(|_| "{}".to_string());

    prompt.push_str(&format!(
        "\n\n--- INCREMENTAL HINTS ---\n\
         A previous commit plan exists for a similar set of changes. \
         Maintain the groupings for unchanged files where possible. \
         Only re-analyze files that have changed.\n\n\
         Previous plan:\n```json\n{previous_json}\n```\n\n\
         File delta:\n{delta_summary}"
    ));

    Ok(prompt)
}

/// Validate that no file appears in multiple commits. If duplicates are found,
/// merge affected commits into one.
fn validate_plan(plan: CommitPlan) -> CommitPlan {
    // Count file occurrences
    let mut file_counts: HashMap<String, usize> = HashMap::new();
    for commit in &plan.commits {
        for file in &commit.files {
            *file_counts.entry(file.clone()).or_default() += 1;
        }
    }

    let dupes: Vec<&String> = file_counts
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(file, _)| file)
        .collect();

    if dupes.is_empty() {
        return plan;
    }

    eprintln!();
    eprintln!("Notice: shared files detected across commits — merging affected commits.");
    eprintln!(
        "Shared files: {}",
        dupes
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );

    // Partition into tainted (has any dupe file) and clean
    let mut tainted = Vec::new();
    let mut clean = Vec::new();

    for commit in plan.commits {
        let is_tainted = commit.files.iter().any(|f| dupes.contains(&f));
        if is_tainted {
            tainted.push(commit);
        } else {
            clean.push(commit);
        }
    }

    // Merge all tainted commits into one
    let merged_message = tainted
        .first()
        .map(|c| c.message.clone())
        .unwrap_or_default();

    let merged_body = tainted
        .iter()
        .filter_map(|c| c.body.as_ref())
        .filter(|b| !b.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n\n");

    let merged_footer = tainted
        .iter()
        .filter_map(|c| c.footer.as_ref())
        .filter(|f| !f.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    let mut merged_files: Vec<String> = tainted
        .iter()
        .flat_map(|c| c.files.iter().cloned())
        .collect();
    merged_files.sort();
    merged_files.dedup();

    let merged_commit = PlannedCommit {
        order: Some(1),
        message: merged_message,
        body: if merged_body.is_empty() {
            None
        } else {
            Some(merged_body)
        },
        footer: if merged_footer.is_empty() {
            None
        } else {
            Some(merged_footer)
        },
        files: merged_files,
    };

    // Re-number: merged first, then clean commits
    let mut result = vec![merged_commit];
    for (i, mut commit) in clean.into_iter().enumerate() {
        commit.order = Some(i as u32 + 2);
        result.push(commit);
    }

    CommitPlan { commits: result }
}

fn execute_plan(repo: &GitRepo, plan: &CommitPlan) -> Result<()> {
    // Unstage everything first
    repo.reset_head()?;

    let total = plan.commits.len();

    for (i, commit) in plan.commits.iter().enumerate() {
        println!();
        println!("Creating commit {}/{total}: {}", i + 1, commit.message);

        // Stage files for this commit
        for file in &commit.files {
            if !repo.stage_file(file)? {
                eprintln!("  Warning: file not found: {file}");
            }
        }

        // Build full commit message
        let mut full_message = commit.message.clone();
        if let Some(body) = &commit.body
            && !body.is_empty()
        {
            full_message.push_str("\n\n");
            full_message.push_str(body);
        }
        if let Some(footer) = &commit.footer
            && !footer.is_empty()
        {
            full_message.push_str("\n\n");
            full_message.push_str(footer);
        }

        // Create commit (only if there are staged files)
        if repo.has_staged_after_add()? {
            repo.commit(&full_message)?;
        } else {
            eprintln!(
                "  Warning: no files staged for this commit (may already be committed or missing)"
            );
        }
    }

    println!();
    println!("Done! Recent commits:");
    println!("{}", repo.recent_commits(total)?);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_plan_no_dupes() {
        let plan = CommitPlan {
            commits: vec![
                PlannedCommit {
                    order: Some(1),
                    message: "feat: add foo".into(),
                    body: Some("reason".into()),
                    footer: None,
                    files: vec!["a.rs".into()],
                },
                PlannedCommit {
                    order: Some(2),
                    message: "fix: fix bar".into(),
                    body: Some("reason".into()),
                    footer: None,
                    files: vec!["b.rs".into()],
                },
            ],
        };

        let result = validate_plan(plan);
        assert_eq!(result.commits.len(), 2);
    }

    #[test]
    fn validate_plan_merges_dupes() {
        let plan = CommitPlan {
            commits: vec![
                PlannedCommit {
                    order: Some(1),
                    message: "feat: add foo".into(),
                    body: Some("reason 1".into()),
                    footer: None,
                    files: vec!["shared.rs".into(), "a.rs".into()],
                },
                PlannedCommit {
                    order: Some(2),
                    message: "fix: fix bar".into(),
                    body: Some("reason 2".into()),
                    footer: None,
                    files: vec!["shared.rs".into(), "b.rs".into()],
                },
                PlannedCommit {
                    order: Some(3),
                    message: "docs: update readme".into(),
                    body: Some("docs".into()),
                    footer: None,
                    files: vec!["README.md".into()],
                },
            ],
        };

        let result = validate_plan(plan);
        // Two tainted merged into one + one clean = 2
        assert_eq!(result.commits.len(), 2);
        assert_eq!(result.commits[0].message, "feat: add foo");
        assert!(result.commits[0].files.contains(&"shared.rs".to_string()));
        assert!(result.commits[0].files.contains(&"a.rs".to_string()));
        assert!(result.commits[0].files.contains(&"b.rs".to_string()));
        assert_eq!(result.commits[1].message, "docs: update readme");
        assert_eq!(result.commits[1].order, Some(2));
    }
}
