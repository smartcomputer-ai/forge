//! HTTP API agent provider.
//!
//! Wraps `ProviderAdapter` + `ToolRegistry` + `ExecutionEnvironment` and runs
//! the same tool loop that `Session::submit_single()` uses, extracted into the
//! unified `AgentProvider` trait. This is the provider used for raw HTTP API
//! backends (OpenAI, Anthropic, etc.) where forge manages the tool loop.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use forge_llm::agent_provider::{
    AgentLoopEvent, AgentProvider, AgentRunOptions, AgentRunResult, ToolActivityRecord,
};
use forge_llm::{Client, Message, Request, SDKError, ToolChoice, Usage};

use crate::config::SessionConfig;
use crate::errors::AgentError;
use crate::events::{EventEmitter, NoopEventEmitter};
use crate::execution::ExecutionEnvironment;
use crate::profiles::ProviderProfile;
use crate::session::utils::{
    approximate_context_tokens, build_environment_context_snapshot, convert_history_to_messages,
    current_timestamp, detect_loop, discover_project_documents, validate_reasoning_effort,
};
use crate::tools::ToolDispatchOptions;
use crate::turn::{AssistantTurn, SteeringTurn, ToolResultTurn, ToolResultsTurn, Turn, UserTurn};

/// Agent provider backed by a raw HTTP LLM API + forge's tool registry.
///
/// This is the existing tool loop from `Session::submit_single()`, extracted
/// into the unified `AgentProvider` trait. It composes `ProviderAdapter` (via
/// `Client`) + `ToolRegistry` (via `ProviderProfile`) + `ExecutionEnvironment`
/// to run the complete agent cycle in-process.
pub struct HttpApiAgentProvider {
    llm_client: Arc<Client>,
    provider_profile: Arc<dyn ProviderProfile>,
    execution_env: Arc<dyn ExecutionEnvironment>,
    event_emitter: Arc<dyn EventEmitter>,
    config: SessionConfig,
}

impl HttpApiAgentProvider {
    pub fn new(
        llm_client: Arc<Client>,
        provider_profile: Arc<dyn ProviderProfile>,
        execution_env: Arc<dyn ExecutionEnvironment>,
        config: SessionConfig,
    ) -> Self {
        Self {
            llm_client,
            provider_profile,
            execution_env,
            event_emitter: Arc::new(NoopEventEmitter),
            config,
        }
    }

    pub fn with_event_emitter(mut self, emitter: Arc<dyn EventEmitter>) -> Self {
        self.event_emitter = emitter;
        self
    }
}

#[async_trait]
impl AgentProvider for HttpApiAgentProvider {
    fn name(&self) -> &str {
        self.provider_profile.id()
    }

    async fn run_to_completion(
        &self,
        prompt: &str,
        options: &AgentRunOptions,
    ) -> Result<AgentRunResult, SDKError> {
        let start = Instant::now();

        // Internal history for this run.
        let mut history: Vec<Turn> = Vec::new();
        let mut tool_activity: Vec<ToolActivityRecord> = Vec::new();
        let mut total_usage = Usage::default();
        let mut final_text = String::new();
        let mut response_id = String::new();
        let mut call_counter = 0u64;

        // Push initial user turn.
        let user_turn = Turn::User(UserTurn::new(prompt.to_string(), current_timestamp()));
        history.push(user_turn);

        let max_tool_rounds = options
            .max_tool_rounds
            .unwrap_or(self.config.max_tool_rounds_per_input);
        let max_turns = options.max_turns.unwrap_or(self.config.max_turns);

        let mut round_count = 0usize;
        let mut context_warning_emitted = false;

        loop {
            // Check tool round limit.
            if round_count >= max_tool_rounds {
                if let Some(ref on_event) = options.on_event {
                    on_event(AgentLoopEvent::Warning {
                        message: format!("Tool round limit reached ({} rounds)", max_tool_rounds),
                    });
                }
                break;
            }

            // Check total turn limit.
            if max_turns > 0 && history.len() >= max_turns {
                if let Some(ref on_event) = options.on_event {
                    on_event(AgentLoopEvent::Warning {
                        message: format!("Turn limit reached ({} turns)", max_turns),
                    });
                }
                break;
            }

            // Context window warning.
            if !context_warning_emitted {
                let context_window_size = self.provider_profile.capabilities().context_window_size;
                if context_window_size > 0 {
                    let approx_tokens = approximate_context_tokens(&history);
                    let warning_threshold = context_window_size.saturating_mul(8) / 10;
                    if approx_tokens > warning_threshold {
                        context_warning_emitted = true;
                        if let Some(ref on_event) = options.on_event {
                            let usage_pct = ((approx_tokens as f64 / context_window_size as f64)
                                * 100.0)
                                .round();
                            on_event(AgentLoopEvent::Warning {
                                message: format!(
                                    "Context window usage at ~{}% ({}/{} tokens)",
                                    usage_pct as usize, approx_tokens, context_window_size
                                ),
                            });
                        }
                    }
                }
            }

            // Build LLM request.
            let request = self
                .build_request(&history, options)
                .map_err(|e| sdk_error_from_agent_error(e))?;

            // Call LLM.
            let response = self.llm_client.complete(request).await?;

            let text = response.text();
            let tool_calls = response.tool_calls();
            let reasoning = response.reasoning();

            // Emit text delta.
            if !text.is_empty() {
                if let Some(ref on_event) = options.on_event {
                    on_event(AgentLoopEvent::TextDelta {
                        delta: text.clone(),
                    });
                }
                final_text = text.clone();
            }

            // Accumulate usage.
            total_usage = total_usage + response.usage.clone();
            response_id = response.id.clone();

            // Record assistant turn in history.
            let assistant_turn = Turn::Assistant(AssistantTurn::new(
                text.clone(),
                tool_calls.clone(),
                reasoning,
                response.usage.clone(),
                Some(response.id),
                current_timestamp(),
            ));
            history.push(assistant_turn);

            // No tool calls → done.
            if tool_calls.is_empty() {
                break;
            }

            // Execute tool calls.
            round_count += 1;
            let supports_parallel = self
                .provider_profile
                .capabilities()
                .supports_parallel_tool_calls;

            let results = self
                .provider_profile
                .tool_registry()
                .dispatch(
                    tool_calls.clone(),
                    self.execution_env.clone(),
                    &self.config,
                    self.event_emitter.clone(),
                    ToolDispatchOptions {
                        session_id: format!("http-agent-{}", start.elapsed().as_nanos()),
                        supports_parallel_tool_calls: supports_parallel,
                        hook: None,
                        hook_strict: false,
                    },
                )
                .await
                .map_err(|e| sdk_error_from_agent_error(e))?;

            // Record tool activity for observability.
            for (i, tc) in tool_calls.iter().enumerate() {
                call_counter += 1;
                let result = results.get(i);
                let is_error = result.map_or(false, |r| r.is_error);
                let result_summary = result.map(|r| {
                    let s = r.content.to_string();
                    if s.len() <= 200 {
                        s
                    } else {
                        format!("{}...", &s[..200])
                    }
                });

                let call_id = tc.id.clone();

                if let Some(ref on_event) = options.on_event {
                    on_event(AgentLoopEvent::ToolCallStart {
                        call_id: call_id.clone(),
                        tool_name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                    });

                    if let Some(ref summary) = result_summary {
                        on_event(AgentLoopEvent::ToolCallEnd {
                            call_id: call_id.clone(),
                            output: summary.clone(),
                            is_error,
                            duration_ms: 0,
                        });
                    }
                }

                tool_activity.push(ToolActivityRecord {
                    tool_name: tc.name.clone(),
                    call_id: format!("http-tc-{}", call_counter),
                    arguments_summary: Some(truncate_value(&tc.arguments, 200)),
                    result_summary,
                    is_error,
                    duration_ms: None,
                });
            }

            // Append tool results to history.
            let result_turns: Vec<ToolResultTurn> = results
                .into_iter()
                .map(|result| ToolResultTurn {
                    tool_call_id: result.tool_call_id,
                    content: result.content,
                    is_error: result.is_error,
                })
                .collect();
            let tool_results_turn =
                Turn::ToolResults(ToolResultsTurn::new(result_turns, current_timestamp()));
            history.push(tool_results_turn);

            // Loop detection.
            if self.config.enable_loop_detection
                && detect_loop(&history, self.config.loop_detection_window)
            {
                let warning = format!(
                    "Loop detected: the last {} tool calls follow a repeating pattern. Try a different approach.",
                    self.config.loop_detection_window
                );
                // Don't double-inject.
                let already_warned = matches!(
                    history.last(),
                    Some(Turn::Steering(turn)) if turn.content == warning
                );
                if !already_warned {
                    history.push(Turn::Steering(SteeringTurn::new(
                        warning.clone(),
                        current_timestamp(),
                    )));
                    if let Some(ref on_event) = options.on_event {
                        on_event(AgentLoopEvent::Warning { message: warning });
                    }
                }
            }
        }

        let elapsed = start.elapsed();
        let model = self.provider_profile.model().to_string();

        Ok(AgentRunResult {
            text: final_text,
            tool_activity,
            usage: total_usage,
            id: response_id,
            model,
            provider: self.provider_profile.id().to_string(),
            cost_usd: None,
            duration_ms: Some(elapsed.as_millis() as u64),
        })
    }
}

impl HttpApiAgentProvider {
    /// Build an LLM request from the current history, mirroring
    /// `Session::build_request()`.
    fn build_request(
        &self,
        history: &[Turn],
        options: &AgentRunOptions,
    ) -> Result<Request, AgentError> {
        let tools = self.provider_profile.tools();
        let environment_context = build_environment_context_snapshot(
            self.provider_profile.as_ref(),
            self.execution_env.as_ref(),
        );
        let project_docs = discover_project_documents(
            self.execution_env.working_directory(),
            self.provider_profile.as_ref(),
        );
        let system_prompt = self.provider_profile.build_system_prompt(
            &environment_context,
            &tools,
            &project_docs,
            options
                .system_prompt_override
                .as_deref()
                .or(self.config.system_prompt_override.as_deref()),
        );

        let mut messages = vec![Message::system(system_prompt)];
        messages.extend(convert_history_to_messages(history));

        let tools = if tools.is_empty() { None } else { Some(tools) };
        let tool_choice = tools.as_ref().map(|_| ToolChoice {
            mode: "auto".to_string(),
            tool_name: None,
        });

        if let Some(value) = options.reasoning_effort.as_deref() {
            validate_reasoning_effort(value)?;
        }
        let reasoning_effort = options
            .reasoning_effort
            .as_ref()
            .map(|value| value.to_ascii_lowercase())
            .or_else(|| self.config.reasoning_effort.clone());

        let model = options
            .model_override
            .as_deref()
            .unwrap_or_else(|| self.provider_profile.model());

        let provider_options = self.provider_profile.provider_options();

        Ok(Request {
            model: model.to_string(),
            messages,
            provider: Some(self.provider_profile.id().to_string()),
            tools,
            tool_choice,
            response_format: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            stop_sequences: None,
            reasoning_effort,
            metadata: None,
            provider_options,
        })
    }
}

fn truncate_value(value: &serde_json::Value, max_len: usize) -> String {
    let s = value.to_string();
    if s.len() <= max_len {
        s
    } else {
        format!("{}...", &s[..max_len])
    }
}

fn sdk_error_from_agent_error(e: AgentError) -> SDKError {
    SDKError::Other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::ProviderCapabilities;
    use forge_llm::types::ToolDefinition;
    use serde_json::json;

    // --- Minimal test doubles ---

    #[allow(dead_code)]
    struct TestProviderProfile {
        tools: Vec<ToolDefinition>,
        tool_registry: Arc<crate::tools::ToolRegistry>,
    }

    #[allow(dead_code)]
    impl TestProviderProfile {
        fn new() -> Self {
            Self {
                tools: Vec::new(),
                tool_registry: Arc::new(crate::tools::ToolRegistry::default()),
            }
        }
    }

    impl ProviderProfile for TestProviderProfile {
        fn id(&self) -> &str {
            "test"
        }
        fn model(&self) -> &str {
            "test-model"
        }
        fn tool_registry(&self) -> Arc<crate::tools::ToolRegistry> {
            self.tool_registry.clone()
        }
        fn base_instructions(&self) -> &str {
            "You are a test agent."
        }
        fn project_instruction_files(&self) -> Vec<String> {
            Vec::new()
        }
        fn build_system_prompt(
            &self,
            _env: &crate::profiles::EnvironmentContext,
            _tools: &[ToolDefinition],
            _docs: &[crate::profiles::ProjectDocument],
            _override: Option<&str>,
        ) -> String {
            "Test system prompt.".to_string()
        }
        fn tools(&self) -> Vec<ToolDefinition> {
            self.tools.clone()
        }
        fn provider_options(&self) -> Option<serde_json::Value> {
            None
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                supports_reasoning: false,
                supports_streaming: false,
                supports_parallel_tool_calls: false,
                context_window_size: 128_000,
            }
        }
        fn knowledge_cutoff(&self) -> Option<&str> {
            None
        }
    }

    #[test]
    fn truncate_value_short_unchanged() {
        let v = json!({"hello": "world"});
        let result = truncate_value(&v, 200);
        assert_eq!(result, v.to_string());
    }

    #[test]
    fn truncate_value_long_truncated() {
        let v = json!({"data": "x".repeat(300)});
        let result = truncate_value(&v, 50);
        assert!(result.len() <= 54); // 50 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn sdk_error_from_agent_error_converts() {
        let err = AgentError::session_closed();
        let sdk_err = sdk_error_from_agent_error(err);
        match sdk_err {
            SDKError::Other(msg) => assert!(!msg.is_empty()),
            _ => panic!("expected SDKError::Other"),
        }
    }
}
