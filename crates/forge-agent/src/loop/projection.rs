//! Derived transcript/projection records from authoritative journal events.

use crate::effects::{AgentEffectKind, AgentReceiptKind};
use crate::events::{AgentEvent, AgentEventKind, EffectEvent, InputEvent, LifecycleEvent};
use crate::ids::{ProjectionItemId, SessionId};
use crate::lifecycle::RunLifecycle;
use crate::projection::{
    ProjectionItem, ProjectionItemKind, ProjectionItemLifecycle, ProjectionJoinIds,
};
use crate::refs::BlobRef;
use crate::transcript::{TranscriptEntryKind, TranscriptItem, TranscriptItemJoins};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectionOutput {
    pub projection_items: Vec<ProjectionItem>,
    pub transcript_items: Vec<TranscriptItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionBuilder {
    pub session_id: SessionId,
    next_item_seq: u64,
    pub projection_items: Vec<ProjectionItem>,
    pub transcript_items: Vec<TranscriptItem>,
}

impl ProjectionBuilder {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            next_item_seq: 1,
            projection_items: Vec::new(),
            transcript_items: Vec::new(),
        }
    }

    pub fn apply_event(&mut self, event: &AgentEvent) -> ProjectionOutput {
        let mut output = ProjectionOutput::default();
        match &event.kind {
            AgentEventKind::Input(InputEvent::RunRequested { input_ref, .. })
            | AgentEventKind::Input(InputEvent::FollowUpInputAppended { input_ref, .. }) => {
                self.push_content_item(
                    event,
                    ProjectionItemKind::User,
                    TranscriptEntryKind::Message,
                    Some(input_ref.clone()),
                    None,
                    &mut output,
                );
            }
            AgentEventKind::Input(InputEvent::RunSteerRequested { instruction_ref }) => {
                self.push_content_item(
                    event,
                    ProjectionItemKind::Status {
                        status: "steering".into(),
                    },
                    TranscriptEntryKind::Message,
                    Some(instruction_ref.clone()),
                    None,
                    &mut output,
                );
            }
            AgentEventKind::Lifecycle(LifecycleEvent::RunLifecycleChanged { to, .. }) => {
                self.push_status(event, run_status(*to), &mut output);
            }
            AgentEventKind::Lifecycle(LifecycleEvent::TurnStarted { .. }) => {
                self.push_status(event, "turn_started", &mut output);
            }
            AgentEventKind::Lifecycle(LifecycleEvent::TurnCompleted { .. }) => {
                self.push_status(event, "turn_completed", &mut output);
            }
            AgentEventKind::Effect(EffectEvent::EffectIntentRecorded { intent }) => {
                if let AgentEffectKind::ToolInvoke(request) = &intent.kind {
                    let mut item = self.new_projection_item(
                        event,
                        ProjectionItemKind::ToolCall {
                            tool_name: request.tool_name.clone(),
                            status: crate::batch::ToolCallStatus::Pending,
                        },
                    );
                    item.title = Some(request.tool_name.clone());
                    item.preview = request.arguments_json.clone();
                    self.finish_projection_item(item, &mut output);
                }
            }
            AgentEventKind::Effect(EffectEvent::EffectReceiptRecorded { receipt }) => {
                match &receipt.kind {
                    AgentReceiptKind::LlmComplete(receipt)
                    | AgentReceiptKind::LlmStream(receipt) => {
                        if let Some(message_ref) = receipt.assistant_message_ref.clone() {
                            self.push_content_item(
                                event,
                                ProjectionItemKind::Assistant,
                                TranscriptEntryKind::Message,
                                Some(message_ref.clone()),
                                None,
                                &mut output,
                            );
                        }
                        if let Some(reasoning_ref) = receipt.reasoning_summary_ref.clone() {
                            self.push_content_item(
                                event,
                                ProjectionItemKind::Reasoning,
                                TranscriptEntryKind::Reasoning,
                                Some(reasoning_ref.clone()),
                                None,
                                &mut output,
                            );
                        }
                        for call in &receipt.tool_calls {
                            let mut item = self.new_projection_item(
                                event,
                                ProjectionItemKind::ToolCall {
                                    tool_name: call.tool_name.clone(),
                                    status: crate::batch::ToolCallStatus::Queued,
                                },
                            );
                            item.joins.tool_call_id = Some(call.call_id.clone());
                            item.title = Some(call.tool_name.clone());
                            item.preview = call.arguments_json.clone();
                            self.finish_projection_item(item, &mut output);
                        }
                        if let Some(usage) = receipt.usage {
                            self.push_projection_only(
                                event,
                                ProjectionItemKind::TokenUsage { usage },
                                None,
                                &mut output,
                            );
                        }
                    }
                    AgentReceiptKind::ToolInvoke(receipt) => {
                        let status = if receipt.is_error {
                            crate::batch::ToolCallStatus::Failed {
                                code: "tool_error".into(),
                                detail: receipt.tool_name.clone(),
                            }
                        } else {
                            crate::batch::ToolCallStatus::Succeeded
                        };
                        self.push_content_item(
                            event,
                            ProjectionItemKind::ToolOutput { status },
                            TranscriptEntryKind::ToolResult,
                            receipt
                                .model_visible_output_ref
                                .clone()
                                .or_else(|| receipt.output_ref.clone()),
                            receipt
                                .model_visible_output_ref
                                .as_ref()
                                .or(receipt.output_ref.as_ref())
                                .map(|ref_| ref_.as_str().to_string()),
                            &mut output,
                        );
                    }
                    AgentReceiptKind::LlmCompact(receipt) => {
                        self.push_content_item(
                            event,
                            ProjectionItemKind::Compaction,
                            TranscriptEntryKind::Summary,
                            receipt.blob_refs.first().cloned(),
                            receipt
                                .blob_refs
                                .first()
                                .map(|ref_| ref_.as_str().to_string()),
                            &mut output,
                        );
                    }
                    AgentReceiptKind::Failed(failure) => {
                        self.push_projection_only(
                            event,
                            ProjectionItemKind::Warning {
                                code: failure.code.clone(),
                            },
                            Some(failure.detail.clone()),
                            &mut output,
                        );
                    }
                    _ => {}
                }
            }
            AgentEventKind::Observation(_)
            | AgentEventKind::Input(_)
            | AgentEventKind::Lifecycle(_)
            | AgentEventKind::Effect(EffectEvent::EffectStreamFrameObserved { .. }) => {}
        }

        self.projection_items
            .extend(output.projection_items.iter().cloned());
        self.transcript_items
            .extend(output.transcript_items.iter().cloned());
        output
    }

    fn push_status(&mut self, event: &AgentEvent, status: &str, output: &mut ProjectionOutput) {
        self.push_projection_only(
            event,
            ProjectionItemKind::Status {
                status: status.into(),
            },
            Some(status.into()),
            output,
        );
    }

    fn push_projection_only(
        &mut self,
        event: &AgentEvent,
        kind: ProjectionItemKind,
        preview: Option<String>,
        output: &mut ProjectionOutput,
    ) {
        let mut item = self.new_projection_item(event, kind);
        item.preview = preview;
        self.finish_projection_item(item, output);
    }

    fn push_content_item(
        &mut self,
        event: &AgentEvent,
        projection_kind: ProjectionItemKind,
        transcript_kind: TranscriptEntryKind,
        content_ref: Option<BlobRef>,
        preview: Option<String>,
        output: &mut ProjectionOutput,
    ) {
        let mut item = self.new_projection_item(event, projection_kind);
        item.preview = preview.clone();
        item.complete(content_ref.clone(), event.observed_at_ms);
        let item_id = item.item_id.clone();
        output.projection_items.push(item);
        output.transcript_items.push(TranscriptItem {
            item_id,
            joins: TranscriptItemJoins {
                session_id: event.session_id.clone(),
                journal_seq: event.journal_seq,
                run_id: event.joins.run_id.clone(),
                turn_id: event.joins.turn_id.clone(),
                effect_id: event.joins.effect_id.clone(),
                tool_batch_id: event.joins.tool_batch_id.clone(),
                tool_call_id: event.joins.tool_call_id.clone(),
            },
            kind: transcript_kind,
            source_event_id: Some(event.event_id.clone()),
            content_ref,
            preview,
            source_range: None,
            metadata: BTreeMap::new(),
            created_at_ms: event.observed_at_ms,
            updated_at_ms: event.observed_at_ms,
        });
    }

    fn new_projection_item(
        &mut self,
        event: &AgentEvent,
        kind: ProjectionItemKind,
    ) -> ProjectionItem {
        let item_id = ProjectionItemId {
            session_id: self.session_id.clone(),
            item_seq: self.next_item_seq,
        };
        self.next_item_seq = self.next_item_seq.saturating_add(1);
        ProjectionItem::new(
            item_id,
            ProjectionJoinIds {
                session_id: event.session_id.clone(),
                run_id: event.joins.run_id.clone(),
                turn_id: event.joins.turn_id.clone(),
                effect_id: event.joins.effect_id.clone(),
                tool_batch_id: event.joins.tool_batch_id.clone(),
                tool_call_id: event.joins.tool_call_id.clone(),
            },
            kind,
            event.observed_at_ms,
        )
    }

    fn finish_projection_item(&mut self, mut item: ProjectionItem, output: &mut ProjectionOutput) {
        item.lifecycle = ProjectionItemLifecycle::Completed;
        item.completed_at_ms = Some(item.updated_at_ms);
        output.projection_items.push(item);
    }
}

fn run_status(lifecycle: RunLifecycle) -> &'static str {
    match lifecycle {
        RunLifecycle::Queued => "run_queued",
        RunLifecycle::Running => "run_running",
        RunLifecycle::Waiting => "run_waiting",
        RunLifecycle::Completed => "run_completed",
        RunLifecycle::Failed => "run_failed",
        RunLifecycle::Cancelled => "run_cancelled",
        RunLifecycle::Interrupted => "run_interrupted",
    }
}
