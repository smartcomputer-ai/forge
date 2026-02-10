use async_trait::async_trait;
use forge_attractor::{
    AttractorError, Graph, Node, NodeExecutor, NodeOutcome, PipelineRunner, PipelineStatus,
    RunConfig, RuntimeContext, find_incoming_edge, parse_dot, resolve_fidelity_mode,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

struct CaptureExecutor {
    captures: Mutex<Vec<(String, RuntimeContext)>>,
}

#[async_trait]
impl NodeExecutor for CaptureExecutor {
    async fn execute(
        &self,
        node: &Node,
        context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        self.captures
            .lock()
            .expect("captures mutex should lock")
            .push((node.id.clone(), context.clone()));
        Ok(NodeOutcome::success())
    }
}

#[test]
fn fidelity_resolution_precedence_expected_edge_then_node_then_graph_then_default() {
    let graph = parse_dot(
        r#"
        digraph G {
            graph [default_fidelity="summary:medium"]
            start [shape=Mdiamond]
            plan [fidelity="summary:low"]
            review
            verify
            start -> plan [fidelity="full"]
            plan -> review
            review -> verify
        }
        "#,
    )
    .expect("graph should parse");

    let incoming_plan = find_incoming_edge(&graph, "plan", Some("start"));
    assert_eq!(resolve_fidelity_mode(&graph, "plan", incoming_plan), "full");

    let incoming_review = find_incoming_edge(&graph, "review", Some("plan"));
    assert_eq!(
        resolve_fidelity_mode(&graph, "review", incoming_review),
        "summary:medium"
    );

    let fallback_graph = parse_dot("digraph G { n1 }").expect("fallback graph should parse");
    assert_eq!(
        resolve_fidelity_mode(&fallback_graph, "n1", None),
        "compact"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn fidelity_degrade_on_resume_expected_first_hop_override_then_clear() {
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

    let temp = TempDir::new().expect("temp dir should create");
    let checkpoint_path = temp.path().join("checkpoint.json");
    forge_attractor::CheckpointState {
        metadata: forge_attractor::CheckpointMetadata {
            schema_version: 1,
            run_id: "run-1".to_string(),
            checkpoint_id: "cp-7".to_string(),
            sequence_no: 7,
            timestamp: "1.000Z".to_string(),
        },
        current_node: "review".to_string(),
        next_node: Some("synth".to_string()),
        completed_nodes: vec!["start".to_string(), "review".to_string()],
        node_retries: BTreeMap::new(),
        node_outcomes: BTreeMap::new(),
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
    .expect("checkpoint should save");

    let executor = Arc::new(CaptureExecutor {
        captures: Mutex::new(Vec::new()),
    });
    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                run_id: Some("run-1".to_string()),
                resume_from_checkpoint: Some(checkpoint_path),
                executor: executor.clone(),
                ..RunConfig::default()
            },
        )
        .await
        .expect("resumed run should succeed");
    assert_eq!(result.status, PipelineStatus::Success);

    let captures = executor
        .captures
        .lock()
        .expect("captures mutex should lock");
    assert_eq!(captures[0].0, "synth");
    assert_eq!(
        captures[0]
            .1
            .get("internal.resume.fidelity_override_once")
            .and_then(Value::as_str),
        Some("summary:high")
    );
    assert_eq!(captures[1].0, "verify");
    assert_eq!(
        captures[1].1.get("internal.resume.fidelity_override_once"),
        None
    );
}
