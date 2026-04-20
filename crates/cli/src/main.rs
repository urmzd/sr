use std::path::Path;
use std::process::ExitCode;

mod examples;

use clap::{CommandFactory, Parser, Subcommand};
use sr_core::changelog::DefaultChangelogFormatter;
use sr_core::commit::TypedCommitParser;
use sr_core::config::{Config, DEFAULT_CONFIG_FILE};
use sr_core::error::ReleaseError;
use sr_core::github::GitHubProvider;
use sr_core::native_git::NativeGitRepository;
use sr_core::release::{ReleaseStrategy, TrunkReleaseStrategy};

#[derive(Parser)]
#[command(name = "sr", about = "Release engineering CLI", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Execute a release (trunk flow: tag + GitHub release). Whole-repo —
    /// monorepos release one tag with one version line for every package.
    Release {
        /// Release channel (e.g. canary, rc, stable) — overrides config fields
        #[arg(long, short)]
        channel: Option<String>,

        /// Preview what would happen without making changes
        #[arg(long)]
        dry_run: bool,

        /// Glob patterns for artifact files to upload to the release (repeatable).
        /// In monorepos, extends the first package's `artifacts` list.
        #[arg(long = "artifacts")]
        artifacts: Vec<String>,

        /// Additional file globs to stage in the release commit (repeatable, e.g. Cargo.lock).
        /// In monorepos, extends the first package's `stage_files` list.
        #[arg(long = "stage-files")]
        stage_files: Vec<String>,

        /// Pre-release identifier (e.g. alpha, beta, rc). Produces versions like 1.2.0-alpha.1
        #[arg(long)]
        prerelease: Option<String>,

        /// Sign tags with GPG/SSH (git tag -s)
        #[arg(long)]
        sign_tags: bool,

        /// Create GitHub release as a draft (requires manual publishing)
        #[arg(long)]
        draft: bool,

        /// Override the git author/committer name for the release commit and tag
        #[arg(long = "git-user-name")]
        git_user_name: Option<String>,

        /// Override the git author/committer email for the release commit and tag
        #[arg(long = "git-user-email")]
        git_user_email: Option<String>,
    },

    /// Plan the next release: compute desired state (next version, intended
    /// outputs), scan actual state (tag, release, assets), print the diff.
    /// No side effects.
    Plan {
        /// Output format
        #[arg(long, default_value = "human")]
        format: PlanFormat,
    },

    /// Prepare the next release: bump version files and write the changelog
    /// to disk. No commit, no tag, no push, no publish. Idempotent — safe
    /// to run multiple times. CI uses this between `sr plan` and `sr release`
    /// so build steps (cargo build, npm pack, uv build) pick up the new
    /// version from the bumped manifest files.
    Prepare {
        /// Pre-release identifier (e.g. alpha, beta, rc). Produces versions like 1.2.0-alpha.1
        #[arg(long)]
        prerelease: Option<String>,

        /// Preview what would be written without touching files.
        #[arg(long)]
        dry_run: bool,
    },

    /// Validate and display resolved configuration
    Config {
        /// Show the fully resolved config with defaults applied
        #[arg(long)]
        resolved: bool,
    },

    /// Create a `sr.yaml` config. With no argument, auto-detects
    /// version files and writes the default template. With an example
    /// name, writes that example verbatim so you can edit it.
    Init {
        /// Name of a bundled example (e.g. "cargo-workspace", "pnpm-workspace",
        /// "uv-workspace", "docker", "multi-language", "custom"). Omit to
        /// use auto-detection. Use `--list` to see available examples.
        example: Option<String>,

        /// List available examples and exit.
        #[arg(long)]
        list: bool,

        /// Overwrite `sr.yaml` if it already exists.
        #[arg(long)]
        force: bool,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },

    /// Update sr to the latest version
    Update,

    /// Show migration guide
    Migrate,
}

#[derive(Clone, clap::ValueEnum)]
enum PlanFormat {
    Human,
    Json,
}

use sr_core::release::NoopVcsProvider;

fn build_local_strategy(
    config: Config,
    prerelease_id: Option<String>,
    draft: bool,
) -> anyhow::Result<
    TrunkReleaseStrategy<
        NativeGitRepository,
        NoopVcsProvider,
        TypedCommitParser,
        DefaultChangelogFormatter,
    >,
> {
    let (name, email) = (config.git.user.name.clone(), config.git.user.email.clone());
    let git = NativeGitRepository::open(Path::new("."))?.with_identity(name, email);
    let commit_types = config.commit.types.into_commit_types();
    let parser = TypedCommitParser::from_types(&commit_types);
    let formatter = DefaultChangelogFormatter::new(
        config.changelog.template.clone(),
        config.changelog.groups.clone(),
    );
    Ok(TrunkReleaseStrategy {
        git,
        vcs: NoopVcsProvider,
        parser,
        formatter,
        config,
        prerelease_id,
        draft,
    })
}

fn build_full_strategy(
    config: Config,
    prerelease_id: Option<String>,
    draft: bool,
) -> anyhow::Result<
    TrunkReleaseStrategy<
        NativeGitRepository,
        GitHubProvider,
        TypedCommitParser,
        DefaultChangelogFormatter,
    >,
> {
    let (name, email) = (config.git.user.name.clone(), config.git.user.email.clone());
    let git = NativeGitRepository::open(Path::new("."))?.with_identity(name, email);
    let (hostname, owner, repo) = git.parse_remote_full()?;

    let token = std::env::var("GH_TOKEN")
        .or_else(|_| std::env::var("GITHUB_TOKEN"))
        .map_err(|_| anyhow::anyhow!("neither GH_TOKEN nor GITHUB_TOKEN is set"))?;

    let git = git.with_http_auth(hostname.clone(), token.clone());
    let vcs = GitHubProvider::new(owner, repo, hostname, token);
    let commit_types = config.commit.types.into_commit_types();
    let parser = TypedCommitParser::from_types(&commit_types);
    let formatter = DefaultChangelogFormatter::new(
        config.changelog.template.clone(),
        config.changelog.groups.clone(),
    );

    Ok(TrunkReleaseStrategy {
        git,
        vcs,
        parser,
        formatter,
        config,
        prerelease_id,
        draft,
    })
}

fn is_no_release_error(err: &anyhow::Error) -> bool {
    if let Some(re) = err.downcast_ref::<ReleaseError>() {
        matches!(
            re,
            ReleaseError::NoCommits { .. } | ReleaseError::NoBump { .. }
        )
    } else {
        false
    }
}

fn load_config() -> anyhow::Result<Config> {
    let config_path = resolve_config_path();
    Config::load(&config_path).map_err(|e| anyhow::anyhow!("{e}"))
}

fn resolve_config_path() -> std::path::PathBuf {
    Config::find_config(Path::new("."))
        .unwrap_or_else(|| std::path::PathBuf::from(DEFAULT_CONFIG_FILE))
}

fn self_update() -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let current_version = env!("CARGO_PKG_VERSION");
    eprintln!("current version: {current_version}");

    let mut req = ureq::get("https://api.github.com/repos/urmzd/sr/releases/latest")
        .header("Accept", "application/vnd.github+json");
    if let Ok(token) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
        req = req.header("Authorization", format!("token {token}"));
    }

    #[derive(serde::Deserialize)]
    struct Asset {
        name: String,
        browser_download_url: String,
    }
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
        assets: Vec<Asset>,
    }

    let release: Release = req
        .call()
        .map_err(|e| anyhow::anyhow!("failed to fetch latest release: {e}"))?
        .into_body()
        .read_json()?;
    let latest = release.tag_name.trim_start_matches('v');

    if latest == current_version {
        eprintln!("already up to date");
        return Ok(());
    }

    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        (os, arch) => anyhow::bail!("unsupported platform: {os}/{arch}"),
    };

    let asset_name = format!("sr-{target}");
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| anyhow::anyhow!("no asset found for {asset_name}"))?;

    let expected_sha256 = release
        .assets
        .iter()
        .find(|a| a.name == format!("{asset_name}.sha256"))
        .and_then(|a| {
            let body = ureq::get(&a.browser_download_url).call().ok()?.into_body();
            let mut buf = Vec::new();
            body.into_reader().read_to_end(&mut buf).ok()?;
            let s = String::from_utf8(buf).ok()?;
            Some(s.split_whitespace().next()?.to_string())
        });

    eprintln!("downloading sr {latest} for {target}...");

    let body = ureq::get(&asset.browser_download_url)
        .call()
        .map_err(|e| anyhow::anyhow!("download failed: {e}"))?
        .into_body();
    let mut bytes = Vec::new();
    body.into_reader().read_to_end(&mut bytes)?;

    if let Some(expected) = &expected_sha256 {
        let actual: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if actual != *expected {
            anyhow::bail!("SHA256 mismatch: expected {expected}, got {actual}");
        }
    }

    let exe = std::env::current_exe()?;
    let backup = exe.with_extension("old");
    let tmp = exe.with_extension("new");

    std::fs::write(&tmp, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    if exe.exists() {
        std::fs::rename(&exe, &backup)?;
    }
    if let Err(e) = std::fs::rename(&tmp, &exe) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, &exe);
        }
        return Err(e.into());
    }
    let _ = std::fs::remove_file(&backup);

    eprintln!("updated: {current_version} → {latest}");
    Ok(())
}

/// Normalized summary emitted as JSON by `sr plan`, `sr prepare`, and
/// `sr release`. The shape is stable across verbs so the GitHub Action
/// (and any other consumer) can parse with a single `jq` path regardless
/// of mode.
#[derive(serde::Serialize)]
struct ReleaseSummary {
    version: String,
    previous_version: String,
    tag: String,
    bump: String,
    floating_tag: String,
    commit_count: usize,
}

fn release_summary(plan: &sr_core::release::ReleasePlan) -> ReleaseSummary {
    ReleaseSummary {
        version: plan.next_version.to_string(),
        previous_version: plan
            .current_version
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default(),
        tag: plan.tag_name.clone(),
        bump: plan.bump.to_string(),
        floating_tag: plan.floating_tag_name.as_deref().unwrap_or("").to_string(),
        commit_count: plan.commits.len(),
    }
}

fn print_migration_guide() {
    print!("{}", include_str!("../docs/migration.md"));
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init {
            example,
            list,
            force,
        } => {
            if list {
                print!("{}", examples::list_formatted());
                return Ok(());
            }

            let path = Path::new(DEFAULT_CONFIG_FILE);
            if path.exists() && !force {
                eprintln!(
                    "{DEFAULT_CONFIG_FILE} already exists (skipping, use --force to overwrite)"
                );
                return Ok(());
            }

            let body = if let Some(name) = example {
                match examples::find(&name) {
                    Some(e) => e.body.to_string(),
                    None => {
                        eprintln!("error: unknown example '{name}'\n");
                        eprint!("{}", examples::list_formatted());
                        return Err(anyhow::anyhow!("unknown example '{name}'"));
                    }
                }
            } else {
                let detected = sr_core::version_files::detect_version_files(Path::new("."));
                if !detected.is_empty() {
                    for f in &detected {
                        eprintln!("detected version file: {f}");
                    }
                }
                sr_core::config::default_config_template(&detected)
            };
            std::fs::write(path, body)?;
            eprintln!("wrote {DEFAULT_CONFIG_FILE}");

            let gitignore = Path::new(".gitignore");
            let needs_entry = if gitignore.exists() {
                let content = std::fs::read_to_string(gitignore)?;
                !content
                    .lines()
                    .any(|l| l.trim() == ".sr" || l.trim() == ".sr/")
            } else {
                true
            };
            if needs_entry {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(gitignore)?;
                writeln!(f, "\n# sr cache and worktrees\n.sr/")?;
                eprintln!("added .sr/ to .gitignore");
            }

            Ok(())
        }

        Commands::Config { resolved } => {
            let config_path = resolve_config_path();
            let config = Config::load(&config_path)?;
            if resolved {
                let yaml = serde_yaml_ng::to_string(&config)?;
                print!("{yaml}");
            } else if config_path.exists() {
                let raw = std::fs::read_to_string(&config_path)?;
                print!("{raw}");
            } else {
                eprintln!("no config file found; showing defaults");
                let yaml = serde_yaml_ng::to_string(&config)?;
                print!("{yaml}");
            }
            Ok(())
        }

        Commands::Plan { format } => {
            let config = load_config()?;

            let branch_output = std::process::Command::new("git")
                .args(["branch", "--show-current"])
                .output()?;
            let branch = String::from_utf8_lossy(&branch_output.stdout)
                .trim()
                .to_string();

            let strategy = build_local_strategy(config.clone(), None, false)?;
            let plan_result = strategy.plan();

            match (format, plan_result) {
                (PlanFormat::Json, Ok(plan)) => {
                    let env = [
                        ("SR_VERSION", plan.next_version.to_string()),
                        ("SR_TAG", plan.tag_name.clone()),
                    ];
                    let env_refs: Vec<(&str, &str)> =
                        env.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    let diff = sr_core::diff::build_diff(
                        &plan,
                        &strategy.git,
                        &strategy.vcs,
                        &config,
                        &env_refs,
                    )?;
                    #[derive(serde::Serialize)]
                    struct Out<'a> {
                        branch: &'a str,
                        #[serde(flatten)]
                        summary: &'a ReleaseSummary,
                        #[serde(flatten)]
                        diff: &'a sr_core::diff::ReleaseDiff,
                    }
                    let summary = release_summary(&plan);
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&Out {
                            branch: &branch,
                            summary: &summary,
                            diff: &diff,
                        })?
                    );
                }
                (PlanFormat::Json, Err(e)) => {
                    let msg = if matches!(
                        &e,
                        ReleaseError::NoCommits { .. } | ReleaseError::NoBump { .. }
                    ) {
                        "no unreleased changes"
                    } else {
                        "error"
                    };
                    println!("{{\"branch\":\"{branch}\",\"status\":\"{msg}\"}}");
                }
                (PlanFormat::Human, Ok(plan)) => {
                    println!("  Branch: {branch}");
                    let env = [
                        ("SR_VERSION", plan.next_version.to_string()),
                        ("SR_TAG", plan.tag_name.clone()),
                    ];
                    let env_refs: Vec<(&str, &str)> =
                        env.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    let diff = sr_core::diff::build_diff(
                        &plan,
                        &strategy.git,
                        &strategy.vcs,
                        &config,
                        &env_refs,
                    )?;
                    println!();
                    print!("{}", sr_core::diff::render_human(&diff));
                }
                (PlanFormat::Human, Err(e)) => {
                    println!("  Branch: {branch}");
                    match &e {
                        ReleaseError::NoCommits { .. } | ReleaseError::NoBump { .. } => {
                            println!("  No unreleased changes");
                        }
                        _ => println!("  Release: error — {e}"),
                    }
                }
            }
            Ok(())
        }

        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "sr", &mut std::io::stdout());
            Ok(())
        }

        Commands::Prepare {
            prerelease,
            dry_run,
        } => {
            let config = load_config()?;
            let channel_name = config.channels.default.clone();
            let resolved_channel = config.resolve_channel(&channel_name)?.clone();
            let prerelease_id = prerelease.or(resolved_channel.prerelease);
            let strategy = build_local_strategy(config, prerelease_id, false)?;
            let plan = strategy.plan()?;
            strategy.prepare(&plan, dry_run)?;
            let summary = release_summary(&plan);
            println!("{}", serde_json::to_string(&summary)?);
            Ok(())
        }

        Commands::Release {
            channel,
            dry_run,
            artifacts,
            stage_files,
            prerelease,
            sign_tags,
            draft,
            git_user_name,
            git_user_email,
        } => {
            let mut config = load_config()?;

            // Resolve channel
            let channel_name = channel.unwrap_or_else(|| config.channels.default.clone());
            let resolved_channel = config.resolve_channel(&channel_name)?.clone();

            // Channel provides prerelease/draft, CLI flags override
            let prerelease_id = prerelease.or(resolved_channel.prerelease);
            let draft = draft || resolved_channel.draft;

            // CLI overrides for git config
            if sign_tags {
                config.git.sign_tags = true;
            }

            // Git identity precedence: CLI flag > sr.yaml > env > git's own resolution.
            config.git.user.name = git_user_name
                .or(config.git.user.name.take())
                .or_else(|| std::env::var("SR_GIT_USER_NAME").ok());
            config.git.user.email = git_user_email
                .or(config.git.user.email.take())
                .or_else(|| std::env::var("SR_GIT_USER_EMAIL").ok());

            // CLI overrides for package config (apply to root package)
            if (!artifacts.is_empty() || !stage_files.is_empty())
                && let Some(pkg) = config.packages.first_mut()
            {
                pkg.artifacts.extend(artifacts);
                pkg.stage_files.extend(stage_files);
            }

            let plan = match build_full_strategy(config.clone(), prerelease_id.clone(), draft) {
                Ok(strategy) => {
                    let plan = strategy.plan()?;
                    strategy.execute(&plan, dry_run)?;
                    plan
                }
                Err(e) => {
                    if dry_run {
                        eprintln!("warning: {e} (continuing dry-run without GitHub)");
                        let strategy = build_local_strategy(config, prerelease_id, draft)?;
                        let plan = strategy.plan()?;
                        strategy.execute(&plan, dry_run)?;
                        plan
                    } else {
                        return Err(e);
                    }
                }
            };
            let output = release_summary(&plan);
            println!("{}", serde_json::to_string(&output)?);
            Ok(())
        }

        Commands::Update => self_update(),
        Commands::Migrate => {
            print_migration_guide();
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            if is_no_release_error(&e) {
                eprintln!("{e:#}");
                ExitCode::from(2)
            } else {
                eprintln!("error: {e:#}");
                ExitCode::from(1)
            }
        }
    }
}
