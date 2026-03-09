//! Claude Code CLI agent provider.
//!
//! Spawns `claude -p <prompt> --output-format stream-json --verbose` and parses
//! the JSONL output stream. The CLI handles tool execution internally using
//! Claude's trained tools.

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

/// Agent provider that wraps the Claude Code CLI.
pub struct ClaudeCodeAgentProvider {
    /// Path to the `claude` binary. Defaults to "claude".
    binary_path: String,
    /// Default model to request.
    model: Option<String>,
}

impl ClaudeCodeAgentProvider {
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
        cmd.arg("-p")
            .arg(prompt)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose");

        let model = options
            .model_override
            .as_deref()
            .or(self.model.as_deref());
        if let Some(model) = model {
            cmd.arg("--model").arg(model);
        }
        if let Some(max_turns) = options.max_turns {
            cmd.arg("--max-turns").arg(max_turns.to_string());
        }

        cmd.current_dir(&options.working_directory);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Prevent nesting detection when launched from within Claude Code.
        cmd.env_remove("CLAUDECODE");

        if let Some(env_vars) = &options.env_vars {
            for (key, value) in env_vars {
                cmd.env(key, value);
            }
        }

        cmd
    }
}

#[async_trait]
impl AgentProvider for ClaudeCodeAgentProvider {
    fn name(&self) -> &str {
        "claude-code"
    }

    async fn run_to_completion(
        &self,
        prompt: &str,
        options: &AgentRunOptions,
    ) -> Result<AgentRunResult, SDKError> {
        let start = Instant::now();
        let mut cmd = self.build_command(prompt, options);
        let mut child = cmd.spawn().map_err(|e| cli_error(&self.binary_path, e))?;

        let stdout = child.stdout.take().expect("stdout should be piped");
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        let mut final_text = String::new();
        let mut tool_activity = Vec::new();
        let mut total_usage = Usage::default();
        let mut cost_usd = None;
        let mut session_model = self.model.clone().unwrap_or_else(|| "claude-code".to_string());

        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| cli_io_error("reading stdout", e))?
        {
            let event: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            match event.get("type").and_then(|v| v.as_str()) {
                Some("system") => {
                    if let Some(m) = event.get("model").and_then(|v| v.as_str()) {
                        session_model = m.to_string();
                    }
                }
                Some("assistant") => {
                    if let Some(message) = event.get("message") {
                        if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                            for block in content {
                                let block_type =
                                    block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                                match block_type {
                                    "text" => {
                                        if let Some(text) =
                                            block.get("text").and_then(|v| v.as_str())
                                        {
                                            if let Some(ref on_event) = options.on_event {
                                                on_event(AgentLoopEvent::TextDelta {
                                                    delta: text.to_string(),
                                                });
                                            }
                                        }
                                    }
                                    "tool_use" => {
                                        let tool_name = block
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("unknown")
                                            .to_string();
                                        let call_id = block
                                            .get("id")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        let arguments = block
                                            .get("input")
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
                                            call_id: call_id.clone(),
                                            arguments_summary: Some(truncate_json(
                                                &arguments,
                                                200,
                                            )),
                                            result_summary: None,
                                            is_error: false,
                                            duration_ms: None,
                                        });
                                    }
                                    _ => {}
                                }
                            }
                        }
                        // Accumulate usage from assistant messages.
                        if let Some(usage) = message.get("usage") {
                            accumulate_usage(&mut total_usage, usage);
                        }
                    }
                }
                Some("user") => {
                    // Tool result messages — update the last tool_activity record.
                    if let Some(message) = event.get("message") {
                        if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                            for block in content {
                                if block.get("type").and_then(|v| v.as_str())
                                    == Some("tool_result")
                                {
                                    let tool_use_id = block
                                        .get("tool_use_id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let is_error = block
                                        .get("is_error")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(false);
                                    let result_content = block
                                        .get("content")
                                        .map(|v| truncate_json(v, 200))
                                        .unwrap_or_default();

                                    // Find matching tool_activity and update it.
                                    if let Some(record) = tool_activity
                                        .iter_mut()
                                        .rev()
                                        .find(|r| r.call_id == tool_use_id)
                                    {
                                        record.result_summary = Some(result_content.clone());
                                        record.is_error = is_error;
                                    }

                                    if let Some(ref on_event) = options.on_event {
                                        on_event(AgentLoopEvent::ToolCallEnd {
                                            call_id: tool_use_id.to_string(),
                                            output: result_content,
                                            is_error,
                                            duration_ms: 0,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                Some("result") => {
                    if let Some(text) = event.get("result").and_then(|v| v.as_str()) {
                        final_text = text.to_string();
                    }
                    if let Some(usage) = event.get("usage") {
                        accumulate_usage(&mut total_usage, usage);
                    }
                    if let Some(cost) = event.get("total_cost_usd").and_then(|v| v.as_f64()) {
                        cost_usd = Some(cost);
                    }
                }
                _ => {}
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| cli_error("waiting for process", e))?;

        if !status.success() && final_text.is_empty() {
            return Err(SDKError::Provider(ProviderError {
                info: ErrorInfo::new(format!("claude CLI exited with status: {}", status)),
                provider: "claude-code".to_string(),
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
            provider: "claude-code".to_string(),
            cost_usd,
            duration_ms: Some(elapsed.as_millis() as u64),
        })
    }
}

fn cli_error(context: &str, error: std::io::Error) -> SDKError {
    SDKError::Provider(ProviderError {
        info: ErrorInfo::new(format!("claude-code CLI: {}: {}", context, error)),
        provider: "claude-code".to_string(),
        kind: ProviderErrorKind::Other,
        status_code: None,
        error_code: None,
        retryable: false,
        retry_after: None,
        raw: None,
    })
}

fn cli_io_error(context: &str, error: std::io::Error) -> SDKError {
    cli_error(context, error)
}

fn accumulate_usage(total: &mut Usage, raw: &serde_json::Value) {
    if let Some(v) = raw.get("input_tokens").and_then(|v| v.as_u64()) {
        total.input_tokens += v;
    }
    if let Some(v) = raw.get("output_tokens").and_then(|v| v.as_u64()) {
        total.output_tokens += v;
    }
    total.total_tokens = total.input_tokens + total.output_tokens;
}

fn truncate_json(value: &serde_json::Value, max_len: usize) -> String {
    let s = value.to_string();
    if s.len() <= max_len {
        s
    } else {
        // Find the nearest char boundary at or before max_len
        let boundary = s.floor_char_boundary(max_len);
        format!("{}...", &s[..boundary])
    }
}

fn generate_run_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("claude-code-{}-{}", ts.as_secs(), ts.subsec_nanos())
}

/// Parse a recorded JSONL string (for testing).
#[cfg(test)]
pub(crate) fn parse_claude_code_jsonl(
    jsonl: &str,
) -> (String, Vec<ToolActivityRecord>, Usage, Option<f64>) {
    let mut final_text = String::new();
    let mut tool_activity = Vec::new();
    let mut total_usage = Usage::default();
    let mut cost_usd = None;

    for line in jsonl.lines() {
        let event: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match event.get("type").and_then(|v| v.as_str()) {
            Some("assistant") => {
                if let Some(message) = event.get("message") {
                    if let Some(content) = message.get("content").and_then(|v| v.as_array()) {
                        for block in content {
                            if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                                tool_activity.push(ToolActivityRecord {
                                    tool_name: block
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown")
                                        .to_string(),
                                    call_id: block
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                    arguments_summary: block
                                        .get("input")
                                        .map(|v| truncate_json(v, 200)),
                                    result_summary: None,
                                    is_error: false,
                                    duration_ms: None,
                                });
                            }
                        }
                    }
                    if let Some(usage) = message.get("usage") {
                        accumulate_usage(&mut total_usage, usage);
                    }
                }
            }
            Some("result") => {
                if let Some(text) = event.get("result").and_then(|v| v.as_str()) {
                    final_text = text.to_string();
                }
                if let Some(usage) = event.get("usage") {
                    accumulate_usage(&mut total_usage, usage);
                }
                if let Some(cost) = event.get("total_cost_usd").and_then(|v| v.as_f64()) {
                    cost_usd = Some(cost);
                }
            }
            _ => {}
        }
    }

    (final_text, tool_activity, total_usage, cost_usd)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JSONL: &str = r#"{"type":"system","subtype":"init","session_id":"sess_123","tools":["Read","Write","Bash"],"model":"claude-sonnet-4-20250514"}
{"type":"assistant","uuid":"msg_1","session_id":"sess_123","message":{"role":"assistant","content":[{"type":"text","text":"Let me check the file."},{"type":"tool_use","id":"toolu_abc","name":"Read","input":{"file_path":"/src/main.rs"}}],"usage":{"input_tokens":1500,"output_tokens":80}}}
{"type":"user","uuid":"msg_2","session_id":"sess_123","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_abc","content":"fn main() { println!(\"hello\"); }"}]}}
{"type":"assistant","uuid":"msg_3","session_id":"sess_123","message":{"role":"assistant","content":[{"type":"text","text":"The file contains a simple hello world program."}],"usage":{"input_tokens":2000,"output_tokens":50}}}
{"type":"result","subtype":"success","session_id":"sess_123","is_error":false,"result":"The file contains a simple hello world program.","num_turns":2,"total_cost_usd":0.015,"usage":{"input_tokens":3500,"output_tokens":130}}"#;

    #[test]
    fn parse_claude_code_jsonl_extracts_final_text() {
        let (text, _, _, _) = parse_claude_code_jsonl(SAMPLE_JSONL);
        assert_eq!(text, "The file contains a simple hello world program.");
    }

    #[test]
    fn parse_claude_code_jsonl_extracts_tool_calls() {
        let (_, tools, _, _) = parse_claude_code_jsonl(SAMPLE_JSONL);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_name, "Read");
        assert_eq!(tools[0].call_id, "toolu_abc");
        assert!(tools[0]
            .arguments_summary
            .as_ref()
            .unwrap()
            .contains("main.rs"));
    }

    #[test]
    fn parse_claude_code_jsonl_extracts_usage() {
        let (_, _, usage, _) = parse_claude_code_jsonl(SAMPLE_JSONL);
        // Usage from result event
        assert_eq!(usage.input_tokens, 3500 + 1500 + 2000);
        assert_eq!(usage.output_tokens, 130 + 80 + 50);
    }

    #[test]
    fn parse_claude_code_jsonl_extracts_cost() {
        let (_, _, _, cost) = parse_claude_code_jsonl(SAMPLE_JSONL);
        assert_eq!(cost, Some(0.015));
    }

    #[test]
    fn truncate_json_short_values_unchanged() {
        let v = serde_json::json!({"a": "b"});
        assert_eq!(truncate_json(&v, 100), r#"{"a":"b"}"#);
    }

    #[test]
    fn truncate_json_long_values_truncated() {
        let v = serde_json::json!({"long_key": "a".repeat(300)});
        let result = truncate_json(&v, 20);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 24);
    }
}
