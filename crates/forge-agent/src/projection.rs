//! Stable projection item records.
//!
//! This module will contain CLI, JSONL, and future web-client projection items
//! derived from authoritative events.

use crate::batch::ToolCallStatus;
use crate::context::LlmUsageRecord;
use crate::events::FileChangeObservation;
use crate::ids::{EffectId, ProjectionItemId, RunId, SessionId, ToolBatchId, ToolCallId, TurnId};
use crate::refs::ArtifactRef;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionItemLifecycle {
    #[default]
    Started,
    Updated,
    Completed,
    Failed,
    Cancelled,
}

impl ProjectionItemLifecycle {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionJoinIds {
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    pub effect_id: Option<EffectId>,
    pub tool_batch_id: Option<ToolBatchId>,
    pub tool_call_id: Option<ToolCallId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionItem {
    pub item_id: ProjectionItemId,
    pub parent_item_id: Option<ProjectionItemId>,
    pub lifecycle: ProjectionItemLifecycle,
    pub joins: ProjectionJoinIds,
    pub kind: ProjectionItemKind,
    pub title: Option<String>,
    pub preview: Option<String>,
    pub content_ref: Option<ArtifactRef>,
    pub metadata: BTreeMap<String, String>,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub completed_at_ms: Option<u64>,
}

impl ProjectionItem {
    pub fn new(
        item_id: ProjectionItemId,
        joins: ProjectionJoinIds,
        kind: ProjectionItemKind,
        started_at_ms: u64,
    ) -> Self {
        Self {
            item_id,
            parent_item_id: None,
            lifecycle: ProjectionItemLifecycle::Started,
            joins,
            kind,
            title: None,
            preview: None,
            content_ref: None,
            metadata: BTreeMap::new(),
            started_at_ms,
            updated_at_ms: started_at_ms,
            completed_at_ms: None,
        }
    }

    pub fn update_preview(&mut self, preview: impl Into<String>, at_ms: u64) {
        self.preview = Some(preview.into());
        self.lifecycle = ProjectionItemLifecycle::Updated;
        self.updated_at_ms = at_ms;
    }

    pub fn complete(&mut self, content_ref: Option<ArtifactRef>, at_ms: u64) {
        self.content_ref = content_ref;
        self.lifecycle = ProjectionItemLifecycle::Completed;
        self.updated_at_ms = at_ms;
        self.completed_at_ms = Some(at_ms);
    }

    pub fn fail(&mut self, at_ms: u64) {
        self.lifecycle = ProjectionItemLifecycle::Failed;
        self.updated_at_ms = at_ms;
        self.completed_at_ms = Some(at_ms);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ProjectionItemKind {
    User,
    Assistant,
    Reasoning,
    ToolCall {
        tool_name: String,
        status: ToolCallStatus,
    },
    ToolOutput {
        status: ToolCallStatus,
    },
    Patch {
        path: Option<String>,
    },
    Compaction,
    Warning {
        code: String,
    },
    Status {
        status: String,
    },
    FileChange {
        change: FileChangeObservation,
    },
    TokenUsage {
        usage: LlmUsageRecord,
    },
    Cost {
        amount_micros: u64,
        currency: String,
    },
    Custom {
        custom_kind: String,
    },
}

impl Default for ProjectionItemKind {
    fn default() -> Self {
        Self::Custom {
            custom_kind: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionItemPatch {
    pub item_id: ProjectionItemId,
    pub lifecycle: Option<ProjectionItemLifecycle>,
    pub title: Option<String>,
    pub preview: Option<String>,
    pub content_ref: Option<ArtifactRef>,
    pub metadata: BTreeMap<String, String>,
    pub observed_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::FileChangeKind;
    use crate::ids::IdAllocator;
    use crate::refs::{ArtifactKind, ArtifactRef};

    #[test]
    fn projection_item_tracks_lifecycle_and_join_ids() {
        let mut ids = IdAllocator::new(SessionId::new("session-a"));
        let run_id = ids.allocate_run_id();
        let turn_id = ids.allocate_turn_id(&run_id);
        let effect_id = ids.allocate_effect_id();
        let item_id = ids.allocate_projection_item_id();
        let joins = ProjectionJoinIds {
            session_id: ids.session_id.clone(),
            run_id: Some(run_id.clone()),
            turn_id: Some(turn_id.clone()),
            effect_id: Some(effect_id.clone()),
            tool_batch_id: None,
            tool_call_id: None,
        };

        let mut item = ProjectionItem::new(item_id, joins, ProjectionItemKind::Assistant, 10);
        item.update_preview("partial", 11);
        item.complete(
            Some(ArtifactRef::new(
                "blob://assistant",
                ArtifactKind::AssistantMessage,
            )),
            12,
        );

        assert_eq!(item.lifecycle, ProjectionItemLifecycle::Completed);
        assert!(item.lifecycle.is_terminal());
        assert_eq!(item.joins.run_id, Some(run_id));
        assert_eq!(item.joins.turn_id, Some(turn_id));
        assert_eq!(item.joins.effect_id, Some(effect_id));
        assert_eq!(item.completed_at_ms, Some(12));
    }

    #[test]
    fn projection_item_can_represent_file_change_and_round_trip() {
        let item = ProjectionItem::new(
            ProjectionItemId {
                session_id: SessionId::new("session-a"),
                item_seq: 1,
            },
            ProjectionJoinIds {
                session_id: SessionId::new("session-a"),
                ..Default::default()
            },
            ProjectionItemKind::FileChange {
                change: FileChangeObservation {
                    path: "src/lib.rs".into(),
                    change_kind: FileChangeKind::Modified,
                    before_ref: None,
                    after_ref: Some(ArtifactRef::new("blob://after", ArtifactKind::FileContent)),
                    patch_ref: None,
                },
            },
            10,
        );

        let encoded = serde_json::to_string(&item).expect("serialize projection item");
        let decoded: ProjectionItem =
            serde_json::from_str(&encoded).expect("decode projection item");
        assert_eq!(decoded, item);
    }
}
