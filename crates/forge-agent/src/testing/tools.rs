//! Deterministic tool handlers and drivers for tests.

use crate::effects::{ToolInvocationReceipt, ToolInvocationRequest};
use crate::refs::ArtifactRef;
use crate::tools::{
    DispatchCompletion, DispatchGroup, DispatchOutcome, ToolDispatchDriver,
    ToolDispatchDriverError, ToolExecutionError, ToolHandler, ToolInvocationContext,
    ToolResultMetadata, ToolRuntimeHandle, ToolRuntimeSnapshot,
};
use async_trait::async_trait;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct EchoToolHandler {
    pub output_uri_prefix: String,
}

impl Default for EchoToolHandler {
    fn default() -> Self {
        Self {
            output_uri_prefix: "forge://test-tool-output".into(),
        }
    }
}

#[async_trait]
impl ToolHandler for EchoToolHandler {
    async fn invoke(
        &self,
        request: ToolInvocationRequest,
        _context: ToolInvocationContext,
    ) -> Result<ToolInvocationReceipt, ToolExecutionError> {
        let preview = request.arguments_json.clone().unwrap_or_default();
        let output_ref =
            ArtifactRef::new(format!("{}/{}", self.output_uri_prefix, request.call_id))
                .with_preview(preview);
        Ok(ToolInvocationReceipt {
            call_id: request.call_id,
            tool_id: request.tool_id,
            tool_name: request.tool_name,
            output_ref: Some(output_ref.clone()),
            model_visible_output_ref: Some(output_ref),
            is_error: false,
            metadata: request.metadata,
        })
    }
}

#[derive(Clone, Debug)]
pub struct StaticToolHandler {
    pub result: Result<ToolInvocationReceipt, ToolExecutionError>,
}

#[derive(Clone, Debug)]
pub struct BackgroundStartHandler {
    pub handle_id: String,
    pub poll_tool_id: String,
    pub snapshot_ref: ArtifactRef,
}

#[async_trait]
impl ToolHandler for BackgroundStartHandler {
    async fn invoke(
        &self,
        request: ToolInvocationRequest,
        _context: ToolInvocationContext,
    ) -> Result<ToolInvocationReceipt, ToolExecutionError> {
        let mut receipt = ToolInvocationReceipt {
            call_id: request.call_id,
            tool_id: request.tool_id,
            tool_name: request.tool_name,
            output_ref: Some(self.snapshot_ref.clone()),
            model_visible_output_ref: Some(self.snapshot_ref.clone()),
            is_error: false,
            metadata: request.metadata,
        };
        ToolResultMetadata::still_running(
            ToolRuntimeHandle {
                handle_id: self.handle_id.clone(),
                kind: "background_job".into(),
                continuation_tool_ids: vec![self.poll_tool_id.clone()],
                metadata: BTreeMap::new(),
            },
            ToolRuntimeSnapshot {
                output_snapshot_ref: Some(self.snapshot_ref.clone()),
                observed_at_ms: None,
            },
        )
        .apply_to_receipt(&mut receipt);
        Ok(receipt)
    }
}

#[derive(Clone, Debug)]
pub struct BackgroundPollHandler {
    pub completed_ref: ArtifactRef,
}

#[async_trait]
impl ToolHandler for BackgroundPollHandler {
    async fn invoke(
        &self,
        request: ToolInvocationRequest,
        _context: ToolInvocationContext,
    ) -> Result<ToolInvocationReceipt, ToolExecutionError> {
        let mut metadata = request.metadata;
        metadata.insert("tool_status".into(), "complete".into());
        Ok(ToolInvocationReceipt {
            call_id: request.call_id,
            tool_id: request.tool_id,
            tool_name: request.tool_name,
            output_ref: Some(self.completed_ref.clone()),
            model_visible_output_ref: Some(self.completed_ref.clone()),
            is_error: false,
            metadata,
        })
    }
}

#[async_trait]
impl ToolHandler for StaticToolHandler {
    async fn invoke(
        &self,
        _request: ToolInvocationRequest,
        _context: ToolInvocationContext,
    ) -> Result<ToolInvocationReceipt, ToolExecutionError> {
        self.result.clone()
    }
}

#[derive(Clone, Debug, Default)]
pub struct CompletionOrderDriver {
    pub completion_order: Vec<usize>,
}

#[async_trait]
impl ToolDispatchDriver for CompletionOrderDriver {
    async fn execute_group(
        &self,
        group: DispatchGroup,
    ) -> Result<DispatchOutcome, ToolDispatchDriverError> {
        let calls_by_order = group
            .calls
            .into_iter()
            .map(|call| (call.order, call))
            .collect::<BTreeMap<_, _>>();
        let mut completions = Vec::new();
        for order in &self.completion_order {
            if let Some(call) = calls_by_order.get(order) {
                completions.push(completion_for_call(*order, call.request.clone()));
            }
        }
        for (order, call) in calls_by_order {
            if !self.completion_order.contains(&order) {
                completions.push(completion_for_call(order, call.request));
            }
        }
        Ok(DispatchOutcome { completions })
    }
}

fn completion_for_call(order: usize, request: ToolInvocationRequest) -> DispatchCompletion {
    DispatchCompletion {
        order,
        effect_id: None,
        receipt: ToolInvocationReceipt {
            call_id: request.call_id,
            tool_id: request.tool_id,
            tool_name: request.tool_name,
            output_ref: Some(ArtifactRef::new(format!(
                "forge://test-tool-output/{order}"
            ))),
            model_visible_output_ref: None,
            is_error: false,
            metadata: BTreeMap::new(),
        },
    }
}
