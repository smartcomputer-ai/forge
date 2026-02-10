use crate::{AttractorError, NodeOutcome, NodeStatus, PipelineStatus, RuntimeContext};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const CHECKPOINT_FILE_NAME: &str = "checkpoint.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointMetadata {
    pub schema_version: u32,
    pub run_id: String,
    pub checkpoint_id: String,
    pub sequence_no: u64,
    pub timestamp: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckpointNodeOutcome {
    pub status: String,
    pub notes: Option<String>,
    pub preferred_label: Option<String>,
    pub suggested_next_ids: Vec<String>,
}

impl CheckpointNodeOutcome {
    pub fn from_runtime(outcome: &NodeOutcome) -> Self {
        Self {
            status: outcome.status.as_str().to_string(),
            notes: outcome.notes.clone(),
            preferred_label: outcome.preferred_label.clone(),
            suggested_next_ids: outcome.suggested_next_ids.clone(),
        }
    }

    pub fn to_runtime(&self) -> Result<NodeOutcome, AttractorError> {
        let status = NodeStatus::try_from(self.status.as_str())?;
        Ok(NodeOutcome {
            status,
            notes: self.notes.clone(),
            context_updates: RuntimeContext::new(),
            preferred_label: self.preferred_label.clone(),
            suggested_next_ids: self.suggested_next_ids.clone(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckpointState {
    pub metadata: CheckpointMetadata,
    pub current_node: String,
    pub next_node: Option<String>,
    pub completed_nodes: Vec<String>,
    pub node_retries: BTreeMap<String, u32>,
    pub node_outcomes: BTreeMap<String, CheckpointNodeOutcome>,
    pub context_values: RuntimeContext,
    pub logs: Vec<String>,
    pub current_node_fidelity: Option<String>,
    pub terminal_status: Option<String>,
    pub terminal_failure_reason: Option<String>,
    #[serde(default)]
    pub graph_dot_source_hash: Option<String>,
    #[serde(default)]
    pub graph_dot_source_ref: Option<String>,
    #[serde(default)]
    pub graph_snapshot_hash: Option<String>,
    #[serde(default)]
    pub graph_snapshot_ref: Option<String>,
}

impl CheckpointState {
    pub fn save_to_path(&self, path: &Path) -> Result<(), AttractorError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                AttractorError::Runtime(format!(
                    "failed to create checkpoint parent directory '{}': {}",
                    parent.display(),
                    error
                ))
            })?;
        }

        let bytes = serde_json::to_vec_pretty(self).map_err(|error| {
            AttractorError::Runtime(format!("failed to serialize checkpoint: {error}"))
        })?;

        fs::write(path, bytes).map_err(|error| {
            AttractorError::Runtime(format!(
                "failed writing checkpoint file '{}': {}",
                path.display(),
                error
            ))
        })
    }

    pub fn load_from_path(path: &Path) -> Result<Self, AttractorError> {
        let bytes = fs::read(path).map_err(|error| {
            AttractorError::Runtime(format!(
                "failed reading checkpoint file '{}': {}",
                path.display(),
                error
            ))
        })?;
        serde_json::from_slice(&bytes).map_err(|error| {
            AttractorError::Runtime(format!(
                "failed deserializing checkpoint file '{}': {}",
                path.display(),
                error
            ))
        })
    }

    pub fn terminal_pipeline_status(&self) -> Result<Option<PipelineStatus>, AttractorError> {
        match self.terminal_status.as_deref() {
            Some("success") => Ok(Some(PipelineStatus::Success)),
            Some("fail") => Ok(Some(PipelineStatus::Fail)),
            Some(other) => Err(AttractorError::Runtime(format!(
                "checkpoint has unknown terminal status '{other}'"
            ))),
            None => Ok(None),
        }
    }
}

pub fn checkpoint_file_path(logs_root: &Path) -> PathBuf {
    logs_root.join(CHECKPOINT_FILE_NAME)
}

impl TryFrom<&str> for NodeStatus {
    type Error = AttractorError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "success" => Ok(Self::Success),
            "partial_success" => Ok(Self::PartialSuccess),
            "retry" => Ok(Self::Retry),
            "fail" => Ok(Self::Fail),
            other => Err(AttractorError::Runtime(format!(
                "unknown node status '{other}' in checkpoint"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn checkpoint_roundtrip_path_expected_preserves_fields() {
        let temp = TempDir::new().expect("temp dir should be created");
        let path = checkpoint_file_path(temp.path());
        let checkpoint = CheckpointState {
            metadata: CheckpointMetadata {
                schema_version: 1,
                run_id: "run-1".to_string(),
                checkpoint_id: "cp-1".to_string(),
                sequence_no: 5,
                timestamp: "123.000Z".to_string(),
            },
            current_node: "plan".to_string(),
            next_node: Some("review".to_string()),
            completed_nodes: vec!["start".to_string(), "plan".to_string()],
            node_retries: BTreeMap::from([("plan".to_string(), 1)]),
            node_outcomes: BTreeMap::from([(
                "plan".to_string(),
                CheckpointNodeOutcome {
                    status: "success".to_string(),
                    notes: Some("ok".to_string()),
                    preferred_label: None,
                    suggested_next_ids: vec![],
                },
            )]),
            context_values: BTreeMap::from([("outcome".to_string(), json!("success"))]),
            logs: vec!["checkpoint saved".to_string()],
            current_node_fidelity: Some("full".to_string()),
            terminal_status: None,
            terminal_failure_reason: None,
            graph_dot_source_hash: Some("dot-hash".to_string()),
            graph_dot_source_ref: Some("artifact://dot".to_string()),
            graph_snapshot_hash: Some("snapshot-hash".to_string()),
            graph_snapshot_ref: Some("artifact://snapshot".to_string()),
        };

        checkpoint
            .save_to_path(&path)
            .expect("checkpoint should save");
        let loaded = CheckpointState::load_from_path(&path).expect("checkpoint should load");
        assert_eq!(loaded, checkpoint);
    }

    #[test]
    fn checkpoint_node_outcome_to_runtime_expected_status_mapping() {
        let checkpoint_outcome = CheckpointNodeOutcome {
            status: "partial_success".to_string(),
            notes: Some("n".to_string()),
            preferred_label: Some("yes".to_string()),
            suggested_next_ids: vec!["a".to_string()],
        };

        let runtime = checkpoint_outcome
            .to_runtime()
            .expect("conversion should succeed");
        assert_eq!(runtime.status, NodeStatus::PartialSuccess);
        assert_eq!(runtime.preferred_label.as_deref(), Some("yes"));
    }
}
