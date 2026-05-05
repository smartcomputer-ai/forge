//! Runtime-neutral tool dispatch records and driver trait.

use crate::batch::ActiveToolBatch;
use crate::effects::{
    AgentEffectIntent, AgentEffectKind, ToolInvocationReceipt, ToolInvocationRequest,
};
use crate::error::ModelError;
use crate::ids::{EffectId, RunId, SessionId, ToolCallId, TurnId};
use crate::refs::BlobRef;
use crate::tooling::PlannedToolCall;
use crate::tooling::ToolRuntimeContext;
use crate::tools::dispatcher::{ToolDispatcher, ToolDispatcherError};
use crate::tools::handler::ToolResultStatus;
use async_trait::async_trait;
use futures::future::join_all;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedToolDispatch {
    pub order: usize,
    pub effect_id: EffectId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    pub request: ToolInvocationRequest,
}

impl PreparedToolDispatch {
    pub fn from_intent(order: usize, intent: &AgentEffectIntent) -> Result<Self, ModelError> {
        let AgentEffectKind::ToolInvoke(request) = &intent.kind else {
            return Err(ModelError::InvalidValue {
                field: "intent.kind",
                message: "prepared tool dispatch requires a ToolInvoke intent".into(),
            });
        };
        Ok(Self {
            order,
            effect_id: intent.effect_id.clone(),
            session_id: intent.session_id.clone(),
            run_id: intent.run_id.clone(),
            turn_id: intent.turn_id.clone(),
            request: request.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchCall {
    pub order: usize,
    pub planned: PlannedToolCall,
    pub request: ToolInvocationRequest,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DispatchGroup {
    pub group_index: usize,
    pub calls: Vec<DispatchCall>,
}

impl DispatchGroup {
    pub fn cancelled_outcome(&self, cancellation: &DispatchCancellation) -> DispatchOutcome {
        DispatchOutcome {
            completions: self
                .calls
                .iter()
                .map(|call| DispatchCompletion {
                    order: call.order,
                    effect_id: None,
                    receipt: cancelled_receipt_for_call(call, cancellation),
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DispatchRunRequest {
    pub groups: Vec<DispatchGroup>,
}

impl DispatchRunRequest {
    pub fn from_active_batch(batch: &ActiveToolBatch) -> Self {
        let planned_by_call_id = batch
            .plan
            .planned_calls
            .iter()
            .enumerate()
            .map(|(order, call)| (call.call_id.clone(), (order, call.clone())))
            .collect::<BTreeMap<ToolCallId, (usize, PlannedToolCall)>>();

        let groups = batch
            .execution_groups()
            .iter()
            .enumerate()
            .map(|(group_index, group)| {
                let calls = group
                    .call_ids
                    .iter()
                    .filter_map(|call_id| planned_by_call_id.get(call_id))
                    .map(|(order, planned)| DispatchCall {
                        order: *order,
                        request: ToolInvocationRequest {
                            call_id: planned.call_id.clone(),
                            provider_call_id: planned.provider_call_id.clone(),
                            tool_id: planned.tool_id.clone(),
                            tool_name: planned.tool_name.clone(),
                            arguments_json: planned.arguments_json.clone(),
                            arguments_ref: planned.arguments_ref.clone(),
                            handler_id: match &planned.executor {
                                crate::tooling::ToolExecutorKind::Handler { handler_id } => {
                                    Some(handler_id.clone())
                                }
                                _ => None,
                            },
                            context_ref: None,
                            metadata: BTreeMap::new(),
                        },
                        planned: planned.clone(),
                    })
                    .collect();
                DispatchGroup { group_index, calls }
            })
            .collect();

        Self { groups }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchCompletion {
    pub order: usize,
    pub effect_id: Option<EffectId>,
    pub receipt: ToolInvocationReceipt,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DispatchOutcome {
    pub completions: Vec<DispatchCompletion>,
}

impl DispatchOutcome {
    pub fn stable_model_order(mut self) -> Self {
        self.completions.sort_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then_with(|| left.receipt.call_id.cmp(&right.receipt.call_id))
        });
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchCancellation {
    pub mode: DispatchCancellationMode,
    pub reason: Option<String>,
    pub reason_ref: Option<BlobRef>,
}

impl DispatchCancellation {
    pub fn cancelled(reason: impl Into<String>) -> Self {
        Self {
            mode: DispatchCancellationMode::Cancelled,
            reason: Some(reason.into()),
            reason_ref: None,
        }
    }

    pub fn abandoned(reason: impl Into<String>) -> Self {
        Self {
            mode: DispatchCancellationMode::Abandoned,
            reason: Some(reason.into()),
            reason_ref: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchCancellationMode {
    Cancelled,
    Abandoned,
}

#[derive(Debug, Error)]
pub enum ToolDispatchDriverError {
    #[error("tool dispatch driver error: {message}")]
    Driver { message: String },

    #[error("tool dispatcher error: {0}")]
    Dispatcher(#[from] ToolDispatcherError),
}

#[async_trait]
pub trait ToolDispatchDriver: Send + Sync {
    async fn execute_group(
        &self,
        group: DispatchGroup,
    ) -> Result<DispatchOutcome, ToolDispatchDriverError>;

    async fn cancel_group(
        &self,
        group: DispatchGroup,
        cancellation: DispatchCancellation,
    ) -> Result<DispatchOutcome, ToolDispatchDriverError> {
        Ok(group.cancelled_outcome(&cancellation))
    }
}

#[derive(Clone)]
pub struct InProcessToolDispatchDriver {
    dispatcher: ToolDispatcher,
    runtime: ToolRuntimeContext,
}

fn cancelled_receipt_for_call(
    call: &DispatchCall,
    cancellation: &DispatchCancellation,
) -> ToolInvocationReceipt {
    let status = match cancellation.mode {
        DispatchCancellationMode::Cancelled => ToolResultStatus::Cancelled,
        DispatchCancellationMode::Abandoned => ToolResultStatus::Abandoned,
    };
    let reason = cancellation
        .reason
        .clone()
        .unwrap_or_else(|| status.as_str().to_string());
    let mut metadata = call.request.metadata.clone();
    metadata.insert("tool_status".into(), status.as_str().into());
    metadata.insert("cancellation_mode".into(), status.as_str().into());
    metadata.insert("cancellation_reason".into(), reason.clone());
    if let Some(reason_ref) = cancellation.reason_ref.as_ref() {
        metadata.insert(
            "cancellation_reason_ref".into(),
            reason_ref.as_str().to_string(),
        );
    }
    ToolInvocationReceipt {
        call_id: call.request.call_id.clone(),
        tool_id: call.request.tool_id.clone(),
        tool_name: call.request.tool_name.clone(),
        output_ref: Some(BlobRef::from_bytes(reason.as_bytes())),
        model_visible_output_ref: None,
        is_error: true,
        metadata,
    }
}

impl InProcessToolDispatchDriver {
    pub fn new(dispatcher: ToolDispatcher, runtime: ToolRuntimeContext) -> Self {
        Self {
            dispatcher,
            runtime,
        }
    }
}

#[async_trait]
impl ToolDispatchDriver for InProcessToolDispatchDriver {
    async fn execute_group(
        &self,
        group: DispatchGroup,
    ) -> Result<DispatchOutcome, ToolDispatchDriverError> {
        let dispatcher = self.dispatcher.clone();
        let runtime = self.runtime.clone();
        let futures = group.calls.into_iter().map(move |call| {
            let dispatcher = dispatcher.clone();
            let runtime = runtime.clone();
            async move {
                let receipt = dispatcher.dispatch(call.request, runtime).await?;
                Ok::<_, ToolDispatcherError>(DispatchCompletion {
                    order: call.order,
                    effect_id: None,
                    receipt,
                })
            }
        });
        let completions = join_all(futures)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        Ok(DispatchOutcome { completions })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::ActiveToolBatch;
    use crate::effects::{AgentEffectIntent, AgentEffectKind};
    use crate::ids::{IdAllocator, SessionId, ToolCallId};
    use crate::testing::tools::{ActivityStyleDriver, CompletionOrderDriver, EchoToolHandler};
    use crate::tooling::{
        PlannedToolCall, ToolBatchPlan, ToolCallObserved, ToolExecutorKind, ToolRegistry, ToolSpec,
    };
    use std::sync::Arc;

    fn planned_call(id: &str, parallel_safe: bool, resource_key: Option<&str>) -> PlannedToolCall {
        PlannedToolCall {
            call_id: ToolCallId::new(id),
            provider_call_id: Some(format!("provider-{id}")),
            tool_id: Some(format!("tool-{id}")),
            tool_name: format!("tool_{id}"),
            arguments_json: Some("{}".into()),
            parallel_safe,
            resource_key: resource_key.map(str::to_string),
            accepted: true,
            ..Default::default()
        }
    }

    #[test]
    fn dispatch_run_request_preserves_execution_groups_and_order() {
        let mut ids = IdAllocator::new(SessionId::new("session-a"));
        let run_id = ids.allocate_run_id();
        let effect_id = ids.allocate_effect_id();
        let planned = vec![
            planned_call("a", true, Some("fs:/a")),
            planned_call("b", true, Some("fs:/b")),
            planned_call("c", false, None),
        ];
        let observed = planned
            .iter()
            .map(|call| ToolCallObserved {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                ..Default::default()
            })
            .collect();
        let batch = ActiveToolBatch::new(
            ids.allocate_tool_batch_id(&run_id),
            effect_id,
            None,
            ToolBatchPlan::from_planned_calls(observed, planned),
        );

        let request = DispatchRunRequest::from_active_batch(&batch);

        assert_eq!(request.groups.len(), 2);
        assert_eq!(request.groups[0].calls.len(), 2);
        assert_eq!(request.groups[0].calls[0].order, 0);
        assert_eq!(request.groups[0].calls[1].order, 1);
        assert_eq!(request.groups[1].calls[0].order, 2);
    }

    #[test]
    fn prepared_dispatch_rejects_non_tool_intent() {
        let mut ids = IdAllocator::new(SessionId::new("session-a"));
        let intent = AgentEffectIntent::new(
            ids.allocate_effect_id(),
            SessionId::new("session-a"),
            AgentEffectKind::McpCall(crate::effects::McpCallRequest {
                server_id: "server".into(),
                tool_name: "tool".into(),
                arguments: serde_json::json!({}),
                arguments_ref: None,
            }),
            1,
        );

        let error = PreparedToolDispatch::from_intent(0, &intent).expect_err("not tool");

        assert!(matches!(
            error,
            ModelError::InvalidValue {
                field: "intent.kind",
                ..
            }
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn completion_order_can_be_sorted_back_to_model_order() {
        let group = DispatchGroup {
            group_index: 0,
            calls: vec![
                DispatchCall {
                    order: 0,
                    planned: planned_call("a", true, None),
                    request: ToolInvocationRequest {
                        call_id: ToolCallId::new("a"),
                        tool_id: Some("tool-a".into()),
                        tool_name: "tool_a".into(),
                        provider_call_id: None,
                        arguments_json: Some("{}".into()),
                        arguments_ref: None,
                        handler_id: None,
                        context_ref: None,
                        metadata: BTreeMap::new(),
                    },
                },
                DispatchCall {
                    order: 1,
                    planned: planned_call("b", true, None),
                    request: ToolInvocationRequest {
                        call_id: ToolCallId::new("b"),
                        tool_id: Some("tool-b".into()),
                        tool_name: "tool_b".into(),
                        provider_call_id: None,
                        arguments_json: Some("{}".into()),
                        arguments_ref: None,
                        handler_id: None,
                        context_ref: None,
                        metadata: BTreeMap::new(),
                    },
                },
            ],
        };
        let driver = CompletionOrderDriver {
            completion_order: vec![1, 0],
        };

        let completion_order = driver.execute_group(group).await.expect("execute group");
        assert_eq!(
            completion_order.completions[0].receipt.call_id,
            ToolCallId::new("b")
        );

        let model_order = completion_order.stable_model_order();
        assert_eq!(
            model_order.completions[0].receipt.call_id,
            ToolCallId::new("a")
        );
        assert_eq!(
            model_order.completions[1].receipt.call_id,
            ToolCallId::new("b")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn in_process_driver_executes_group_with_dispatcher() {
        let mut registry = ToolRegistry::default();
        registry.insert_tool(ToolSpec {
            tool_id: "echo".into(),
            tool_name: "echo".into(),
            description: "Echo".into(),
            args_schema: serde_json::json!({"type":"object"}),
            executor: ToolExecutorKind::Handler {
                handler_id: "echo-handler".into(),
            },
            ..Default::default()
        });
        let dispatcher = crate::tools::ToolDispatcher::builder(registry)
            .register_handler("echo-handler", Arc::new(EchoToolHandler::default()))
            .expect("register handler")
            .build();
        let driver = InProcessToolDispatchDriver::new(dispatcher, ToolRuntimeContext::default());
        let group = DispatchGroup {
            group_index: 0,
            calls: vec![DispatchCall {
                order: 0,
                planned: planned_call("a", true, None),
                request: ToolInvocationRequest {
                    call_id: ToolCallId::new("a"),
                    tool_id: Some("echo".into()),
                    tool_name: "echo".into(),
                    provider_call_id: None,
                    arguments_json: Some(r#"{"text":"hi"}"#.into()),
                    arguments_ref: None,
                    handler_id: None,
                    context_ref: None,
                    metadata: BTreeMap::new(),
                },
            }],
        };

        let outcome = driver.execute_group(group).await.expect("execute group");

        assert_eq!(outcome.completions.len(), 1);
        assert!(!outcome.completions[0].receipt.is_error);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancel_group_returns_terminal_cancelled_receipts() {
        let group = DispatchGroup {
            group_index: 0,
            calls: vec![
                DispatchCall {
                    order: 0,
                    planned: planned_call("a", true, None),
                    request: ToolInvocationRequest {
                        call_id: ToolCallId::new("a"),
                        tool_id: Some("tool-a".into()),
                        tool_name: "tool_a".into(),
                        provider_call_id: None,
                        arguments_json: Some("{}".into()),
                        arguments_ref: None,
                        handler_id: None,
                        context_ref: None,
                        metadata: BTreeMap::new(),
                    },
                },
                DispatchCall {
                    order: 1,
                    planned: planned_call("b", true, None),
                    request: ToolInvocationRequest {
                        call_id: ToolCallId::new("b"),
                        tool_id: Some("tool-b".into()),
                        tool_name: "tool_b".into(),
                        provider_call_id: None,
                        arguments_json: Some("{}".into()),
                        arguments_ref: None,
                        handler_id: None,
                        context_ref: None,
                        metadata: BTreeMap::new(),
                    },
                },
            ],
        };
        let driver = CompletionOrderDriver::default();

        let outcome = driver
            .cancel_group(
                group,
                DispatchCancellation::cancelled("user interrupted run"),
            )
            .await
            .expect("cancel group");

        assert_eq!(outcome.completions.len(), 2);
        assert!(outcome.completions.iter().all(|completion| {
            completion.receipt.is_error
                && completion
                    .receipt
                    .metadata
                    .get("tool_status")
                    .is_some_and(|status| status == "cancelled")
        }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn activity_style_driver_schedules_and_cancels_without_spawn_api() {
        let group = DispatchGroup {
            group_index: 0,
            calls: vec![DispatchCall {
                order: 0,
                planned: planned_call("activity", true, None),
                request: ToolInvocationRequest {
                    call_id: ToolCallId::new("activity"),
                    tool_id: Some("tool-activity".into()),
                    tool_name: "tool_activity".into(),
                    provider_call_id: None,
                    arguments_json: Some("{}".into()),
                    arguments_ref: None,
                    handler_id: None,
                    context_ref: None,
                    metadata: BTreeMap::new(),
                },
            }],
        };
        let driver = ActivityStyleDriver::default();

        let outcome = driver
            .execute_group(group.clone())
            .await
            .expect("execute activity group");
        assert_eq!(outcome.completions.len(), 1);
        assert_eq!(driver.scheduled_call_ids(), vec!["activity"]);

        let cancelled = driver
            .cancel_group(group, DispatchCancellation::abandoned("workflow cancelled"))
            .await
            .expect("cancel activity group");

        assert_eq!(driver.cancelled_call_ids(), vec!["activity"]);
        assert_eq!(
            cancelled.completions[0]
                .receipt
                .metadata
                .get("tool_status")
                .map(String::as_str),
            Some("abandoned")
        );
    }
}
