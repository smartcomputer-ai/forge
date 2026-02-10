use crate::storage::{BlobHash, ContextId, TurnId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ATTRACTOR_RUN_LIFECYCLE_TYPE_ID: &str = "forge.attractor.run_lifecycle";
pub const ATTRACTOR_STAGE_LIFECYCLE_TYPE_ID: &str = "forge.attractor.stage_lifecycle";
pub const ATTRACTOR_PARALLEL_LIFECYCLE_TYPE_ID: &str = "forge.attractor.parallel_lifecycle";
pub const ATTRACTOR_INTERVIEW_LIFECYCLE_TYPE_ID: &str = "forge.attractor.interview_lifecycle";
pub const ATTRACTOR_CHECKPOINT_SAVED_TYPE_ID: &str = "forge.attractor.checkpoint_saved";
pub const ATTRACTOR_ROUTE_DECISION_TYPE_ID: &str = "forge.attractor.route_decision";
pub const ATTRACTOR_STAGE_TO_AGENT_LINK_TYPE_ID: &str = "forge.link.stage_to_agent";
pub const ATTRACTOR_DOT_SOURCE_TYPE_ID: &str = "forge.attractor.dot_source";
pub const ATTRACTOR_GRAPH_SNAPSHOT_TYPE_ID: &str = "forge.attractor.graph_snapshot";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FsSnapshotStats {
    pub file_count: u64,
    pub dir_count: u64,
    pub symlink_count: u64,
    pub total_bytes: u64,
    pub bytes_uploaded: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunLifecycleRecord {
    pub kind: String,
    pub timestamp: String,
    pub run_id: String,
    pub graph_id: String,
    pub lineage_root_run_id: String,
    pub lineage_attempt: u32,
    pub status: Option<String>,
    pub reason: Option<String>,
    pub restart_target: Option<String>,
    pub dot_source_hash: Option<String>,
    pub dot_source_ref: Option<String>,
    pub graph_snapshot_hash: Option<String>,
    pub graph_snapshot_ref: Option<String>,
    pub sequence_no: u64,
    pub fs_root_hash: Option<String>,
    pub snapshot_policy_id: Option<String>,
    pub snapshot_stats: Option<FsSnapshotStats>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageLifecycleRecord {
    pub kind: String,
    pub timestamp: String,
    pub run_id: String,
    pub node_id: String,
    pub stage_attempt_id: String,
    pub attempt: u32,
    pub status: Option<String>,
    pub notes: Option<String>,
    pub will_retry: Option<bool>,
    pub next_attempt: Option<u32>,
    pub delay_ms: Option<u64>,
    pub sequence_no: u64,
    pub fs_root_hash: Option<String>,
    pub snapshot_policy_id: Option<String>,
    pub snapshot_stats: Option<FsSnapshotStats>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParallelLifecycleRecord {
    pub kind: String,
    pub timestamp: String,
    pub run_id: String,
    pub node_id: String,
    pub branch_count: Option<usize>,
    pub branch_id: Option<String>,
    pub branch_index: Option<usize>,
    pub target_node: Option<String>,
    pub status: Option<String>,
    pub notes: Option<String>,
    pub success_count: Option<usize>,
    pub failure_count: Option<usize>,
    pub sequence_no: u64,
    pub fs_root_hash: Option<String>,
    pub snapshot_policy_id: Option<String>,
    pub snapshot_stats: Option<FsSnapshotStats>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InterviewLifecycleRecord {
    pub kind: String,
    pub timestamp: String,
    pub run_id: String,
    pub node_id: String,
    pub selected: Option<String>,
    pub default_selected: Option<String>,
    pub sequence_no: u64,
    pub fs_root_hash: Option<String>,
    pub snapshot_policy_id: Option<String>,
    pub snapshot_stats: Option<FsSnapshotStats>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckpointSavedRecord {
    pub timestamp: String,
    pub run_id: String,
    pub node_id: String,
    pub stage_attempt_id: String,
    pub checkpoint_id: String,
    pub state_summary: Value,
    pub checkpoint_hash: Option<BlobHash>,
    pub sequence_no: u64,
    pub fs_root_hash: Option<String>,
    pub snapshot_policy_id: Option<String>,
    pub snapshot_stats: Option<FsSnapshotStats>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RouteDecisionRecord {
    pub timestamp: String,
    pub run_id: String,
    pub node_id: String,
    pub stage_attempt_id: String,
    pub selected_edge: Option<String>,
    pub loop_restart: bool,
    pub terminated_status: Option<String>,
    pub terminated_reason: Option<String>,
    pub sequence_no: u64,
    pub fs_root_hash: Option<String>,
    pub snapshot_policy_id: Option<String>,
    pub snapshot_stats: Option<FsSnapshotStats>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageToAgentLinkRecord {
    pub timestamp: String,
    pub run_id: String,
    pub pipeline_context_id: ContextId,
    pub node_id: String,
    pub stage_attempt_id: String,
    pub agent_session_id: String,
    pub agent_context_id: ContextId,
    pub agent_head_turn_id: Option<TurnId>,
    pub parent_turn_id: Option<TurnId>,
    pub sequence_no: u64,
    pub thread_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DotSourceRecord {
    pub timestamp: String,
    pub run_id: String,
    pub dot_source: Option<String>,
    pub artifact_blob_hash: Option<BlobHash>,
    pub content_hash: BlobHash,
    pub size_bytes: u64,
    pub sequence_no: u64,
    pub fs_root_hash: Option<String>,
    pub snapshot_policy_id: Option<String>,
    pub snapshot_stats: Option<FsSnapshotStats>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphSnapshotRecord {
    pub timestamp: String,
    pub run_id: String,
    pub graph_snapshot: Option<Value>,
    pub artifact_blob_hash: Option<BlobHash>,
    pub content_hash: BlobHash,
    pub size_bytes: u64,
    pub sequence_no: u64,
    pub fs_root_hash: Option<String>,
    pub snapshot_policy_id: Option<String>,
    pub snapshot_stats: Option<FsSnapshotStats>,
}
