//! Context-window, token, pressure, and compaction model records.
//!
//! This module keeps the bounded context-control snapshot needed to plan the
//! next turn. Full transcript and compaction history lives in journal and
//! projection records.

use crate::refs::ArtifactRef;
use crate::transcript::TranscriptRange;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCompatibility {
    pub provider: String,
    pub api_kind: String,
    pub model: Option<String>,
    pub model_family: Option<String>,
    pub artifact_type: String,
    pub opaque: bool,
    pub encrypted: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveWindowItemKind {
    #[default]
    MessageRef,
    SummaryRef,
    ProviderNativeArtifactRef,
    ProviderRawWindowRef,
    ToolDefinitionRef,
    ResponseFormatRef,
    ProviderOptionsRef,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextInputLane {
    System,
    Developer,
    #[default]
    Conversation,
    ToolResult,
    Steer,
    Summary,
    Memory,
    Skill,
    Domain,
    RuntimeHint,
    Custom,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextMetadataEntry {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveWindowItem {
    pub item_id: String,
    pub kind: ActiveWindowItemKind,
    pub content_ref: ArtifactRef,
    pub lane: Option<ContextInputLane>,
    pub source_range: Option<TranscriptRange>,
    pub source_refs: Vec<ArtifactRef>,
    pub provider_compatibility: Option<ProviderCompatibility>,
    pub estimated_tokens: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

impl ActiveWindowItem {
    pub fn message_ref(
        item_id: impl Into<String>,
        content_ref: ArtifactRef,
        lane: Option<ContextInputLane>,
        source_range: Option<TranscriptRange>,
    ) -> Self {
        Self {
            item_id: item_id.into(),
            kind: ActiveWindowItemKind::MessageRef,
            source_refs: vec![content_ref.clone()],
            content_ref,
            lane,
            source_range,
            provider_compatibility: None,
            estimated_tokens: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmUsageRecord {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmTokenCountQuality {
    Exact,
    ProviderEstimate,
    LocalEstimate,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmTokenCountRecord {
    pub input_tokens: Option<u64>,
    pub original_input_tokens: Option<u64>,
    pub tool_tokens: Option<u64>,
    pub response_format_tokens: Option<u64>,
    pub quality: LlmTokenCountQuality,
    pub provider: String,
    pub model: String,
    pub candidate_plan_id: Option<String>,
    pub provider_metadata_ref: Option<ArtifactRef>,
    pub warnings_ref: Option<ArtifactRef>,
    pub counted_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStrategy {
    ProviderNative,
    Summary,
    #[default]
    Auto,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionArtifactKind {
    Summary,
    ProviderNative,
    #[default]
    Mixed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextPressureReason {
    ProviderContextLimit,
    ProviderRecommended,
    UsageHighWater,
    LocalWindowPolicy,
    Manual,
    CountTokensOverBudget,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPressureRecord {
    pub reason: ContextPressureReason,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub candidate_plan_id: Option<String>,
    pub observed_usage: Option<LlmUsageRecord>,
    pub error_kind: Option<String>,
    pub error_ref: Option<ArtifactRef>,
    pub recommended_strategy: Option<CompactionStrategy>,
    pub observed_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextOperationPhase {
    #[default]
    Idle,
    NeedsCompaction,
    CountingTokens,
    Compacting,
    ApplyingCompaction,
    Failed,
}

impl ContextOperationPhase {
    pub fn blocks_generation(self) -> bool {
        !matches!(self, Self::Idle)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextOperationState {
    pub operation_id: String,
    pub phase: ContextOperationPhase,
    pub reason: ContextPressureReason,
    pub candidate_plan_id: Option<String>,
    pub strategy: CompactionStrategy,
    pub source_range: Option<TranscriptRange>,
    pub source_items_ref: Option<ArtifactRef>,
    pub effect_intent_id: Option<String>,
    pub params_hash: Option<String>,
    pub failure: Option<String>,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
}

impl ContextOperationState {
    pub fn blocks_generation(&self) -> bool {
        self.phase.blocks_generation()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionRecord {
    pub operation_id: String,
    pub strategy: CompactionStrategy,
    pub artifact_kind: CompactionArtifactKind,
    pub artifact_refs: Vec<ArtifactRef>,
    pub source_range: TranscriptRange,
    pub source_refs: Vec<ArtifactRef>,
    pub active_window_items: Vec<ActiveWindowItem>,
    pub provider_compatibility: Option<ProviderCompatibility>,
    pub usage: Option<LlmUsageRecord>,
    pub created_at_ms: u64,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionSummary {
    pub operation_id: String,
    pub strategy: CompactionStrategy,
    pub artifact_kind: CompactionArtifactKind,
    pub artifact_refs: Vec<ArtifactRef>,
    pub source_range: TranscriptRange,
    pub compacted_through: Option<u64>,
    pub active_window_item_count: u64,
    pub usage: Option<LlmUsageRecord>,
    pub created_at_ms: u64,
    pub warnings: Vec<String>,
}

impl From<&CompactionRecord> for CompactionSummary {
    fn from(record: &CompactionRecord) -> Self {
        Self {
            operation_id: record.operation_id.clone(),
            strategy: record.strategy,
            artifact_kind: record.artifact_kind,
            artifact_refs: record.artifact_refs.clone(),
            source_range: record.source_range.clone(),
            compacted_through: Some(record.source_range.end_seq),
            active_window_item_count: record.active_window_items.len() as u64,
            usage: record.usage,
            created_at_ms: record.created_at_ms,
            warnings: record.warnings.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextState {
    pub next_transcript_seq: u64,
    pub active_transcript_range: Option<TranscriptRange>,
    pub active_window_items: Vec<ActiveWindowItem>,
    pub compacted_through: Option<u64>,
    pub pending_context_operation: Option<ContextOperationState>,
    pub last_llm_usage: Option<LlmUsageRecord>,
    pub last_context_pressure: Option<ContextPressureRecord>,
    pub last_token_count: Option<LlmTokenCountRecord>,
    pub last_compaction: Option<CompactionSummary>,
}

impl ContextState {
    pub fn set_pending_operation(&mut self, operation: ContextOperationState) {
        if matches!(operation.phase, ContextOperationPhase::Idle) {
            self.pending_context_operation = None;
        } else {
            self.pending_context_operation = Some(operation);
        }
    }

    pub fn clear_pending_operation(&mut self) {
        self.pending_context_operation = None;
    }

    pub fn append_message_ref(
        &mut self,
        item_id: impl Into<String>,
        content_ref: ArtifactRef,
        lane: Option<ContextInputLane>,
    ) -> ActiveWindowItem {
        let seq = self.next_transcript_seq;
        self.next_transcript_seq = self.next_transcript_seq.saturating_add(1);
        let source_range = TranscriptRange::single(seq);
        self.active_transcript_range = Some(match self.active_transcript_range.take() {
            Some(mut range) => {
                range.end_seq = range.end_seq.max(source_range.end_seq);
                range
            }
            None => source_range.clone(),
        });
        let item = ActiveWindowItem::message_ref(item_id, content_ref, lane, Some(source_range));
        self.active_window_items.push(item.clone());
        item
    }

    pub fn apply_compaction(&mut self, record: CompactionRecord) {
        self.active_window_items = record.active_window_items.clone();
        self.compacted_through = Some(record.source_range.end_seq);
        self.active_transcript_range = Some(TranscriptRange {
            start_seq: record.source_range.end_seq,
            end_seq: self.next_transcript_seq.max(record.source_range.end_seq),
        });
        self.last_compaction = Some(CompactionSummary::from(&record));
        self.clear_pending_operation();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_state_appends_message_to_active_window_with_bounded_sequence() {
        let mut state = ContextState::default();
        let item = state.append_message_ref(
            "item-1",
            ArtifactRef::new("blob://user-1"),
            Some(ContextInputLane::Conversation),
        );

        assert_eq!(state.next_transcript_seq, 1);
        assert_eq!(state.active_window_items, vec![item]);
        assert_eq!(
            state.active_window_items[0].source_range,
            Some(TranscriptRange::single(0))
        );
        assert_eq!(
            state.active_transcript_range,
            Some(TranscriptRange::single(0))
        );
    }

    #[test]
    fn non_idle_context_operation_blocks_generation() {
        let operation = ContextOperationState {
            operation_id: "compact-1".into(),
            phase: ContextOperationPhase::NeedsCompaction,
            reason: ContextPressureReason::UsageHighWater,
            strategy: CompactionStrategy::Summary,
            ..Default::default()
        };

        assert!(operation.blocks_generation());
    }

    #[test]
    fn compaction_record_preserves_source_range_and_artifact_refs() {
        let record = CompactionRecord {
            operation_id: "compact-1".into(),
            strategy: CompactionStrategy::Summary,
            artifact_kind: CompactionArtifactKind::Summary,
            artifact_refs: vec![ArtifactRef::new("blob://summary")],
            source_range: TranscriptRange {
                start_seq: 0,
                end_seq: 4,
            },
            source_refs: vec![ArtifactRef::new("blob://source")],
            ..Default::default()
        };

        assert_eq!(record.source_range.end_seq, 4);
        assert_eq!(record.artifact_refs.len(), 1);
    }

    #[test]
    fn context_state_applies_compaction_without_retaining_history_vector() {
        let mut state = ContextState::default();
        state.next_transcript_seq = 4;
        let record = CompactionRecord {
            operation_id: "compact-1".into(),
            strategy: CompactionStrategy::Summary,
            artifact_kind: CompactionArtifactKind::Summary,
            artifact_refs: vec![ArtifactRef::new("blob://summary")],
            source_range: TranscriptRange {
                start_seq: 0,
                end_seq: 4,
            },
            active_window_items: vec![ActiveWindowItem::message_ref(
                "summary",
                ArtifactRef::new("blob://summary"),
                Some(ContextInputLane::Summary),
                Some(TranscriptRange {
                    start_seq: 0,
                    end_seq: 4,
                }),
            )],
            ..Default::default()
        };

        state.apply_compaction(record);

        assert_eq!(state.compacted_through, Some(4));
        assert_eq!(state.active_window_items.len(), 1);
        assert_eq!(
            state
                .last_compaction
                .as_ref()
                .map(|summary| summary.active_window_item_count),
            Some(1)
        );
    }
}
