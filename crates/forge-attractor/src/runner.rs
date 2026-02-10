use crate::{
    AttrValue, AttractorCheckpointEventRecord, AttractorCorrelation, AttractorDotSourceRecord,
    AttractorError, AttractorGraphSnapshotRecord, AttractorRunEventRecord,
    AttractorStageEventRecord, CheckpointEvent, CheckpointMetadata, CheckpointNodeOutcome,
    CheckpointState, ContextStore, Graph, InterviewEvent, Node, NodeOutcome, NodeStatus,
    ParallelEvent, PipelineEvent, PipelineRunResult, PipelineStatus, RetryPolicy, RunConfig,
    RuntimeContext, RuntimeEvent, RuntimeEventKind, RuntimeEventSink, StageEvent,
    apply_resume_fidelity_override, build_resume_runtime_state, build_retry_policy,
    checkpoint_path_for_run, delay_for_attempt_ms, finalize_retry_exhausted, find_incoming_edge,
    resolve_fidelity_mode, resolve_thread_key, select_next_edge, should_retry_outcome,
    validate_or_raise,
};
use forge_turnstore::{ContextId, attractor_idempotency_key};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::thread::sleep;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Default)]
struct PersistedRunGraphMetadata {
    dot_source_hash: Option<String>,
    dot_source_ref: Option<String>,
    graph_snapshot_hash: Option<String>,
    graph_snapshot_ref: Option<String>,
}

#[derive(Debug, Default)]
pub struct PipelineRunner;

impl PipelineRunner {
    pub async fn run(
        &self,
        graph: &Graph,
        mut config: RunConfig,
    ) -> Result<PipelineRunResult, AttractorError> {
        validate_or_raise(graph, &[])?;
        let event_sink = config.events.clone();
        let mut event_sequence_no = 0u64;

        let lineage_root_run_id = config
            .run_id
            .take()
            .unwrap_or_else(|| format!("{}-run", graph.id));
        let mut storage_writer = config.storage.take();
        let mut base_turn_id = config.base_turn_id.take();
        let mut resume_path_for_attempt = config.resume_from_checkpoint.take();
        let mut restart_start_node: Option<String> = None;
        let mut lineage_attempt = 1u32;

        loop {
            let active_run_id = if lineage_attempt == 1 {
                lineage_root_run_id.clone()
            } else {
                format!("{}:attempt:{}", lineage_root_run_id, lineage_attempt)
            };
            let attempt_logs_root =
                prepare_attempt_logs_root(config.logs_root.as_ref(), lineage_attempt)?;
            let checkpoint_path = checkpoint_path_for_run(
                attempt_logs_root.as_deref(),
                resume_path_for_attempt.as_deref(),
            );

            let mut context_store = ContextStore::from_values(mirror_graph_attributes(graph));
            if let Some(logs_root) = attempt_logs_root.as_ref() {
                context_store.set(
                    "runtime.logs_root",
                    Value::String(logs_root.to_string_lossy().to_string()),
                )?;
                context_store.set(
                    "runtime.artifacts_dir",
                    Value::String(logs_root.join("artifacts").to_string_lossy().to_string()),
                )?;
            }
            context_store.set(
                "internal.lineage.root_run_id",
                Value::String(lineage_root_run_id.clone()),
            )?;
            context_store.set(
                "internal.lineage.attempt",
                Value::Number((lineage_attempt as u64).into()),
            )?;
            if lineage_attempt > 1 {
                context_store.set(
                    "internal.lineage.parent_run_id",
                    Value::String(format!(
                        "{}:attempt:{}",
                        lineage_root_run_id,
                        lineage_attempt - 1
                    )),
                )?;
            }

            let mut completed_nodes: Vec<String> = Vec::new();
            let mut node_outcomes: BTreeMap<String, NodeOutcome> = BTreeMap::new();
            let mut node_retry_counts: BTreeMap<String, u32> = BTreeMap::new();
            let mut current_node_id = restart_start_node
                .clone()
                .unwrap_or(resolve_start_node(graph)?.id.clone());
            let mut terminal_failure: Option<String> = None;
            let mut forced_terminal_status: Option<PipelineStatus> = None;
            let mut resume_fidelity_degrade_pending = false;
            let mut restart_target: Option<String> = None;

            if let Some(resume_path) = resume_path_for_attempt.as_ref() {
                let resume = build_resume_runtime_state(graph, resume_path)?;
                if active_run_id != resume.checkpoint_run_id() {
                    return Err(AttractorError::Runtime(format!(
                        "resume run_id mismatch: config run_id '{}' vs checkpoint run_id '{}'",
                        active_run_id,
                        resume.checkpoint_run_id()
                    )));
                }
                context_store = ContextStore::from_values(resume.context);
                if let Some(logs_root) = attempt_logs_root.as_ref() {
                    context_store.set(
                        "runtime.logs_root",
                        Value::String(logs_root.to_string_lossy().to_string()),
                    )?;
                    context_store.set(
                        "runtime.artifacts_dir",
                        Value::String(logs_root.join("artifacts").to_string_lossy().to_string()),
                    )?;
                }
                context_store.set(
                    "internal.lineage.root_run_id",
                    Value::String(lineage_root_run_id.clone()),
                )?;
                context_store.set(
                    "internal.lineage.attempt",
                    Value::Number((lineage_attempt as u64).into()),
                )?;

                completed_nodes = resume.completed_nodes;
                node_outcomes = resume.node_outcomes;
                node_retry_counts = resume.node_retries;
                terminal_failure = resume.terminal_failure_reason;
                forced_terminal_status = resume.terminal_status;
                resume_fidelity_degrade_pending = resume.degrade_fidelity_once;
                apply_resume_fidelity_override(&context_store, resume_fidelity_degrade_pending)?;
                if let Some(next_node_id) = resume.next_node_id {
                    current_node_id = next_node_id;
                } else if forced_terminal_status.is_none() {
                    return Err(AttractorError::Runtime(
                        "resume checkpoint has no next node and no terminal status".to_string(),
                    ));
                }
            }

            let mut storage = RunStorage::new(
                storage_writer.take(),
                active_run_id.clone(),
                base_turn_id.take(),
            )
            .await?;
            let graph_metadata = storage.persist_run_graph_metadata(graph).await?;

            emit_runtime_event(
                &event_sink,
                &mut event_sequence_no,
                RuntimeEventKind::Pipeline(PipelineEvent::Started {
                    run_id: active_run_id.clone(),
                    graph_id: graph.id.clone(),
                    lineage_attempt,
                }),
            );

            storage
                .append_run_event(
                    "run_initialized",
                    json!({
                        "graph_id": graph.id,
                        "lineage_root_run_id": lineage_root_run_id,
                        "lineage_attempt": lineage_attempt,
                        "dot_source_hash": graph_metadata.dot_source_hash,
                        "dot_source_ref": graph_metadata.dot_source_ref,
                        "graph_snapshot_hash": graph_metadata.graph_snapshot_hash,
                        "graph_snapshot_ref": graph_metadata.graph_snapshot_ref,
                    }),
                )
                .await?;
            if resume_path_for_attempt.is_some() {
                emit_runtime_event(
                    &event_sink,
                    &mut event_sequence_no,
                    RuntimeEventKind::Pipeline(PipelineEvent::Resumed {
                        run_id: active_run_id.clone(),
                        graph_id: graph.id.clone(),
                        lineage_attempt,
                    }),
                );
                storage
                    .append_run_event(
                        "run_resumed",
                        json!({
                            "graph_id": graph.id,
                            "lineage_root_run_id": lineage_root_run_id,
                            "lineage_attempt": lineage_attempt,
                        }),
                    )
                    .await?;
            }

            while forced_terminal_status.is_none() {
                let node = graph.nodes.get(&current_node_id).ok_or_else(|| {
                    AttractorError::InvalidGraph(format!(
                        "runtime traversal reached unknown node '{}'",
                        current_node_id
                    ))
                })?;
                context_store.set("current_node", Value::String(node.id.clone()))?;
                let previous_node_id = completed_nodes.last().cloned();
                let incoming_edge =
                    find_incoming_edge(graph, &node.id, previous_node_id.as_deref());
                let mut effective_fidelity = resolve_fidelity_mode(graph, &node.id, incoming_edge);
                if let Some(resume_override) = context_store
                    .get("internal.resume.fidelity_override_once")?
                    .and_then(|value| value.as_str().map(ToOwned::to_owned))
                {
                    effective_fidelity = resume_override;
                }
                context_store.set(
                    "internal.fidelity.mode",
                    Value::String(effective_fidelity.clone()),
                )?;
                context_store.set("fidelity", Value::String(effective_fidelity.clone()))?;
                if effective_fidelity == "full" {
                    let thread_key = resolve_thread_key(
                        graph,
                        &node.id,
                        incoming_edge,
                        previous_node_id.as_deref(),
                    );
                    if let Some(thread_key) = thread_key {
                        context_store.set("thread_key", Value::String(thread_key.clone()))?;
                        context_store
                            .set("internal.fidelity.thread_key", Value::String(thread_key))?;
                    } else {
                        context_store.remove("thread_key")?;
                        context_store.remove("internal.fidelity.thread_key")?;
                    }
                } else {
                    context_store.remove("thread_key")?;
                    context_store.remove("internal.fidelity.thread_key")?;
                }

                if is_terminal_node(node) {
                    if let Some(failed_gate_id) = first_unsatisfied_goal_gate(graph, &node_outcomes)
                    {
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
                if is_interview_node(node) {
                    emit_runtime_event(
                        &event_sink,
                        &mut event_sequence_no,
                        RuntimeEventKind::Interview(InterviewEvent::Started {
                            run_id: active_run_id.clone(),
                            node_id: node.id.clone(),
                        }),
                    );
                }
                emit_parallel_start_events(
                    &event_sink,
                    &mut event_sequence_no,
                    &active_run_id,
                    node,
                    graph,
                );
                let context_snapshot = context_store.snapshot()?;
                let (outcome, attempts_used) = execute_with_retry(
                    node,
                    graph,
                    &context_snapshot.values,
                    &*config.executor,
                    &retry_policy,
                    &mut storage,
                    &active_run_id,
                    &event_sink,
                    &mut event_sequence_no,
                )
                .await?;
                emit_parallel_completion_events(
                    &event_sink,
                    &mut event_sequence_no,
                    &active_run_id,
                    node,
                    &outcome,
                );
                emit_interview_completion_event(
                    &event_sink,
                    &mut event_sequence_no,
                    &active_run_id,
                    node,
                    &outcome,
                );

                completed_nodes.push(node.id.clone());
                node_outcomes.insert(node.id.clone(), outcome.clone());
                let retries_used = attempts_used.saturating_sub(1);
                node_retry_counts.insert(node.id.clone(), retries_used);
                context_store.set(
                    format!("internal.retry_count.{}", node.id),
                    Value::Number(serde_json::Number::from(retries_used as u64)),
                )?;
                apply_outcome_to_context(&context_store, &outcome)?;

                let route_decision = decide_route_after_outcome(
                    graph,
                    node,
                    &outcome,
                    &context_store.snapshot()?.values,
                );
                let checkpoint_terminal_status = match &route_decision {
                    RouteDecision::TerminateSuccess => Some("success".to_string()),
                    RouteDecision::TerminateFail(_) => Some("fail".to_string()),
                    RouteDecision::Next { .. } => None,
                };
                let checkpoint_terminal_failure_reason = match &route_decision {
                    RouteDecision::TerminateFail(reason) => Some(reason.clone()),
                    _ => None,
                };
                let checkpoint_next_node = match &route_decision {
                    RouteDecision::Next { node_id, .. } => Some(node_id.clone()),
                    _ => None,
                };
                if let Some(path) = checkpoint_path.as_ref() {
                    let context_snapshot = context_store.snapshot()?;
                    let checkpoint = CheckpointState {
                        metadata: CheckpointMetadata {
                            schema_version: 1,
                            run_id: active_run_id.clone(),
                            checkpoint_id: format!("cp-{}", completed_nodes.len()),
                            sequence_no: completed_nodes.len() as u64,
                            timestamp: timestamp_now(),
                        },
                        current_node: node.id.clone(),
                        next_node: checkpoint_next_node.clone(),
                        completed_nodes: completed_nodes.clone(),
                        node_retries: node_retry_counts.clone(),
                        node_outcomes: node_outcomes
                            .iter()
                            .map(|(node_id, node_outcome)| {
                                (
                                    node_id.clone(),
                                    CheckpointNodeOutcome::from_runtime(node_outcome),
                                )
                            })
                            .collect(),
                        context_values: context_snapshot.values.clone(),
                        logs: context_snapshot.logs,
                        current_node_fidelity: Some(effective_fidelity.clone()),
                        terminal_status: checkpoint_terminal_status.clone(),
                        terminal_failure_reason: checkpoint_terminal_failure_reason.clone(),
                        graph_dot_source_hash: graph_metadata.dot_source_hash.clone(),
                        graph_dot_source_ref: graph_metadata.dot_source_ref.clone(),
                        graph_snapshot_hash: graph_metadata.graph_snapshot_hash.clone(),
                        graph_snapshot_ref: graph_metadata.graph_snapshot_ref.clone(),
                    };
                    checkpoint.save_to_path(path)?;
                    emit_runtime_event(
                        &event_sink,
                        &mut event_sequence_no,
                        RuntimeEventKind::Checkpoint(CheckpointEvent::Saved {
                            run_id: active_run_id.clone(),
                            node_id: node.id.clone(),
                            checkpoint_id: checkpoint.metadata.checkpoint_id.clone(),
                        }),
                    );
                }

                storage
                    .append_checkpoint_event(
                        &node.id,
                        &stage_attempt_id(node, attempts_used),
                        json!({
                            "current_node_id": node.id,
                            "next_node_id": checkpoint_next_node,
                            "completed_nodes_count": completed_nodes.len(),
                            "context_keys_count": context_store.snapshot()?.values.len(),
                            "retry_counter_count": node_retry_counts.len(),
                            "dot_source_hash": graph_metadata.dot_source_hash,
                            "dot_source_ref": graph_metadata.dot_source_ref,
                            "graph_snapshot_hash": graph_metadata.graph_snapshot_hash,
                            "graph_snapshot_ref": graph_metadata.graph_snapshot_ref,
                        }),
                    )
                    .await?;

                if resume_fidelity_degrade_pending {
                    resume_fidelity_degrade_pending = false;
                    apply_resume_fidelity_override(&context_store, false)?;
                }

                match route_decision {
                    RouteDecision::Next {
                        node_id,
                        loop_restart,
                    } => {
                        if loop_restart {
                            restart_target = Some(node_id);
                            break;
                        }
                        current_node_id = node_id;
                    }
                    RouteDecision::TerminateSuccess => break,
                    RouteDecision::TerminateFail(reason) => {
                        terminal_failure = Some(reason);
                        break;
                    }
                }
            }

            let status = if restart_target.is_some() {
                PipelineStatus::Success
            } else {
                forced_terminal_status.unwrap_or_else(|| {
                    if terminal_failure.is_some() {
                        PipelineStatus::Fail
                    } else {
                        PipelineStatus::Success
                    }
                })
            };

            storage
                .append_run_event(
                    "run_finalized",
                    json!({
                        "graph_id": graph.id,
                        "status": if restart_target.is_some() {
                            "restarted"
                        } else {
                            match status {
                                PipelineStatus::Success => "success",
                                PipelineStatus::Fail => "fail",
                            }
                        },
                        "lineage_root_run_id": lineage_root_run_id,
                        "lineage_attempt": lineage_attempt,
                        "restart_target": restart_target,
                    }),
                )
                .await?;
            match (restart_target.as_ref(), status, terminal_failure.as_ref()) {
                (Some(_), _, _) => {}
                (None, PipelineStatus::Success, _) => emit_runtime_event(
                    &event_sink,
                    &mut event_sequence_no,
                    RuntimeEventKind::Pipeline(PipelineEvent::Completed {
                        run_id: active_run_id.clone(),
                        graph_id: graph.id.clone(),
                        lineage_attempt,
                    }),
                ),
                (None, PipelineStatus::Fail, Some(reason)) => emit_runtime_event(
                    &event_sink,
                    &mut event_sequence_no,
                    RuntimeEventKind::Pipeline(PipelineEvent::Failed {
                        run_id: active_run_id.clone(),
                        graph_id: graph.id.clone(),
                        lineage_attempt,
                        reason: reason.clone(),
                    }),
                ),
                (None, PipelineStatus::Fail, None) => emit_runtime_event(
                    &event_sink,
                    &mut event_sequence_no,
                    RuntimeEventKind::Pipeline(PipelineEvent::Failed {
                        run_id: active_run_id.clone(),
                        graph_id: graph.id.clone(),
                        lineage_attempt,
                        reason: "pipeline failed".to_string(),
                    }),
                ),
            }
            storage_writer = storage.take_writer();

            if let Some(target) = restart_target {
                if lineage_attempt >= config.max_loop_restarts {
                    return Err(AttractorError::Runtime(format!(
                        "loop_restart exceeded max_loop_restarts={}",
                        config.max_loop_restarts
                    )));
                }
                lineage_attempt += 1;
                restart_start_node = Some(target);
                resume_path_for_attempt = None;
                continue;
            }

            return Ok(PipelineRunResult {
                run_id: active_run_id,
                status,
                failure_reason: terminal_failure,
                completed_nodes,
                node_outcomes,
                context: context_store.snapshot()?.values,
            });
        }
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

enum RouteDecision {
    Next { node_id: String, loop_restart: bool },
    TerminateSuccess,
    TerminateFail(String),
}

fn decide_route_after_outcome(
    graph: &Graph,
    node: &Node,
    outcome: &NodeOutcome,
    context: &RuntimeContext,
) -> RouteDecision {
    if outcome.status == NodeStatus::Fail {
        if let Some(edge) = select_fail_edge(graph, &node.id) {
            return RouteDecision::Next {
                node_id: edge.to.clone(),
                loop_restart: edge.attrs.get_bool("loop_restart") == Some(true),
            };
        }
        if let Some(target) = resolve_node_failure_target(graph, node) {
            return RouteDecision::Next {
                node_id: target,
                loop_restart: false,
            };
        }
        return RouteDecision::TerminateFail(
            outcome
                .notes
                .clone()
                .unwrap_or_else(|| "stage failed with no routing target".to_string()),
        );
    }

    let Some(next_edge) = select_next_edge(graph, &node.id, outcome, context) else {
        return RouteDecision::TerminateSuccess;
    };
    RouteDecision::Next {
        node_id: next_edge.to.clone(),
        loop_restart: next_edge.attrs.get_bool("loop_restart") == Some(true),
    }
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

fn apply_outcome_to_context(
    context: &ContextStore,
    outcome: &NodeOutcome,
) -> Result<(), AttractorError> {
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

fn prepare_attempt_logs_root(
    base_logs_root: Option<&PathBuf>,
    lineage_attempt: u32,
) -> Result<Option<PathBuf>, AttractorError> {
    let Some(base) = base_logs_root else {
        return Ok(None);
    };
    let path = if lineage_attempt <= 1 {
        base.clone()
    } else {
        base.join(format!("attempt-{lineage_attempt}"))
    };
    fs::create_dir_all(path.join("artifacts")).map_err(|error| {
        AttractorError::Runtime(format!(
            "failed to prepare attempt logs root '{}': {}",
            path.display(),
            error
        ))
    })?;
    Ok(Some(path))
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
    event_sink: &RuntimeEventSink,
    event_sequence_no: &mut u64,
) -> Result<(NodeOutcome, u32), AttractorError> {
    for attempt in 1..=retry_policy.max_attempts {
        let stage_attempt_id = stage_attempt_id(node, attempt);
        let mut attempt_context = context.clone();
        attempt_context.insert(
            "stage_attempt_id".to_string(),
            Value::String(stage_attempt_id.clone()),
        );
        emit_runtime_event(
            event_sink,
            event_sequence_no,
            RuntimeEventKind::Stage(StageEvent::Started {
                run_id: run_id.to_string(),
                node_id: node.id.clone(),
                stage_attempt_id: stage_attempt_id.clone(),
                attempt,
            }),
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
        let will_retry = should_retry_outcome(&outcome) && attempt < retry_policy.max_attempts;
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
            emit_runtime_event(
                event_sink,
                event_sequence_no,
                RuntimeEventKind::Stage(StageEvent::Completed {
                    run_id: run_id.to_string(),
                    node_id: node.id.clone(),
                    stage_attempt_id: stage_attempt_id.clone(),
                    attempt,
                    status: outcome.status.as_str().to_string(),
                    notes: outcome.notes.clone(),
                }),
            );
        } else {
            emit_runtime_event(
                event_sink,
                event_sequence_no,
                RuntimeEventKind::Stage(StageEvent::Failed {
                    run_id: run_id.to_string(),
                    node_id: node.id.clone(),
                    stage_attempt_id: stage_attempt_id.clone(),
                    attempt,
                    status: outcome.status.as_str().to_string(),
                    notes: outcome.notes.clone(),
                    will_retry,
                }),
            );
        }

        if outcome.status.is_success_like() {
            return Ok((outcome, attempt));
        }

        if will_retry {
            let delay_ms = delay_for_attempt_ms(
                attempt,
                &retry_policy.backoff,
                hash_run_node(run_id, &node.id),
            );
            emit_runtime_event(
                event_sink,
                event_sequence_no,
                RuntimeEventKind::Stage(StageEvent::Retrying {
                    run_id: run_id.to_string(),
                    node_id: node.id.clone(),
                    stage_attempt_id: stage_attempt_id.clone(),
                    attempt,
                    next_attempt: attempt + 1,
                    delay_ms,
                }),
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

fn emit_runtime_event(sink: &RuntimeEventSink, sequence_no: &mut u64, kind: RuntimeEventKind) {
    if !sink.is_enabled() {
        return;
    }
    *sequence_no += 1;
    sink.emit(RuntimeEvent {
        sequence_no: *sequence_no,
        timestamp: timestamp_now(),
        kind,
    });
}

fn emit_parallel_start_events(
    sink: &RuntimeEventSink,
    sequence_no: &mut u64,
    run_id: &str,
    node: &Node,
    graph: &Graph,
) {
    if !is_parallel_node(node) {
        return;
    }
    let branches: Vec<(String, String)> = graph
        .outgoing_edges(&node.id)
        .map(|edge| {
            let branch_id = edge
                .attrs
                .get_str("label")
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(edge.to.as_str())
                .to_string();
            (branch_id, edge.to.clone())
        })
        .collect();
    emit_runtime_event(
        sink,
        sequence_no,
        RuntimeEventKind::Parallel(ParallelEvent::Started {
            run_id: run_id.to_string(),
            node_id: node.id.clone(),
            branch_count: branches.len(),
        }),
    );
    for (index, (branch_id, target_node)) in branches.into_iter().enumerate() {
        emit_runtime_event(
            sink,
            sequence_no,
            RuntimeEventKind::Parallel(ParallelEvent::BranchStarted {
                run_id: run_id.to_string(),
                node_id: node.id.clone(),
                branch_id,
                branch_index: index,
                target_node,
            }),
        );
    }
}

fn emit_parallel_completion_events(
    sink: &RuntimeEventSink,
    sequence_no: &mut u64,
    run_id: &str,
    node: &Node,
    outcome: &NodeOutcome,
) {
    if !is_parallel_node(node) {
        return;
    }
    let mut success_count = 0usize;
    let mut failure_count = 0usize;

    let results = outcome
        .context_updates
        .get("parallel.results")
        .and_then(Value::as_array);
    if let Some(results) = results {
        for (index, result) in results.iter().enumerate() {
            let Some(result_obj) = result.as_object() else {
                continue;
            };
            let status = result_obj
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            if status == "success" || status == "partial_success" {
                success_count += 1;
            } else if status == "fail" {
                failure_count += 1;
            }
            let branch_id = result_obj
                .get("branch_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let target_node = result_obj
                .get("target_node")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let notes = result_obj
                .get("notes")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            emit_runtime_event(
                sink,
                sequence_no,
                RuntimeEventKind::Parallel(ParallelEvent::BranchCompleted {
                    run_id: run_id.to_string(),
                    node_id: node.id.clone(),
                    branch_id,
                    branch_index: index,
                    target_node,
                    status,
                    notes,
                }),
            );
        }
    }

    emit_runtime_event(
        sink,
        sequence_no,
        RuntimeEventKind::Parallel(ParallelEvent::Completed {
            run_id: run_id.to_string(),
            node_id: node.id.clone(),
            success_count,
            failure_count,
        }),
    );
}

fn emit_interview_completion_event(
    sink: &RuntimeEventSink,
    sequence_no: &mut u64,
    run_id: &str,
    node: &Node,
    outcome: &NodeOutcome,
) {
    if !is_interview_node(node) {
        return;
    }
    if outcome.status == NodeStatus::Retry {
        emit_runtime_event(
            sink,
            sequence_no,
            RuntimeEventKind::Interview(InterviewEvent::Timeout {
                run_id: run_id.to_string(),
                node_id: node.id.clone(),
                default_selected: outcome
                    .context_updates
                    .get("human.gate.selected")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            }),
        );
        return;
    }
    emit_runtime_event(
        sink,
        sequence_no,
        RuntimeEventKind::Interview(InterviewEvent::Completed {
            run_id: run_id.to_string(),
            node_id: node.id.clone(),
            selected: outcome
                .context_updates
                .get("human.gate.selected")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
        }),
    );
}

fn is_parallel_node(node: &Node) -> bool {
    infer_node_handler_type(node) == "parallel"
}

fn is_interview_node(node: &Node) -> bool {
    infer_node_handler_type(node) == "wait.human"
}

fn infer_node_handler_type(node: &Node) -> &'static str {
    if let Some(explicit_type) = node.attrs.get_str("type").map(str::trim) {
        if !explicit_type.is_empty() {
            return match explicit_type {
                "start" => "start",
                "exit" => "exit",
                "wait.human" => "wait.human",
                "conditional" => "conditional",
                "parallel" => "parallel",
                "parallel.fan_in" => "parallel.fan_in",
                "tool" => "tool",
                "stack.manager_loop" => "stack.manager_loop",
                _ => "codergen",
            };
        }
    }

    match node
        .attrs
        .get_str("shape")
        .map(str::trim)
        .unwrap_or("box")
        .to_ascii_lowercase()
        .as_str()
    {
        "mdiamond" => "start",
        "msquare" => "exit",
        "hexagon" => "wait.human",
        "diamond" => "conditional",
        "component" => "parallel",
        "tripleoctagon" => "parallel.fan_in",
        "parallelogram" => "tool",
        "house" => "stack.manager_loop",
        _ => "codergen",
    }
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

    async fn persist_run_graph_metadata(
        &mut self,
        graph: &Graph,
    ) -> Result<PersistedRunGraphMetadata, AttractorError> {
        let Some(writer) = self.writer.as_ref().cloned() else {
            return Ok(PersistedRunGraphMetadata::default());
        };
        let Some(context_id) = self.context_id.as_ref().cloned() else {
            return Ok(PersistedRunGraphMetadata::default());
        };

        let mut metadata = PersistedRunGraphMetadata::default();
        if let Some(dot_source) = graph.source_dot.as_deref() {
            let dot_bytes = dot_source.as_bytes();
            let dot_hash = blake3::hash(dot_bytes).to_hex().to_string();
            let sequence_no = self.next_sequence_no();
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
            let idempotency_key = attractor_idempotency_key(
                &self.run_id,
                "__run__",
                "__run__",
                "dot_source_persisted",
                sequence_no,
            );
            let stored_turn = writer
                .append_dot_source(
                    &context_id,
                    AttractorDotSourceRecord {
                        timestamp: timestamp_now(),
                        dot_source: dot_source.to_string(),
                        content_hash: dot_hash.clone(),
                        size_bytes: dot_bytes.len() as u64,
                        correlation,
                    },
                    idempotency_key,
                )
                .await?;
            metadata.dot_source_hash = Some(dot_hash);
            metadata.dot_source_ref = Some(format!(
                "turnstore://{}/{}",
                stored_turn.context_id, stored_turn.turn_id
            ));
        }

        let snapshot_json = serde_json::to_value(graph).map_err(|error| {
            AttractorError::Runtime(format!(
                "failed to encode normalized graph snapshot: {error}"
            ))
        })?;
        let snapshot_bytes = serde_json::to_vec(&snapshot_json).map_err(|error| {
            AttractorError::Runtime(format!(
                "failed to serialize normalized graph snapshot bytes: {error}"
            ))
        })?;
        let snapshot_hash = blake3::hash(&snapshot_bytes).to_hex().to_string();
        let sequence_no = self.next_sequence_no();
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
        let idempotency_key = attractor_idempotency_key(
            &self.run_id,
            "__run__",
            "__run__",
            "graph_snapshot_persisted",
            sequence_no,
        );
        let stored_turn = writer
            .append_graph_snapshot(
                &context_id,
                AttractorGraphSnapshotRecord {
                    timestamp: timestamp_now(),
                    graph_snapshot: snapshot_json,
                    content_hash: snapshot_hash.clone(),
                    size_bytes: snapshot_bytes.len() as u64,
                    correlation,
                },
                idempotency_key,
            )
            .await?;
        metadata.graph_snapshot_hash = Some(snapshot_hash);
        metadata.graph_snapshot_ref = Some(format!(
            "turnstore://{}/{}",
            stored_turn.context_id, stored_turn.turn_id
        ));
        Ok(metadata)
    }

    fn next_sequence_no(&mut self) -> u64 {
        self.sequence_no += 1;
        self.sequence_no
    }

    fn take_writer(&mut self) -> Option<crate::storage::SharedAttractorStorageWriter> {
        self.writer.take()
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
        AttractorDotSourceRecord, AttractorGraphSnapshotRecord, AttractorStorageWriter,
        CheckpointMetadata, CheckpointNodeOutcome, CheckpointState, NodeExecutor, NodeOutcome,
        NodeStatus, PipelineEvent, RuntimeEventKind, RuntimeEventSink, StageEvent, parse_dot,
        runtime_event_channel, storage::SharedAttractorStorageWriter,
    };
    use async_trait::async_trait;
    use forge_turnstore::{ContextId, StoreContext, StoredTurn, TurnId, TurnStoreError};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex, atomic::AtomicUsize, atomic::Ordering};
    use tempfile::TempDir;

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

        async fn append_dot_source(
            &self,
            context_id: &ContextId,
            _record: AttractorDotSourceRecord,
            _idempotency_key: String,
        ) -> Result<StoredTurn, TurnStoreError> {
            self.events
                .lock()
                .expect("events mutex should lock")
                .push((context_id.clone(), "dot_source_persisted".to_string()));
            Ok(StoredTurn {
                context_id: context_id.clone(),
                turn_id: "4".to_string(),
                parent_turn_id: "0".to_string(),
                depth: 1,
                type_id: "forge.attractor.dot_source".to_string(),
                type_version: 1,
                payload: Vec::new(),
                idempotency_key: None,
                content_hash: None,
            })
        }

        async fn append_graph_snapshot(
            &self,
            context_id: &ContextId,
            _record: AttractorGraphSnapshotRecord,
            _idempotency_key: String,
        ) -> Result<StoredTurn, TurnStoreError> {
            self.events
                .lock()
                .expect("events mutex should lock")
                .push((context_id.clone(), "graph_snapshot_persisted".to_string()));
            Ok(StoredTurn {
                context_id: context_id.clone(),
                turn_id: "5".to_string(),
                parent_turn_id: "0".to_string(),
                depth: 1,
                type_id: "forge.attractor.graph_snapshot".to_string(),
                type_version: 1,
                payload: Vec::new(),
                idempotency_key: None,
                content_hash: None,
            })
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

    #[derive(Default)]
    struct RecordingExecutor {
        calls: Mutex<Vec<(String, RuntimeContext)>>,
    }

    #[async_trait]
    impl NodeExecutor for RecordingExecutor {
        async fn execute(
            &self,
            node: &Node,
            context: &RuntimeContext,
            _graph: &Graph,
        ) -> Result<NodeOutcome, AttractorError> {
            self.calls
                .lock()
                .expect("calls mutex should lock")
                .push((node.id.clone(), context.clone()));
            Ok(NodeOutcome::success())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_resume_from_checkpoint_expected_continuation_without_reexecuting_completed_node() {
        let temp = TempDir::new().expect("temp dir should be created");
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                plan
                review
                exit [shape=Msquare]
                start -> plan -> review -> exit
            }
            "#,
        )
        .expect("graph should parse");
        let checkpoint_path = crate::checkpoint_file_path(temp.path());
        CheckpointState {
            metadata: CheckpointMetadata {
                schema_version: 1,
                run_id: "G-run".to_string(),
                checkpoint_id: "cp-2".to_string(),
                sequence_no: 2,
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
            context_values: BTreeMap::from([
                ("graph.goal".to_string(), json!("ship")),
                ("outcome".to_string(), json!("success")),
            ]),
            logs: vec!["plan completed".to_string()],
            current_node_fidelity: Some("compact".to_string()),
            terminal_status: None,
            terminal_failure_reason: None,
            graph_dot_source_hash: None,
            graph_dot_source_ref: None,
            graph_snapshot_hash: None,
            graph_snapshot_ref: None,
        }
        .save_to_path(&checkpoint_path)
        .expect("checkpoint save should succeed");

        let executor = Arc::new(RecordingExecutor::default());
        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    executor: executor.clone(),
                    logs_root: Some(temp.path().to_path_buf()),
                    resume_from_checkpoint: Some(checkpoint_path),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");

        assert_eq!(result.status, PipelineStatus::Success);
        let calls = executor.calls.lock().expect("calls mutex should lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "review");
        assert!(result.completed_nodes.iter().any(|node| node == "plan"));
        assert!(result.completed_nodes.iter().any(|node| node == "review"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_resume_full_fidelity_expected_degrade_marker_first_hop_only() {
        let temp = TempDir::new().expect("temp dir should be created");
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                review
                synth
                verify
                exit [shape=Msquare]
                start -> review -> synth -> verify -> exit
            }
            "#,
        )
        .expect("graph should parse");
        let checkpoint_path = crate::checkpoint_file_path(temp.path());
        CheckpointState {
            metadata: CheckpointMetadata {
                schema_version: 1,
                run_id: "G-run".to_string(),
                checkpoint_id: "cp-2".to_string(),
                sequence_no: 2,
                timestamp: "1.000Z".to_string(),
            },
            current_node: "review".to_string(),
            next_node: Some("synth".to_string()),
            completed_nodes: vec!["start".to_string(), "review".to_string()],
            node_retries: BTreeMap::new(),
            node_outcomes: BTreeMap::from([(
                "review".to_string(),
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
            graph_dot_source_hash: None,
            graph_dot_source_ref: None,
            graph_snapshot_hash: None,
            graph_snapshot_ref: None,
        }
        .save_to_path(&checkpoint_path)
        .expect("checkpoint save should succeed");

        let executor = Arc::new(RecordingExecutor::default());
        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    executor: executor.clone(),
                    logs_root: Some(temp.path().to_path_buf()),
                    resume_from_checkpoint: Some(checkpoint_path),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");
        assert_eq!(result.status, PipelineStatus::Success);

        let calls = executor.calls.lock().expect("calls mutex should lock");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "synth");
        assert_eq!(calls[1].0, "verify");
        assert_eq!(
            calls[0]
                .1
                .get("internal.resume.fidelity_override_once")
                .and_then(Value::as_str),
            Some("summary:high")
        );
        assert_eq!(
            calls[1].1.get("internal.resume.fidelity_override_once"),
            None
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_fidelity_thread_resolution_expected_deterministic_precedence_and_full_only_threads()
     {
        let graph = parse_dot(
            r#"
            digraph G {
                graph [default_fidelity="summary:medium", thread_id="graph-thread"]
                start [shape=Mdiamond]
                plan [fidelity="summary:low"]
                review [fidelity="truncate", class="review-cluster"]
                exit [shape=Msquare]
                start -> plan [fidelity="full", thread_id="edge-thread"]
                plan -> review
                review -> exit
            }
            "#,
        )
        .expect("graph should parse");

        let executor = Arc::new(RecordingExecutor::default());
        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    executor: executor.clone(),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");
        assert_eq!(result.status, PipelineStatus::Success);

        let calls = executor.calls.lock().expect("calls mutex should lock");
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].0, "start");
        assert_eq!(calls[1].0, "plan");
        assert_eq!(calls[2].0, "review");

        assert_eq!(
            calls[1]
                .1
                .get("internal.fidelity.mode")
                .and_then(Value::as_str),
            Some("full")
        );
        assert_eq!(
            calls[1].1.get("thread_key").and_then(Value::as_str),
            Some("edge-thread")
        );

        assert_eq!(
            calls[2]
                .1
                .get("internal.fidelity.mode")
                .and_then(Value::as_str),
            Some("truncate")
        );
        assert_eq!(calls[2].1.get("thread_key"), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_loop_restart_expected_fresh_lineage_attempt_and_logs_root() {
        let temp = TempDir::new().expect("temp dir should be created");
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                plan
                restart
                exit [shape=Msquare]
                start -> plan
                plan -> restart [loop_restart=true]
                restart -> exit
            }
            "#,
        )
        .expect("graph should parse");

        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    logs_root: Some(temp.path().to_path_buf()),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");

        assert_eq!(result.status, PipelineStatus::Success);
        assert_eq!(result.run_id, "G-run:attempt:2".to_string());
        assert_eq!(result.completed_nodes, vec!["restart".to_string()]);
        assert_eq!(
            result
                .context
                .get("internal.lineage.attempt")
                .and_then(Value::as_u64),
            Some(2)
        );
        assert!(temp.path().join("attempt-2").exists());
        assert!(temp.path().join("attempt-2").join("artifacts").exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_events_stream_expected_pipeline_and_stage_timeline() {
        let graph = parse_dot(
            r#"
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box]
                exit [shape=Msquare]
                start -> plan -> exit
            }
            "#,
        )
        .expect("graph should parse");
        let (tx, mut rx) = runtime_event_channel();

        let result = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    events: RuntimeEventSink::with_sender(tx),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");
        assert_eq!(result.status, PipelineStatus::Success);

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        assert!(!events.is_empty());

        let mut prior = 0u64;
        for event in &events {
            assert!(event.sequence_no > prior);
            prior = event.sequence_no;
        }

        assert!(events.iter().any(|event| {
            matches!(
                event.kind,
                RuntimeEventKind::Pipeline(PipelineEvent::Started { .. })
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event.kind,
                RuntimeEventKind::Pipeline(PipelineEvent::Completed { .. })
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event.kind,
                RuntimeEventKind::Stage(StageEvent::Started { ref node_id, .. }) if node_id == "start"
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event.kind,
                RuntimeEventKind::Stage(StageEvent::Completed { ref node_id, .. }) if node_id == "plan"
            )
        }));
    }
}
