use async_trait::async_trait;
use forge_attractor::{
    AttractorCheckpointEventRecord, AttractorDotSourceRecord, AttractorGraphSnapshotRecord,
    AttractorRunEventRecord, AttractorStageEventRecord, AttractorStageToAgentLinkRecord,
    AttractorStorageWriter, ContextId, CxdbPersistenceMode, Graph, Node, NodeExecutor, NodeOutcome,
    NodeStatus, PipelineRunner, PipelineStatus, RunConfig, RuntimeContext, StoreContext,
    StoredTurn, TurnId, TurnStoreError, parse_dot,
};
use forge_cxdb_runtime::{CxdbRuntimeStore, MockCxdb};
use std::sync::{Arc, Mutex, atomic::AtomicUsize, atomic::Ordering};

#[derive(Default)]
struct RecordingStorage {
    events: Mutex<Vec<String>>,
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
        _context_id: &ContextId,
        record: AttractorRunEventRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        self.events.lock().expect("mutex").push(record.event_kind);
        Ok(stub_turn("forge.attractor.run_event"))
    }

    async fn append_stage_event(
        &self,
        _context_id: &ContextId,
        record: AttractorStageEventRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        self.events.lock().expect("mutex").push(record.event_kind);
        Ok(stub_turn("forge.attractor.stage_event"))
    }

    async fn append_checkpoint_event(
        &self,
        _context_id: &ContextId,
        _record: AttractorCheckpointEventRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        self.events
            .lock()
            .expect("mutex")
            .push("checkpoint_saved".to_string());
        Ok(stub_turn("forge.attractor.checkpoint_event"))
    }

    async fn append_stage_to_agent_link(
        &self,
        _context_id: &ContextId,
        _record: AttractorStageToAgentLinkRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        Err(TurnStoreError::Unsupported("unused".to_string()))
    }

    async fn append_dot_source(
        &self,
        _context_id: &ContextId,
        record: AttractorDotSourceRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        self.events
            .lock()
            .expect("mutex")
            .push(format!("dot_source:{}", record.content_hash));
        Ok(stub_turn("forge.attractor.dot_source"))
    }

    async fn append_graph_snapshot(
        &self,
        _context_id: &ContextId,
        record: AttractorGraphSnapshotRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        self.events
            .lock()
            .expect("mutex")
            .push(format!("graph_snapshot:{}", record.content_hash));
        Ok(stub_turn("forge.attractor.graph_snapshot"))
    }
}

fn stub_turn(type_id: &str) -> StoredTurn {
    StoredTurn {
        context_id: "ctx-1".to_string(),
        turn_id: "1".to_string(),
        parent_turn_id: "0".to_string(),
        depth: 1,
        type_id: type_id.to_string(),
        type_version: 1,
        payload: Vec::new(),
        idempotency_key: None,
        content_hash: None,
    }
}

fn parse(source: &str) -> Graph {
    parse_dot(source).expect("graph should parse")
}

struct PreferredNoExecutor;

#[async_trait]
impl NodeExecutor for PreferredNoExecutor {
    async fn execute(
        &self,
        node: &Node,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, forge_attractor::AttractorError> {
        if node.id == "gate" {
            return Ok(NodeOutcome {
                status: NodeStatus::Success,
                notes: None,
                context_updates: RuntimeContext::new(),
                preferred_label: Some("No".to_string()),
                suggested_next_ids: vec![],
            });
        }
        Ok(NodeOutcome::success())
    }
}

struct RetryOnceExecutor {
    calls: AtomicUsize,
}

#[async_trait]
impl NodeExecutor for RetryOnceExecutor {
    async fn execute(
        &self,
        node: &Node,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, forge_attractor::AttractorError> {
        if node.id == "work" {
            let attempt = self.calls.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                return Ok(NodeOutcome {
                    status: NodeStatus::Retry,
                    notes: Some("retry".to_string()),
                    context_updates: RuntimeContext::new(),
                    preferred_label: None,
                    suggested_next_ids: vec![],
                });
            }
        }
        Ok(NodeOutcome::success())
    }
}

struct GoalGateExecutor {
    work_calls: AtomicUsize,
}

#[async_trait]
impl NodeExecutor for GoalGateExecutor {
    async fn execute(
        &self,
        node: &Node,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, forge_attractor::AttractorError> {
        if node.id == "work" {
            let attempt = self.work_calls.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                return Ok(NodeOutcome::failure("goal not met"));
            }
        }
        Ok(NodeOutcome::success())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn execution_linear_store_off_and_on_expected_equivalent_status() {
    let graph = parse(
        r#"
        digraph G {
            start [shape=Mdiamond]
            plan [shape=box, prompt="Plan"]
            exit [shape=Msquare]
            start -> plan -> exit
        }
        "#,
    );
    let runner = PipelineRunner;
    let off = runner
        .run(&graph, RunConfig::default())
        .await
        .expect("run should succeed");

    let storage = Arc::new(RecordingStorage::default());
    let on = runner
        .run(
            &graph,
            RunConfig {
                storage: Some(storage.clone()),
                cxdb_persistence: CxdbPersistenceMode::Required,
                ..RunConfig::default()
            },
        )
        .await
        .expect("run should succeed");

    assert_eq!(off.status, on.status);
    assert_eq!(off.completed_nodes, on.completed_nodes);
    let events = storage.events.lock().expect("mutex");
    assert!(events.iter().any(|kind| kind.starts_with("dot_source:")));
    assert!(
        events
            .iter()
            .any(|kind| kind.starts_with("graph_snapshot:"))
    );
    assert!(events.iter().any(|kind| kind == "run_initialized"));
    assert!(events.iter().any(|kind| kind == "run_finalized"));
}

#[tokio::test(flavor = "current_thread")]
async fn execution_store_enabled_memory_turnstore_expected_persisted_turns() {
    let graph = parse(
        r#"
        digraph G {
            start [shape=Mdiamond]
            plan [shape=box, prompt="Plan"]
            exit [shape=Msquare]
            start -> plan -> exit
        }
        "#,
    );
    let backend = Arc::new(MockCxdb::default());
    let store = Arc::new(CxdbRuntimeStore::new(backend.clone(), backend.clone()));

    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                storage: Some(store.clone()),
                cxdb_persistence: CxdbPersistenceMode::Required,
                ..RunConfig::default()
            },
        )
        .await
        .expect("run should succeed");

    assert_eq!(result.status, PipelineStatus::Success);
    let turns = store
        .list_turns(&"1".to_string(), None, 128)
        .await
        .expect("turns should be queryable");
    assert!(!turns.is_empty());
    assert!(
        turns
            .iter()
            .any(|turn| turn.type_id == "forge.attractor.run_event")
    );
    assert!(
        turns
            .iter()
            .any(|turn| turn.type_id == "forge.attractor.dot_source")
    );
    assert!(
        turns
            .iter()
            .any(|turn| turn.type_id == "forge.attractor.graph_snapshot")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn execution_branching_preferred_label_expected_no_branch() {
    let graph = parse(
        r#"
        digraph G {
            start [shape=Mdiamond]
            gate [shape=diamond]
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
    );

    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                executor: Arc::new(PreferredNoExecutor),
                ..RunConfig::default()
            },
        )
        .await
        .expect("run should succeed");

    assert_eq!(result.status, PipelineStatus::Success);
    assert!(result.completed_nodes.iter().any(|n| n == "no"));
    assert!(!result.completed_nodes.iter().any(|n| n == "yes"));
}

#[tokio::test(flavor = "current_thread")]
async fn execution_retry_then_success_expected_attempts_observed() {
    let graph = parse(
        r#"
        digraph G {
            start [shape=Mdiamond]
            work [max_retries=1]
            exit [shape=Msquare]
            start -> work -> exit
        }
        "#,
    );

    let executor = Arc::new(RetryOnceExecutor {
        calls: AtomicUsize::new(0),
    });
    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                executor: executor.clone(),
                retry_backoff: forge_attractor::RetryBackoffConfig {
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
async fn execution_goal_gate_retry_target_expected_recovery_before_exit() {
    let graph = parse(
        r#"
        digraph G {
            start [shape=Mdiamond]
            work [goal_gate=true, retry_target="fix"]
            fix
            exit [shape=Msquare]
            start -> work -> exit
            work -> fix [condition="outcome=fail"]
            fix -> work
        }
        "#,
    );

    let executor = Arc::new(GoalGateExecutor {
        work_calls: AtomicUsize::new(0),
    });
    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                executor,
                ..RunConfig::default()
            },
        )
        .await
        .expect("run should succeed");

    assert_eq!(result.status, PipelineStatus::Success);
    assert!(result.completed_nodes.iter().any(|n| n == "fix"));
}
