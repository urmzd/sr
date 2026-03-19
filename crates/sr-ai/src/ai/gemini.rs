use super::{AiBackend, AiRequest, AiResponse};
use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::process::Command;

pub struct GeminiBackend {
    model: Option<String>,
    debug: bool,
}

impl GeminiBackend {
    pub fn new(model: Option<String>, debug: bool) -> Self {
        Self { model, debug }
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

    async fn request(&self, req: &AiRequest) -> Result<AiResponse> {
        // Gemini CLI doesn't support --json-schema natively, so we embed the schema in the prompt
        let mut prompt = format!("{}\n\n", req.system_prompt);

        if let Some(schema) = &req.json_schema {
            prompt.push_str(&format!(
                "You MUST respond with valid JSON matching this schema:\n```json\n{schema}\n```\n\n\
                 Respond ONLY with the JSON object, no markdown fences, no explanation.\n\n"
            ));
        }

        prompt.push_str(&req.user_prompt);

        let mut cmd = Command::new("gemini");
        cmd.current_dir(&req.working_dir)
            .arg("--prompt")
            .arg(&prompt)
            .arg("--yolo");

        if let Some(model) = &self.model {
            cmd.arg("--model").arg(model);
        }

        if self.debug {
            eprintln!(
                "[DEBUG] Calling gemini (model={})",
                self.model.as_deref().unwrap_or("default")
            );
        }

        let output = cmd.output().await.context("failed to run gemini CLI")?;

        let raw = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr);

        if self.debug {
            eprintln!("[DEBUG] Gemini exit code: {}", output.status);
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
                "gemini CLI failed (exit {}): {}",
                output.status,
                stderr.trim()
            )));
        }

        // Extract JSON from response (may be wrapped in markdown fences)
        let text = extract_json(&raw).unwrap_or(raw);

        Ok(AiResponse { text })
    }
}

/// Extract JSON from a response that may contain markdown code fences.
fn extract_json(raw: &str) -> Option<String> {
    let trimmed = raw.trim();

    // Try direct parse first
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return Some(trimmed.to_string());
    }

    // Try extracting from ```json ... ``` fences
    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            let json_str = after[..end].trim();
            if serde_json::from_str::<serde_json::Value>(json_str).is_ok() {
                return Some(json_str.to_string());
            }
        }
    }

    // Try extracting from ``` ... ``` fences
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        // Skip optional language identifier on same line
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

    // Try finding first { ... } or [ ... ]
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
