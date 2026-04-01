use super::{AiBackend, AiEvent, AiRequest, AiResponse};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::io::Write as _;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

/// Inline TOML policy that allows only read-only git commands and file reading.
/// Passed to `--policy` via a temporary file to replace the deprecated `--allowed-tools`.
const POLICY_TOML: &str = r#"
# Allow read-only git commands
[[rule]]
toolName = "run_shell_command"
commandRegex = "^git (status|log|diff|show|branch|tag|remote|rev-parse|ls-files|blame|shortlog|describe|cat-file|ls-tree|name-rev|reflog|rev-list)"
decision = "allow"
priority = 100

# Allow file reading, searching, and listing
[[rule]]
toolName = ["read_file", "glob", "grep_search", "list_directory"]
decision = "allow"
priority = 100

# Deny all other shell commands
[[rule]]
toolName = "run_shell_command"
decision = "deny"
priority = 50
denyMessage = "Only read-only git commands are permitted."

# Deny file writing/editing
[[rule]]
toolName = ["write_file", "replace"]
decision = "deny"
priority = 50
denyMessage = "Write operations are not permitted."

# Deny everything else
[[rule]]
toolName = "*"
decision = "deny"
priority = 10
denyMessage = "Only read-only operations are allowed."
"#;

pub struct GeminiBackend {
    model: Option<String>,
    debug: bool,
}

impl GeminiBackend {
    pub fn new(model: Option<String>, debug: bool) -> Self {
        Self { model, debug }
    }

    /// Streaming mode: --output-format stream-json for real-time tool call events.
    async fn request_streaming(
        &self,
        req: &AiRequest,
        events: mpsc::UnboundedSender<AiEvent>,
    ) -> Result<AiResponse> {
        let prompt = build_prompt(req);

        // Write policy to a temp file so it lives long enough for the process.
        let mut policy_file =
            tempfile::NamedTempFile::new().context("failed to create policy temp file")?;
        policy_file
            .write_all(POLICY_TOML.as_bytes())
            .context("failed to write policy file")?;

        let mut cmd = Command::new("gemini");
        cmd.current_dir(&req.working_dir)
            .arg("--prompt")
            .arg(&prompt)
            .arg("--sandbox")
            .arg("--policy")
            .arg(policy_file.path())
            .arg("--output-format")
            .arg("stream-json");

        if let Some(model) = &self.model {
            cmd.arg("--model").arg(model);
        }

        if self.debug {
            eprintln!(
                "[DEBUG] gemini stream-json (model={})",
                self.model.as_deref().unwrap_or("default")
            );
        }

        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("failed to run gemini CLI")?;

        let stdout = child.stdout.take().unwrap();
        let stderr_handle = child.stderr.take().unwrap();

        let stderr_task = tokio::spawn(async move {
            let mut buf = String::new();
            let _ = tokio::io::AsyncReadExt::read_to_string(
                &mut BufReader::new(stderr_handle),
                &mut buf,
            )
            .await;
            buf
        });

        let mut reader = BufReader::new(stdout).lines();
        let mut result_text = String::new();

        while let Ok(Some(line)) = reader.next_line().await {
            let event: serde_json::Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if self.debug {
                let t = event.get("type").and_then(|t| t.as_str()).unwrap_or("?");
                eprintln!("[DEBUG] event: {t}");
            }

            // Reuse Claude's tool call parser (same NDJSON event format)
            super::claude::parse_tool_calls(&event, &events);

            // Final result event
            if event.get("type").and_then(|t| t.as_str()) == Some("result")
                && let Some(r) = event.get("result")
            {
                let raw = match r {
                    serde_json::Value::String(s) => s.clone(),
                    _ => r.to_string(),
                };
                result_text = super::copilot::extract_json(&raw).unwrap_or(raw);
            }
        }

        let stderr_text = stderr_task.await.unwrap_or_default();
        let status = child.wait().await?;

        if !status.success() {
            anyhow::bail!(crate::error::SrAiError::AiBackend(format!(
                "gemini CLI failed (exit {}): {}",
                status,
                stderr_text.trim()
            )));
        }

        if result_text.is_empty() {
            anyhow::bail!(crate::error::SrAiError::ParseResponse(
                "no result in gemini stream".into()
            ));
        }

        Ok(AiResponse {
            text: result_text,
            usage: None,
        })
    }

    /// Batch mode: collect all output then parse.
    async fn request_batch(&self, req: &AiRequest) -> Result<AiResponse> {
        let prompt = build_prompt(req);

        let mut policy_file =
            tempfile::NamedTempFile::new().context("failed to create policy temp file")?;
        policy_file
            .write_all(POLICY_TOML.as_bytes())
            .context("failed to write policy file")?;

        let mut cmd = Command::new("gemini");
        cmd.current_dir(&req.working_dir)
            .arg("--prompt")
            .arg(&prompt)
            .arg("--sandbox")
            .arg("--policy")
            .arg(policy_file.path());

        if let Some(model) = &self.model {
            cmd.arg("--model").arg(model);
        }

        if self.debug {
            eprintln!(
                "[DEBUG] gemini (model={})",
                self.model.as_deref().unwrap_or("default")
            );
        }

        let output = cmd.output().await.context("failed to run gemini CLI")?;
        let raw = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr);

        if self.debug {
            eprintln!("[DEBUG] exit: {}", output.status);
            eprintln!("[DEBUG] stdout (first 500): {}", &raw[..raw.len().min(500)]);
            if !stderr.is_empty() {
                eprintln!("[DEBUG] stderr: {stderr}");
            }
        }

        if !output.status.success() {
            anyhow::bail!(crate::error::SrAiError::AiBackend(format!(
                "gemini CLI failed (exit {}): {}",
                output.status,
                stderr.trim()
            )));
        }

        let text = extract_json(&raw).unwrap_or(raw);
        Ok(AiResponse { text, usage: None })
    }
}

#[async_trait]
impl AiBackend for GeminiBackend {
    fn name(&self) -> &str {
        "gemini"
    }

    async fn is_available(&self) -> bool {
        Command::new("gemini")
            .arg("--help")
            .output()
            .await
            .is_ok_and(|o| o.status.success())
    }

    async fn request(
        &self,
        req: &AiRequest,
        events: Option<mpsc::UnboundedSender<AiEvent>>,
    ) -> Result<AiResponse> {
        match events {
            Some(tx) => self.request_streaming(req, tx).await,
            None => self.request_batch(req).await,
        }
    }
}

/// Build the full prompt, embedding the JSON schema when present.
fn build_prompt(req: &AiRequest) -> String {
    let mut prompt = format!("{}\n\n", req.system_prompt);

    if let Some(schema) = &req.json_schema {
        prompt.push_str(&format!(
            "You MUST respond with valid JSON matching this schema:\n```json\n{schema}\n```\n\n\
             Respond ONLY with the JSON object, no markdown fences, no explanation.\n\n"
        ));
    }

    prompt.push_str(&req.user_prompt);
    prompt
}

/// Extract JSON from a response that may contain markdown code fences.
fn extract_json(raw: &str) -> Option<String> {
    let trimmed = raw.trim();

    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return Some(trimmed.to_string());
    }

    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            let json_str = after[..end].trim();
            if serde_json::from_str::<serde_json::Value>(json_str).is_ok() {
                return Some(json_str.to_string());
            }
        }
    }

    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        let after = if let Some(nl) = after.find('\n') {
            &after[nl + 1..]
        } else {
            after
        };
        if let Some(end) = after.find("```") {
            let json_str = after[..end].trim();
            if serde_json::from_str::<serde_json::Value>(json_str).is_ok() {
                return Some(json_str.to_string());
            }
        }
    }

    for (open, close) in [("{", "}"), ("[", "]")] {
        if let Some(start) = trimmed.find(open)
            && let Some(end) = trimmed.rfind(close)
            && end > start
        {
            let candidate = &trimmed[start..=end];
            if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                return Some(candidate.to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_direct_json() {
        let input = r#"{"commits": []}"#;
        assert_eq!(extract_json(input), Some(input.to_string()));
    }

    #[test]
    fn extract_from_fences() {
        let input = "Here is the plan:\n```json\n{\"commits\": []}\n```\nDone.";
        assert_eq!(extract_json(input), Some(r#"{"commits": []}"#.to_string()));
    }

    #[test]
    fn extract_from_surrounding_text() {
        let input = "The result is {\"commits\": []} and that's it.";
        assert_eq!(extract_json(input), Some(r#"{"commits": []}"#.to_string()));
    }
}
