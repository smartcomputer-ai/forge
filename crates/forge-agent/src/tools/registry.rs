use crate::{
    AgentError, EventEmitter, ExecutionEnvironment, SessionConfig, SessionEvent,
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
            return Ok(super::tool_error_result(tool_call.id, message));
        };

        let parsed_arguments = match super::parse_tool_arguments(&tool_call) {
            Ok(arguments) => arguments,
            Err(error) => {
                event_emitter.emit(SessionEvent::tool_call_end_error(
                    session_id.to_string(),
                    tool_call.id.clone(),
                    error.to_string(),
                ))?;
                return Ok(super::tool_error_result(tool_call.id, error.to_string()));
            }
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
            event_emitter.emit(SessionEvent::tool_call_end_error(
                session_id.to_string(),
                tool_call.id.clone(),
                error.to_string(),
            ))?;
            return Ok(super::tool_error_result(tool_call.id, error.to_string()));
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
