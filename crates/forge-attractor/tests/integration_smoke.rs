use async_trait::async_trait;
use forge_attractor::handlers::codergen::{CodergenBackend, CodergenBackendResult};
use forge_attractor::handlers::registry::RegistryNodeExecutor;
use forge_attractor::{
    AttractorError, Graph, Node, NodeOutcome, NodeStatus, PipelineRunner, PipelineStatus,
    RunConfig, RuntimeContext, parse_dot,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[derive(Default)]
struct SpecSmokeBackend {
    implement_calls: Mutex<usize>,
}

#[async_trait]
impl CodergenBackend for SpecSmokeBackend {
    async fn run(
        &self,
        node: &Node,
        _prompt: &str,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<CodergenBackendResult, AttractorError> {
        let mut updates = RuntimeContext::new();
        updates.insert(format!("context.completed.{}", node.id), json!(true));

        if node.id == "implement" {
            let mut calls = self.implement_calls.lock().expect("mutex");
            if *calls == 0 {
                *calls += 1;
                return Ok(CodergenBackendResult::Outcome(NodeOutcome {
                    status: NodeStatus::Fail,
                    notes: Some("mock fail once".to_string()),
                    context_updates: updates,
                    preferred_label: None,
                    suggested_next_ids: vec![],
                }));
            }
        }

        Ok(CodergenBackendResult::Outcome(NodeOutcome {
            status: NodeStatus::Success,
            notes: Some(format!("mock success {}", node.id)),
            context_updates: updates,
            preferred_label: None,
            suggested_next_ids: vec![],
        }))
    }
}

fn smoke_graph() -> forge_attractor::Graph {
    parse_dot(
        r#"
        digraph test_pipeline {
            graph [goal="Create a hello world Python script"]

            start       [shape=Mdiamond]
            plan        [shape=box, prompt="Plan how to create a hello world script for: $goal"]
            implement   [shape=box, prompt="Write the code based on the plan", goal_gate=true]
            review      [shape=box, prompt="Review the code for correctness"]
            done        [shape=Msquare]

            start -> plan
            plan -> implement
            implement -> review [condition="outcome=success"]
            implement -> plan   [condition="outcome=fail", label="Retry"]
            review -> done      [condition="outcome=success"]
            review -> implement [condition="outcome=fail", label="Fix"]
        }
        "#,
    )
    .expect("graph should parse")
}

#[tokio::test(flavor = "current_thread")]
async fn integration_smoke_spec_style_expected_success_reroute_goal_gate_and_artifacts() {
    let backend = Arc::new(SpecSmokeBackend::default());
    let registry = forge_attractor::handlers::core_registry_with_codergen_backend(Some(backend));
    let logs_root = TempDir::new().expect("tempdir should create");

    let result = PipelineRunner
        .run(
            &smoke_graph(),
            RunConfig {
                run_id: Some("smoke-run".to_string()),
                logs_root: Some(logs_root.path().to_path_buf()),
                executor: Arc::new(RegistryNodeExecutor::new(registry)),
                ..RunConfig::default()
            },
        )
        .await
        .expect("run should succeed");

    assert_eq!(result.status, PipelineStatus::Success);
    assert!(result.completed_nodes.iter().any(|id| id == "implement"));
    assert!(result.completed_nodes.iter().any(|id| id == "review"));
    assert!(
        result
            .completed_nodes
            .iter()
            .filter(|id| id.as_str() == "plan")
            .count()
            >= 2,
        "fail path should route implement -> plan at least once"
    );

    assert!(logs_root.path().join("plan").join("prompt.md").exists());
    assert!(logs_root.path().join("plan").join("response.md").exists());
    assert!(logs_root.path().join("plan").join("status.json").exists());
    assert!(
        logs_root
            .path()
            .join("implement")
            .join("prompt.md")
            .exists()
    );
    assert!(
        logs_root
            .path()
            .join("implement")
            .join("response.md")
            .exists()
    );
    assert!(
        logs_root
            .path()
            .join("implement")
            .join("status.json")
            .exists()
    );
    assert!(logs_root.path().join("review").join("prompt.md").exists());
    assert!(logs_root.path().join("review").join("response.md").exists());
    assert!(logs_root.path().join("review").join("status.json").exists());

    assert_eq!(
        result.context.get("context.completed.implement"),
        Some(&json!(true))
    );
    assert_eq!(
        result.context.get("context.completed.review"),
        Some(&json!(true))
    );

    let checkpoint =
        forge_attractor::CheckpointState::load_from_path(&logs_root.path().join("checkpoint.json"))
            .expect("checkpoint should load");
    assert_eq!(checkpoint.next_node.as_deref(), Some("done"));
    assert_eq!(checkpoint.terminal_status, None);
    assert!(checkpoint.completed_nodes.iter().any(|id| id == "plan"));
    assert!(
        checkpoint
            .completed_nodes
            .iter()
            .any(|id| id == "implement")
    );
    assert!(checkpoint.completed_nodes.iter().any(|id| id == "review"));
}
