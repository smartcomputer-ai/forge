use async_trait::async_trait;
use forge_attractor::{
    AttractorError, AttractorStorageWriter, CheckpointMetadata, CheckpointNodeOutcome,
    CheckpointState, Graph, Node, NodeExecutor, NodeOutcome, PipelineRunner, PipelineStatus,
    RunConfig, RuntimeContext, parse_dot,
};
use forge_turnstore::{FsTurnStore, MemoryTurnStore, StoredTurn, TurnStore};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

#[derive(Clone)]
enum StoreHarness {
    Memory(Arc<MemoryTurnStore>),
    Fs(Arc<FsTurnStore>),
}

impl StoreHarness {
    fn writer(&self) -> Arc<dyn AttractorStorageWriter> {
        match self {
            Self::Memory(store) => store.clone(),
            Self::Fs(store) => store.clone(),
        }
    }

    async fn list_turns(&self, context_id: &str) -> Vec<StoredTurn> {
        let context_id = context_id.to_string();
        match self {
            Self::Memory(store) => store
                .list_turns(&context_id, None, 200)
                .await
                .expect("memory turns should list"),
            Self::Fs(store) => store
                .list_turns(&context_id, None, 200)
                .await
                .expect("fs turns should list"),
        }
    }
}

struct RecordingExecutor {
    calls: Mutex<Vec<String>>,
}

#[async_trait]
impl NodeExecutor for RecordingExecutor {
    async fn execute(
        &self,
        node: &Node,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, AttractorError> {
        self.calls
            .lock()
            .expect("calls mutex should lock")
            .push(node.id.clone());

        let mut outcome = NodeOutcome::success();
        if node.id == "plan" {
            outcome
                .context_updates
                .insert("context.plan.status".to_string(), json!("done"));
        }
        Ok(outcome)
    }
}

fn graph_under_test() -> Graph {
    parse_dot(
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
    .expect("graph should parse")
}

#[tokio::test(flavor = "current_thread")]
async fn checkpoint_roundtrip_and_resume_parity_memory_and_fs_expected_deterministic_continuation()
{
    for harness in [
        StoreHarness::Memory(Arc::new(MemoryTurnStore::new())),
        StoreHarness::Fs(Arc::new(
            FsTurnStore::new(
                TempDir::new()
                    .expect("temp dir should create")
                    .path()
                    .to_path_buf(),
            )
            .expect("fs turnstore should initialize"),
        )),
    ] {
        let logs_root = TempDir::new().expect("temp dir should create");
        let graph = graph_under_test();

        let first = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    run_id: Some("run-1".to_string()),
                    logs_root: Some(logs_root.path().to_path_buf()),
                    storage: Some(harness.writer()),
                    executor: Arc::new(RecordingExecutor {
                        calls: Mutex::new(Vec::new()),
                    }),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("first run should succeed");
        assert_eq!(first.status, PipelineStatus::Success);
        let checkpoint_path = logs_root.path().join("checkpoint.json");
        assert!(checkpoint_path.exists());
        let checkpoint = CheckpointState::load_from_path(&checkpoint_path)
            .expect("checkpoint should deserialize");
        assert!(checkpoint.graph_dot_source_hash.is_some());
        assert!(checkpoint.graph_dot_source_ref.is_some());
        assert!(checkpoint.graph_snapshot_hash.is_some());
        assert!(checkpoint.graph_snapshot_ref.is_some());

        let manual_checkpoint_path = logs_root.path().join("checkpoint-manual.json");
        CheckpointState {
            metadata: CheckpointMetadata {
                schema_version: 1,
                run_id: "run-1".to_string(),
                checkpoint_id: "cp-manual".to_string(),
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
            context_values: BTreeMap::from([("context.plan.status".to_string(), json!("done"))]),
            logs: vec!["plan completed".to_string()],
            current_node_fidelity: Some("compact".to_string()),
            terminal_status: None,
            terminal_failure_reason: None,
            graph_dot_source_hash: None,
            graph_dot_source_ref: None,
            graph_snapshot_hash: None,
            graph_snapshot_ref: None,
        }
        .save_to_path(&manual_checkpoint_path)
        .expect("manual checkpoint should save");

        let recorder = Arc::new(RecordingExecutor {
            calls: Mutex::new(Vec::new()),
        });
        let resumed = PipelineRunner
            .run(
                &graph,
                RunConfig {
                    run_id: Some("run-1".to_string()),
                    logs_root: Some(logs_root.path().to_path_buf()),
                    resume_from_checkpoint: Some(manual_checkpoint_path),
                    storage: Some(harness.writer()),
                    executor: recorder.clone(),
                    ..RunConfig::default()
                },
            )
            .await
            .expect("resumed run should succeed");

        assert_eq!(resumed.status, PipelineStatus::Success);
        assert_eq!(
            recorder
                .calls
                .lock()
                .expect("calls mutex should lock")
                .as_slice(),
            ["review"]
        );
        assert_eq!(
            resumed.context.get("context.plan.status"),
            Some(&Value::String("done".to_string()))
        );

        let first_context_turns = harness.list_turns("1").await;
        let second_context_turns = harness.list_turns("2").await;
        assert!(!first_context_turns.is_empty());
        assert!(!second_context_turns.is_empty());
    }
}
