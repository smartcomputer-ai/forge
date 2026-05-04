//! Adapter that wraps any `AgentProvider` (from `forge-llm`) to implement
//! the `AgentSubmitter` trait, allowing CLI agent providers (Claude Code,
//! Codex, Gemini) to plug into the Attractor pipeline.

use crate::backends::forge_agent::AgentSubmitter;
use async_trait::async_trait;
use forge_agent::{
    AgentError, SessionPersistenceSnapshot, SessionState, SubmitOptions, SubmitResult, ToolCallHook,
};
use forge_llm::agent_provider::{AgentProvider, AgentRunOptions};
use std::path::PathBuf;
use std::sync::Arc;

/// Wraps an `AgentProvider` to implement `AgentSubmitter`.
///
/// This adapter enables CLI agent providers (Claude Code, Codex CLI, Gemini CLI)
/// to serve as backends for Attractor pipeline stages. The provider owns the
/// complete agent loop; the adapter translates between `SubmitOptions` and
/// `AgentRunOptions`, and maps `AgentRunResult` back to `SubmitResult`.
pub struct AgentProviderSubmitter {
    provider: Arc<dyn AgentProvider>,
    working_directory: PathBuf,
    thread_key: Option<String>,
}

impl AgentProviderSubmitter {
    pub fn new(provider: Arc<dyn AgentProvider>, working_directory: PathBuf) -> Self {
        Self {
            provider,
            working_directory,
            thread_key: None,
        }
    }
}

#[async_trait]
impl AgentSubmitter for AgentProviderSubmitter {
    async fn submit_with_result(
        &mut self,
        user_input: String,
        options: SubmitOptions,
    ) -> Result<SubmitResult, AgentError> {
        let run_options = AgentRunOptions {
            working_directory: self.working_directory.clone(),
            model_override: options.model,
            reasoning_effort: options.reasoning_effort,
            system_prompt_override: options.system_prompt_override,
            ..Default::default()
        };

        let result = self
            .provider
            .run_to_completion(&user_input, &run_options)
            .await
            .map_err(AgentError::Llm)?;

        let tool_call_count = result.tool_activity.len();
        let tool_call_ids: Vec<String> = result
            .tool_activity
            .iter()
            .map(|t| t.call_id.clone())
            .collect();
        let tool_error_count = result.tool_activity.iter().filter(|t| t.is_error).count();

        Ok(SubmitResult {
            final_state: SessionState::Idle,
            assistant_text: result.text,
            tool_call_count,
            tool_call_ids,
            tool_error_count,
            usage: Some(result.usage),
            thread_key: self.thread_key.clone(),
        })
    }

    fn thread_key(&self) -> Option<&str> {
        self.thread_key.as_deref()
    }

    fn set_thread_key(&mut self, thread_key: Option<String>) {
        self.thread_key = thread_key;
    }

    fn session_id(&self) -> &str {
        "agent-provider"
    }

    fn set_tool_call_hook(&mut self, _hook: Option<Arc<dyn ToolCallHook>>) {
        // CLI agent providers manage their own tools; hook is not applicable.
    }

    async fn persistence_snapshot(&mut self) -> Result<SessionPersistenceSnapshot, AgentError> {
        Ok(SessionPersistenceSnapshot::default())
    }
}
