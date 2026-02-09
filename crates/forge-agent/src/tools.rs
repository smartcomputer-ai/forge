use crate::{
    AgentError, EventEmitter, ExecutionEnvironment, SessionConfig, SessionEvent, ToolError,
    truncate_tool_output,
};
use forge_llm::{ToolCall, ToolDefinition, ToolResult};
use futures::future::join_all;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type ToolFuture = Pin<Box<dyn Future<Output = Result<String, AgentError>> + Send>>;
pub type ToolExecutor =
    Arc<dyn Fn(Value, Arc<dyn ExecutionEnvironment>) -> ToolFuture + Send + Sync>;

#[derive(Clone)]
pub struct ToolDispatchOptions {
    pub session_id: String,
    pub supports_parallel_tool_calls: bool,
}

#[derive(Clone)]
pub struct RegisteredTool {
    pub definition: ToolDefinition,
    pub executor: ToolExecutor,
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: HashMap<String, RegisteredTool>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: RegisteredTool) {
        self.tools.insert(tool.definition.name.clone(), tool);
    }

    pub fn unregister(&mut self, name: &str) -> Option<RegisteredTool> {
        self.tools.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools.get(name)
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions: Vec<ToolDefinition> = self
            .tools
            .values()
            .map(|tool| tool.definition.clone())
            .collect();
        definitions.sort_by(|a, b| a.name.cmp(&b.name));
        definitions
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tools.keys().cloned().collect();
        names.sort_unstable();
        names
    }

    pub async fn dispatch(
        &self,
        tool_calls: Vec<ToolCall>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        config: &SessionConfig,
        event_emitter: Arc<dyn EventEmitter>,
        options: ToolDispatchOptions,
    ) -> Result<Vec<ToolResult>, AgentError> {
        if options.supports_parallel_tool_calls && tool_calls.len() > 1 {
            let futures = tool_calls.into_iter().map(|tool_call| {
                self.dispatch_single(
                    tool_call,
                    execution_env.clone(),
                    config,
                    event_emitter.clone(),
                    &options.session_id,
                )
            });
            return Ok(join_all(futures)
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?);
        }

        let mut results = Vec::with_capacity(tool_calls.len());
        for tool_call in tool_calls {
            results.push(
                self.dispatch_single(
                    tool_call,
                    execution_env.clone(),
                    config,
                    event_emitter.clone(),
                    &options.session_id,
                )
                .await?,
            );
        }
        Ok(results)
    }

    async fn dispatch_single(
        &self,
        tool_call: ToolCall,
        execution_env: Arc<dyn ExecutionEnvironment>,
        config: &SessionConfig,
        event_emitter: Arc<dyn EventEmitter>,
        session_id: &str,
    ) -> Result<ToolResult, AgentError> {
        event_emitter.emit(SessionEvent::tool_call_start(
            session_id.to_string(),
            tool_call.name.clone(),
            tool_call.id.clone(),
        ))?;

        let Some(registered) = self.get(&tool_call.name) else {
            let message = format!("Unknown tool: {}", tool_call.name);
            event_emitter.emit(SessionEvent::tool_call_end_error(
                session_id.to_string(),
                tool_call.id.clone(),
                message.clone(),
            ))?;
            return Ok(tool_error_result(tool_call.id, message));
        };

        let parsed_arguments = match parse_tool_arguments(&tool_call) {
            Ok(arguments) => arguments,
            Err(error) => {
                event_emitter.emit(SessionEvent::tool_call_end_error(
                    session_id.to_string(),
                    tool_call.id.clone(),
                    error.to_string(),
                ))?;
                return Ok(tool_error_result(tool_call.id, error.to_string()));
            }
        };

        if let Err(error) =
            validate_tool_arguments(&registered.definition.parameters, &parsed_arguments)
        {
            event_emitter.emit(SessionEvent::tool_call_end_error(
                session_id.to_string(),
                tool_call.id.clone(),
                error.to_string(),
            ))?;
            return Ok(tool_error_result(tool_call.id, error.to_string()));
        }

        let raw_output = match (registered.executor)(parsed_arguments, execution_env).await {
            Ok(output) => output,
            Err(error) => {
                let error_text = error.to_string();
                event_emitter.emit(SessionEvent::tool_call_end_error(
                    session_id.to_string(),
                    tool_call.id.clone(),
                    error_text.clone(),
                ))?;
                return Ok(tool_error_result(tool_call.id, error_text));
            }
        };

        let truncated = truncate_tool_output(&raw_output, &tool_call.name, config);
        event_emitter.emit(SessionEvent::tool_call_end_output(
            session_id.to_string(),
            tool_call.id.clone(),
            raw_output,
        ))?;

        Ok(ToolResult {
            tool_call_id: tool_call.id,
            content: Value::String(truncated),
            is_error: false,
        })
    }
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
    use crate::{BufferedEventEmitter, EventKind, NoopEventEmitter};
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
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
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, EventKind::ToolCallStart);
        assert_eq!(events[1].kind, EventKind::ToolCallEnd);
        assert_eq!(events[0].data.get_str("call_id"), Some("call-1"));
        assert_eq!(events[1].data.get_str("call_id"), Some("call-1"));
        assert_eq!(events[1].data.get_str("output"), Some("done"));
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
}
