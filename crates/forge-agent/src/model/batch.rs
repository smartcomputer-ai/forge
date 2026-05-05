//! Tool-batch model records.
//!
//! This module will contain active batch state, per-call status, execution
//! groups, and result references.

use crate::ids::{EffectId, ToolBatchId, ToolCallId};
use crate::refs::BlobRef;
use crate::tooling::{ToolBatchPlan, ToolCallObserved, ToolExecutionGroup, ToolExecutionPlan};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ToolCallStatus {
    Queued,
    Pending,
    Succeeded,
    Failed { code: String, detail: String },
    Ignored,
    Cancelled,
}

impl Default for ToolCallStatus {
    fn default() -> Self {
        Self::Queued
    }
}

impl ToolCallStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed { .. } | Self::Ignored | Self::Cancelled
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallModelResult {
    pub call_id: ToolCallId,
    pub tool_id: Option<String>,
    pub tool_name: String,
    pub is_error: bool,
    pub output_ref: BlobRef,
    pub model_visible_output_ref: Option<BlobRef>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingToolEffect {
    pub call_id: ToolCallId,
    pub effect_id: EffectId,
    pub emitted_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveToolBatch {
    pub tool_batch_id: ToolBatchId,
    pub source_effect_id: EffectId,
    pub source_output_ref: Option<BlobRef>,
    pub params_hash: Option<String>,
    pub plan: ToolBatchPlan,
    pub call_status: BTreeMap<ToolCallId, ToolCallStatus>,
    pub execution_plan: ToolExecutionPlan,
    pub pending_effects: BTreeMap<ToolCallId, PendingToolEffect>,
    pub model_results: BTreeMap<ToolCallId, ToolCallModelResult>,
    pub results_ref: Option<BlobRef>,
}

impl ActiveToolBatch {
    pub fn new(
        tool_batch_id: ToolBatchId,
        source_effect_id: EffectId,
        source_output_ref: Option<BlobRef>,
        plan: ToolBatchPlan,
    ) -> Self {
        let mut call_status = BTreeMap::new();
        for call in &plan.planned_calls {
            let status = if call.accepted {
                ToolCallStatus::Queued
            } else {
                ToolCallStatus::Ignored
            };
            call_status.insert(call.call_id.clone(), status);
        }

        Self {
            tool_batch_id,
            source_effect_id,
            source_output_ref,
            execution_plan: plan.execution_plan.clone(),
            plan,
            call_status,
            params_hash: None,
            pending_effects: BTreeMap::new(),
            model_results: BTreeMap::new(),
            results_ref: None,
        }
    }

    pub fn contains_call(&self, call_id: &ToolCallId) -> bool {
        self.call_status.contains_key(call_id)
    }

    pub fn set_call_status(&mut self, call_id: ToolCallId, status: ToolCallStatus) {
        self.call_status.insert(call_id, status);
    }

    pub fn is_settled(&self) -> bool {
        self.call_status.values().all(ToolCallStatus::is_terminal)
    }

    pub fn observed_calls(&self) -> &[ToolCallObserved] {
        &self.plan.observed_calls
    }

    pub fn execution_groups(&self) -> &[ToolExecutionGroup] {
        &self.execution_plan.groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{IdAllocator, SessionId};
    use crate::tooling::{PlannedToolCall, ToolBatchPlan, ToolCallObserved};

    #[test]
    fn active_batch_initializes_accepted_and_ignored_call_status() {
        let mut ids = IdAllocator::new(SessionId::new("session-a"));
        let run_id = ids.allocate_run_id();
        let batch_id = ids.allocate_tool_batch_id(&run_id);
        let effect_id = ids.allocate_effect_id();
        let accepted_call = ToolCallId::new("call-1");
        let ignored_call = ToolCallId::new("call-2");
        let observed = vec![
            ToolCallObserved {
                call_id: accepted_call.clone(),
                tool_name: "read".into(),
                ..Default::default()
            },
            ToolCallObserved {
                call_id: ignored_call.clone(),
                tool_name: "missing".into(),
                ..Default::default()
            },
        ];
        let planned = vec![
            PlannedToolCall {
                call_id: accepted_call.clone(),
                accepted: true,
                ..Default::default()
            },
            PlannedToolCall {
                call_id: ignored_call.clone(),
                accepted: false,
                ..Default::default()
            },
        ];
        let plan = ToolBatchPlan::from_planned_calls(observed, planned);

        let batch = ActiveToolBatch::new(batch_id, effect_id, None, plan);

        assert_eq!(
            batch.call_status.get(&accepted_call),
            Some(&ToolCallStatus::Queued)
        );
        assert_eq!(
            batch.call_status.get(&ignored_call),
            Some(&ToolCallStatus::Ignored)
        );
        assert!(!batch.is_settled());
    }

    #[test]
    fn tool_call_status_terminal_helper_matches_batch_settlement() {
        let mut ids = IdAllocator::new(SessionId::new("session-a"));
        let run_id = ids.allocate_run_id();
        let call_id = ToolCallId::new("call-1");
        let plan = ToolBatchPlan::from_planned_calls(
            vec![ToolCallObserved {
                call_id: call_id.clone(),
                tool_name: "read".into(),
                ..Default::default()
            }],
            vec![PlannedToolCall {
                call_id: call_id.clone(),
                accepted: true,
                ..Default::default()
            }],
        );
        let mut batch = ActiveToolBatch::new(
            ids.allocate_tool_batch_id(&run_id),
            ids.allocate_effect_id(),
            None,
            plan,
        );

        assert!(!batch.is_settled());
        batch.set_call_status(call_id, ToolCallStatus::Succeeded);
        assert!(batch.is_settled());
    }
}
