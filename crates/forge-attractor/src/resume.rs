use crate::{
    AttractorError, CheckpointState, ContextStore, Graph, NodeOutcome, PipelineStatus,
    RuntimeContext, checkpoint_file_path, select_next_edge,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ResumeState {
    pub checkpoint: CheckpointState,
    pub next_node_id: Option<String>,
    pub terminal_status: Option<PipelineStatus>,
    pub terminal_failure_reason: Option<String>,
    pub degrade_fidelity_once: bool,
}

pub fn resolve_resume_state(
    graph: &Graph,
    checkpoint_path: &Path,
) -> Result<ResumeState, AttractorError> {
    let checkpoint = CheckpointState::load_from_path(checkpoint_path)?;
    let terminal_status = checkpoint.terminal_pipeline_status()?;
    let next_node_id = if terminal_status.is_some() {
        None
    } else {
        checkpoint
            .next_node
            .clone()
            .or_else(|| infer_next_node_from_checkpoint(graph, &checkpoint).ok().flatten())
    };

    if let Some(next_node) = next_node_id.as_deref() {
        if !graph.nodes.contains_key(next_node) {
            return Err(AttractorError::Runtime(format!(
                "resume checkpoint points to unknown next node '{}'",
                next_node
            )));
        }
    }

    Ok(ResumeState {
        degrade_fidelity_once: checkpoint.current_node_fidelity.as_deref() == Some("full")
            && next_node_id.is_some(),
        terminal_failure_reason: checkpoint.terminal_failure_reason.clone(),
        checkpoint,
        next_node_id,
        terminal_status,
    })
}

pub fn checkpoint_path_for_run(
    logs_root: Option<&Path>,
    explicit_checkpoint_path: Option<&Path>,
) -> Option<PathBuf> {
    explicit_checkpoint_path
        .map(Path::to_path_buf)
        .or_else(|| logs_root.map(checkpoint_file_path))
}

pub fn apply_resume_fidelity_override(
    context_store: &ContextStore,
    degrade_fidelity_once: bool,
) -> Result<(), AttractorError> {
    if degrade_fidelity_once {
        context_store.set(
            "internal.resume.fidelity_override_once",
            Value::String("summary:high".to_string()),
        )?;
        context_store.set(
            "internal.resume.fidelity_degrade_pending",
            Value::Bool(true),
        )?;
        return Ok(());
    }

    context_store.remove("internal.resume.fidelity_override_once")?;
    context_store.remove("internal.resume.fidelity_degrade_pending")?;
    Ok(())
}

pub fn effective_node_fidelity(
    graph: &Graph,
    target_node_id: &str,
    incoming_from_node_id: Option<&str>,
) -> String {
    if let Some(from) = incoming_from_node_id {
        for edge in graph.outgoing_edges(from) {
            if edge.to == target_node_id {
                if let Some(fidelity) = edge.attrs.get_str("fidelity") {
                    let trimmed = fidelity.trim();
                    if !trimmed.is_empty() {
                        return trimmed.to_string();
                    }
                }
            }
        }
    }

    if let Some(node) = graph.nodes.get(target_node_id) {
        if let Some(fidelity) = node.attrs.get_str("fidelity") {
            let trimmed = fidelity.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }

    if let Some(fidelity) = graph.attrs.get_str("default_fidelity") {
        let trimmed = fidelity.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    "compact".to_string()
}

pub fn build_resume_runtime_state(
    graph: &Graph,
    checkpoint_path: &Path,
) -> Result<ResumeRuntimeState, AttractorError> {
    let resume = resolve_resume_state(graph, checkpoint_path)?;

    let mut node_outcomes = std::collections::BTreeMap::new();
    for (node_id, stored) in &resume.checkpoint.node_outcomes {
        node_outcomes.insert(node_id.clone(), stored.to_runtime()?);
    }

    Ok(ResumeRuntimeState {
        checkpoint_run_id: resume.checkpoint.metadata.run_id.clone(),
        context: resume.checkpoint.context_values.clone(),
        completed_nodes: resume.checkpoint.completed_nodes.clone(),
        node_retries: resume.checkpoint.node_retries.clone(),
        node_outcomes,
        next_node_id: resume.next_node_id,
        terminal_status: resume.terminal_status,
        terminal_failure_reason: resume.terminal_failure_reason,
        degrade_fidelity_once: resume.degrade_fidelity_once,
    })
}

#[derive(Clone, Debug)]
pub struct ResumeRuntimeState {
    pub checkpoint_run_id: String,
    pub context: RuntimeContext,
    pub completed_nodes: Vec<String>,
    pub node_retries: std::collections::BTreeMap<String, u32>,
    pub node_outcomes: std::collections::BTreeMap<String, NodeOutcome>,
    pub next_node_id: Option<String>,
    pub terminal_status: Option<PipelineStatus>,
    pub terminal_failure_reason: Option<String>,
    pub degrade_fidelity_once: bool,
}

impl ResumeRuntimeState {
    pub fn checkpoint_run_id(&self) -> &str {
        &self.checkpoint_run_id
    }
}

pub fn infer_next_node_from_checkpoint(
    graph: &Graph,
    checkpoint: &CheckpointState,
) -> Result<Option<String>, AttractorError> {
    if checkpoint.next_node.is_some() {
        return Ok(checkpoint.next_node.clone());
    }

    let Some(current) = checkpoint.completed_nodes.last() else {
        return Ok(None);
    };
    let Some(outcome) = checkpoint.node_outcomes.get(current) else {
        return Ok(None);
    };
    let runtime_outcome = outcome.to_runtime()?;

    Ok(select_next_edge(graph, current, &runtime_outcome, &checkpoint.context_values)
        .map(|edge| edge.to.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckpointMetadata, CheckpointNodeOutcome, parse_dot};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn write_checkpoint(path: &Path, checkpoint: CheckpointState) {
        checkpoint
            .save_to_path(path)
            .expect("checkpoint should save");
    }

    #[test]
    fn effective_node_fidelity_edge_precedence_expected_edge_value() {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [default_fidelity="compact"]
                start [shape=Mdiamond]
                plan [fidelity="summary:low"]
                start -> plan [fidelity="full"]
            }
            "#,
        )
        .expect("graph should parse");

        let resolved = effective_node_fidelity(&graph, "plan", Some("start"));
        assert_eq!(resolved, "full");
    }

    #[test]
    fn resolve_resume_state_full_fidelity_expected_degrade_once_true() {
        let temp = TempDir::new().expect("temp dir should create");
        let path = checkpoint_file_path(temp.path());
        write_checkpoint(
            &path,
            CheckpointState {
                metadata: CheckpointMetadata {
                    schema_version: 1,
                    run_id: "run-1".to_string(),
                    checkpoint_id: "cp-7".to_string(),
                    sequence_no: 7,
                    timestamp: "1.000Z".to_string(),
                },
                current_node: "plan".to_string(),
                next_node: Some("review".to_string()),
                completed_nodes: vec!["start".to_string(), "plan".to_string()],
                node_retries: BTreeMap::new(),
                node_outcomes: BTreeMap::from([(
                    "plan".to_string(),
                    CheckpointNodeOutcome {
                        status: "success".to_string(),
                        notes: None,
                        preferred_label: None,
                        suggested_next_ids: vec![],
                    },
                )]),
                context_values: BTreeMap::new(),
                logs: vec![],
                current_node_fidelity: Some("full".to_string()),
                terminal_status: None,
                terminal_failure_reason: None,
            },
        );

        let resolved = resolve_resume_state(
            &parse_dot("digraph G { review } ").expect("graph parse"),
            &path,
        )
        .expect("resume should resolve");

        assert!(resolved.degrade_fidelity_once);
        assert_eq!(resolved.next_node_id.as_deref(), Some("review"));
    }
}
