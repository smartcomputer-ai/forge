use crate::{
    AttrValue, AttractorCheckpointEventRecord, AttractorCorrelation, AttractorError,
    AttractorRunEventRecord, AttractorStageEventRecord, ContextStore, Graph, Node, NodeOutcome,
    NodeStatus, PipelineRunResult, PipelineStatus, RetryPolicy, RunConfig, RuntimeContext,
    build_retry_policy,
    delay_for_attempt_ms, finalize_retry_exhausted, select_next_edge, should_retry_outcome,
    validate_or_raise,
};
use forge_turnstore::{ContextId, attractor_idempotency_key};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::thread::sleep;
use std::time::Duration;
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
        let context_store = ContextStore::from_values(mirror_graph_attributes(graph));
        if let Some(logs_root) = config.logs_root.as_ref() {
            context_store.set(
                "runtime.logs_root",
                Value::String(logs_root.to_string_lossy().to_string()),
            )?;
        }
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
            context_store.set("current_node", Value::String(node.id.clone()))?;

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

            let retry_policy = build_retry_policy(node, graph, config.retry_backoff.clone());
            let context_snapshot = context_store.snapshot()?;
            let (outcome, attempts_used) = execute_with_retry(
                node,
                graph,
                &context_snapshot.values,
                &*config.executor,
                &retry_policy,
                &mut storage,
                &run_id,
            )
            .await?;

            completed_nodes.push(node.id.clone());
            node_outcomes.insert(node.id.clone(), outcome.clone());
            apply_outcome_to_context(&context_store, &outcome)?;

            storage
                .append_checkpoint_event(
                    &node.id,
                    &stage_attempt_id(node, attempts_used),
                    json!({
                        "current_node_id": node.id,
                        "completed_nodes_count": completed_nodes.len(),
                        "context_keys_count": context_store.snapshot()?.values.len(),
                    }),
                )
                .await?;

            if outcome.status == NodeStatus::Fail {
                if let Some(edge) = select_fail_edge(graph, &node.id) {
                    current_node_id = edge.to.clone();
                    continue;
                }
                if let Some(target) = resolve_node_failure_target(graph, node) {
                    current_node_id = target;
                    continue;
                }
                terminal_failure = Some(
                    outcome
                        .notes
                        .clone()
                        .unwrap_or_else(|| "stage failed with no routing target".to_string()),
                );
                break;
            }

            let routing_context = context_store.snapshot()?;
            let Some(next_edge) =
                select_next_edge(graph, &node.id, &outcome, &routing_context.values)
            else {
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
            context: context_store.snapshot()?.values,
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

fn resolve_node_failure_target(graph: &Graph, node: &Node) -> Option<String> {
    for key in ["retry_target", "fallback_retry_target"] {
        let target = node.attrs.get_str(key).unwrap_or_default();
        if !target.is_empty() && graph.nodes.contains_key(target) {
            return Some(target.to_string());
        }
    }
    None
}

fn select_fail_edge<'a>(graph: &'a Graph, node_id: &'a str) -> Option<&'a crate::Edge> {
    let fail_edges: Vec<&crate::Edge> = graph
        .outgoing_edges(node_id)
        .filter(|edge| is_fail_condition(edge.attrs.get_str("condition").unwrap_or_default()))
        .collect();
    if fail_edges.is_empty() {
        return None;
    }

    fail_edges.into_iter().max_by(|left, right| {
        edge_weight(left)
            .cmp(&edge_weight(right))
            .then_with(|| right.to.cmp(&left.to))
    })
}

fn is_fail_condition(condition: &str) -> bool {
    condition.replace(' ', "") == "outcome=fail"
}

fn edge_weight(edge: &crate::Edge) -> i64 {
    edge.attrs
        .get("weight")
        .and_then(|value| value.as_i64())
        .unwrap_or(0)
}

fn apply_outcome_to_context(context: &ContextStore, outcome: &NodeOutcome) -> Result<(), AttractorError> {
    context.apply_updates(&outcome.context_updates)?;
    context.set(
        "outcome",
        Value::String(outcome.status.as_str().to_string()),
    )?;
    if let Some(label) = &outcome.preferred_label {
        context.set("preferred_label", Value::String(label.clone()))?;
    }
    Ok(())
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

fn stage_attempt_id(node: &Node, attempt: u32) -> String {
    format!("{}:attempt:{attempt}", node.id)
}

async fn execute_with_retry(
    node: &Node,
    graph: &Graph,
    context: &RuntimeContext,
    executor: &dyn crate::NodeExecutor,
    retry_policy: &RetryPolicy,
    storage: &mut RunStorage,
    run_id: &str,
) -> Result<(NodeOutcome, u32), AttractorError> {
    for attempt in 1..=retry_policy.max_attempts {
        let stage_attempt_id = stage_attempt_id(node, attempt);
        let mut attempt_context = context.clone();
        attempt_context.insert(
            "stage_attempt_id".to_string(),
            Value::String(stage_attempt_id.clone()),
        );
        storage
            .append_stage_event(
                &node.id,
                &stage_attempt_id,
                "stage_started",
                json!({ "node_id": node.id, "attempt": attempt }),
            )
            .await?;

        let outcome = match executor.execute(node, &attempt_context, graph).await {
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
                    "attempt": attempt,
                    "status": outcome.status.as_str(),
                    "notes": outcome.notes,
                }),
            )
            .await?;

        if outcome.status.is_success_like() {
            return Ok((outcome, attempt));
        }

        if should_retry_outcome(&outcome) && attempt < retry_policy.max_attempts {
            let delay_ms = delay_for_attempt_ms(
                attempt,
                &retry_policy.backoff,
                hash_run_node(run_id, &node.id),
            );
            if delay_ms > 0 {
                sleep(Duration::from_millis(delay_ms));
            }
            continue;
        }

        if outcome.status == NodeStatus::Retry && attempt >= retry_policy.max_attempts {
            return Ok((finalize_retry_exhausted(node), attempt));
        }

        return Ok((outcome, attempt));
    }

    Ok((
        NodeOutcome::failure("max retries exceeded"),
        retry_policy.max_attempts,
    ))
}

fn hash_run_node(run_id: &str, node_id: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in run_id.bytes().chain(node_id.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
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
    use crate::{
        AttractorStorageWriter, NodeExecutor, NodeOutcome, NodeStatus, parse_dot,
        storage::SharedAttractorStorageWriter,
    };
    use async_trait::async_trait;
    use forge_turnstore::{ContextId, StoreContext, StoredTurn, TurnId, TurnStoreError};
    use std::sync::{Arc, Mutex, atomic::AtomicUsize, atomic::Ordering};

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

    struct PreferredLabelExecutor;

    #[async_trait]
    impl NodeExecutor for PreferredLabelExecutor {
        async fn execute(
            &self,
            node: &Node,
            _context: &RuntimeContext,
            _graph: &Graph,
        ) -> Result<NodeOutcome, AttractorError> {
            if node.id == "gate" {
                return Ok(NodeOutcome {
                    status: NodeStatus::Success,
                    notes: None,
                    context_updates: RuntimeContext::new(),
                    preferred_label: Some("No".to_string()),
                    suggested_next_ids: Vec::new(),
                });
            }
            Ok(NodeOutcome::success())
        }
    }

    struct ConditionFailExecutor;

    #[async_trait]
    impl NodeExecutor for ConditionFailExecutor {
        async fn execute(
            &self,
            node: &Node,
            _context: &RuntimeContext,
            _graph: &Graph,
        ) -> Result<NodeOutcome, AttractorError> {
            if node.id == "gate" {
                return Ok(NodeOutcome {
                    status: NodeStatus::Fail,
                    notes: Some("intentional failure".to_string()),
                    context_updates: RuntimeContext::new(),
                    preferred_label: None,
                    suggested_next_ids: Vec::new(),
                });
            }
            Ok(NodeOutcome::success())
        }
    }

    struct RetryThenSuccessExecutor {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl NodeExecutor for RetryThenSuccessExecutor {
        async fn execute(
            &self,
            node: &Node,
            _context: &RuntimeContext,
            _graph: &Graph,
        ) -> Result<NodeOutcome, AttractorError> {
            if node.id != "work" {
                return Ok(NodeOutcome::success());
            }
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call < 3 {
                return Ok(NodeOutcome {
                    status: NodeStatus::Retry,
                    notes: Some("retry requested".to_string()),
                    context_updates: RuntimeContext::new(),
                    preferred_label: None,
                    suggested_next_ids: Vec::new(),
                });
            }
            Ok(NodeOutcome::success())
        }
    }

    struct FailThenSuccessExecutor {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl NodeExecutor for FailThenSuccessExecutor {
        async fn execute(
            &self,
            node: &Node,
            _context: &RuntimeContext,
            _graph: &Graph,
        ) -> Result<NodeOutcome, AttractorError> {
            if node.id != "work" {
                return Ok(NodeOutcome::success());
            }
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call < 2 {
                return Ok(NodeOutcome::failure("first failure"));
            }
            Ok(NodeOutcome::success())
        }
    }

    struct AlwaysRetryExecutor;

    #[async_trait]
    impl NodeExecutor for AlwaysRetryExecutor {
        async fn execute(
            &self,
            node: &Node,
            _context: &RuntimeContext,
            _graph: &Graph,
        ) -> Result<NodeOutcome, AttractorError> {
            if node.id != "work" {
                return Ok(NodeOutcome::success());
            }
            Ok(NodeOutcome {
                status: NodeStatus::Retry,
                notes: Some("keep retrying".to_string()),
                context_updates: RuntimeContext::new(),
                preferred_label: None,
                suggested_next_ids: Vec::new(),
            })
        }
    }

    struct AlwaysFailAtGateExecutor;

    #[async_trait]
    impl NodeExecutor for AlwaysFailAtGateExecutor {
        async fn execute(
            &self,
            node: &Node,
            _context: &RuntimeContext,
            _graph: &Graph,
        ) -> Result<NodeOutcome, AttractorError> {
            if node.id == "gate" {
                return Ok(NodeOutcome::failure("gate failed"));
            }
            Ok(NodeOutcome::success())
        }
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

    #[tokio::test(flavor = "current_thread")]
    async fn run_branching_graph_preferred_label_expected_selected_branch() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate
                yes
                no
                exit [shape=Msquare]
                start -> gate
                gate -> yes [label="Yes"]
                gate -> no [label="No"]
                yes -> exit
                no -> exit
            }
            "#,
        )
        .expect("graph should parse");

        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    executor: Arc::new(PreferredLabelExecutor),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");

        assert!(result.completed_nodes.iter().any(|node| node == "no"));
        assert!(!result.completed_nodes.iter().any(|node| node == "yes"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_branching_graph_condition_route_expected_fail_edge_taken() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate
                retry_path
                success_path
                exit [shape=Msquare]
                start -> gate
                gate -> retry_path [condition="outcome=fail"]
                gate -> success_path [condition="outcome=success"]
                retry_path -> exit
                success_path -> exit
            }
            "#,
        )
        .expect("graph should parse");

        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    executor: Arc::new(ConditionFailExecutor),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");

        assert!(
            result
                .completed_nodes
                .iter()
                .any(|node| node == "retry_path")
        );
        assert!(
            !result
                .completed_nodes
                .iter()
                .any(|node| node == "success_path")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_retries_on_retry_status_expected_attempts_and_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [max_retries=2]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("graph should parse");
        let executor = Arc::new(RetryThenSuccessExecutor {
            calls: AtomicUsize::new(0),
        });

        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    executor: executor.clone(),
                    retry_backoff: crate::RetryBackoffConfig {
                        initial_delay_ms: 0,
                        backoff_factor: 1.0,
                        max_delay_ms: 0,
                        jitter: false,
                    },
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");

        assert_eq!(result.status, PipelineStatus::Success);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_retries_on_fail_status_expected_attempts_and_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [max_retries=1]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("graph should parse");
        let executor = Arc::new(FailThenSuccessExecutor {
            calls: AtomicUsize::new(0),
        });

        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    executor: executor.clone(),
                    retry_backoff: crate::RetryBackoffConfig {
                        initial_delay_ms: 0,
                        backoff_factor: 1.0,
                        max_delay_ms: 0,
                        jitter: false,
                    },
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");

        assert_eq!(result.status, PipelineStatus::Success);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_retry_exhausted_allow_partial_expected_partial_success() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                work [max_retries=1, allow_partial=true]
                exit [shape=Msquare]
                start -> work -> exit
            }
            "#,
        )
        .expect("graph should parse");

        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    executor: Arc::new(AlwaysRetryExecutor),
                    retry_backoff: crate::RetryBackoffConfig {
                        initial_delay_ms: 0,
                        backoff_factor: 1.0,
                        max_delay_ms: 0,
                        jitter: false,
                    },
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");

        assert_eq!(result.status, PipelineStatus::Success);
        assert_eq!(
            result
                .node_outcomes
                .get("work")
                .expect("work outcome should exist")
                .status,
            NodeStatus::PartialSuccess
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_failure_routing_fail_edge_beats_retry_targets_expected_fail_edge_taken() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate [retry_target="retry_target", fallback_retry_target="fallback_target"]
                fail_edge_target
                retry_target
                fallback_target
                exit [shape=Msquare]
                start -> gate
                gate -> fail_edge_target [condition="outcome=fail"]
                gate -> retry_target
                gate -> fallback_target
                fail_edge_target -> exit
                retry_target -> exit
                fallback_target -> exit
            }
            "#,
        )
        .expect("graph should parse");

        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    executor: Arc::new(AlwaysFailAtGateExecutor),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");

        assert!(
            result
                .completed_nodes
                .iter()
                .any(|node| node == "fail_edge_target")
        );
        assert!(
            !result
                .completed_nodes
                .iter()
                .any(|node| node == "retry_target")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_failure_routing_retry_target_then_fallback_expected_order() {
        let graph_retry_target = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate [retry_target="retry_target", fallback_retry_target="fallback_target"]
                retry_target
                fallback_target
                exit [shape=Msquare]
                start -> gate
                gate -> retry_target
                gate -> fallback_target
                retry_target -> exit
                fallback_target -> exit
            }
            "#,
        )
        .expect("graph should parse");
        let retry_target_result = PipelineRunner
            .run(
                &graph_retry_target,
                RunConfig {
                    executor: Arc::new(AlwaysFailAtGateExecutor),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");
        assert!(
            retry_target_result
                .completed_nodes
                .iter()
                .any(|node| node == "retry_target")
        );

        let graph_fallback = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate [retry_target="missing", fallback_retry_target="fallback_target"]
                fallback_target
                exit [shape=Msquare]
                start -> gate
                gate -> fallback_target
                fallback_target -> exit
            }
            "#,
        )
        .expect("graph should parse");
        let fallback_result = PipelineRunner
            .run(
                &graph_fallback,
                RunConfig {
                    executor: Arc::new(AlwaysFailAtGateExecutor),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");
        assert!(
            fallback_result
                .completed_nodes
                .iter()
                .any(|node| node == "fallback_target")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_failure_without_route_expected_pipeline_fail() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                gate
                exit [shape=Msquare]
                start -> gate
                gate -> exit
            }
            "#,
        )
        .expect("graph should parse");

        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    executor: Arc::new(AlwaysFailAtGateExecutor),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should complete");

        assert_eq!(result.status, PipelineStatus::Fail);
        assert!(result.failure_reason.is_some());
    }
}
