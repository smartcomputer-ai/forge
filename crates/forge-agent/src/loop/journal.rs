//! Append-only scoped journal helpers.
//!
//! The journal assigns or validates session-local event sequence numbers before
//! reducer code sees events. It does not sample time; callers provide stable
//! `observed_at_ms` values on `AgentEvent`.

use crate::effects::{AgentEffectIntent, AgentEffectReceipt, EffectStreamFrame};
use crate::error::ModelError;
use crate::events::{AgentEvent, AgentEventJoins, AgentEventKind, EffectEvent};
use crate::ids::{JournalSeq, RunId, SessionId, ToolBatchId, TurnId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalAppendResult {
    pub event: AgentEvent,
    pub assigned_seq: JournalSeq,
    pub next_seq: JournalSeq,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InMemoryJournal {
    pub session_id: SessionId,
    events: Vec<AgentEvent>,
    event_ids: BTreeSet<String>,
    next_seq: u64,
}

impl InMemoryJournal {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            events: Vec::new(),
            event_ids: BTreeSet::new(),
            next_seq: 1,
        }
    }

    pub fn append(&mut self, mut event: AgentEvent) -> Result<JournalAppendResult, ModelError> {
        self.validate_event(&event)?;
        if !self.event_ids.insert(event.event_id.clone()) {
            return Err(ModelError::InvalidValue {
                field: "event_id",
                message: format!("duplicate journal event id '{}'", event.event_id),
            });
        }

        let expected = JournalSeq(self.next_seq);
        match event.journal_seq {
            Some(actual) if actual != expected => {
                self.event_ids.remove(&event.event_id);
                return Err(ModelError::InvalidValue {
                    field: "journal_seq",
                    message: format!("expected sequence {}, got {}", expected, actual),
                });
            }
            Some(_) => {}
            None => {
                event.journal_seq = Some(expected);
            }
        }

        self.next_seq = self.next_seq.saturating_add(1);
        let next_seq = JournalSeq(self.next_seq);
        self.events.push(event.clone());

        Ok(JournalAppendResult {
            event,
            assigned_seq: expected,
            next_seq,
        })
    }

    pub fn events(&self) -> &[AgentEvent] {
        &self.events
    }

    pub fn latest_seq(&self) -> Option<JournalSeq> {
        self.events.last().and_then(|event| event.journal_seq)
    }

    pub fn next_seq(&self) -> JournalSeq {
        JournalSeq(self.next_seq)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    fn validate_event(&self, event: &AgentEvent) -> Result<(), ModelError> {
        if event.event_id.is_empty() {
            return Err(ModelError::InvalidValue {
                field: "event_id",
                message: "journal event id must not be empty".into(),
            });
        }
        if event.session_id != self.session_id {
            return Err(ModelError::InvalidValue {
                field: "session_id",
                message: format!(
                    "journal for session '{}' cannot append event for session '{}'",
                    self.session_id, event.session_id
                ),
            });
        }
        validate_join_sessions(&event.session_id, &event.joins)?;
        validate_effect_event(event)?;
        Ok(())
    }
}

fn validate_join_sessions(
    session_id: &SessionId,
    joins: &AgentEventJoins,
) -> Result<(), ModelError> {
    if let Some(run_id) = &joins.run_id {
        validate_run_session(session_id, run_id, "joins.run_id")?;
    }
    if let Some(turn_id) = &joins.turn_id {
        validate_turn_session(session_id, turn_id, "joins.turn_id")?;
    }
    if let Some(effect_id) = &joins.effect_id
        && &effect_id.session_id != session_id
    {
        return Err(ModelError::InvalidValue {
            field: "joins.effect_id",
            message: format!(
                "effect id session '{}' does not match event session '{}'",
                effect_id.session_id, session_id
            ),
        });
    }
    if let Some(tool_batch_id) = &joins.tool_batch_id {
        validate_tool_batch_session(session_id, tool_batch_id, "joins.tool_batch_id")?;
    }
    Ok(())
}

fn validate_run_session(
    session_id: &SessionId,
    run_id: &RunId,
    field: &'static str,
) -> Result<(), ModelError> {
    if &run_id.session_id != session_id {
        return Err(ModelError::InvalidValue {
            field,
            message: format!(
                "run id session '{}' does not match event session '{}'",
                run_id.session_id, session_id
            ),
        });
    }
    Ok(())
}

fn validate_turn_session(
    session_id: &SessionId,
    turn_id: &TurnId,
    field: &'static str,
) -> Result<(), ModelError> {
    validate_run_session(session_id, &turn_id.run_id, field)
}

fn validate_tool_batch_session(
    session_id: &SessionId,
    tool_batch_id: &ToolBatchId,
    field: &'static str,
) -> Result<(), ModelError> {
    validate_run_session(session_id, &tool_batch_id.run_id, field)
}

fn validate_effect_event(event: &AgentEvent) -> Result<(), ModelError> {
    let AgentEventKind::Effect(effect) = &event.kind else {
        return Ok(());
    };

    if event.joins.effect_id.as_ref() != Some(effect.effect_id()) {
        return Err(ModelError::InvalidValue {
            field: "joins.effect_id",
            message: "effect events must join to their effect id".into(),
        });
    }

    match effect {
        EffectEvent::EffectIntentRecorded { intent } => validate_effect_intent_joins(event, intent),
        EffectEvent::EffectReceiptRecorded { receipt } => {
            validate_effect_receipt_joins(event, receipt)
        }
        EffectEvent::EffectStreamFrameObserved { frame } => {
            validate_stream_frame_joins(event, frame)
        }
    }
}

fn validate_effect_intent_joins(
    event: &AgentEvent,
    intent: &AgentEffectIntent,
) -> Result<(), ModelError> {
    if intent.session_id != event.session_id {
        return Err(ModelError::InvalidValue {
            field: "intent.session_id",
            message: "effect intent session must match event session".into(),
        });
    }
    validate_optional_run_join(&event.joins, intent.run_id.as_ref())?;
    validate_optional_turn_join(&event.joins, intent.turn_id.as_ref())?;
    Ok(())
}

fn validate_effect_receipt_joins(
    event: &AgentEvent,
    receipt: &AgentEffectReceipt,
) -> Result<(), ModelError> {
    if receipt.session_id != event.session_id {
        return Err(ModelError::InvalidValue {
            field: "receipt.session_id",
            message: "effect receipt session must match event session".into(),
        });
    }
    validate_optional_run_join(&event.joins, receipt.run_id.as_ref())?;
    validate_optional_turn_join(&event.joins, receipt.turn_id.as_ref())?;
    Ok(())
}

fn validate_stream_frame_joins(
    event: &AgentEvent,
    frame: &EffectStreamFrame,
) -> Result<(), ModelError> {
    if frame.session_id != event.session_id {
        return Err(ModelError::InvalidValue {
            field: "frame.session_id",
            message: "effect stream frame session must match event session".into(),
        });
    }
    validate_optional_run_join(&event.joins, frame.run_id.as_ref())?;
    validate_optional_turn_join(&event.joins, frame.turn_id.as_ref())?;
    Ok(())
}

fn validate_optional_run_join(
    joins: &AgentEventJoins,
    run_id: Option<&RunId>,
) -> Result<(), ModelError> {
    if let Some(run_id) = run_id
        && joins.run_id.as_ref() != Some(run_id)
    {
        return Err(ModelError::InvalidValue {
            field: "joins.run_id",
            message: "effect event run join must match effect payload".into(),
        });
    }
    Ok(())
}

fn validate_optional_turn_join(
    joins: &AgentEventJoins,
    turn_id: Option<&TurnId>,
) -> Result<(), ModelError> {
    if let Some(turn_id) = turn_id
        && joins.turn_id.as_ref() != Some(turn_id)
    {
        return Err(ModelError::InvalidValue {
            field: "joins.turn_id",
            message: "effect event turn join must match effect payload".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::{
        AgentEffectKind, AgentEffectReceipt, AgentReceiptKind, EffectMetadata,
        ToolInvocationReceipt, ToolInvocationRequest,
    };
    use crate::events::{AgentEventJoins, InputEvent};
    use crate::ids::{EffectId, ToolCallId};
    use crate::refs::BlobRef;
    use std::collections::BTreeMap;

    fn input_event(event_id: &str, observed_at_ms: u64) -> AgentEvent {
        AgentEvent::new(
            event_id,
            SessionId::new("session-a"),
            observed_at_ms,
            AgentEventKind::Input(InputEvent::RunRequested {
                input_ref: BlobRef::new_unchecked_for_tests("blob://input"),
                run_overrides: None,
            }),
        )
    }

    #[test]
    fn append_assigns_monotonic_sequences_and_preserves_timestamp() {
        let mut journal = InMemoryJournal::new(SessionId::new("session-a"));

        let first = journal
            .append(input_event("event-1", 10))
            .expect("append first");
        let second = journal
            .append(input_event("event-2", 7))
            .expect("append second");

        assert_eq!(first.assigned_seq, JournalSeq(1));
        assert_eq!(first.event.journal_seq, Some(JournalSeq(1)));
        assert_eq!(first.event.observed_at_ms, 10);
        assert_eq!(second.assigned_seq, JournalSeq(2));
        assert_eq!(journal.latest_seq(), Some(JournalSeq(2)));
        assert_eq!(journal.next_seq(), JournalSeq(3));
        assert_eq!(journal.events()[1].observed_at_ms, 7);
    }

    #[test]
    fn append_accepts_expected_preassigned_sequence() {
        let mut journal = InMemoryJournal::new(SessionId::new("session-a"));
        let event = input_event("event-1", 10).with_journal_seq(JournalSeq(1));

        let result = journal.append(event).expect("append event");

        assert_eq!(result.assigned_seq, JournalSeq(1));
        assert_eq!(result.event.journal_seq, Some(JournalSeq(1)));
    }

    #[test]
    fn append_rejects_duplicate_or_out_of_order_sequences() {
        let mut journal = InMemoryJournal::new(SessionId::new("session-a"));
        journal
            .append(input_event("event-1", 10))
            .expect("append first");

        let duplicate = journal
            .append(input_event("event-2", 11).with_journal_seq(JournalSeq(1)))
            .expect_err("duplicate sequence should fail");
        assert!(matches!(
            duplicate,
            ModelError::InvalidValue {
                field: "journal_seq",
                ..
            }
        ));

        let gap = journal
            .append(input_event("event-3", 12).with_journal_seq(JournalSeq(3)))
            .expect_err("out-of-order sequence should fail");
        assert!(matches!(
            gap,
            ModelError::InvalidValue {
                field: "journal_seq",
                ..
            }
        ));

        assert_eq!(journal.latest_seq(), Some(JournalSeq(1)));
        assert_eq!(journal.next_seq(), JournalSeq(2));
    }

    #[test]
    fn append_rejects_duplicate_event_ids_and_wrong_session() {
        let mut journal = InMemoryJournal::new(SessionId::new("session-a"));
        journal
            .append(input_event("event-1", 10))
            .expect("append first");

        let duplicate_id = journal
            .append(input_event("event-1", 11))
            .expect_err("duplicate id should fail");
        assert!(matches!(
            duplicate_id,
            ModelError::InvalidValue {
                field: "event_id",
                ..
            }
        ));

        let wrong_session = AgentEvent::new(
            "event-2",
            SessionId::new("session-b"),
            12,
            AgentEventKind::Input(InputEvent::SessionPaused),
        );
        let error = journal
            .append(wrong_session)
            .expect_err("wrong session should fail");
        assert!(matches!(
            error,
            ModelError::InvalidValue {
                field: "session_id",
                ..
            }
        ));
    }

    #[test]
    fn append_validates_effect_event_joins() {
        let mut journal = InMemoryJournal::new(SessionId::new("session-a"));
        let effect_id = EffectId {
            session_id: SessionId::new("session-a"),
            effect_seq: 1,
        };
        let intent = AgentEffectIntent::new(
            effect_id.clone(),
            SessionId::new("session-a"),
            AgentEffectKind::ToolInvoke(ToolInvocationRequest {
                call_id: ToolCallId::new("call-1"),
                provider_call_id: None,
                tool_id: Some("tool.echo".into()),
                tool_name: "echo".into(),
                arguments_json: None,
                arguments_ref: None,
                handler_id: Some("test.echo".into()),
                context_ref: None,
                metadata: BTreeMap::new(),
            }),
            10,
        );
        let missing_join = AgentEvent::new(
            "event-1",
            SessionId::new("session-a"),
            10,
            AgentEventKind::Effect(EffectEvent::EffectIntentRecorded {
                intent: intent.clone(),
            }),
        );

        let error = journal
            .append(missing_join)
            .expect_err("effect join should be required");
        assert!(matches!(
            error,
            ModelError::InvalidValue {
                field: "joins.effect_id",
                ..
            }
        ));

        let event = AgentEvent::new(
            "event-2",
            SessionId::new("session-a"),
            10,
            AgentEventKind::Effect(EffectEvent::EffectIntentRecorded { intent }),
        )
        .with_joins(AgentEventJoins {
            effect_id: Some(effect_id),
            ..Default::default()
        });

        journal.append(event).expect("append joined effect");
    }

    #[test]
    fn append_validates_effect_receipt_payload_session() {
        let mut journal = InMemoryJournal::new(SessionId::new("session-a"));
        let effect_id = EffectId {
            session_id: SessionId::new("session-a"),
            effect_seq: 1,
        };
        let receipt = AgentEffectReceipt {
            effect_id: effect_id.clone(),
            session_id: SessionId::new("session-b"),
            run_id: None,
            turn_id: None,
            kind: AgentReceiptKind::ToolInvoke(ToolInvocationReceipt {
                call_id: ToolCallId::new("call-1"),
                tool_id: Some("tool.echo".into()),
                tool_name: "echo".into(),
                output_ref: None,
                model_visible_output_ref: None,
                is_error: false,
                metadata: BTreeMap::new(),
            }),
            completed_at_ms: 11,
            metadata: EffectMetadata::default(),
        };
        let event = AgentEvent::new(
            "event-1",
            SessionId::new("session-a"),
            11,
            AgentEventKind::Effect(EffectEvent::EffectReceiptRecorded { receipt }),
        )
        .with_joins(AgentEventJoins {
            effect_id: Some(effect_id),
            ..Default::default()
        });

        let error = journal
            .append(event)
            .expect_err("receipt session mismatch should fail");
        assert!(matches!(
            error,
            ModelError::InvalidValue {
                field: "receipt.session_id",
                ..
            }
        ));
    }
}
