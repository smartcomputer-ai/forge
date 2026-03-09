//! Unified agent provider contract.
//!
//! Every provider — HTTP API or CLI subprocess — implements `AgentProvider`.
//! The provider owns the complete agent cycle: prompt in, final answer out.
//! See spec/06-unified-agent-provider-spec.md.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::SDKError;
use crate::types::Usage;

/// A provider that owns the complete agent cycle:
/// prompt → [LLM call → tool execution → repeat] → final answer.
///
/// CLI agents (Claude Code, Codex, Gemini) run this internally in their subprocess.
/// HTTP API agents compose `ProviderAdapter` + tools + execution environment to
/// run the loop in-process.
#[async_trait]
pub trait AgentProvider: Send + Sync {
    /// Human-readable name for this provider.
    fn name(&self) -> &str;

    /// Run the agent loop to completion. Returns the final result.
    async fn run_to_completion(
        &self,
        prompt: &str,
        options: &AgentRunOptions,
    ) -> Result<AgentRunResult, SDKError>;
}

/// Options for a single agent run.
#[derive(Clone, Default)]
pub struct AgentRunOptions {
    /// Working directory the agent should operate in.
    pub working_directory: PathBuf,
    /// Override the provider's default model.
    pub model_override: Option<String>,
    /// Maximum LLM call rounds.
    pub max_turns: Option<usize>,
    /// Maximum tool execution rounds per input.
    pub max_tool_rounds: Option<usize>,
    /// Reasoning effort level ("low", "medium", "high").
    pub reasoning_effort: Option<String>,
    /// Override the system prompt.
    pub system_prompt_override: Option<String>,
    /// Environment variables for subprocess / tool execution.
    pub env_vars: Option<HashMap<String, String>>,
    /// Real-time event callback for observability.
    pub on_event: Option<Arc<dyn Fn(AgentLoopEvent) + Send + Sync>>,
}

impl std::fmt::Debug for AgentRunOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentRunOptions")
            .field("working_directory", &self.working_directory)
            .field("model_override", &self.model_override)
            .field("max_turns", &self.max_turns)
            .field("max_tool_rounds", &self.max_tool_rounds)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("system_prompt_override", &self.system_prompt_override)
            .field("env_vars", &self.env_vars)
            .field("on_event", &self.on_event.as_ref().map(|_| "..."))
            .finish()
    }
}

/// Result of a completed agent run.
#[derive(Clone, Debug)]
pub struct AgentRunResult {
    /// Final text response from the agent.
    pub text: String,
    /// Tool calls that were executed during the run (for observability).
    pub tool_activity: Vec<ToolActivityRecord>,
    /// Aggregated token usage across all internal LLM calls.
    pub usage: Usage,
    /// Synthetic response ID.
    pub id: String,
    /// The model that was used.
    pub model: String,
    /// The provider name.
    pub provider: String,
    /// Total cost in USD, if known.
    pub cost_usd: Option<f64>,
    /// Wall clock duration in milliseconds.
    pub duration_ms: Option<u64>,
}

/// Record of a tool call that the provider executed internally.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolActivityRecord {
    /// Name of the tool that was called.
    pub tool_name: String,
    /// Unique ID for this tool call.
    pub call_id: String,
    /// Truncated summary of arguments (for observability, not full payload).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments_summary: Option<String>,
    /// Truncated summary of the result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_summary: Option<String>,
    /// Whether the tool call resulted in an error.
    pub is_error: bool,
    /// Duration in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Events emitted by a provider-managed agent loop for real-time observability.
#[derive(Clone, Debug)]
pub enum AgentLoopEvent {
    /// The agent produced a text delta.
    TextDelta {
        delta: String,
    },
    /// The agent started a tool call.
    ToolCallStart {
        call_id: String,
        tool_name: String,
        arguments: Value,
    },
    /// The agent's tool call completed.
    ToolCallEnd {
        call_id: String,
        output: String,
        is_error: bool,
        duration_ms: u64,
    },
    /// A warning from the agent loop (e.g., context window usage).
    Warning {
        message: String,
    },
}
