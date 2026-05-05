//! Subagent model records.
//!
//! Subagents are modeled as child sessions with parent/child metadata and
//! explicit routing/cancellation state.

use crate::ids::{RunId, SessionId};
use crate::refs::ArtifactRef;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentRole {
    pub role_id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentRelationship {
    pub parent_session_id: SessionId,
    pub parent_run_id: Option<RunId>,
    pub child_session_id: SessionId,
    pub depth: u64,
    pub role: Option<SubagentRole>,
    pub inherited_context_refs: Vec<ArtifactRef>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    #[default]
    Running,
    Interrupted,
    Completed,
    Errored,
    Shutdown,
    NotFound,
}

impl SubagentStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Interrupted | Self::Completed | Self::Errored | Self::Shutdown | Self::NotFound
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationPropagation {
    None,
    #[default]
    ParentToChild,
    Bidirectional,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentEventRouting {
    pub forward_observations: bool,
    pub forward_tool_events: bool,
    pub forward_final_output: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentRecord {
    pub relationship: SubagentRelationship,
    pub status: SubagentStatus,
    pub cancellation: CancellationPropagation,
    pub event_routing: SubagentEventRouting,
    pub spawned_at_ms: u64,
    pub updated_at_ms: u64,
    pub final_output_ref: Option<ArtifactRef>,
    pub failure_ref: Option<ArtifactRef>,
}

impl SubagentRecord {
    pub fn new(relationship: SubagentRelationship, spawned_at_ms: u64) -> Self {
        Self {
            relationship,
            status: SubagentStatus::Running,
            cancellation: CancellationPropagation::ParentToChild,
            event_routing: SubagentEventRouting {
                forward_observations: true,
                forward_tool_events: false,
                forward_final_output: true,
            },
            spawned_at_ms,
            updated_at_ms: spawned_at_ms,
            final_output_ref: None,
            failure_ref: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_statuses_are_terminal() {
        assert!(!SubagentStatus::Running.is_terminal());
        assert!(SubagentStatus::Completed.is_terminal());
        assert!(SubagentStatus::Errored.is_terminal());
    }

    #[test]
    fn subagent_record_starts_running_with_parent_to_child_cancellation() {
        let relationship = SubagentRelationship {
            parent_session_id: SessionId::new("parent"),
            child_session_id: SessionId::new("child"),
            depth: 1,
            ..Default::default()
        };

        let record = SubagentRecord::new(relationship, 10);
        assert_eq!(record.status, SubagentStatus::Running);
        assert_eq!(record.cancellation, CancellationPropagation::ParentToChild);
        assert!(record.event_routing.forward_final_output);
    }
}
