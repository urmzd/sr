use crate::ai::{AiRequest, BackendConfig, resolve_backend};
use crate::git::GitRepo;
use crate::ui;
use anyhow::Result;

#[derive(Debug, clap::Args)]
pub struct AskArgs {
    /// Question to ask about the repo
    pub question: Vec<String>,
}

const SYSTEM_PROMPT: &str = "You are an expert software engineer helping answer questions about a git repository. \
Use the available git tools to explore the codebase and answer the question thoroughly. \
Be specific and reference file paths when relevant.";

pub async fn run(args: &AskArgs, backend_config: &BackendConfig) -> Result<()> {
    let repo = GitRepo::discover()?;
    let backend = resolve_backend(backend_config).await?;

    let question = args.question.join(" ");
    if question.is_empty() {
        anyhow::bail!("please provide a question");
    }

    let spinner = ui::spinner(&format!("Thinking with {}...", backend.name()));

    let request = AiRequest {
        system_prompt: SYSTEM_PROMPT.to_string(),
        user_prompt: question,
        json_schema: None,
        working_dir: repo.root().to_string_lossy().to_string(),
    };

    let response = backend.request(&request, None).await?;
    spinner.finish_and_clear();

    println!("{}", response.text);

    Ok(())
}
