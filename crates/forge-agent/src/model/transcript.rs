//! Transcript ledger and message records.
//!
//! This module will contain source-ranged transcript entries and artifact-backed
//! user, assistant, tool result, system, developer, steering, and summary
//! records.

use crate::ids::{
    EffectId, JournalSeq, ProjectionItemId, RunId, SessionId, ToolBatchId, ToolCallId, TurnId,
};
use crate::refs::ArtifactRef;
use serde::{Deserialize, Serialize};

pub const TRANSCRIPT_LEDGER_RECORD_KIND: &str = "forge.agent.runtime.v2.transcript_ledger";
pub const TRANSCRIPT_ITEM_RECORD_KIND: &str = "forge.agent.runtime.v2.transcript_item";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptRange {
    pub start_seq: u64,
    pub end_seq: u64,
}

impl TranscriptRange {
    pub fn single(seq: u64) -> Self {
        Self {
            start_seq: seq,
            end_seq: seq.saturating_add(1),
        }
    }

    pub fn contains(&self, seq: u64) -> bool {
        self.start_seq <= seq && seq < self.end_seq
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptBoundary {
    pub entry_seq: Option<u64>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptEntryKind {
    #[default]
    Message,
    Reasoning,
    ToolResult,
    Summary,
    ProviderArtifact,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptLedgerEntry {
    pub seq: u64,
    pub kind: TranscriptEntryKind,
    pub content_ref: ArtifactRef,
    pub source: String,
    pub source_range: Option<TranscriptRange>,
    pub appended_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptLedger {
    pub next_seq: u64,
    pub entries: Vec<TranscriptLedgerEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptItemJoins {
    pub session_id: SessionId,
    pub journal_seq: Option<JournalSeq>,
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    pub effect_id: Option<EffectId>,
    pub tool_batch_id: Option<ToolBatchId>,
    pub tool_call_id: Option<ToolCallId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptItem {
    pub item_id: ProjectionItemId,
    pub joins: TranscriptItemJoins,
    pub kind: TranscriptEntryKind,
    pub source_event_id: Option<String>,
    pub content_ref: Option<ArtifactRef>,
    pub preview: Option<String>,
    pub source_range: Option<TranscriptRange>,
    pub metadata: std::collections::BTreeMap<String, String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

impl TranscriptLedger {
    pub fn append(
        &mut self,
        kind: TranscriptEntryKind,
        content_ref: ArtifactRef,
        source: impl Into<String>,
        appended_at_ms: u64,
    ) -> TranscriptLedgerEntry {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let entry = TranscriptLedgerEntry {
            seq,
            kind,
            content_ref,
            source: source.into(),
            source_range: Some(TranscriptRange::single(seq)),
            appended_at_ms,
        };
        self.entries.push(entry.clone());
        entry
    }

    pub fn range(&self) -> Option<TranscriptRange> {
        let first = self.entries.first()?;
        let last = self.entries.last()?;
        Some(TranscriptRange {
            start_seq: first.seq,
            end_seq: last.seq.saturating_add(1),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Developer,
    Steering,
    Tool,
    Summary,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRecord {
    pub role: MessageRole,
    pub content_ref: ArtifactRef,
    pub name: Option<String>,
    pub source_range: Option<TranscriptRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantMessageRecord {
    pub content_ref: Option<ArtifactRef>,
    pub reasoning_ref: Option<ArtifactRef>,
    pub raw_response_ref: Option<ArtifactRef>,
    pub provider_response_id: Option<String>,
    pub source_range: Option<TranscriptRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultRecord {
    pub tool_call_id: ToolCallId,
    pub content_ref: ArtifactRef,
    pub model_visible_ref: Option<ArtifactRef>,
    pub is_error: bool,
    pub source_range: Option<TranscriptRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryRecord {
    pub summary_ref: ArtifactRef,
    pub source_range: TranscriptRange,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum TranscriptRecord {
    User(MessageRecord),
    Assistant(AssistantMessageRecord),
    Reasoning {
        content_ref: ArtifactRef,
        source_range: Option<TranscriptRange>,
    },
    ToolResult(ToolResultRecord),
    System(MessageRecord),
    Developer(MessageRecord),
    Steering(MessageRecord),
    Summary(SummaryRecord),
    Custom {
        custom_kind: String,
        content_ref: ArtifactRef,
        source_range: Option<TranscriptRange>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ledger_append_allocates_sequence_and_source_range() {
        let mut ledger = TranscriptLedger::default();
        let first = ledger.append(
            TranscriptEntryKind::Message,
            ArtifactRef::new("blob://user-1"),
            "user",
            10,
        );
        let second = ledger.append(
            TranscriptEntryKind::Summary,
            ArtifactRef::new("blob://summary-1"),
            "compaction",
            11,
        );

        assert_eq!(first.seq, 0);
        assert_eq!(second.seq, 1);
        assert_eq!(ledger.next_seq, 2);
        assert!(
            second
                .source_range
                .as_ref()
                .is_some_and(|range| range.contains(1))
        );
        assert_eq!(
            ledger.range(),
            Some(TranscriptRange {
                start_seq: 0,
                end_seq: 2
            })
        );
    }

    #[test]
    fn transcript_boundary_round_trips_through_msgpack() {
        let boundary = TranscriptBoundary {
            entry_seq: Some(3),
            event_id: Some("event-3".into()),
        };

        let encoded = rmp_serde::to_vec_named(&boundary).expect("encode transcript boundary");
        let decoded: TranscriptBoundary =
            rmp_serde::from_slice(&encoded).expect("decode transcript boundary");
        assert_eq!(decoded, boundary);
    }

    #[test]
    fn transcript_record_round_trips_through_msgpack() {
        let record = TranscriptRecord::ToolResult(ToolResultRecord {
            tool_call_id: ToolCallId::new("call-1"),
            content_ref: ArtifactRef::new("blob://tool-output"),
            model_visible_ref: None,
            is_error: false,
            source_range: Some(TranscriptRange::single(4)),
        });

        let encoded = rmp_serde::to_vec_named(&record).expect("encode transcript record");
        let decoded: TranscriptRecord =
            rmp_serde::from_slice(&encoded).expect("decode transcript record");
        assert_eq!(decoded, record);
    }

    #[test]
    fn transcript_item_carries_journal_and_join_ids() {
        let item = TranscriptItem {
            item_id: ProjectionItemId {
                session_id: SessionId::new("session-a"),
                item_seq: 1,
            },
            joins: TranscriptItemJoins {
                session_id: SessionId::new("session-a"),
                journal_seq: Some(JournalSeq(7)),
                tool_call_id: Some(ToolCallId::new("call-1")),
                ..Default::default()
            },
            kind: TranscriptEntryKind::ToolResult,
            source_event_id: Some("event-7".into()),
            content_ref: Some(ArtifactRef::new("blob://tool-output")),
            preview: Some("ok".into()),
            source_range: None,
            metadata: std::collections::BTreeMap::new(),
            created_at_ms: 10,
            updated_at_ms: 10,
        };

        let encoded = serde_json::to_string(&item).expect("serialize transcript item");
        let decoded: TranscriptItem =
            serde_json::from_str(&encoded).expect("deserialize transcript item");

        assert_eq!(decoded, item);
    }
}
