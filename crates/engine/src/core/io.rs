//! CoreAgent runtime I/O traits and request/result records.
//!
//! These traits are specific to CoreAgent; the lower-level session kernel
//! should not impose this I/O shape on custom agents.
//!
//! `LlmGenerationRequest`, `LlmGenerationResult`,
//! `ToolInvocationBatchRequest`, and `ToolInvocationBatchResult` are shared
//! serializable records used by both local and workflow substrates. The
//! `CoreAgentLlm` and `CoreAgentTools` traits are execution adapter traits for
//! local runtimes, tests, and workflow activities. Workflow code that cannot
//! hold `Send + Sync` async adapters should fulfill `CoreAgentAction` values
//! directly instead of implementing these traits inside the workflow.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AwaitSpec, BlobRef, ContextCompactionRequest, ContextCompactionResult, ContextEntryInput,
    ContextEntryKind, EnvironmentId, LlmGenerationFacts, LlmGenerationStatus, LlmRequest,
    PromiseId, PromiseOwnership, PromiseScope, PromiseStatus, RunId, SessionId, ToolBatchId,
    ToolCallId, ToolCallStatus, ToolName, TurnId, WorkflowToolBinding, WorkspaceLink,
};

#[async_trait]
pub trait CoreAgentLlm: Send + Sync {
    async fn generate(
        &self,
        request: LlmGenerationRequest,
    ) -> Result<LlmGenerationResult, CoreAgentIoError>;

    async fn compact_context(
        &self,
        request: ContextCompactionRequest,
    ) -> Result<ContextCompactionResult, CoreAgentIoError> {
        let _ = request;
        Err(CoreAgentIoError::Failed {
            message: "context compaction runtime unavailable".to_owned(),
        })
    }
}

#[async_trait]
pub trait CoreAgentTools: Send + Sync {
    async fn invoke_batch(
        &self,
        request: ToolInvocationBatchRequest,
    ) -> Result<ToolBatchOutcome, CoreAgentIoError>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmGenerationRequest {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: TurnId,
    pub request: LlmRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmGenerationResult {
    pub run_id: RunId,
    pub turn_id: TurnId,
    pub status: LlmGenerationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_ref: Option<BlobRef>,
    pub context_entries: Vec<ContextEntryInput>,
    pub facts: LlmGenerationFacts,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum PromiseSourceCheckResult {
    Pending,
    Resolved { payload_ref: Option<BlobRef> },
    Failed { error_ref: Option<BlobRef> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationBatchRequest {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub turn_id: TurnId,
    pub batch_id: ToolBatchId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_links: Vec<WorkspaceLink>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_environment_id: Option<EnvironmentId>,
    pub environment_policy: Option<EnvironmentPolicyRuntime>,
    /// Admitted Fleet policy for this tool batch. Runtime executors consume
    /// this projection directly instead of reconstructing the owning session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet_policy: Option<crate::FleetFeature>,
    pub calls: Vec<ToolInvocationRequest>,
}

/// Admitted session policy needed while resolving live environment resources.
///
/// This is transient runtime input recorded on the activity request, not a
/// durable session document. Environment records and provider observations
/// remain live resolver reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentPolicyRuntime {
    pub version: u32,
    pub allowed_provider_ids: Option<Vec<String>>,
}

impl EnvironmentPolicyRuntime {
    pub const VERSION: u32 = 1;

    pub fn v1(allowed_provider_ids: Option<Vec<String>>) -> Self {
        Self {
            version: Self::VERSION,
            allowed_provider_ids,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationRequest {
    pub call_id: ToolCallId,
    pub tool_name: ToolName,
    pub arguments_ref: BlobRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_tool: Option<WorkflowToolCallRuntime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promise_control: Option<PromiseControlCallRuntime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromiseControlKind {
    Cancel,
    Detach,
}

impl PromiseControlKind {
    fn for_tool_name(tool_name: &ToolName) -> Option<Self> {
        match tool_name.as_str() {
            "cancel" => Some(Self::Cancel),
            "detach" => Some(Self::Detach),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromiseControlArgumentCall {
    pub call_id: ToolCallId,
    pub kind: PromiseControlKind,
    pub arguments_ref: BlobRef,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromiseControlArgumentRequest {
    pub version: u32,
    pub calls: Vec<PromiseControlArgumentCall>,
}

impl PromiseControlArgumentRequest {
    pub const VERSION: u32 = 1;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum PromiseControlArgumentCallFacts {
    Parsed {
        call_id: ToolCallId,
        promise_ids: Vec<PromiseId>,
    },
    Invalid {
        call_id: ToolCallId,
    },
}

impl PromiseControlArgumentCallFacts {
    pub fn call_id(&self) -> &ToolCallId {
        match self {
            Self::Parsed { call_id, .. } | Self::Invalid { call_id } => call_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromiseControlArgumentFacts {
    pub version: u32,
    pub calls: Vec<PromiseControlArgumentCallFacts>,
}

impl PromiseControlArgumentFacts {
    pub const VERSION: u32 = 1;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromiseControlCallRuntime {
    pub version: u32,
    pub controls: Vec<PromiseControlRuntime>,
}

impl PromiseControlCallRuntime {
    pub const VERSION: u32 = 1;

    pub fn v1(controls: Vec<PromiseControlRuntime>) -> Self {
        Self {
            version: Self::VERSION,
            controls,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromiseControlRuntime {
    pub promise_id: PromiseId,
    pub state: PromiseControlStateRuntime,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum PromiseControlStateRuntime {
    Unknown,
    Known {
        ownership: PromiseOwnership,
        scope: PromiseScope,
        promise_status: PromiseStatus,
    },
}

impl ToolInvocationBatchRequest {
    pub fn promise_control_argument_request(&self) -> Option<PromiseControlArgumentRequest> {
        let calls = self
            .calls
            .iter()
            .filter_map(|call| {
                PromiseControlKind::for_tool_name(&call.tool_name).map(|kind| {
                    PromiseControlArgumentCall {
                        call_id: call.call_id.clone(),
                        kind,
                        arguments_ref: call.arguments_ref.clone(),
                    }
                })
            })
            .collect::<Vec<_>>();
        (!calls.is_empty()).then_some(PromiseControlArgumentRequest {
            version: PromiseControlArgumentRequest::VERSION,
            calls,
        })
    }
}

/// Bounded session-owned facts needed to execute one admitted workflow-tool call.
///
/// This is transient runtime input, not durable session vocabulary. Carrying it
/// on the activity request makes retries use the exact binding and emission
/// count observed when the deterministic owner scheduled the batch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowToolCallRuntime {
    pub version: u32,
    pub binding: WorkflowToolBinding,
    pub prior_emission_count: u32,
}

impl WorkflowToolCallRuntime {
    pub const VERSION: u32 = 1;

    pub fn v1(binding: WorkflowToolBinding, prior_emission_count: u32) -> Self {
        Self {
            version: Self::VERSION,
            binding,
            prior_emission_count,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationBatchResult {
    pub run_id: RunId,
    pub turn_id: TurnId,
    pub batch_id: ToolBatchId,
    pub results: Vec<ToolInvocationResult>,
}

impl ToolInvocationBatchResult {
    pub fn single_result(self) -> Result<ToolInvocationResult, CoreAgentIoError> {
        let mut results = self.results;
        if results.len() != 1 {
            return Err(CoreAgentIoError::Failed {
                message: format!(
                    "expected exactly one tool invocation result, got {}",
                    results.len()
                ),
            });
        }
        Ok(results.remove(0))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ToolBatchOutcome {
    Completed {
        result: ToolInvocationBatchResult,
    },
    Deferred {
        batch_id: ToolBatchId,
        call_id: ToolCallId,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        completed_results: Vec<ToolInvocationResult>,
        spec: AwaitSpec,
    },
}

impl ToolBatchOutcome {
    pub fn completed(result: ToolInvocationBatchResult) -> Self {
        Self::Completed { result }
    }

    pub fn completed_result(self) -> Result<ToolInvocationBatchResult, CoreAgentIoError> {
        match self {
            Self::Completed { result } => Ok(result),
            Self::Deferred { batch_id, .. } => Err(CoreAgentIoError::Failed {
                message: format!("tool batch {batch_id} deferred instead of completing"),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ToolEffect {
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub data: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationResult {
    pub call_id: ToolCallId,
    pub status: ToolCallStatus,
    pub output_ref: Option<BlobRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_visible_context_entries: Vec<ContextEntryInput>,
    pub error_ref: Option<BlobRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ToolEffect>,
}

impl ToolInvocationResult {
    pub fn tool_result_context_entry(
        call_id: &ToolCallId,
        status: ToolCallStatus,
        content_ref: BlobRef,
    ) -> ContextEntryInput {
        ContextEntryInput {
            kind: ContextEntryKind::ToolResult {
                call_id: call_id.clone(),
                is_error: status.is_error(),
            },
            content_ref,
            media_type: None,
            preview: None,
            provider_kind: None,
            provider_item_id: None,
            token_estimate: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CoreAgentIoError {
    #[error("core agent I/O failed: {message}")]
    Failed { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_batch_single_result_requires_exactly_one_result() {
        let empty = ToolInvocationBatchResult {
            run_id: RunId::new(1),
            turn_id: TurnId::new(1),
            batch_id: ToolBatchId::new(1),
            results: Vec::new(),
        };
        assert!(empty.single_result().is_err());
    }
}
