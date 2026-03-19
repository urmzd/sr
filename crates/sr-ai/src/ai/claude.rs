use super::{AiBackend, AiRequest, AiResponse};
use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::process::Command;

pub struct ClaudeBackend {
    model: Option<String>,
    budget: f64,
    debug: bool,
}

impl ClaudeBackend {
    pub fn new(model: Option<String>, budget: f64, debug: bool) -> Self {
        Self {
            model,
            budget,
            debug,
        }
    }
}

#[async_trait]
impl AiBackend for ClaudeBackend {
    fn name(&self) -> &str {
        "claude"
    }

    async fn is_available(&self) -> bool {
        Command::new("claude")
            .arg("--version")
            .output()
            .await
            .is_ok_and(|o| o.status.success())
    }

    async fn request(&self, req: &AiRequest) -> Result<AiResponse> {
        let model = self.model.as_deref().unwrap_or("haiku");

        let mut cmd = Command::new("claude");
        cmd.current_dir(&req.working_dir)
            .arg("--model")
            .arg(model)
            .arg("--allowed-tools")
            .arg("Bash(git:*)")
            .arg("--output-format")
            .arg("json")
            .arg("--max-budget-usd")
            .arg(format!("{:.2}", self.budget))
            .arg("--system-prompt")
            .arg(&req.system_prompt)
            .arg("-p")
            .arg(&req.user_prompt);

        if let Some(schema) = &req.json_schema {
            cmd.arg("--json-schema").arg(schema);
        }

        if self.debug {
            eprintln!(
                "[DEBUG] Calling claude (model={model}, budget={:.2})",
                self.budget
            );
        }

        let output = cmd.output().await.context("failed to run claude CLI")?;

        let raw = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr);

        if self.debug {
            eprintln!("[DEBUG] Claude exit code: {}", output.status);
            eprintln!(
                "[DEBUG] Raw response (first 500 chars): {}",
                &raw[..raw.len().min(500)]
            );
            if !stderr.is_empty() {
                eprintln!("[DEBUG] Stderr: {stderr}");
            }
        }

        if !output.status.success() {
            anyhow::bail!(crate::error::SrAiError::AiBackend(format!(
                "claude CLI failed (exit {}): {}",
                output.status,
                stderr.trim()
            )));
        }

        // With --json-schema + --output-format json, structured output is in .structured_output
        if req.json_schema.is_some() {
            let parsed: serde_json::Value =
                serde_json::from_str(&raw).context("failed to parse claude JSON response")?;

            let structured = &parsed["structured_output"];
            if structured.is_null() {
                anyhow::bail!(crate::error::SrAiError::ParseResponse(
                    "empty structured_output from claude".into()
                ));
            }

            Ok(AiResponse {
                text: structured.to_string(),
            })
        } else {
            // Plain text mode — extract result from JSON envelope
            let parsed: serde_json::Value = serde_json::from_str(&raw)
                .map(|v: serde_json::Value| v.get("result").cloned().unwrap_or(v))
                .unwrap_or_else(|_| serde_json::Value::String(raw.clone()));

            let text = match &parsed {
                serde_json::Value::String(s) => s.clone(),
                _ => parsed.to_string(),
            };

            Ok(AiResponse { text })
        }
    }
}
