//! Transcript ledger and message records.
//!
//! This module will contain source-ranged transcript entries and artifact-backed
//! user, assistant, tool result, system, developer, steering, and summary
//! records.

use crate::ids::ToolCallId;
use crate::refs::ArtifactRef;
use serde::{Deserialize, Serialize};

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
    use crate::refs::ArtifactKind;

    #[test]
    fn ledger_append_allocates_sequence_and_source_range() {
        let mut ledger = TranscriptLedger::default();
        let first = ledger.append(
            TranscriptEntryKind::Message,
            ArtifactRef::new("blob://user-1", ArtifactKind::UserPrompt),
            "user",
            10,
        );
        let second = ledger.append(
            TranscriptEntryKind::Summary,
            ArtifactRef::new("blob://summary-1", ArtifactKind::Compaction),
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
    fn transcript_record_round_trips_through_msgpack() {
        let record = TranscriptRecord::ToolResult(ToolResultRecord {
            tool_call_id: ToolCallId::new("call-1"),
            content_ref: ArtifactRef::new("blob://tool-output", ArtifactKind::ToolOutput),
            model_visible_ref: None,
            is_error: false,
            source_range: Some(TranscriptRange::single(4)),
        });

        let encoded = rmp_serde::to_vec_named(&record).expect("encode transcript record");
        let decoded: TranscriptRecord =
            rmp_serde::from_slice(&encoded).expect("decode transcript record");
        assert_eq!(decoded, record);
    }
}
