//! Implementer-facing tool handler contract.

use crate::effects::{ToolInvocationReceipt, ToolInvocationRequest};
use crate::refs::ArtifactRef;
use crate::tooling::ToolRuntimeContext;
use crate::tools::artifacts::ToolArtifactStore;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use thiserror::Error;

#[derive(Clone)]
pub struct ToolInvocationContext {
    pub runtime: ToolRuntimeContext,
    pub artifacts: Arc<dyn ToolArtifactStore>,
}

impl ToolInvocationContext {
    pub fn new(runtime: ToolRuntimeContext, artifacts: Arc<dyn ToolArtifactStore>) -> Self {
        Self { runtime, artifacts }
    }
}

#[async_trait]
pub trait ToolHandler: Send + Sync {
    async fn invoke(
        &self,
        request: ToolInvocationRequest,
        context: ToolInvocationContext,
    ) -> Result<ToolInvocationReceipt, ToolExecutionError>;
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("tool execution error {code}: {detail}")]
pub struct ToolExecutionError {
    pub code: String,
    pub detail: String,
    pub retryable: bool,
    pub model_visible: bool,
    pub output_ref: Option<ArtifactRef>,
    pub model_visible_output_ref: Option<ArtifactRef>,
    pub metadata: BTreeMap<String, String>,
}

impl ToolExecutionError {
    pub fn tool_failure(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
            retryable: false,
            model_visible: true,
            output_ref: None,
            model_visible_output_ref: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn system_failure(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
            retryable: false,
            model_visible: false,
            output_ref: None,
            model_visible_output_ref: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    Complete,
    StillRunning,
    Cancelled,
    Abandoned,
}

impl ToolResultStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::StillRunning => "still_running",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRuntimeHandle {
    pub handle_id: String,
    pub kind: String,
    pub continuation_tool_ids: Vec<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRuntimeSnapshot {
    pub output_snapshot_ref: Option<ArtifactRef>,
    pub observed_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultMetadata {
    pub status: Option<ToolResultStatus>,
    pub handle: Option<ToolRuntimeHandle>,
    pub snapshot: Option<ToolRuntimeSnapshot>,
    pub labels: BTreeMap<String, String>,
}

impl ToolResultMetadata {
    pub fn still_running(handle: ToolRuntimeHandle, snapshot: ToolRuntimeSnapshot) -> Self {
        Self {
            status: Some(ToolResultStatus::StillRunning),
            handle: Some(handle),
            snapshot: Some(snapshot),
            labels: BTreeMap::new(),
        }
    }

    pub fn apply_to_receipt(&self, receipt: &mut ToolInvocationReceipt) {
        if let Some(status) = self.status.as_ref() {
            receipt
                .metadata
                .insert("tool_status".into(), status.as_str().into());
        }
        if let Some(handle) = self.handle.as_ref() {
            receipt
                .metadata
                .insert("runtime_handle_id".into(), handle.handle_id.clone());
            receipt
                .metadata
                .insert("runtime_handle_kind".into(), handle.kind.clone());
            if !handle.continuation_tool_ids.is_empty() {
                receipt.metadata.insert(
                    "continuation_tool_ids".into(),
                    handle.continuation_tool_ids.join(","),
                );
            }
            for (key, value) in &handle.metadata {
                receipt
                    .metadata
                    .insert(format!("runtime_handle.{key}"), value.clone());
            }
        }
        if let Some(snapshot) = self.snapshot.as_ref() {
            if let Some(snapshot_ref) = snapshot.output_snapshot_ref.as_ref() {
                receipt
                    .metadata
                    .insert("output_snapshot_ref".into(), snapshot_ref.uri.clone());
            }
            if let Some(observed_at_ms) = snapshot.observed_at_ms {
                receipt
                    .metadata
                    .insert("snapshot_observed_at_ms".into(), observed_at_ms.to_string());
            }
        }
        receipt.metadata.extend(self.labels.clone());
    }
}

pub type ToolInvocationStatus = ToolResultStatus;
