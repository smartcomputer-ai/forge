mod support;

use async_trait::async_trait;
use forge_agent::{LocalExecutionEnvironment, Session, SessionConfig, TurnStoreWriteMode};
use forge_turnstore_cxdb::{
    BinaryAppendTurnRequest, BinaryAppendTurnResponse, BinaryContextHead, BinaryStoredTurn,
    CxdbBinaryClient, CxdbClientError, CxdbHttpClient, HttpStoredTurn,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use support::{all_fixtures, client_with_adapter, enqueue, text_response};
use tempfile::tempdir;

#[derive(Clone, Debug, Default)]
struct WorkingMockCxdb {
    inner: Arc<Mutex<WorkingState>>,
}

#[derive(Clone, Debug, Default)]
struct WorkingState {
    next_context_id: u64,
    next_turn_id: u64,
    contexts: BTreeMap<u64, (u64, u32)>,
    turns: BTreeMap<u64, BinaryStoredTurn>,
    idempotency: BTreeMap<String, u64>,
}

#[async_trait]
impl CxdbBinaryClient for WorkingMockCxdb {
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
        let (current_head, _) = state
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
            current_head
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

        let turn = BinaryStoredTurn {
            context_id: request.context_id,
            turn_id,
            parent_turn_id,
            depth: parent_depth + 1,
            type_id: request.type_id,
            type_version: request.type_version,
            payload: request.payload,
            idempotency_key: Some(request.idempotency_key),
            content_hash: [0; 32],
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
            content_hash: turn.content_hash,
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
        Ok(format!("blob-{}", raw_bytes.len()))
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
impl CxdbHttpClient for WorkingMockCxdb {
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

#[derive(Clone, Debug, Default)]
struct FailingCxdb;

#[async_trait]
impl CxdbBinaryClient for FailingCxdb {
    async fn ctx_create(&self, _base_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        Err(CxdbClientError::Backend(
            "forced create failure".to_string(),
        ))
    }

    async fn ctx_fork(&self, _from_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        Err(CxdbClientError::Backend("forced fork failure".to_string()))
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
        Err(CxdbClientError::Backend("forced head failure".to_string()))
    }

    async fn get_last(
        &self,
        _context_id: u64,
        _limit: usize,
        _include_payload: bool,
    ) -> Result<Vec<BinaryStoredTurn>, CxdbClientError> {
        Err(CxdbClientError::Backend("forced list failure".to_string()))
    }

    async fn put_blob(&self, _raw_bytes: &[u8]) -> Result<String, CxdbClientError> {
        Err(CxdbClientError::Backend("forced blob failure".to_string()))
    }

    async fn get_blob(&self, _content_hash: &String) -> Result<Option<Vec<u8>>, CxdbClientError> {
        Err(CxdbClientError::Backend("forced blob failure".to_string()))
    }

    async fn attach_fs(
        &self,
        _turn_id: u64,
        _fs_root_hash: &String,
    ) -> Result<(), CxdbClientError> {
        Err(CxdbClientError::Backend("forced fs failure".to_string()))
    }
}

#[async_trait]
impl CxdbHttpClient for FailingCxdb {
    async fn list_turns(
        &self,
        _context_id: u64,
        _before_turn_id: Option<u64>,
        _limit: usize,
    ) -> Result<Vec<HttpStoredTurn>, CxdbClientError> {
        Err(CxdbClientError::Backend("forced list failure".to_string()))
    }

    async fn publish_registry_bundle(
        &self,
        _bundle_id: &str,
        _bundle_json: &[u8],
    ) -> Result<(), CxdbClientError> {
        Err(CxdbClientError::Backend(
            "forced registry failure".to_string(),
        ))
    }

    async fn get_registry_bundle(
        &self,
        _bundle_id: &str,
    ) -> Result<Option<Vec<u8>>, CxdbClientError> {
        Err(CxdbClientError::Backend(
            "forced registry failure".to_string(),
        ))
    }
}

async fn run_once_with_cxdb(
    mode: TurnStoreWriteMode,
    binary: Arc<dyn CxdbBinaryClient>,
    http: Arc<dyn CxdbHttpClient>,
) -> Result<(), forge_agent::AgentError> {
    let fixture = all_fixtures()[0].clone();
    let dir = tempdir().expect("temp dir should create");
    let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
    let (client, responses, _requests) = client_with_adapter(fixture.id());
    let profile = fixture.profile();
    let mut config = SessionConfig::default();
    config.turn_store_mode = mode;
    let mut session =
        Session::new_with_cxdb_turn_store(profile, env, client, config, binary, http)?;

    enqueue(
        &responses,
        text_response(fixture.id(), fixture.model(), "resp-1", "done"),
    );
    session.submit("run once").await?;
    session.close()?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
async fn cxdb_required_mode_persists_queryable_turns() {
    let fixture = all_fixtures()[0].clone();
    let dir = tempdir().expect("temp dir should create");
    let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
    let (client, responses, _requests) = client_with_adapter(fixture.id());
    let profile = fixture.profile();
    let mut config = SessionConfig::default();
    config.turn_store_mode = TurnStoreWriteMode::Required;
    let backend = Arc::new(WorkingMockCxdb::default());
    let mut session = Session::new_with_cxdb_turn_store(
        profile,
        env,
        client,
        config,
        backend.clone(),
        backend.clone(),
    )
    .expect("session should initialize");

    enqueue(
        &responses,
        text_response(fixture.id(), fixture.model(), "resp-1", "done"),
    );
    session
        .submit("run once")
        .await
        .expect("submit should succeed");
    session.close().expect("close should succeed");

    let snapshot = session
        .persistence_snapshot()
        .await
        .expect("snapshot should succeed");
    let context_id = snapshot.context_id.expect("context should exist");
    let turns = backend
        .list_turns(context_id.parse::<u64>().expect("u64 context id"), None, 64)
        .await
        .expect("list should succeed");
    assert!(!turns.is_empty());
    assert!(
        turns
            .iter()
            .any(|turn| turn.type_id == "forge.agent.user_turn")
    );
    assert!(
        turns
            .iter()
            .any(|turn| turn.type_id == "forge.agent.assistant_turn")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cxdb_mode_best_effort_create_failure_expected_submit_succeeds() {
    run_once_with_cxdb(
        TurnStoreWriteMode::BestEffort,
        Arc::new(FailingCxdb),
        Arc::new(FailingCxdb),
    )
    .await
    .expect("best_effort should tolerate cxdb failures");
}

#[tokio::test(flavor = "current_thread")]
async fn cxdb_mode_off_create_failure_expected_submit_succeeds() {
    run_once_with_cxdb(
        TurnStoreWriteMode::Off,
        Arc::new(FailingCxdb),
        Arc::new(FailingCxdb),
    )
    .await
    .expect("off mode should not touch cxdb");
}

#[tokio::test(flavor = "current_thread")]
async fn cxdb_mode_required_create_failure_expected_constructor_error() {
    let fixture = all_fixtures()[0].clone();
    let dir = tempdir().expect("temp dir should create");
    let env = Arc::new(LocalExecutionEnvironment::new(dir.path()));
    let (client, _responses, _requests) = client_with_adapter(fixture.id());
    let profile = fixture.profile();
    let mut config = SessionConfig::default();
    config.turn_store_mode = TurnStoreWriteMode::Required;

    let result = Session::new_with_cxdb_turn_store(
        profile,
        env,
        client,
        config,
        Arc::new(FailingCxdb),
        Arc::new(FailingCxdb),
    );
    let error = match result {
        Ok(_) => panic!("required mode should fail constructor"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("create_context failed"));
}
