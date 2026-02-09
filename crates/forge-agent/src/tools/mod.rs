mod apply_patch;
mod edit_file;
mod glob;
mod grep;
mod read_file;
mod registry;
mod shell;
mod subagents;
mod write_file;

use crate::{SessionConfig, ToolError};
use forge_llm::{ToolCall, ToolResult};
use serde_json::Value;

pub use registry::{RegisteredTool, ToolDispatchOptions, ToolExecutor, ToolFuture, ToolRegistry};

pub const READ_FILE_TOOL: &str = "read_file";
pub const WRITE_FILE_TOOL: &str = "write_file";
pub const EDIT_FILE_TOOL: &str = "edit_file";
pub const APPLY_PATCH_TOOL: &str = "apply_patch";
pub const SHELL_TOOL: &str = "shell";
pub const GREP_TOOL: &str = "grep";
pub const GLOB_TOOL: &str = "glob";
pub const SPAWN_AGENT_TOOL: &str = "spawn_agent";
pub const SEND_INPUT_TOOL: &str = "send_input";
pub const WAIT_TOOL: &str = "wait";
pub const CLOSE_AGENT_TOOL: &str = "close_agent";

pub fn build_openai_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    register_shared_core_tools(&mut registry);
    register_subagent_tools(&mut registry);
    registry.register(apply_patch::apply_patch_tool());
    registry
}

pub fn build_anthropic_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    register_shared_core_tools(&mut registry);
    register_subagent_tools(&mut registry);
    registry.register(edit_file::edit_file_tool());
    registry
}

pub fn build_gemini_tool_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::default();
    register_shared_core_tools(&mut registry);
    register_subagent_tools(&mut registry);
    registry.register(edit_file::edit_file_tool());
    registry
}

pub fn register_shared_core_tools(registry: &mut ToolRegistry) {
    registry.register(read_file::read_file_tool());
    registry.register(write_file::write_file_tool());
    registry.register(shell::shell_tool());
    registry.register(grep::grep_tool());
    registry.register(glob::glob_tool());
}

pub fn register_subagent_tools(registry: &mut ToolRegistry) {
    registry.register(subagents::spawn_agent_tool());
    registry.register(subagents::send_input_tool());
    registry.register(subagents::wait_tool());
    registry.register(subagents::close_agent_tool());
}

fn normalize_tool_arguments_for_dispatch(
    tool_name: &str,
    arguments: Value,
    schema: &Value,
    config: &SessionConfig,
) -> Value {
    if tool_name != SHELL_TOOL {
        return arguments;
    }

    let has_timeout_property = schema
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get("timeout_ms"))
        .is_some();
    if !has_timeout_property {
        return arguments;
    }

    let Some(object) = arguments.as_object() else {
        return arguments;
    };
    let mut normalized = object.clone();
    let (default_timeout_ms, max_timeout_ms) = effective_shell_timeout_policy(config);

    let timeout_ms = match normalized.get("timeout_ms") {
        Some(Value::Number(number)) => {
            if let Some(value) = number.as_u64() {
                value.min(max_timeout_ms)
            } else {
                return Value::Object(normalized);
            }
        }
        Some(_) => return Value::Object(normalized),
        None => default_timeout_ms,
    };

    normalized.insert("timeout_ms".to_string(), Value::from(timeout_ms));
    Value::Object(normalized)
}

fn effective_shell_timeout_policy(config: &SessionConfig) -> (u64, u64) {
    let default_timeout_ms = if config.default_command_timeout_ms == 0 {
        10_000
    } else {
        config.default_command_timeout_ms
    };
    let max_timeout_ms = if config.max_command_timeout_ms == 0 {
        600_000
    } else {
        config.max_command_timeout_ms
    };
    let max_timeout_ms = max_timeout_ms.max(default_timeout_ms);
    (default_timeout_ms, max_timeout_ms)
}
fn required_string_argument(arguments: &Value, key: &str) -> Result<String, ToolError> {
    optional_string_argument(arguments, key)?
        .ok_or_else(|| ToolError::Validation(format!("missing required argument '{}'", key)))
}

fn optional_string_argument(arguments: &Value, key: &str) -> Result<Option<String>, ToolError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(ToolError::Validation(format!(
            "argument '{}' must be a string",
            key
        )));
    };
    Ok(Some(value.to_string()))
}

fn optional_bool_argument(arguments: &Value, key: &str) -> Result<Option<bool>, ToolError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_bool() else {
        return Err(ToolError::Validation(format!(
            "argument '{}' must be a boolean",
            key
        )));
    };
    Ok(Some(value))
}

fn optional_u64_argument(arguments: &Value, key: &str) -> Result<Option<u64>, ToolError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let Some(value) = value.as_u64() else {
        return Err(ToolError::Validation(format!(
            "argument '{}' must be a positive integer",
            key
        )));
    };
    Ok(Some(value))
}

fn optional_usize_argument(arguments: &Value, key: &str) -> Result<Option<usize>, ToolError> {
    Ok(optional_u64_argument(arguments, key)?.map(|value| value as usize))
}

fn format_line_numbered_content(content: &str, start_line: usize) -> String {
    if content.is_empty() {
        return String::new();
    }
    content
        .lines()
        .enumerate()
        .map(|(idx, line)| format!("{} | {}", start_line + idx, line))
        .collect::<Vec<String>>()
        .join("\n")
}

fn format_exec_result(result: &crate::ExecResult) -> String {
    let mut output = format!(
        "exit_code: {}\nduration_ms: {}",
        result.exit_code, result.duration_ms
    );
    if !result.stdout.is_empty() {
        output.push_str("\nstdout:\n");
        output.push_str(&result.stdout);
    }
    if !result.stderr.is_empty() {
        output.push_str("\nstderr:\n");
        output.push_str(&result.stderr);
    }
    output
}

fn tool_error_result(tool_call_id: String, message: String) -> ToolResult {
    ToolResult {
        tool_call_id,
        content: Value::String(message),
        is_error: true,
    }
}

fn parse_tool_arguments(tool_call: &ToolCall) -> Result<Value, ToolError> {
    if let Some(raw_arguments) = &tool_call.raw_arguments {
        let parsed = serde_json::from_str::<Value>(raw_arguments).map_err(|error| {
            ToolError::Validation(format!(
                "invalid JSON arguments for tool '{}': {}",
                tool_call.name, error
            ))
        })?;
        return Ok(parsed);
    }

    Ok(tool_call.arguments.clone())
}

fn validate_tool_arguments(schema: &Value, arguments: &Value) -> Result<(), ToolError> {
    let object = arguments
        .as_object()
        .ok_or_else(|| ToolError::Validation("tool arguments must be a JSON object".to_string()))?;

    let schema_object = schema.as_object().ok_or_else(|| {
        ToolError::Validation("tool schema root must be a JSON object".to_string())
    })?;

    if schema_object
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|schema_type| schema_type != "object")
    {
        return Err(ToolError::Validation(
            "tool schema root type must be 'object'".to_string(),
        ));
    }

    if let Some(required) = schema_object.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                return Err(ToolError::Validation(format!(
                    "missing required argument '{}'",
                    key
                )));
            }
        }
    }

    let properties = schema_object
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let additional_allowed = schema_object
        .get("additionalProperties")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    for (key, value) in object {
        let Some(property) = properties.get(key) else {
            if additional_allowed {
                continue;
            }
            return Err(ToolError::Validation(format!(
                "unexpected argument '{}' not allowed by schema",
                key
            )));
        };

        if let Some(type_name) = property.get("type").and_then(Value::as_str) {
            let is_valid = match type_name {
                "string" => value.is_string(),
                "number" => value.is_number(),
                "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
                "boolean" => value.is_boolean(),
                "array" => value.is_array(),
                "object" => value.is_object(),
                "null" => value.is_null(),
                _ => true,
            };

            if !is_valid {
                return Err(ToolError::Validation(format!(
                    "argument '{}' expected type '{}' but received '{}'",
                    key,
                    type_name,
                    json_type_name(value)
                )));
            }
        }
    }

    Ok(())
}

fn json_type_name(value: &Value) -> &'static str {
    if value.is_null() {
        "null"
    } else if value.is_boolean() {
        "boolean"
    } else if value.is_string() {
        "string"
    } else if value.is_number() {
        "number"
    } else if value.is_array() {
        "array"
    } else {
        "object"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentError, BufferedEventEmitter, EventKind, ExecutionEnvironment,
        LocalExecutionEnvironment, NoopEventEmitter,
    };
    use async_trait::async_trait;
    use forge_llm::ToolDefinition;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use tempfile::tempdir;
    use tokio::time::{Duration, Instant, sleep};

    fn dummy_executor() -> ToolExecutor {
        Arc::new(|_args, _env| Box::pin(async move { Ok("ok".to_string()) }))
    }

    #[test]
    fn tool_registry_latest_registration_wins() {
        let mut registry = ToolRegistry::default();

        let first = RegisteredTool {
            definition: ToolDefinition {
                name: "read_file".to_string(),
                description: "first".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            },
            executor: dummy_executor(),
        };
        registry.register(first);

        let second = RegisteredTool {
            definition: ToolDefinition {
                name: "read_file".to_string(),
                description: "second".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            },
            executor: dummy_executor(),
        };
        registry.register(second);

        let registered = registry
            .get("read_file")
            .expect("tool should be present after replacement");
        assert_eq!(registered.definition.description, "second");
    }

    #[test]
    fn tool_registry_definitions_are_sorted_by_name() {
        let mut registry = ToolRegistry::default();
        registry.register(RegisteredTool {
            definition: ToolDefinition {
                name: "zeta".to_string(),
                description: "z".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            },
            executor: dummy_executor(),
        });
        registry.register(RegisteredTool {
            definition: ToolDefinition {
                name: "alpha".to_string(),
                description: "a".to_string(),
                parameters: serde_json::json!({ "type": "object" }),
            },
            executor: dummy_executor(),
        });

        let names: Vec<String> = registry
            .definitions()
            .into_iter()
            .map(|definition| definition.name)
            .collect();
        assert_eq!(names, vec!["alpha".to_string(), "zeta".to_string()]);
    }

    struct TestExecutionEnvironment {
        working_dir: PathBuf,
    }

    impl Default for TestExecutionEnvironment {
        fn default() -> Self {
            Self {
                working_dir: PathBuf::from("."),
            }
        }
    }

    struct TimeoutCaptureEnv {
        working_dir: PathBuf,
        observed_timeout_ms: Arc<AtomicU64>,
    }

    impl TimeoutCaptureEnv {
        fn new(observed_timeout_ms: Arc<AtomicU64>) -> Self {
            Self {
                working_dir: PathBuf::from("."),
                observed_timeout_ms,
            }
        }
    }

    #[async_trait]
    impl ExecutionEnvironment for TimeoutCaptureEnv {
        async fn read_file(
            &self,
            _path: &str,
            _offset: Option<usize>,
            _limit: Option<usize>,
        ) -> Result<String, AgentError> {
            Err(AgentError::NotImplemented("read_file".to_string()))
        }

        async fn write_file(&self, _path: &str, _content: &str) -> Result<(), AgentError> {
            Err(AgentError::NotImplemented("write_file".to_string()))
        }

        async fn delete_file(&self, _path: &str) -> Result<(), AgentError> {
            Err(AgentError::NotImplemented("delete_file".to_string()))
        }

        async fn move_file(&self, _from: &str, _to: &str) -> Result<(), AgentError> {
            Err(AgentError::NotImplemented("move_file".to_string()))
        }

        async fn file_exists(&self, _path: &str) -> Result<bool, AgentError> {
            Err(AgentError::NotImplemented("file_exists".to_string()))
        }

        async fn list_directory(
            &self,
            _path: &str,
            _depth: usize,
        ) -> Result<Vec<crate::DirEntry>, AgentError> {
            Err(AgentError::NotImplemented("list_directory".to_string()))
        }

        async fn exec_command(
            &self,
            _command: &str,
            timeout_ms: u64,
            _working_dir: Option<&str>,
            _env_vars: Option<HashMap<String, String>>,
        ) -> Result<crate::ExecResult, AgentError> {
            self.observed_timeout_ms.store(timeout_ms, Ordering::SeqCst);
            Ok(crate::ExecResult {
                stdout: "ok".to_string(),
                stderr: String::new(),
                exit_code: 0,
                timed_out: false,
                duration_ms: 1,
            })
        }

        async fn grep(
            &self,
            _pattern: &str,
            _path: &str,
            _options: crate::GrepOptions,
        ) -> Result<String, AgentError> {
            Err(AgentError::NotImplemented("grep".to_string()))
        }

        async fn glob(&self, _pattern: &str, _path: &str) -> Result<Vec<String>, AgentError> {
            Err(AgentError::NotImplemented("glob".to_string()))
        }

        fn working_directory(&self) -> &Path {
            &self.working_dir
        }

        fn platform(&self) -> &str {
            "linux"
        }

        fn os_version(&self) -> &str {
            "test"
        }
    }

    #[async_trait]
    impl ExecutionEnvironment for TestExecutionEnvironment {
        async fn read_file(
            &self,
            _path: &str,
            _offset: Option<usize>,
            _limit: Option<usize>,
        ) -> Result<String, AgentError> {
            Err(AgentError::NotImplemented("read_file".to_string()))
        }

        async fn write_file(&self, _path: &str, _content: &str) -> Result<(), AgentError> {
            Err(AgentError::NotImplemented("write_file".to_string()))
        }

        async fn delete_file(&self, _path: &str) -> Result<(), AgentError> {
            Err(AgentError::NotImplemented("delete_file".to_string()))
        }

        async fn move_file(&self, _from: &str, _to: &str) -> Result<(), AgentError> {
            Err(AgentError::NotImplemented("move_file".to_string()))
        }

        async fn file_exists(&self, _path: &str) -> Result<bool, AgentError> {
            Err(AgentError::NotImplemented("file_exists".to_string()))
        }

        async fn list_directory(
            &self,
            _path: &str,
            _depth: usize,
        ) -> Result<Vec<crate::DirEntry>, AgentError> {
            Err(AgentError::NotImplemented("list_directory".to_string()))
        }

        async fn exec_command(
            &self,
            _command: &str,
            _timeout_ms: u64,
            _working_dir: Option<&str>,
            _env_vars: Option<HashMap<String, String>>,
        ) -> Result<crate::ExecResult, AgentError> {
            Err(AgentError::NotImplemented("exec_command".to_string()))
        }

        async fn grep(
            &self,
            _pattern: &str,
            _path: &str,
            _options: crate::GrepOptions,
        ) -> Result<String, AgentError> {
            Err(AgentError::NotImplemented("grep".to_string()))
        }

        async fn glob(&self, _pattern: &str, _path: &str) -> Result<Vec<String>, AgentError> {
            Err(AgentError::NotImplemented("glob".to_string()))
        }

        fn working_directory(&self) -> &Path {
            &self.working_dir
        }

        fn platform(&self) -> &str {
            "linux"
        }

        fn os_version(&self) -> &str {
            "test"
        }
    }

    fn command_tool(executor: ToolExecutor) -> RegisteredTool {
        RegisteredTool {
            definition: ToolDefinition {
                name: "shell".to_string(),
                description: "run command".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "required": ["command"],
                    "properties": {
                        "command": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            },
            executor,
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_unknown_tool_returns_error_result_instead_of_failing_session() {
        let registry = ToolRegistry::default();
        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "unknown".to_string(),
                    arguments: serde_json::json!({}),
                    raw_arguments: None,
                }],
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should not fail");

        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
        assert_eq!(results[0].tool_call_id, "call-1");
        assert!(
            results[0]
                .content
                .as_str()
                .unwrap_or_default()
                .contains("Unknown tool")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_validation_error_returns_structured_tool_error_without_execution() {
        let execution_count = Arc::new(AtomicUsize::new(0));
        let count = execution_count.clone();
        let executor: ToolExecutor = Arc::new(move |_args, _env| {
            let count = count.clone();
            Box::pin(async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok("should not run".to_string())
            })
        });

        let mut registry = ToolRegistry::default();
        registry.register(command_tool(executor));

        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({ "not_command": 1 }),
                    raw_arguments: None,
                }],
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should not fail");

        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
        assert_eq!(execution_count.load(Ordering::SeqCst), 0);
        assert!(
            results[0]
                .content
                .as_str()
                .unwrap_or_default()
                .contains("missing required argument 'command'")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_parses_raw_json_arguments_and_validates_schema() {
        let executor: ToolExecutor = Arc::new(move |args, _env| {
            Box::pin(async move {
                let cmd = args
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(format!("ran {cmd}"))
            })
        });
        let mut registry = ToolRegistry::default();
        registry.register(command_tool(executor));

        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({}),
                    raw_arguments: Some("{\"command\":\"echo hi\"}".to_string()),
                }],
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should not fail");

        assert_eq!(results.len(), 1);
        assert!(!results[0].is_error);
        assert_eq!(results[0].content.as_str(), Some("ran echo hi"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_parallel_mode_keeps_input_order_and_call_ids_stable() {
        let executor: ToolExecutor = Arc::new(move |args, _env| {
            Box::pin(async move {
                let delay_ms = args
                    .get("delay_ms")
                    .and_then(Value::as_u64)
                    .expect("delay_ms should be present");
                let output = args
                    .get("output")
                    .and_then(Value::as_str)
                    .expect("output should be present")
                    .to_string();
                sleep(Duration::from_millis(delay_ms)).await;
                Ok(output)
            })
        });

        let mut registry = ToolRegistry::default();
        registry.register(RegisteredTool {
            definition: ToolDefinition {
                name: "sleep_echo".to_string(),
                description: "sleep and echo".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "required": ["delay_ms", "output"],
                    "properties": {
                        "delay_ms": { "type": "integer" },
                        "output": { "type": "string" }
                    },
                    "additionalProperties": false
                }),
            },
            executor,
        });

        let calls = vec![
            ToolCall {
                id: "call-a".to_string(),
                name: "sleep_echo".to_string(),
                arguments: serde_json::json!({"delay_ms": 80, "output": "a"}),
                raw_arguments: None,
            },
            ToolCall {
                id: "call-b".to_string(),
                name: "sleep_echo".to_string(),
                arguments: serde_json::json!({"delay_ms": 20, "output": "b"}),
                raw_arguments: None,
            },
            ToolCall {
                id: "call-c".to_string(),
                name: "sleep_echo".to_string(),
                arguments: serde_json::json!({"delay_ms": 60, "output": "c"}),
                raw_arguments: None,
            },
        ];

        let started = Instant::now();
        let results = registry
            .dispatch(
                calls,
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: true,
                },
            )
            .await
            .expect("dispatch should not fail");
        let elapsed = started.elapsed();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].tool_call_id, "call-a");
        assert_eq!(results[1].tool_call_id, "call-b");
        assert_eq!(results[2].tool_call_id, "call-c");
        assert_eq!(results[0].content.as_str(), Some("a"));
        assert_eq!(results[1].content.as_str(), Some("b"));
        assert_eq!(results[2].content.as_str(), Some("c"));
        assert!(elapsed < Duration::from_millis(170));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_emits_tool_call_start_and_end_events_in_order() {
        let mut registry = ToolRegistry::default();
        registry.register(command_tool(Arc::new(|_args, _env| {
            Box::pin(async move { Ok("done".to_string()) })
        })));

        let emitter = Arc::new(BufferedEventEmitter::default());
        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({"command": "echo hi"}),
                    raw_arguments: None,
                }],
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                emitter.clone(),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(!results[0].is_error);
        let events = emitter.snapshot();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].kind, EventKind::ToolCallStart);
        assert_eq!(events[1].kind, EventKind::ToolCallOutputDelta);
        assert_eq!(events[2].kind, EventKind::ToolCallEnd);
        assert_eq!(events[0].data.get_str("call_id"), Some("call-1"));
        assert_eq!(events[1].data.get_str("call_id"), Some("call-1"));
        assert_eq!(events[1].data.get_str("delta"), Some("done"));
        assert_eq!(events[2].data.get_str("call_id"), Some("call-1"));
        assert_eq!(events[2].data.get_str("output"), Some("done"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dispatch_returns_truncated_result_to_llm_but_emits_full_output_event() {
        let full_output = "x".repeat(40_000);
        let mut registry = ToolRegistry::default();
        registry.register(command_tool(Arc::new(move |_args, _env| {
            let full_output = full_output.clone();
            Box::pin(async move { Ok(full_output) })
        })));

        let emitter = Arc::new(BufferedEventEmitter::default());
        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: serde_json::json!({"command": "echo hi"}),
                    raw_arguments: None,
                }],
                Arc::new(TestExecutionEnvironment::default()),
                &SessionConfig::default(),
                emitter.clone(),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        let llm_output = results[0]
            .content
            .as_str()
            .expect("output should be a string");
        assert!(llm_output.contains("[WARNING: Tool output was truncated."));
        assert!(llm_output.chars().count() < 40_000);

        let events = emitter.snapshot();
        let end_event = events
            .iter()
            .find(|event| event.kind == EventKind::ToolCallEnd)
            .expect("tool end event should be present");
        let event_output = end_event
            .data
            .get_str("output")
            .expect("output field should be present");
        assert_eq!(event_output.chars().count(), 40_000);
        assert!(event_output.chars().all(|ch| ch == 'x'));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_dispatch_injects_default_timeout_from_session_config() {
        let observed_timeout = Arc::new(AtomicU64::new(0));
        let env = Arc::new(TimeoutCaptureEnv::new(observed_timeout.clone()));
        let mut registry = ToolRegistry::default();
        registry.register(shell::shell_tool());

        let mut config = SessionConfig::default();
        config.default_command_timeout_ms = 12_345;
        config.max_command_timeout_ms = 60_000;

        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "echo hi" }),
                    raw_arguments: None,
                }],
                env,
                &config,
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(!results[0].is_error);
        assert_eq!(observed_timeout.load(Ordering::SeqCst), 12_345);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_dispatch_clamps_timeout_to_session_max() {
        let observed_timeout = Arc::new(AtomicU64::new(0));
        let env = Arc::new(TimeoutCaptureEnv::new(observed_timeout.clone()));
        let mut registry = ToolRegistry::default();
        registry.register(shell::shell_tool());

        let mut config = SessionConfig::default();
        config.default_command_timeout_ms = 1_000;
        config.max_command_timeout_ms = 1_500;

        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "echo hi", "timeout_ms": 30_000 }),
                    raw_arguments: None,
                }],
                env,
                &config,
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(!results[0].is_error);
        assert_eq!(observed_timeout.load(Ordering::SeqCst), 1_500);
    }

    #[test]
    fn build_openai_registry_uses_apply_patch_variant() {
        let openai = build_openai_tool_registry();
        let anthropic = build_anthropic_tool_registry();
        let gemini = build_gemini_tool_registry();

        assert!(openai.names().contains(&APPLY_PATCH_TOOL.to_string()));
        assert!(!openai.names().contains(&EDIT_FILE_TOOL.to_string()));
        assert!(anthropic.names().contains(&EDIT_FILE_TOOL.to_string()));
        assert!(!anthropic.names().contains(&APPLY_PATCH_TOOL.to_string()));
        assert!(gemini.names().contains(&EDIT_FILE_TOOL.to_string()));
        assert!(!gemini.names().contains(&APPLY_PATCH_TOOL.to_string()));
        assert!(openai.names().contains(&SPAWN_AGENT_TOOL.to_string()));
        assert!(openai.names().contains(&SEND_INPUT_TOOL.to_string()));
        assert!(openai.names().contains(&WAIT_TOOL.to_string()));
        assert!(openai.names().contains(&CLOSE_AGENT_TOOL.to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn edit_file_returns_ambiguity_error_when_match_is_not_unique() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("target.txt", "alpha\nalpha\n")
            .await
            .expect("seed file should write");

        let registry = build_anthropic_tool_registry();
        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: EDIT_FILE_TOOL.to_string(),
                    arguments: json!({
                        "file_path": "target.txt",
                        "old_string": "alpha",
                        "new_string": "beta"
                    }),
                    raw_arguments: None,
                }],
                env,
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(results[0].is_error);
        assert!(
            results[0]
                .content
                .as_str()
                .unwrap_or_default()
                .contains("not unique")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn edit_file_fuzzy_fallback_matches_whitespace_variants() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("target.txt", "fn  main() {\n    println!(\"hi\");\n}\n")
            .await
            .expect("seed file should write");

        let registry = build_anthropic_tool_registry();
        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: EDIT_FILE_TOOL.to_string(),
                    arguments: json!({
                        "file_path": "target.txt",
                        "old_string": "fn main() {\n println!(\"hi\");\n}",
                        "new_string": "fn main() {\n    println!(\"hello\");\n}"
                    }),
                    raw_arguments: None,
                }],
                env.clone(),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(!results[0].is_error);
        let updated = env
            .read_file("target.txt", None, None)
            .await
            .expect("updated file should read");
        assert!(updated.contains("println!(\"hello\")"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn apply_patch_returns_parse_error_for_invalid_hunk_header() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("target.txt", "one\n")
            .await
            .expect("seed file should write");

        let registry = build_openai_tool_registry();
        let patch = "*** Begin Patch\n*** Update File: target.txt\nnot-a-hunk\n*** End Patch";
        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: APPLY_PATCH_TOOL.to_string(),
                    arguments: json!({
                        "patch": patch
                    }),
                    raw_arguments: None,
                }],
                env,
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(results[0].is_error);
        assert!(
            results[0]
                .content
                .as_str()
                .unwrap_or_default()
                .contains("invalid hunk header")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn apply_patch_supports_successful_multi_file_operations() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("a.txt", "line1\nline2\n")
            .await
            .expect("seed a.txt");
        env.write_file("old_name.txt", "use old_dep;\n")
            .await
            .expect("seed old_name");
        env.write_file("delete_me.txt", "bye\n")
            .await
            .expect("seed delete_me");

        let registry = build_openai_tool_registry();
        let patch = "\
*** Begin Patch
*** Add File: new_file.txt
+alpha
+beta
*** Update File: a.txt
@@ replace line
 line1
-line2
+line-two
*** Update File: old_name.txt
*** Move to: new_name.txt
@@ rename import
-use old_dep;
+use new_dep;
*** Delete File: delete_me.txt
*** End Patch";

        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: APPLY_PATCH_TOOL.to_string(),
                    arguments: json!({
                        "patch": patch
                    }),
                    raw_arguments: None,
                }],
                env.clone(),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(!results[0].is_error);
        let summary = results[0].content.as_str().unwrap_or_default();
        assert!(summary.contains("A new_file.txt"));
        assert!(summary.contains("M a.txt"));
        assert!(summary.contains("R old_name.txt -> new_name.txt"));
        assert!(summary.contains("D delete_me.txt"));

        let updated_a = env
            .read_file("a.txt", None, None)
            .await
            .expect("updated a.txt should read");
        assert_eq!(updated_a, "line1\nline-two\n");

        let new_file = env
            .read_file("new_file.txt", None, None)
            .await
            .expect("new file should read");
        assert_eq!(new_file, "alpha\nbeta");

        let renamed = env
            .read_file("new_name.txt", None, None)
            .await
            .expect("renamed file should read");
        assert_eq!(renamed, "use new_dep;\n");

        assert!(
            !env.file_exists("old_name.txt")
                .await
                .expect("old name existence should be checked")
        );
        assert!(
            !env.file_exists("delete_me.txt")
                .await
                .expect("delete target existence should be checked")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn apply_patch_fuzzy_hunk_matching_recovers_on_whitespace_differences() {
        let dir = tempdir().expect("temp dir should be created");
        let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
        env.write_file("fuzzy.txt", "fn  greet() {\n    println!(\"hi\");\n}\n")
            .await
            .expect("seed file should write");

        let registry = build_openai_tool_registry();
        let patch = "\
*** Begin Patch
*** Update File: fuzzy.txt
@@ update greeting
-fn greet() {
-    println!(\"hi\");
+fn greet() {
+    println!(\"hello\");
 }
*** End Patch";

        let results = registry
            .dispatch(
                vec![ToolCall {
                    id: "call-1".to_string(),
                    name: APPLY_PATCH_TOOL.to_string(),
                    arguments: json!({ "patch": patch }),
                    raw_arguments: None,
                }],
                env.clone(),
                &SessionConfig::default(),
                Arc::new(NoopEventEmitter),
                ToolDispatchOptions {
                    session_id: "session-1".to_string(),
                    supports_parallel_tool_calls: false,
                },
            )
            .await
            .expect("dispatch should succeed");

        assert!(!results[0].is_error);
        let updated = env
            .read_file("fuzzy.txt", None, None)
            .await
            .expect("updated file should read");
        assert!(updated.contains("println!(\"hello\")"));
    }
}
