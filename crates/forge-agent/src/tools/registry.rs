use crate::{
    AgentError, EventEmitter, ExecutionEnvironment, SessionConfig, SessionEvent,
    truncate_tool_output,
};
use async_trait::async_trait;
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

#[derive(Clone, Debug, PartialEq)]
pub struct ToolHookContext {
    pub session_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolPreHookOutcome {
    Continue,
    Skip { message: String, is_error: bool },
    Fail { message: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolPostHookContext {
    pub tool: ToolHookContext,
    pub duration_ms: u128,
    pub output: Option<String>,
    pub error: Option<String>,
    pub is_error: bool,
}

#[async_trait]
pub trait ToolCallHook: Send + Sync {
    async fn before_tool_call(
        &self,
        _context: &ToolHookContext,
    ) -> Result<ToolPreHookOutcome, AgentError> {
        Ok(ToolPreHookOutcome::Continue)
    }

    async fn after_tool_call(&self, _context: &ToolPostHookContext) -> Result<(), AgentError> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct ToolDispatchOptions {
    pub session_id: String,
    pub supports_parallel_tool_calls: bool,
    pub hook: Option<Arc<dyn ToolCallHook>>,
    pub hook_strict: bool,
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
                    &options,
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
                    &options,
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
        options: &ToolDispatchOptions,
    ) -> Result<ToolResult, AgentError> {
        let session_id = &options.session_id;
        let start_time = std::time::Instant::now();
        let parsed_arguments = match super::parse_tool_arguments(&tool_call) {
            Ok(arguments) => arguments,
            Err(error) => {
                let duration_ms = start_time.elapsed().as_millis();
                event_emitter.emit(SessionEvent::tool_call_end(
                    session_id.to_string(),
                    tool_call.id.clone(),
                    Option::<String>::None,
                    Some(error.to_string()),
                    duration_ms,
                    true,
                ))?;
                return Ok(super::tool_error_result(tool_call.id, error.to_string()));
            }
        };
        let hook_context = ToolHookContext {
            session_id: session_id.to_string(),
            call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            arguments: parsed_arguments.clone(),
        };

        event_emitter.emit(SessionEvent::tool_call_start(
            session_id.to_string(),
            tool_call.name.clone(),
            tool_call.id.clone(),
            Some(parsed_arguments.clone()),
        ))?;

        if let Some(hook) = &options.hook {
            match hook.before_tool_call(&hook_context).await {
                Ok(ToolPreHookOutcome::Continue) => {}
                Ok(ToolPreHookOutcome::Skip { message, is_error }) => {
                    let duration_ms = start_time.elapsed().as_millis();
                    event_emitter.emit(SessionEvent::warning(
                        session_id.to_string(),
                        format!("tool pre-hook skipped '{}': {}", tool_call.name, message),
                    ))?;
                    event_emitter.emit(SessionEvent::tool_call_end(
                        session_id.to_string(),
                        tool_call.id,
                        None,
                        if is_error {
                            Some(message.clone())
                        } else {
                            None
                        },
                        duration_ms,
                        is_error,
                    ))?;
                    return Ok(ToolResult {
                        tool_call_id: hook_context.call_id,
                        content: Value::String(message),
                        is_error,
                    });
                }
                Ok(ToolPreHookOutcome::Fail { message }) => {
                    let duration_ms = start_time.elapsed().as_millis();
                    event_emitter.emit(SessionEvent::error(
                        session_id.to_string(),
                        format!("tool pre-hook failed '{}': {}", tool_call.name, message),
                    ))?;
                    event_emitter.emit(SessionEvent::tool_call_end(
                        session_id.to_string(),
                        tool_call.id,
                        None,
                        Some(message.clone()),
                        duration_ms,
                        true,
                    ))?;
                    return Ok(super::tool_error_result(hook_context.call_id, message));
                }
                Err(error) => {
                    if options.hook_strict {
                        let message =
                            format!("tool pre-hook error for '{}': {}", tool_call.name, error);
                        let duration_ms = start_time.elapsed().as_millis();
                        event_emitter
                            .emit(SessionEvent::error(session_id.to_string(), message.clone()))?;
                        event_emitter.emit(SessionEvent::tool_call_end(
                            session_id.to_string(),
                            tool_call.id,
                            None,
                            Some(message.clone()),
                            duration_ms,
                            true,
                        ))?;
                        return Ok(super::tool_error_result(hook_context.call_id, message));
                    }
                    event_emitter.emit(SessionEvent::warning(
                        session_id.to_string(),
                        format!(
                            "tool pre-hook error for '{}': {}; continuing",
                            tool_call.name, error
                        ),
                    ))?;
                }
            }
        }

        let Some(registered) = self.get(&tool_call.name) else {
            let message = format!("Unknown tool: {}", tool_call.name);
            let duration_ms = start_time.elapsed().as_millis();
            event_emitter.emit(SessionEvent::tool_call_end(
                session_id.to_string(),
                tool_call.id.clone(),
                None,
                Some(message.clone()),
                duration_ms,
                true,
            ))?;
            return Ok(super::tool_error_result(tool_call.id, message));
        };

        let parsed_arguments = super::normalize_tool_arguments_for_dispatch(
            &tool_call.name,
            parsed_arguments,
            &registered.definition.parameters,
            config,
        );

        if let Err(error) =
            super::validate_tool_arguments(&registered.definition.parameters, &parsed_arguments)
        {
            let duration_ms = start_time.elapsed().as_millis();
            event_emitter.emit(SessionEvent::tool_call_end(
                session_id.to_string(),
                tool_call.id.clone(),
                None,
                Some(error.to_string()),
                duration_ms,
                true,
            ))?;
            return Ok(super::tool_error_result(tool_call.id, error.to_string()));
        }

        let raw_output = match (registered.executor)(parsed_arguments, execution_env).await {
            Ok(output) => output,
            Err(error) => {
                let error_text = error.to_string();
                let duration_ms = start_time.elapsed().as_millis();
                event_emitter.emit(SessionEvent::tool_call_end(
                    session_id.to_string(),
                    tool_call.id.clone(),
                    None,
                    Some(error_text.clone()),
                    duration_ms,
                    true,
                ))?;

                if let Some(hook) = &options.hook {
                    let post_ctx = ToolPostHookContext {
                        tool: hook_context.clone(),
                        duration_ms,
                        output: None,
                        error: Some(error_text.clone()),
                        is_error: true,
                    };
                    if let Err(hook_error) = hook.after_tool_call(&post_ctx).await {
                        if options.hook_strict {
                            return Ok(super::tool_error_result(
                                tool_call.id,
                                format!(
                                    "tool post-hook error for '{}': {}",
                                    tool_call.name, hook_error
                                ),
                            ));
                        }
                        event_emitter.emit(SessionEvent::warning(
                            session_id.to_string(),
                            format!(
                                "tool post-hook error for '{}': {}; continuing",
                                tool_call.name, hook_error
                            ),
                        ))?;
                    }
                }

                return Ok(super::tool_error_result(tool_call.id, error_text));
            }
        };

        if !raw_output.is_empty() {
            event_emitter.emit(SessionEvent::tool_call_output_delta(
                session_id.to_string(),
                tool_call.id.clone(),
                raw_output.clone(),
            ))?;
        }
        let truncated = truncate_tool_output(&raw_output, &tool_call.name, config);
        let duration_ms = start_time.elapsed().as_millis();
        event_emitter.emit(SessionEvent::tool_call_end(
            session_id.to_string(),
            tool_call.id.clone(),
            Some(raw_output.clone()),
            None,
            duration_ms,
            false,
        ))?;

        if let Some(hook) = &options.hook {
            let post_ctx = ToolPostHookContext {
                tool: hook_context,
                duration_ms,
                output: Some(raw_output),
                error: None,
                is_error: false,
            };
            if let Err(error) = hook.after_tool_call(&post_ctx).await {
                if options.hook_strict {
                    return Ok(super::tool_error_result(
                        tool_call.id,
                        format!("tool post-hook error for '{}': {}", tool_call.name, error),
                    ));
                }
                event_emitter.emit(SessionEvent::warning(
                    session_id.to_string(),
                    format!(
                        "tool post-hook error for '{}': {}; continuing",
                        tool_call.name, error
                    ),
                ))?;
            }
        }

        Ok(ToolResult {
            tool_call_id: tool_call.id,
            content: Value::String(truncated),
            is_error: false,
        })
    }
}
