use crate::{
    AttrValue, AttractorCheckpointEventRecord, AttractorCorrelation, AttractorError,
    AttractorRunEventRecord, AttractorStageEventRecord, Graph, Node, NodeOutcome, NodeStatus,
    PipelineRunResult, PipelineStatus, RunConfig, RuntimeContext, validate_or_raise,
};
use forge_turnstore::{ContextId, attractor_idempotency_key};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Default)]
pub struct PipelineRunner;

impl PipelineRunner {
    pub async fn run(
        &self,
        graph: &Graph,
        mut config: RunConfig,
    ) -> Result<PipelineRunResult, AttractorError> {
        validate_or_raise(graph, &[])?;

        let run_id = config
            .run_id
            .take()
            .unwrap_or_else(|| format!("{}-run", graph.id));
        let mut context = mirror_graph_attributes(graph);
        let mut completed_nodes = Vec::new();
        let mut node_outcomes = BTreeMap::new();
        let mut storage = RunStorage::new(
            config.storage.take(),
            run_id.clone(),
            config.base_turn_id.take(),
        )
        .await?;

        storage
            .append_run_event("run_initialized", json!({ "graph_id": graph.id }))
            .await?;

        let start = resolve_start_node(graph)?;
        let mut current_node_id = start.id.clone();
        let mut terminal_failure: Option<String> = None;

        loop {
            let node = graph.nodes.get(&current_node_id).ok_or_else(|| {
                AttractorError::InvalidGraph(format!(
                    "runtime traversal reached unknown node '{}'",
                    current_node_id
                ))
            })?;

            if is_terminal_node(node) {
                if let Some(failed_gate_id) = first_unsatisfied_goal_gate(graph, &node_outcomes) {
                    if let Some(retry_target) = resolve_retry_target(graph, &failed_gate_id) {
                        current_node_id = retry_target;
                        continue;
                    }
                    terminal_failure = Some(format!(
                        "goal gate node '{}' did not reach success and no retry target is configured",
                        failed_gate_id
                    ));
                    break;
                }
                break;
            }

            let stage_attempt_id = stage_attempt_id(node);
            storage
                .append_stage_event(
                    &node.id,
                    &stage_attempt_id,
                    "stage_started",
                    json!({ "node_id": node.id }),
                )
                .await?;

            let outcome = match config.executor.execute(node, &context, graph).await {
                Ok(outcome) => outcome,
                Err(error) => NodeOutcome::failure(error.to_string()),
            };

            let completion_kind = if outcome.status == NodeStatus::Fail {
                "stage_failed"
            } else {
                "stage_completed"
            };
            storage
                .append_stage_event(
                    &node.id,
                    &stage_attempt_id,
                    completion_kind,
                    json!({
                        "node_id": node.id,
                        "status": outcome.status.as_str(),
                        "notes": outcome.notes,
                    }),
                )
                .await?;

            completed_nodes.push(node.id.clone());
            node_outcomes.insert(node.id.clone(), outcome.clone());
            apply_outcome_to_context(&mut context, &outcome);

            storage
                .append_checkpoint_event(
                    &node.id,
                    &stage_attempt_id,
                    json!({
                        "current_node_id": node.id,
                        "completed_nodes_count": completed_nodes.len(),
                        "context_keys_count": context.len(),
                    }),
                )
                .await?;

            let Some(next_edge) = select_next_edge(graph, &node.id) else {
                if outcome.status == NodeStatus::Fail {
                    terminal_failure = Some(
                        outcome
                            .notes
                            .unwrap_or_else(|| "stage failed with no routing target".to_string()),
                    );
                }
                break;
            };
            current_node_id = next_edge.to.clone();
        }

        let status = if terminal_failure.is_some() {
            PipelineStatus::Fail
        } else {
            PipelineStatus::Success
        };

        storage
            .append_run_event(
                "run_finalized",
                json!({
                    "graph_id": graph.id,
                    "status": match status {
                        PipelineStatus::Success => "success",
                        PipelineStatus::Fail => "fail",
                    },
                }),
            )
            .await?;

        Ok(PipelineRunResult {
            run_id,
            status,
            failure_reason: terminal_failure,
            completed_nodes,
            node_outcomes,
            context,
        })
    }
}

fn resolve_start_node(graph: &Graph) -> Result<&Node, AttractorError> {
    graph
        .start_candidates()
        .into_iter()
        .next()
        .ok_or_else(|| AttractorError::InvalidGraph("graph does not have a start node".to_string()))
}

fn is_terminal_node(node: &Node) -> bool {
    node.attrs.get_str("shape") == Some("Msquare")
        || matches!(node.id.to_ascii_lowercase().as_str(), "exit" | "end")
}

fn select_next_edge<'a>(graph: &'a Graph, from_node_id: &'a str) -> Option<&'a crate::Edge> {
    graph
        .outgoing_edges(from_node_id)
        .min_by(|left, right| left.to.cmp(&right.to))
}

fn first_unsatisfied_goal_gate(
    graph: &Graph,
    node_outcomes: &BTreeMap<String, NodeOutcome>,
) -> Option<String> {
    for (node_id, outcome) in node_outcomes {
        let Some(node) = graph.nodes.get(node_id) else {
            continue;
        };
        if node.attrs.get_bool("goal_gate") == Some(true) && !outcome.status.is_success_like() {
            return Some(node_id.clone());
        }
    }
    None
}

fn resolve_retry_target(graph: &Graph, node_id: &str) -> Option<String> {
    let node = graph.nodes.get(node_id)?;
    for key in ["retry_target", "fallback_retry_target"] {
        let target = node.attrs.get_str(key).unwrap_or_default();
        if !target.is_empty() && graph.nodes.contains_key(target) {
            return Some(target.to_string());
        }
    }

    for key in ["retry_target", "fallback_retry_target"] {
        let target = graph.attrs.get_str(key).unwrap_or_default();
        if !target.is_empty() && graph.nodes.contains_key(target) {
            return Some(target.to_string());
        }
    }

    None
}

fn apply_outcome_to_context(context: &mut RuntimeContext, outcome: &NodeOutcome) {
    for (key, value) in &outcome.context_updates {
        context.insert(key.clone(), value.clone());
    }
    context.insert(
        "outcome".to_string(),
        Value::String(outcome.status.as_str().to_string()),
    );
    if let Some(label) = &outcome.preferred_label {
        context.insert("preferred_label".to_string(), Value::String(label.clone()));
    }
}

fn mirror_graph_attributes(graph: &Graph) -> RuntimeContext {
    let mut context = RuntimeContext::new();
    for (key, value) in graph.attrs.values() {
        context.insert(format!("graph.{key}"), attr_value_to_json(value));
    }
    context
}

fn attr_value_to_json(value: &AttrValue) -> Value {
    match value {
        AttrValue::String(inner) => Value::String(inner.clone()),
        AttrValue::Integer(inner) => json!(inner),
        AttrValue::Float(inner) => json!(inner),
        AttrValue::Boolean(inner) => json!(inner),
        AttrValue::Duration(inner) => Value::String(inner.raw.clone()),
    }
}

fn stage_attempt_id(node: &Node) -> String {
    format!("{}:attempt:1", node.id)
}

struct RunStorage {
    writer: Option<crate::storage::SharedAttractorStorageWriter>,
    run_id: String,
    context_id: Option<ContextId>,
    sequence_no: u64,
}

impl RunStorage {
    async fn new(
        writer: Option<crate::storage::SharedAttractorStorageWriter>,
        run_id: String,
        base_turn_id: Option<String>,
    ) -> Result<Self, AttractorError> {
        if let Some(writer_ref) = writer.as_ref() {
            let store_context = writer_ref.create_run_context(base_turn_id).await?;
            return Ok(Self {
                writer,
                run_id,
                context_id: Some(store_context.context_id),
                sequence_no: 0,
            });
        }
        Ok(Self {
            writer: None,
            run_id,
            context_id: None,
            sequence_no: 0,
        })
    }

    async fn append_run_event(
        &mut self,
        event_kind: &str,
        payload: Value,
    ) -> Result<(), AttractorError> {
        let sequence_no = self.next_sequence_no();
        let Some(writer) = self.writer.as_ref().cloned() else {
            return Ok(());
        };
        let Some(context_id) = self.context_id.as_ref().cloned() else {
            return Ok(());
        };
        let correlation = AttractorCorrelation {
            run_id: self.run_id.clone(),
            pipeline_context_id: Some(context_id.clone()),
            node_id: None,
            stage_attempt_id: None,
            parent_turn_id: None,
            sequence_no,
            agent_session_id: None,
            agent_context_id: None,
            agent_head_turn_id: None,
        };
        let idempotency_key =
            attractor_idempotency_key(&self.run_id, "__run__", "__run__", event_kind, sequence_no);
        writer
            .append_run_event(
                &context_id,
                AttractorRunEventRecord {
                    event_kind: event_kind.to_string(),
                    timestamp: timestamp_now(),
                    payload,
                    correlation,
                },
                idempotency_key,
            )
            .await?;
        Ok(())
    }

    async fn append_stage_event(
        &mut self,
        node_id: &str,
        stage_attempt_id: &str,
        event_kind: &str,
        payload: Value,
    ) -> Result<(), AttractorError> {
        let sequence_no = self.next_sequence_no();
        let Some(writer) = self.writer.as_ref().cloned() else {
            return Ok(());
        };
        let Some(context_id) = self.context_id.as_ref().cloned() else {
            return Ok(());
        };
        let correlation = AttractorCorrelation {
            run_id: self.run_id.clone(),
            pipeline_context_id: Some(context_id.clone()),
            node_id: Some(node_id.to_string()),
            stage_attempt_id: Some(stage_attempt_id.to_string()),
            parent_turn_id: None,
            sequence_no,
            agent_session_id: None,
            agent_context_id: None,
            agent_head_turn_id: None,
        };
        let idempotency_key = attractor_idempotency_key(
            &self.run_id,
            node_id,
            stage_attempt_id,
            event_kind,
            sequence_no,
        );
        writer
            .append_stage_event(
                &context_id,
                AttractorStageEventRecord {
                    event_kind: event_kind.to_string(),
                    timestamp: timestamp_now(),
                    payload,
                    correlation,
                },
                idempotency_key,
            )
            .await?;
        Ok(())
    }

    async fn append_checkpoint_event(
        &mut self,
        node_id: &str,
        stage_attempt_id: &str,
        state_summary: Value,
    ) -> Result<(), AttractorError> {
        let sequence_no = self.next_sequence_no();
        let Some(writer) = self.writer.as_ref().cloned() else {
            return Ok(());
        };
        let Some(context_id) = self.context_id.as_ref().cloned() else {
            return Ok(());
        };
        let correlation = AttractorCorrelation {
            run_id: self.run_id.clone(),
            pipeline_context_id: Some(context_id.clone()),
            node_id: Some(node_id.to_string()),
            stage_attempt_id: Some(stage_attempt_id.to_string()),
            parent_turn_id: None,
            sequence_no,
            agent_session_id: None,
            agent_context_id: None,
            agent_head_turn_id: None,
        };
        let checkpoint_id = format!("cp-{}", sequence_no);
        let idempotency_key = attractor_idempotency_key(
            &self.run_id,
            node_id,
            stage_attempt_id,
            "checkpoint_saved",
            sequence_no,
        );
        writer
            .append_checkpoint_event(
                &context_id,
                AttractorCheckpointEventRecord {
                    checkpoint_id,
                    timestamp: timestamp_now(),
                    state_summary,
                    checkpoint_hash: None,
                    correlation,
                },
                idempotency_key,
            )
            .await?;
        Ok(())
    }

    fn next_sequence_no(&mut self) -> u64 {
        self.sequence_no += 1;
        self.sequence_no
    }
}

fn timestamp_now() -> String {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!(
        "{}.{:03}Z",
        since_epoch.as_secs(),
        since_epoch.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AttractorStorageWriter, parse_dot, storage::SharedAttractorStorageWriter};
    use async_trait::async_trait;
    use forge_turnstore::{ContextId, StoreContext, StoredTurn, TurnId, TurnStoreError};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingStorage {
        events: Mutex<Vec<(String, String)>>,
    }

    impl RecordingStorage {
        fn event_kinds(&self) -> Vec<String> {
            self.events
                .lock()
                .expect("events mutex should lock")
                .iter()
                .map(|(_, kind)| kind.clone())
                .collect()
        }
    }

    #[async_trait]
    impl AttractorStorageWriter for RecordingStorage {
        async fn create_run_context(
            &self,
            _base_turn_id: Option<TurnId>,
        ) -> Result<StoreContext, TurnStoreError> {
            Ok(StoreContext {
                context_id: "ctx-1".to_string(),
                head_turn_id: "0".to_string(),
                head_depth: 0,
            })
        }

        async fn append_run_event(
            &self,
            context_id: &ContextId,
            record: AttractorRunEventRecord,
            _idempotency_key: String,
        ) -> Result<StoredTurn, TurnStoreError> {
            self.events
                .lock()
                .expect("events mutex should lock")
                .push((context_id.clone(), record.event_kind.clone()));
            Ok(StoredTurn {
                context_id: context_id.clone(),
                turn_id: "1".to_string(),
                parent_turn_id: "0".to_string(),
                depth: 1,
                type_id: "forge.attractor.run_event".to_string(),
                type_version: 1,
                payload: Vec::new(),
                idempotency_key: None,
                content_hash: None,
            })
        }

        async fn append_stage_event(
            &self,
            context_id: &ContextId,
            record: AttractorStageEventRecord,
            _idempotency_key: String,
        ) -> Result<StoredTurn, TurnStoreError> {
            self.events
                .lock()
                .expect("events mutex should lock")
                .push((context_id.clone(), record.event_kind.clone()));
            Ok(StoredTurn {
                context_id: context_id.clone(),
                turn_id: "2".to_string(),
                parent_turn_id: "0".to_string(),
                depth: 1,
                type_id: "forge.attractor.stage_event".to_string(),
                type_version: 1,
                payload: Vec::new(),
                idempotency_key: None,
                content_hash: None,
            })
        }

        async fn append_checkpoint_event(
            &self,
            context_id: &ContextId,
            _record: AttractorCheckpointEventRecord,
            _idempotency_key: String,
        ) -> Result<StoredTurn, TurnStoreError> {
            self.events
                .lock()
                .expect("events mutex should lock")
                .push((context_id.clone(), "checkpoint_saved".to_string()));
            Ok(StoredTurn {
                context_id: context_id.clone(),
                turn_id: "3".to_string(),
                parent_turn_id: "0".to_string(),
                depth: 1,
                type_id: "forge.attractor.checkpoint_event".to_string(),
                type_version: 1,
                payload: Vec::new(),
                idempotency_key: None,
                content_hash: None,
            })
        }

        async fn append_stage_to_agent_link(
            &self,
            _context_id: &ContextId,
            _record: crate::AttractorStageToAgentLinkRecord,
            _idempotency_key: String,
        ) -> Result<StoredTurn, TurnStoreError> {
            Err(TurnStoreError::Unsupported(
                "stage_to_agent_link is unused in this test".to_string(),
            ))
        }
    }

    fn linear_graph() -> Graph {
        parse_dot(
            r#"
            digraph G {
                graph [goal="ship"]
                start [shape=Mdiamond]
                plan [shape=box]
                exit [shape=Msquare]
                start -> plan -> exit
            }
            "#,
        )
        .expect("graph should parse")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_linear_graph_store_disabled_expected_success() {
        let graph = linear_graph();
        let result = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run should succeed");

        assert_eq!(result.status, PipelineStatus::Success);
        assert_eq!(
            result.completed_nodes,
            vec!["start".to_string(), "plan".to_string()]
        );
        assert_eq!(
            result.context.get("graph.goal"),
            Some(&Value::String("ship".to_string()))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_linear_graph_store_enabled_expected_equivalent_outcome() {
        let graph = linear_graph();
        let storage = Arc::new(RecordingStorage::default());

        let without_store = PipelineRunner
            .run(&graph, RunConfig::default())
            .await
            .expect("run without store should succeed");

        let with_store = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    storage: Some(storage.clone() as SharedAttractorStorageWriter),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run with store should succeed");

        assert_eq!(with_store.status, without_store.status);
        assert_eq!(with_store.completed_nodes, without_store.completed_nodes);
        assert_eq!(with_store.failure_reason, without_store.failure_reason);

        let event_kinds = storage.event_kinds();
        assert!(event_kinds.iter().any(|kind| kind == "run_initialized"));
        assert!(event_kinds.iter().any(|kind| kind == "run_finalized"));
        assert!(event_kinds.iter().any(|kind| kind == "stage_started"));
        assert!(event_kinds.iter().any(|kind| kind == "stage_completed"));
        assert!(event_kinds.iter().any(|kind| kind == "checkpoint_saved"));
    }
}
