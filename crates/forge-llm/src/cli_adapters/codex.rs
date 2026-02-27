//! Codex CLI agent provider.
//!
//! Spawns `codex exec --json "<prompt>"` and parses the JSONL output stream.
//! The CLI handles tool execution internally using Codex's trained tools.

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

/// Agent provider that wraps the Codex CLI.
pub struct CodexAgentProvider {
    /// Path to the `codex` binary. Defaults to "codex".
    binary_path: String,
    /// Default model to request.
    model: Option<String>,
}

impl CodexAgentProvider {
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
        cmd.arg("exec").arg("--json").arg(prompt);

        let model = options
            .model_override
            .as_deref()
            .or(self.model.as_deref());
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
impl AgentProvider for CodexAgentProvider {
    fn name(&self) -> &str {
        "codex-cli"
    }

    async fn run_to_completion(
        &self,
        prompt: &str,
        options: &AgentRunOptions,
    ) -> Result<AgentRunResult, SDKError> {
        let start = Instant::now();
        let mut cmd = self.build_command(prompt, options);
        let mut child = cmd.spawn().map_err(|e| codex_error(&self.binary_path, e))?;

        let stdout = child.stdout.take().expect("stdout should be piped");
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        let mut final_text = String::new();
        let mut tool_activity = Vec::new();
        let mut total_usage = Usage::default();
        let mut thread_id = String::new();

        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|e| codex_io_error("reading stdout", e))?
        {
            let event: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            match event.get("type").and_then(|v| v.as_str()) {
                Some("thread.started") => {
                    if let Some(id) = event.get("thread_id").and_then(|v| v.as_str()) {
                        thread_id = id.to_string();
                    }
                }
                Some("item.started") => {
                    if let Some(item) = event.get("item") {
                        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        let item_id = item
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        match item_type {
                            "command_execution" => {
                                let command = item
                                    .get("command")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str())
                                            .collect::<Vec<_>>()
                                            .join(" ")
                                    })
                                    .unwrap_or_default();

                                if let Some(ref on_event) = options.on_event {
                                    on_event(AgentLoopEvent::ToolCallStart {
                                        call_id: item_id.clone(),
                                        tool_name: "shell".to_string(),
                                        arguments: serde_json::json!({"command": command}),
                                    });
                                }

                                tool_activity.push(ToolActivityRecord {
                                    tool_name: "shell".to_string(),
                                    call_id: item_id,
                                    arguments_summary: Some(truncate(&command, 200)),
                                    result_summary: None,
                                    is_error: false,
                                    duration_ms: None,
                                });
                            }
                            "file_change" => {
                                if let Some(ref on_event) = options.on_event {
                                    on_event(AgentLoopEvent::ToolCallStart {
                                        call_id: item_id.clone(),
                                        tool_name: "file_change".to_string(),
                                        arguments: serde_json::Value::Null,
                                    });
                                }

                                tool_activity.push(ToolActivityRecord {
                                    tool_name: "file_change".to_string(),
                                    call_id: item_id,
                                    arguments_summary: None,
                                    result_summary: None,
                                    is_error: false,
                                    duration_ms: None,
                                });
                            }
                            _ => {}
                        }
                    }
                }
                Some("item.completed") => {
                    if let Some(item) = event.get("item") {
                        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        let item_id = item
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        match item_type {
                            "agent_message" => {
                                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                                    final_text = text.to_string();
                                    if let Some(ref on_event) = options.on_event {
                                        on_event(AgentLoopEvent::TextDelta {
                                            delta: text.to_string(),
                                        });
                                    }
                                }
                            }
                            "command_execution" => {
                                let exit_code = item
                                    .get("exitCode")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(-1);
                                let output = item
                                    .get("aggregatedOutput")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let duration = item
                                    .get("durationMs")
                                    .and_then(|v| v.as_u64());
                                let is_error = exit_code != 0;

                                if let Some(record) = tool_activity
                                    .iter_mut()
                                    .rev()
                                    .find(|r| r.call_id == item_id)
                                {
                                    record.result_summary = Some(truncate(output, 200));
                                    record.is_error = is_error;
                                    record.duration_ms = duration;
                                }

                                if let Some(ref on_event) = options.on_event {
                                    on_event(AgentLoopEvent::ToolCallEnd {
                                        call_id: item_id,
                                        output: truncate(output, 500),
                                        is_error,
                                        duration_ms: duration.unwrap_or(0),
                                    });
                                }
                            }
                            "file_change" => {
                                if let Some(record) = tool_activity
                                    .iter_mut()
                                    .rev()
                                    .find(|r| r.call_id == item_id)
                                {
                                    let status = item
                                        .get("status")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown");
                                    record.result_summary = Some(status.to_string());
                                    record.is_error = status == "failed";
                                }

                                if let Some(ref on_event) = options.on_event {
                                    on_event(AgentLoopEvent::ToolCallEnd {
                                        call_id: item_id,
                                        output: "file_change completed".to_string(),
                                        is_error: false,
                                        duration_ms: 0,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Some("turn.completed") => {
                    if let Some(usage) = event.get("usage") {
                        accumulate_usage(&mut total_usage, usage);
                    }
                }
                _ => {}
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| codex_error("waiting for process", e))?;

        if !status.success() && final_text.is_empty() {
            return Err(SDKError::Provider(ProviderError {
                info: ErrorInfo::new(format!("codex CLI exited with status: {}", status)),
                provider: "codex-cli".to_string(),
                kind: ProviderErrorKind::Other,
                status_code: status.code().map(|c| c as u16),
                error_code: None,
                retryable: false,
                retry_after: None,
                raw: None,
            }));
        }

        let elapsed = start.elapsed();
        let model = self
            .model
            .clone()
            .unwrap_or_else(|| "codex-cli".to_string());

        Ok(AgentRunResult {
            text: final_text,
            tool_activity,
            usage: total_usage,
            id: if thread_id.is_empty() {
                generate_run_id()
            } else {
                thread_id
            },
            model,
            provider: "codex-cli".to_string(),
            cost_usd: None,
            duration_ms: Some(elapsed.as_millis() as u64),
        })
    }
}

fn codex_error(context: &str, error: std::io::Error) -> SDKError {
    SDKError::Provider(ProviderError {
        info: ErrorInfo::new(format!("codex CLI: {}: {}", context, error)),
        provider: "codex-cli".to_string(),
        kind: ProviderErrorKind::Other,
        status_code: None,
        error_code: None,
        retryable: false,
        retry_after: None,
        raw: None,
    })
}

fn codex_io_error(context: &str, error: std::io::Error) -> SDKError {
    codex_error(context, error)
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

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

fn generate_run_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("codex-cli-{}-{}", ts.as_secs(), ts.subsec_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_JSONL: &str = r#"{"type":"thread.started","thread_id":"thr_abc123"}
{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":["ls","-la"]}}
{"type":"item.completed","item":{"id":"item_1","type":"command_execution","exitCode":0,"aggregatedOutput":"total 42\ndrwxr-xr-x 5 user user 4096","durationMs":150}}
{"type":"item.started","item":{"id":"item_2","type":"agent_message"}}
{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"The directory contains 5 items."}}
{"type":"turn.completed","usage":{"input_tokens":5000,"output_tokens":200}}"#;

    #[test]
    fn codex_parse_extracts_final_text() {
        // Quick parse test — manually parse the sample
        let mut final_text = String::new();
        for line in SAMPLE_JSONL.lines() {
            let event: serde_json::Value = serde_json::from_str(line).unwrap();
            if event.get("type").and_then(|v| v.as_str()) == Some("item.completed") {
                if let Some(item) = event.get("item") {
                    if item.get("type").and_then(|v| v.as_str()) == Some("agent_message") {
                        final_text = item
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                    }
                }
            }
        }
        assert_eq!(final_text, "The directory contains 5 items.");
    }

    #[test]
    fn codex_parse_extracts_tool_calls() {
        let mut tool_activity = Vec::new();
        for line in SAMPLE_JSONL.lines() {
            let event: serde_json::Value = serde_json::from_str(line).unwrap();
            if event.get("type").and_then(|v| v.as_str()) == Some("item.started") {
                if let Some(item) = event.get("item") {
                    if item.get("type").and_then(|v| v.as_str()) == Some("command_execution") {
                        tool_activity.push(
                            item.get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                        );
                    }
                }
            }
        }
        assert_eq!(tool_activity.len(), 1);
        assert_eq!(tool_activity[0], "item_1");
    }

    #[test]
    fn codex_parse_extracts_usage() {
        let mut usage = Usage::default();
        for line in SAMPLE_JSONL.lines() {
            let event: serde_json::Value = serde_json::from_str(line).unwrap();
            if event.get("type").and_then(|v| v.as_str()) == Some("turn.completed") {
                if let Some(u) = event.get("usage") {
                    accumulate_usage(&mut usage, u);
                }
            }
        }
        assert_eq!(usage.input_tokens, 5000);
        assert_eq!(usage.output_tokens, 200);
    }
}
