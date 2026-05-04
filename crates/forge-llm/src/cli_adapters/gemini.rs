//! Gemini CLI agent provider.
//!
//! Spawns `gemini -p "<prompt>" -o stream-json` and parses the JSONL output
//! stream. The CLI handles tool execution internally using Gemini's trained tools.
//!
//! Note: Gemini CLI's non-interactive mode is still stabilizing. This adapter
//! handles the known output format but may need updates as the CLI evolves.

use std::process::Stdio;
use std::time::Instant;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::agent_provider::{
    AgentLoopEvent, AgentProvider, AgentRunOptions, AgentRunResult, ToolActivityRecord,
};
use crate::errors::{ErrorInfo, ProviderError, ProviderErrorKind, SDKError};
use crate::types::Usage;

/// Agent provider that wraps the Gemini CLI.
pub struct GeminiAgentProvider {
    /// Path to the `gemini` binary. Defaults to "gemini".
    binary_path: String,
    /// Default model to request.
    model: Option<String>,
}

impl GeminiAgentProvider {
    pub fn new(binary_path: impl Into<String>) -> Self {
        Self {
            binary_path: binary_path.into(),
            model: None,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    fn build_command(&self, prompt: &str, options: &AgentRunOptions) -> Command {
        let mut cmd = Command::new(&self.binary_path);
        cmd.arg("-p").arg(prompt).arg("-o").arg("stream-json");

        let model = options.model_override.as_deref().or(self.model.as_deref());
        if let Some(model) = model {
            cmd.arg("--model").arg(model);
        }

        cmd.current_dir(&options.working_directory);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        if let Some(env_vars) = &options.env_vars {
            for (key, value) in env_vars {
                cmd.env(key, value);
            }
        }

        cmd
    }
}

#[async_trait]
impl AgentProvider for GeminiAgentProvider {
    fn name(&self) -> &str {
        "gemini-cli"
    }

    async fn run_to_completion(
        &self,
        prompt: &str,
        options: &AgentRunOptions,
    ) -> Result<AgentRunResult, SDKError> {
        let start = Instant::now();
        let mut cmd = self.build_command(prompt, options);
        let mut child = cmd
            .spawn()
            .map_err(|e| gemini_error(&self.binary_path, e))?;

        let stdout = child.stdout.take().expect("stdout should be piped");
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        let mut final_text = String::new();
        let mut tool_activity = Vec::new();
        let mut total_usage = Usage::default();
        let mut call_counter = 0u64;
        let mut session_model = self
            .model
            .clone()
            .unwrap_or_else(|| "gemini-cli".to_string());

        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| gemini_io_error("reading stdout", e))?
        {
            let event: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Gemini CLI stream-json format:
            // - {"type":"init", "model":"...", "session_id":"..."} — session start
            // - {"type":"message", "role":"user", "content":"..."} — echoed input
            // - {"type":"message", "role":"assistant", "content":"...", "delta":true} — response text
            // - {"type":"message", "role":"assistant", "content":[...tool_use...]} — tool calls (array form)
            // - {"type":"result", "status":"...", "stats":{...}} — completion stats
            // Also handles legacy/raw API passthrough with "candidates" array.

            if let Some(event_type) = event.get("type").and_then(|v| v.as_str()) {
                match event_type {
                    "init" => {
                        if let Some(m) = event.get("model").and_then(|v| v.as_str()) {
                            session_model = m.to_string();
                        }
                    }
                    "message" => {
                        let role = event.get("role").and_then(|v| v.as_str()).unwrap_or("");
                        if role == "assistant" {
                            // Content can be a string (text delta) or an array (tool calls).
                            if let Some(text) = event.get("content").and_then(|v| v.as_str()) {
                                if !text.is_empty() {
                                    final_text = text.to_string();
                                    if let Some(ref on_event) = options.on_event {
                                        on_event(AgentLoopEvent::TextDelta {
                                            delta: text.to_string(),
                                        });
                                    }
                                }
                            } else if let Some(parts) =
                                event.get("content").and_then(|v| v.as_array())
                            {
                                for part in parts {
                                    let part_type =
                                        part.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                    match part_type {
                                        "text" => {
                                            if let Some(text) =
                                                part.get("text").and_then(|v| v.as_str())
                                            {
                                                final_text = text.to_string();
                                                if let Some(ref on_event) = options.on_event {
                                                    on_event(AgentLoopEvent::TextDelta {
                                                        delta: text.to_string(),
                                                    });
                                                }
                                            }
                                        }
                                        "tool_use" | "functionCall" => {
                                            call_counter += 1;
                                            let tool_name = part
                                                .get("name")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("unknown")
                                                .to_string();
                                            let call_id = part
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .map(|s| s.to_string())
                                                .unwrap_or_else(|| {
                                                    format!("gemini-tc-{}", call_counter)
                                                });
                                            let arguments = part
                                                .get("input")
                                                .or_else(|| part.get("args"))
                                                .cloned()
                                                .unwrap_or(serde_json::Value::Null);

                                            if let Some(ref on_event) = options.on_event {
                                                on_event(AgentLoopEvent::ToolCallStart {
                                                    call_id: call_id.clone(),
                                                    tool_name: tool_name.clone(),
                                                    arguments: arguments.clone(),
                                                });
                                            }

                                            tool_activity.push(ToolActivityRecord {
                                                tool_name,
                                                call_id,
                                                arguments_summary: Some(truncate_json(
                                                    &arguments, 200,
                                                )),
                                                result_summary: None,
                                                is_error: false,
                                                duration_ms: None,
                                            });
                                        }
                                        "tool_result" => {
                                            let result_text = part
                                                .get("content")
                                                .or_else(|| part.get("output"))
                                                .map(|v| truncate_json(v, 200))
                                                .unwrap_or_default();
                                            let is_error = part
                                                .get("is_error")
                                                .and_then(|v| v.as_bool())
                                                .unwrap_or(false);
                                            let tool_use_id = part
                                                .get("tool_use_id")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("");

                                            // Find matching record by ID, or fall back to last.
                                            let idx = tool_activity
                                                .iter()
                                                .rposition(|r| r.call_id == tool_use_id)
                                                .or_else(|| {
                                                    if tool_activity.is_empty() {
                                                        None
                                                    } else {
                                                        Some(tool_activity.len() - 1)
                                                    }
                                                });
                                            if let Some(record) =
                                                idx.and_then(|i| tool_activity.get_mut(i))
                                            {
                                                record.result_summary = Some(result_text.clone());
                                                record.is_error = is_error;

                                                if let Some(ref on_event) = options.on_event {
                                                    on_event(AgentLoopEvent::ToolCallEnd {
                                                        call_id: record.call_id.clone(),
                                                        output: result_text,
                                                        is_error,
                                                        duration_ms: 0,
                                                    });
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                    "result" => {
                        // {"type":"result","status":"success","stats":{"total_tokens":...,"input_tokens":...,"output_tokens":...}}
                        if let Some(stats) = event.get("stats") {
                            if let Some(v) = stats
                                .get("input_tokens")
                                .and_then(|v| v.as_u64())
                                .or_else(|| stats.get("input").and_then(|v| v.as_u64()))
                            {
                                total_usage.input_tokens = v;
                            }
                            if let Some(v) = stats.get("output_tokens").and_then(|v| v.as_u64()) {
                                total_usage.output_tokens = v;
                            }
                            if let Some(v) = stats.get("total_tokens").and_then(|v| v.as_u64()) {
                                total_usage.total_tokens = v;
                            } else {
                                total_usage.total_tokens =
                                    total_usage.input_tokens + total_usage.output_tokens;
                            }
                        }
                    }
                    // Legacy event types for older CLI versions.
                    "text" | "response" => {
                        if let Some(text) = event.get("text").and_then(|v| v.as_str()) {
                            final_text = text.to_string();
                            if let Some(ref on_event) = options.on_event {
                                on_event(AgentLoopEvent::TextDelta {
                                    delta: text.to_string(),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }

            // Handle Gemini-style candidates (raw API passthrough).
            if let Some(candidates) = event.get("candidates").and_then(|v| v.as_array()) {
                for candidate in candidates {
                    if let Some(content) = candidate.get("content") {
                        if let Some(parts) = content.get("parts").and_then(|v| v.as_array()) {
                            for part in parts {
                                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                                    final_text = text.to_string();
                                    if let Some(ref on_event) = options.on_event {
                                        on_event(AgentLoopEvent::TextDelta {
                                            delta: text.to_string(),
                                        });
                                    }
                                }
                                if let Some(fc) = part.get("functionCall") {
                                    call_counter += 1;
                                    let tool_name = fc
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown")
                                        .to_string();
                                    let call_id = format!("gemini-tc-{}", call_counter);
                                    let arguments =
                                        fc.get("args").cloned().unwrap_or(serde_json::Value::Null);

                                    tool_activity.push(ToolActivityRecord {
                                        tool_name,
                                        call_id,
                                        arguments_summary: Some(truncate_json(&arguments, 200)),
                                        result_summary: None,
                                        is_error: false,
                                        duration_ms: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // Handle usageMetadata (Gemini native).
            if let Some(usage_meta) = event.get("usageMetadata") {
                if let Some(v) = usage_meta.get("promptTokenCount").and_then(|v| v.as_u64()) {
                    total_usage.input_tokens = v;
                }
                if let Some(v) = usage_meta
                    .get("candidatesTokenCount")
                    .and_then(|v| v.as_u64())
                {
                    total_usage.output_tokens = v;
                }
                total_usage.total_tokens = total_usage.input_tokens + total_usage.output_tokens;
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| gemini_error("waiting for process", e))?;

        if !status.success() && final_text.is_empty() {
            return Err(SDKError::Provider(ProviderError {
                info: ErrorInfo::new(format!("gemini CLI exited with status: {}", status)),
                provider: "gemini-cli".to_string(),
                kind: ProviderErrorKind::Other,
                status_code: status.code().map(|c| c as u16),
                error_code: None,
                retryable: false,
                retry_after: None,
                raw: None,
            }));
        }

        let elapsed = start.elapsed();

        Ok(AgentRunResult {
            text: final_text,
            tool_activity,
            usage: total_usage,
            id: generate_run_id(),
            model: session_model,
            provider: "gemini-cli".to_string(),
            cost_usd: None,
            duration_ms: Some(elapsed.as_millis() as u64),
        })
    }
}

fn gemini_error(context: &str, error: std::io::Error) -> SDKError {
    SDKError::Provider(ProviderError {
        info: ErrorInfo::new(format!("gemini CLI: {}: {}", context, error)),
        provider: "gemini-cli".to_string(),
        kind: ProviderErrorKind::Other,
        status_code: None,
        error_code: None,
        retryable: false,
        retry_after: None,
        raw: None,
    })
}

fn gemini_io_error(context: &str, error: std::io::Error) -> SDKError {
    gemini_error(context, error)
}

fn truncate_json(value: &serde_json::Value, max_len: usize) -> String {
    let s = value.to_string();
    if s.len() <= max_len {
        s
    } else {
        let boundary = s.floor_char_boundary(max_len);
        format!("{}...", &s[..boundary])
    }
}

fn generate_run_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("gemini-cli-{}-{}", ts.as_secs(), ts.subsec_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_handles_candidates_format() {
        let line = r#"{"candidates":[{"content":{"parts":[{"text":"Hello world"}]}}],"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":50}}"#;
        let event: serde_json::Value = serde_json::from_str(line).unwrap();

        let mut text = String::new();
        let mut usage = Usage::default();

        if let Some(candidates) = event.get("candidates").and_then(|v| v.as_array()) {
            for candidate in candidates {
                if let Some(content) = candidate.get("content") {
                    if let Some(parts) = content.get("parts").and_then(|v| v.as_array()) {
                        for part in parts {
                            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                                text = t.to_string();
                            }
                        }
                    }
                }
            }
        }
        if let Some(usage_meta) = event.get("usageMetadata") {
            if let Some(v) = usage_meta.get("promptTokenCount").and_then(|v| v.as_u64()) {
                usage.input_tokens = v;
            }
            if let Some(v) = usage_meta
                .get("candidatesTokenCount")
                .and_then(|v| v.as_u64())
            {
                usage.output_tokens = v;
            }
        }

        assert_eq!(text, "Hello world");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
    }

    #[test]
    fn gemini_handles_function_call_format() {
        let line = r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read_file","args":{"path":"/src/main.rs"}}}]}}]}"#;
        let event: serde_json::Value = serde_json::from_str(line).unwrap();

        let mut tool_calls = Vec::new();
        if let Some(candidates) = event.get("candidates").and_then(|v| v.as_array()) {
            for candidate in candidates {
                if let Some(content) = candidate.get("content") {
                    if let Some(parts) = content.get("parts").and_then(|v| v.as_array()) {
                        for part in parts {
                            if let Some(fc) = part.get("functionCall") {
                                tool_calls.push(
                                    fc.get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                );
                            }
                        }
                    }
                }
            }
        }

        assert_eq!(tool_calls, vec!["read_file"]);
    }

    #[test]
    fn gemini_stream_json_message_extracts_text() {
        let line = r#"{"type":"message","timestamp":"2026-02-27T11:52:23.571Z","role":"assistant","content":"4","delta":true}"#;
        let event: serde_json::Value = serde_json::from_str(line).unwrap();

        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap();
        assert_eq!(event_type, "message");

        let role = event.get("role").and_then(|v| v.as_str()).unwrap();
        assert_eq!(role, "assistant");

        let content = event.get("content").and_then(|v| v.as_str()).unwrap();
        assert_eq!(content, "4");
    }

    #[test]
    fn gemini_stream_json_result_extracts_stats() {
        let line = r#"{"type":"result","timestamp":"2026-02-27T11:52:23.580Z","status":"success","stats":{"total_tokens":18613,"input_tokens":18234,"output_tokens":26,"cached":0,"input":18234,"duration_ms":4611,"tool_calls":0}}"#;
        let event: serde_json::Value = serde_json::from_str(line).unwrap();

        let stats = event.get("stats").unwrap();
        assert_eq!(
            stats.get("input_tokens").and_then(|v| v.as_u64()),
            Some(18234)
        );
        assert_eq!(
            stats.get("output_tokens").and_then(|v| v.as_u64()),
            Some(26)
        );
        assert_eq!(
            stats.get("total_tokens").and_then(|v| v.as_u64()),
            Some(18613)
        );
    }

    #[test]
    fn gemini_stream_json_init_extracts_model() {
        let line = r#"{"type":"init","timestamp":"2026-02-27T11:52:18.969Z","session_id":"ae01d4ca-1cd5-4705-94c0-749e0e99f8c2","model":"auto-gemini-3"}"#;
        let event: serde_json::Value = serde_json::from_str(line).unwrap();

        let model = event.get("model").and_then(|v| v.as_str()).unwrap();
        assert_eq!(model, "auto-gemini-3");
    }
}
