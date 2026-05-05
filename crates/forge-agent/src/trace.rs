//! Bounded run trace records.
//!
//! Run traces explain why the state machine made decisions without requiring
//! consumers to replay every low-level event.

use crate::ids::{EffectId, ToolBatchId, ToolCallId, TurnId};
use crate::refs::ArtifactRef;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_RUN_TRACE_MAX_ENTRIES: u64 = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RunTraceEntryKind {
    #[default]
    RunStarted,
    TurnPlanned,
    LlmRequested,
    LlmReceived,
    ToolCallsObserved,
    ToolBatchPlanned,
    EffectEmitted,
    StreamFrameObserved,
    ReceiptSettled,
    ContextOperationStateChanged,
    ContextPressureObserved,
    CompactionRequested,
    CompactionReceived,
    TokenCountRequested,
    TokenCountReceived,
    ActiveWindowUpdated,
    InterventionRequested,
    InterventionApplied,
    RunFinished,
    Custom {
        custom_kind: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RunTraceRef {
    Artifact {
        ref_: ArtifactRef,
    },
    Effect {
        effect_id: EffectId,
    },
    Turn {
        turn_id: TurnId,
    },
    ToolBatch {
        tool_batch_id: ToolBatchId,
    },
    ToolCall {
        call_id: ToolCallId,
    },
    Value {
        label: String,
        value: String,
    },
    #[default]
    None,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTraceEntry {
    pub seq: u64,
    pub observed_at_ms: u64,
    pub kind: RunTraceEntryKind,
    pub summary: String,
    pub refs: Vec<RunTraceRef>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTrace {
    pub max_entries: u64,
    pub dropped_entries: u64,
    pub next_seq: u64,
    pub entries: Vec<RunTraceEntry>,
}

impl Default for RunTrace {
    fn default() -> Self {
        Self {
            max_entries: DEFAULT_RUN_TRACE_MAX_ENTRIES,
            dropped_entries: 0,
            next_seq: 0,
            entries: Vec::new(),
        }
    }
}

impl RunTrace {
    pub fn with_max_entries(max_entries: u64) -> Self {
        Self {
            max_entries,
            ..Default::default()
        }
    }

    pub fn push(
        &mut self,
        observed_at_ms: u64,
        kind: RunTraceEntryKind,
        summary: impl Into<String>,
        refs: Vec<RunTraceRef>,
        metadata: BTreeMap<String, String>,
    ) -> RunTraceEntry {
        let entry = RunTraceEntry {
            seq: self.next_seq,
            observed_at_ms,
            kind,
            summary: summary.into(),
            refs,
            metadata,
        };
        self.next_seq = self.next_seq.saturating_add(1);

        if self.max_entries == 0 {
            self.dropped_entries = self.dropped_entries.saturating_add(1);
            return entry;
        }

        while self.entries.len() >= self.max_entries as usize {
            self.entries.remove(0);
            self.dropped_entries = self.dropped_entries.saturating_add(1);
        }
        self.entries.push(entry.clone());
        entry
    }

    pub fn summarize(&self) -> RunTraceSummary {
        let first = self.entries.first();
        let last = self.entries.last();
        RunTraceSummary {
            entry_count: self.entries.len() as u64,
            dropped_entries: self.dropped_entries,
            first_seq: first.map(|entry| entry.seq),
            last_seq: last.map(|entry| entry.seq),
            last_kind: last.map(|entry| entry.kind.clone()),
            last_summary: last.map(|entry| entry.summary.clone()),
            last_observed_at_ms: last.map(|entry| entry.observed_at_ms),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTraceSummary {
    pub entry_count: u64,
    pub dropped_entries: u64,
    pub first_seq: Option<u64>,
    pub last_seq: Option<u64>,
    pub last_kind: Option<RunTraceEntryKind>,
    pub last_summary: Option<String>,
    pub last_observed_at_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_retention_is_bounded_and_summarized() {
        let mut trace = RunTrace::with_max_entries(2);
        trace.push(
            10,
            RunTraceEntryKind::RunStarted,
            "run started",
            Vec::new(),
            BTreeMap::new(),
        );
        trace.push(
            11,
            RunTraceEntryKind::TurnPlanned,
            "turn planned",
            Vec::new(),
            BTreeMap::new(),
        );
        trace.push(
            12,
            RunTraceEntryKind::LlmRequested,
            "llm requested",
            Vec::new(),
            BTreeMap::new(),
        );

        let summary = trace.summarize();
        assert_eq!(summary.entry_count, 2);
        assert_eq!(summary.dropped_entries, 1);
        assert_eq!(summary.first_seq, Some(1));
        assert_eq!(summary.last_seq, Some(2));
        assert_eq!(summary.last_summary.as_deref(), Some("llm requested"));
    }

    #[test]
    fn zero_retention_drops_all_entries() {
        let mut trace = RunTrace::with_max_entries(0);
        trace.push(
            10,
            RunTraceEntryKind::RunStarted,
            "run started",
            Vec::new(),
            BTreeMap::new(),
        );

        assert!(trace.entries.is_empty());
        assert_eq!(trace.dropped_entries, 1);
        assert_eq!(trace.summarize().entry_count, 0);
    }
}
