use thiserror::Error;

#[derive(Debug, Error)]
pub enum SrAiError {
    #[error("not in a git repository")]
    NotAGitRepo,

    #[error("no changes to commit")]
    NoChanges,

    #[error("no commits in plan")]
    EmptyPlan,

    #[error("git command failed: {0}")]
    GitCommand(String),

    #[error("AI backend failed: {0}")]
    AiBackend(String),

    #[error("no AI backend available (install `claude` or `gemini` CLI)")]
    NoBackendAvailable,

    #[error("failed to parse AI response: {0}")]
    ParseResponse(String),

    #[error("user cancelled")]
    Cancelled,
}
