use async_trait::async_trait;
use forge_attractor::{
    AttractorCheckpointEventRecord, AttractorDotSourceRecord, AttractorGraphSnapshotRecord,
    AttractorRunEventRecord, AttractorStageEventRecord, AttractorStageToAgentLinkRecord,
    AttractorStorageWriter, Graph, Node, NodeExecutor, NodeOutcome, PipelineRunner, PipelineStatus,
    RunConfig, RuntimeContext, StorageWriteMode, parse_dot,
};
use forge_turnstore::{ContextId, FsTurnStore, StoreContext, StoredTurn, TurnId, TurnStoreError};
use forge_turnstore_cxdb::{
    BinaryAppendTurnRequest, BinaryAppendTurnResponse, BinaryContextHead, BinaryStoredTurn,
    CxdbBinaryClient, CxdbClientError, CxdbHttpClient, CxdbTurnStore, HttpStoredTurn,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

#[derive(Clone, Debug, Default)]
struct MockCxdb {
    inner: Arc<Mutex<MockCxdbState>>,
}

#[derive(Clone, Debug, Default)]
struct MockCxdbState {
    next_context_id: u64,
    next_turn_id: u64,
    contexts: BTreeMap<u64, (u64, u32)>,
    turns: BTreeMap<u64, BinaryStoredTurn>,
    idempotency: BTreeMap<String, u64>,
}

#[async_trait]
impl CxdbBinaryClient for MockCxdb {
    async fn ctx_create(&self, base_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let mut state = self.inner.lock().expect("mutex");
        if state.next_context_id == 0 {
            state.next_context_id = 1;
        }
        let context_id = state.next_context_id;
        state.next_context_id += 1;
        let (head, depth) = if base_turn_id == 0 {
            (0, 0)
        } else {
            let turn = state
                .turns
                .get(&base_turn_id)
                .ok_or_else(|| CxdbClientError::NotFound {
                    resource: "turn",
                    id: base_turn_id.to_string(),
                })?;
            (turn.turn_id, turn.depth)
        };
        state.contexts.insert(context_id, (head, depth));
        Ok(BinaryContextHead {
            context_id,
            head_turn_id: head,
            head_depth: depth,
        })
    }

    async fn ctx_fork(&self, from_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        self.ctx_create(from_turn_id).await
    }

    async fn append_turn(
        &self,
        request: BinaryAppendTurnRequest,
    ) -> Result<BinaryAppendTurnResponse, CxdbClientError> {
        let mut state = self.inner.lock().expect("mutex");
        let (head, _) = state
            .contexts
            .get(&request.context_id)
            .copied()
            .ok_or_else(|| CxdbClientError::NotFound {
                resource: "context",
                id: request.context_id.to_string(),
            })?;

        let id_key = format!("{}|{}", request.context_id, request.idempotency_key);
        if let Some(existing) = state.idempotency.get(&id_key).copied() {
            let turn = state.turns.get(&existing).ok_or_else(|| {
                CxdbClientError::Backend("idempotency index corrupted".to_string())
            })?;
            return Ok(BinaryAppendTurnResponse {
                context_id: turn.context_id,
                new_turn_id: turn.turn_id,
                new_depth: turn.depth,
                content_hash: turn.content_hash,
            });
        }

        if state.next_turn_id == 0 {
            state.next_turn_id = 1;
        }
        let turn_id = state.next_turn_id;
        state.next_turn_id += 1;
        let parent_turn_id = if request.parent_turn_id == 0 {
            head
        } else {
            request.parent_turn_id
        };
        let parent_depth = if parent_turn_id == 0 {
            0
        } else {
            state
                .turns
                .get(&parent_turn_id)
                .map(|turn| turn.depth)
                .ok_or_else(|| CxdbClientError::NotFound {
                    resource: "turn",
                    id: parent_turn_id.to_string(),
                })?
        };

        let content_hash = *blake3::hash(&request.payload).as_bytes();
        let turn = BinaryStoredTurn {
            context_id: request.context_id,
            turn_id,
            parent_turn_id,
            depth: parent_depth + 1,
            type_id: request.type_id,
            type_version: request.type_version,
            payload: request.payload,
            idempotency_key: Some(request.idempotency_key),
            content_hash,
        };
        state.turns.insert(turn_id, turn.clone());
        state
            .contexts
            .insert(turn.context_id, (turn.turn_id, turn.depth));
        if let Some(key) = turn.idempotency_key {
            state
                .idempotency
                .insert(format!("{}|{}", turn.context_id, key), turn.turn_id);
        }

        Ok(BinaryAppendTurnResponse {
            context_id: turn.context_id,
            new_turn_id: turn.turn_id,
            new_depth: turn.depth,
            content_hash,
        })
    }

    async fn get_head(&self, context_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let state = self.inner.lock().expect("mutex");
        let (head, depth) =
            state
                .contexts
                .get(&context_id)
                .copied()
                .ok_or_else(|| CxdbClientError::NotFound {
                    resource: "context",
                    id: context_id.to_string(),
                })?;
        Ok(BinaryContextHead {
            context_id,
            head_turn_id: head,
            head_depth: depth,
        })
    }

    async fn get_last(
        &self,
        context_id: u64,
        limit: usize,
        _include_payload: bool,
    ) -> Result<Vec<BinaryStoredTurn>, CxdbClientError> {
        let state = self.inner.lock().expect("mutex");
        let (head, _) =
            state
                .contexts
                .get(&context_id)
                .copied()
                .ok_or_else(|| CxdbClientError::NotFound {
                    resource: "context",
                    id: context_id.to_string(),
                })?;
        let mut cursor = head;
        let mut turns = Vec::new();
        while cursor != 0 && turns.len() < limit {
            let turn = state
                .turns
                .get(&cursor)
                .ok_or_else(|| CxdbClientError::NotFound {
                    resource: "turn",
                    id: cursor.to_string(),
                })?;
            turns.push(turn.clone());
            cursor = turn.parent_turn_id;
        }
        turns.reverse();
        Ok(turns)
    }

    async fn put_blob(&self, raw_bytes: &[u8]) -> Result<String, CxdbClientError> {
        Ok(blake3::hash(raw_bytes).to_hex().to_string())
    }

    async fn get_blob(&self, _content_hash: &String) -> Result<Option<Vec<u8>>, CxdbClientError> {
        Ok(None)
    }

    async fn attach_fs(
        &self,
        _turn_id: u64,
        _fs_root_hash: &String,
    ) -> Result<(), CxdbClientError> {
        Ok(())
    }
}

#[async_trait]
impl CxdbHttpClient for MockCxdb {
    async fn list_turns(
        &self,
        context_id: u64,
        before_turn_id: Option<u64>,
        limit: usize,
    ) -> Result<Vec<HttpStoredTurn>, CxdbClientError> {
        let state = self.inner.lock().expect("mutex");
        let (head, _) =
            state
                .contexts
                .get(&context_id)
                .copied()
                .ok_or_else(|| CxdbClientError::NotFound {
                    resource: "context",
                    id: context_id.to_string(),
                })?;
        let mut cursor = before_turn_id
            .and_then(|turn| state.turns.get(&turn).map(|t| t.parent_turn_id))
            .unwrap_or(head);
        let mut turns = Vec::new();
        while cursor != 0 && turns.len() < limit {
            let turn = state
                .turns
                .get(&cursor)
                .ok_or_else(|| CxdbClientError::NotFound {
                    resource: "turn",
                    id: cursor.to_string(),
                })?;
            turns.push(HttpStoredTurn {
                context_id: turn.context_id,
                turn_id: turn.turn_id,
                parent_turn_id: turn.parent_turn_id,
                depth: turn.depth,
                type_id: turn.type_id.clone(),
                type_version: turn.type_version,
                payload: turn.payload.clone(),
                idempotency_key: turn.idempotency_key.clone(),
                content_hash: turn.content_hash,
            });
            cursor = turn.parent_turn_id;
        }
        turns.reverse();
        Ok(turns)
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
async fn storage_mode_off_ignores_failing_store_expected_success() {
    let graph = graph_under_test();
    let failing = Arc::new(FailingStorageWriter::default());
    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                storage: Some(failing.clone()),
                storage_mode: StorageWriteMode::Off,
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
async fn storage_mode_best_effort_tolerates_failing_store_expected_success() {
    let graph = graph_under_test();
    let failing = Arc::new(FailingStorageWriter::default());
    let result = PipelineRunner
        .run(
            &graph,
            RunConfig {
                storage: Some(failing),
                storage_mode: StorageWriteMode::BestEffort,
                executor: Arc::new(AlwaysSuccessExecutor),
                ..RunConfig::default()
            },
        )
        .await
        .expect("best_effort should succeed");

    assert_eq!(result.status, PipelineStatus::Success);
}

#[tokio::test(flavor = "current_thread")]
async fn storage_mode_required_failing_store_expected_error() {
    let graph = graph_under_test();
    let failing = Arc::new(FailingStorageWriter::default());
    let error = PipelineRunner
        .run(
            &graph,
            RunConfig {
                storage: Some(failing),
                storage_mode: StorageWriteMode::Required,
                executor: Arc::new(AlwaysSuccessExecutor),
                ..RunConfig::default()
            },
        )
        .await
        .expect_err("required mode should fail");

    assert!(error.to_string().contains("forced create failure"));
}

#[tokio::test(flavor = "current_thread")]
async fn cxdb_best_effort_failure_from_write_path_expected_success() {
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
            Err(CxdbClientError::Backend("unused".to_string()))
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
            Err(CxdbClientError::Backend("unused".to_string()))
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
                storage_mode: StorageWriteMode::BestEffort,
                executor: Arc::new(AlwaysSuccessExecutor),
                ..RunConfig::default()
            },
        )
        .await
        .expect("best_effort should continue on append failures");

    assert_eq!(result.status, PipelineStatus::Success);
    assert_eq!(
        result
            .context
            .get("graph.goal")
            .cloned()
            .unwrap_or(Value::Null),
        Value::Null
    );
}
