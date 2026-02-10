use async_trait::async_trait;
use forge_attractor::handlers::codergen::{CodergenBackend, CodergenBackendResult};
use forge_attractor::handlers::registry::RegistryNodeExecutor;
use forge_attractor::handlers::wait_human::{HumanAnswer, QueueInterviewer, WaitHumanHandler};
use forge_attractor::{
    AttractorError, AttractorStorageWriter, Graph, Node, NodeOutcome, NodeStatus, PipelineRunner,
    PipelineStatus, RunConfig, RuntimeContext, parse_dot,
};
use forge_turnstore::{FsTurnStore, MemoryTurnStore};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[derive(Clone)]
enum StorageHarness {
    Memory(Arc<MemoryTurnStore>),
    Fs(Arc<FsTurnStore>),
}

impl StorageHarness {
    fn writer(&self) -> Arc<dyn AttractorStorageWriter> {
        match self {
            Self::Memory(store) => store.clone(),
            Self::Fs(store) => store.clone(),
        }
    }
}

#[derive(Default)]
struct MockCodergenBackend {
    implement_attempts: Mutex<usize>,
}

#[async_trait]
impl CodergenBackend for MockCodergenBackend {
    async fn run(
        &self,
        node: &Node,
        _prompt: &str,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<CodergenBackendResult, AttractorError> {
        if node.id == "implement" {
            let mut attempts = self.implement_attempts.lock().expect("mutex");
            if *attempts == 0 {
                *attempts += 1;
                return Ok(CodergenBackendResult::Outcome(NodeOutcome {
                    status: NodeStatus::Retry,
                    notes: Some("retry implement".to_string()),
                    context_updates: RuntimeContext::new(),
                    preferred_label: None,
                    suggested_next_ids: vec![],
                }));
            }
        }

        let mut updates = RuntimeContext::new();
        updates.insert(format!("context.stage.{}", node.id), json!("ok"));
        Ok(CodergenBackendResult::Outcome(NodeOutcome {
            status: NodeStatus::Success,
            notes: Some(format!("mock completed {}", node.id)),
            context_updates: updates,
            preferred_label: None,
            suggested_next_ids: vec![],
        }))
    }
}

fn spec_like_graph() -> forge_attractor::Graph {
    parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            gate [shape=hexagon]
            plan [shape=box, prompt="Plan $goal"]
            implement [shape=box, prompt="Implement", goal_gate=true, max_retries=1]
            review [shape=box, prompt="Review"]
            exit [shape=Msquare]

            start -> gate
            gate -> plan [label="[R] Revise"]
            gate -> plan [label="[A] Approve"]
            plan -> implement
            implement -> review [condition="outcome=success"]
            implement -> plan [condition="outcome=fail", label="Retry"]
            review -> exit [condition="outcome=success"]
            review -> implement [condition="outcome=fail", label="Fix"]
        }
        "#,
    )
    .expect("graph should parse")
}

#[tokio::test(flavor = "current_thread")]
async fn conformance_runtime_memory_and_fs_expected_routing_retry_goal_gate_and_hitl() {
    let fs_temp = TempDir::new().expect("tempdir should create");
    let harnesses = vec![
        StorageHarness::Memory(Arc::new(MemoryTurnStore::new())),
        StorageHarness::Fs(Arc::new(
            FsTurnStore::new(fs_temp.path()).expect("fs store should init"),
        )),
    ];

    for harness in harnesses {
        let logs_root = TempDir::new().expect("tempdir should create");
        let backend = Arc::new(MockCodergenBackend::default());
        let interviewer = Arc::new(QueueInterviewer::with_answers(vec![HumanAnswer::Selected(
            "A".to_string(),
        )]));

        let mut registry =
            forge_attractor::handlers::core_registry_with_codergen_backend(Some(backend));
        registry.register_type("wait.human", Arc::new(WaitHumanHandler::new(interviewer)));

        let result = PipelineRunner
            .run(
                &spec_like_graph(),
                RunConfig {
                    run_id: Some("conformance-runtime".to_string()),
                    logs_root: Some(logs_root.path().to_path_buf()),
                    storage: Some(harness.writer()),
                    executor: Arc::new(RegistryNodeExecutor::new(registry)),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("run should succeed");

        assert_eq!(result.status, PipelineStatus::Success);
        assert!(
            result
                .completed_nodes
                .iter()
                .any(|node| node == "implement")
        );
        assert_eq!(
            result.context.get("context.stage.review"),
            Some(&json!("ok"))
        );

        assert!(logs_root.path().join("plan").join("prompt.md").exists());
        assert!(
            logs_root
                .path()
                .join("implement")
                .join("response.md")
                .exists()
        );
        assert!(logs_root.path().join("review").join("status.json").exists());
    }
}
