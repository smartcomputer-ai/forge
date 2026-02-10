use async_trait::async_trait;
use forge_attractor::{
    AttractorCheckpointEventRecord, AttractorDotSourceRecord, AttractorGraphSnapshotRecord,
    AttractorRunEventRecord, AttractorStageEventRecord, AttractorStageToAgentLinkRecord,
    AttractorStorageWriter, CxdbPersistenceMode, Graph, Node, NodeExecutor, NodeOutcome,
    PipelineRunner, PipelineStatus, RunConfig, RuntimeContext, parse_dot,
};
use forge_turnstore::{ContextId, FsTurnStore, StoreContext, StoredTurn, TurnId, TurnStoreError};
use forge_turnstore_cxdb::{
    BinaryAppendTurnRequest, BinaryAppendTurnResponse, BinaryContextHead, BinaryStoredTurn,
    CxdbBinaryClient, CxdbClientError, CxdbHttpClient, CxdbTurnStore, HttpStoredTurn, MockCxdb,
};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

fn graph_under_test() -> Graph {
    parse_dot(
        r#"
        digraph G {
            start [shape=Mdiamond]
            plan [shape=box, prompt="Plan"]
            exit [shape=Msquare]
            start -> plan -> exit
        }
        "#,
    )
    .expect("graph should parse")
}

#[derive(Default)]
struct AlwaysSuccessExecutor;

#[async_trait]
impl NodeExecutor for AlwaysSuccessExecutor {
    async fn execute(
        &self,
        _node: &Node,
        _context: &RuntimeContext,
        _graph: &Graph,
    ) -> Result<NodeOutcome, forge_attractor::AttractorError> {
        Ok(NodeOutcome::success())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cxdb_memory_fs_parity_expected_equivalent_status_and_nodes() {
    let graph = graph_under_test();

    let memory_store = Arc::new(forge_turnstore::MemoryTurnStore::new());
    let memory = PipelineRunner
        .run(
            &graph,
            RunConfig {
                storage: Some(memory_store),
                executor: Arc::new(AlwaysSuccessExecutor),
                ..RunConfig::default()
            },
        )
        .await
        .expect("memory run should succeed");

    let tmp = tempdir().expect("tempdir should create");
    let fs_store = Arc::new(FsTurnStore::new(tmp.path()).expect("fs store should create"));
    let fs = PipelineRunner
        .run(
            &graph,
            RunConfig {
                storage: Some(fs_store),
                executor: Arc::new(AlwaysSuccessExecutor),
                ..RunConfig::default()
            },
        )
        .await
        .expect("fs run should succeed");

    let cxdb_backend = Arc::new(MockCxdb::default());
    let cxdb_store = Arc::new(CxdbTurnStore::new(cxdb_backend.clone(), cxdb_backend));
    let cxdb = PipelineRunner
        .run(
            &graph,
            RunConfig {
                storage: Some(cxdb_store),
                executor: Arc::new(AlwaysSuccessExecutor),
                ..RunConfig::default()
            },
        )
        .await
        .expect("cxdb run should succeed");

    assert_eq!(memory.status, PipelineStatus::Success);
    assert_eq!(memory.status, fs.status);
    assert_eq!(memory.status, cxdb.status);
    assert_eq!(memory.completed_nodes, fs.completed_nodes);
    assert_eq!(memory.completed_nodes, cxdb.completed_nodes);
}

#[derive(Default)]
struct FailingStorageWriter {
    calls: Mutex<u64>,
}

#[async_trait]
impl AttractorStorageWriter for FailingStorageWriter {
    async fn create_run_context(
        &self,
        _base_turn_id: Option<TurnId>,
    ) -> Result<StoreContext, TurnStoreError> {
        *self.calls.lock().expect("mutex") += 1;
        Err(TurnStoreError::Backend("forced create failure".to_string()))
    }

    async fn append_run_event(
        &self,
        _context_id: &ContextId,
        _record: AttractorRunEventRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        Err(TurnStoreError::Backend("forced append failure".to_string()))
    }

    async fn append_stage_event(
        &self,
        _context_id: &ContextId,
        _record: AttractorStageEventRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        Err(TurnStoreError::Backend("forced append failure".to_string()))
    }

    async fn append_checkpoint_event(
        &self,
        _context_id: &ContextId,
        _record: AttractorCheckpointEventRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        Err(TurnStoreError::Backend("forced append failure".to_string()))
    }

    async fn append_stage_to_agent_link(
        &self,
        _context_id: &ContextId,
        _record: AttractorStageToAgentLinkRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        Err(TurnStoreError::Backend("forced append failure".to_string()))
    }

    async fn append_dot_source(
        &self,
        _context_id: &ContextId,
        _record: AttractorDotSourceRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        Err(TurnStoreError::Backend("forced append failure".to_string()))
    }

    async fn append_graph_snapshot(
        &self,
        _context_id: &ContextId,
        _record: AttractorGraphSnapshotRecord,
        _idempotency_key: String,
    ) -> Result<StoredTurn, TurnStoreError> {
        Err(TurnStoreError::Backend("forced append failure".to_string()))
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cxdb_persistence_off_ignores_failing_store_expected_success() {
    let graph = graph_under_test();
    let failing = Arc::new(FailingStorageWriter::default());
    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                storage: Some(failing.clone()),
                cxdb_persistence: CxdbPersistenceMode::Off,
                executor: Arc::new(AlwaysSuccessExecutor),
                ..RunConfig::default()
            },
        )
        .await
        .expect("off mode should succeed");

    assert_eq!(result.status, PipelineStatus::Success);
    assert_eq!(*failing.calls.lock().expect("mutex"), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn cxdb_persistence_required_failing_store_expected_error() {
    let graph = graph_under_test();
    let failing = Arc::new(FailingStorageWriter::default());
    let error = PipelineRunner
        .run(
            &graph,
            RunConfig {
                storage: Some(failing),
                cxdb_persistence: CxdbPersistenceMode::Required,
                executor: Arc::new(AlwaysSuccessExecutor),
                ..RunConfig::default()
            },
        )
        .await
        .expect_err("required mode should fail");

    assert!(error.to_string().contains("forced create failure"));
}

#[tokio::test(flavor = "current_thread")]
async fn cxdb_required_failure_from_write_path_expected_error() {
    struct FailAfterCreate;
    #[async_trait]
    impl CxdbBinaryClient for FailAfterCreate {
        async fn ctx_create(
            &self,
            _base_turn_id: u64,
        ) -> Result<BinaryContextHead, CxdbClientError> {
            Ok(BinaryContextHead {
                context_id: 1,
                head_turn_id: 0,
                head_depth: 0,
            })
        }
        async fn ctx_fork(&self, _from_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
            Err(CxdbClientError::Backend("unused".to_string()))
        }
        async fn append_turn(
            &self,
            _request: BinaryAppendTurnRequest,
        ) -> Result<BinaryAppendTurnResponse, CxdbClientError> {
            Err(CxdbClientError::Backend(
                "forced append failure".to_string(),
            ))
        }
        async fn get_head(&self, _context_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
            Ok(BinaryContextHead {
                context_id: 1,
                head_turn_id: 0,
                head_depth: 0,
            })
        }
        async fn get_last(
            &self,
            _context_id: u64,
            _limit: usize,
            _include_payload: bool,
        ) -> Result<Vec<BinaryStoredTurn>, CxdbClientError> {
            Err(CxdbClientError::Backend("unused".to_string()))
        }
        async fn put_blob(&self, _raw_bytes: &[u8]) -> Result<String, CxdbClientError> {
            Ok("blob-hash".to_string())
        }
        async fn get_blob(
            &self,
            _content_hash: &String,
        ) -> Result<Option<Vec<u8>>, CxdbClientError> {
            Err(CxdbClientError::Backend("unused".to_string()))
        }
        async fn attach_fs(
            &self,
            _turn_id: u64,
            _fs_root_hash: &String,
        ) -> Result<(), CxdbClientError> {
            Err(CxdbClientError::Backend("unused".to_string()))
        }
    }
    #[async_trait]
    impl CxdbHttpClient for FailAfterCreate {
        async fn list_turns(
            &self,
            _context_id: u64,
            _before_turn_id: Option<u64>,
            _limit: usize,
        ) -> Result<Vec<HttpStoredTurn>, CxdbClientError> {
            Ok(Vec::new())
        }
        async fn publish_registry_bundle(
            &self,
            _bundle_id: &str,
            _bundle_json: &[u8],
        ) -> Result<(), CxdbClientError> {
            Ok(())
        }
        async fn get_registry_bundle(
            &self,
            _bundle_id: &str,
        ) -> Result<Option<Vec<u8>>, CxdbClientError> {
            Ok(None)
        }
    }

    let graph = graph_under_test();
    let failing = Arc::new(CxdbTurnStore::new(
        Arc::new(FailAfterCreate),
        Arc::new(FailAfterCreate),
    ));
    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                storage: Some(failing),
                cxdb_persistence: CxdbPersistenceMode::Required,
                executor: Arc::new(AlwaysSuccessExecutor),
                ..RunConfig::default()
            },
        )
        .await;
    let error = result.expect_err("required mode should fail on append errors");
    assert!(error.to_string().contains("forced append failure"));
}
