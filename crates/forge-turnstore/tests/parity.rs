use async_trait::async_trait;
use forge_turnstore::{
    AppendTurnRequest, ArtifactStore, FsTurnStore, MemoryTurnStore, RegistryBundle, TurnStore,
    TypedTurnStore,
};
use forge_turnstore_cxdb::{
    BinaryAppendTurnRequest, BinaryAppendTurnResponse, BinaryContextHead, BinaryStoredTurn,
    CxdbBinaryClient, CxdbClientError, CxdbHttpClient, CxdbTurnStore, HttpStoredTurn,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
struct MockCxdbBackend {
    inner: Arc<Mutex<MockState>>,
}

#[derive(Clone, Debug)]
struct MockState {
    next_context_id: u64,
    next_turn_id: u64,
    now_tick: u64,
    idempotency_ttl_ticks: u64,
    contexts: BTreeMap<u64, MockContext>,
    turns: BTreeMap<u64, BinaryStoredTurn>,
    idempotency: BTreeMap<String, (u64, u64)>,
    blobs: BTreeMap<String, Vec<u8>>,
    turn_fs_roots: BTreeMap<u64, String>,
    registry_bundles: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug, Default)]
struct MockContext {
    head_turn_id: u64,
    head_depth: u32,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            next_context_id: 1,
            next_turn_id: 1,
            now_tick: 0,
            idempotency_ttl_ticks: 24,
            contexts: BTreeMap::new(),
            turns: BTreeMap::new(),
            idempotency: BTreeMap::new(),
            blobs: BTreeMap::new(),
            turn_fs_roots: BTreeMap::new(),
            registry_bundles: BTreeMap::new(),
        }
    }
}

impl MockState {
    fn context_has_turn(&self, context: &MockContext, turn_id: u64) -> bool {
        if turn_id == 0 {
            return true;
        }
        let mut cursor = context.head_turn_id;
        while cursor != 0 {
            if cursor == turn_id {
                return true;
            }
            let Some(turn) = self.turns.get(&cursor) else {
                return false;
            };
            cursor = turn.parent_turn_id;
        }
        false
    }
}

impl MockCxdbBackend {
    fn new(idempotency_ttl_ticks: u64) -> Self {
        let state = MockState {
            idempotency_ttl_ticks,
            ..MockState::default()
        };
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    fn advance_ticks(&self, delta: u64) {
        let mut state = self.inner.lock().expect("mutex should lock");
        state.now_tick = state.now_tick.saturating_add(delta);
    }
}

#[async_trait]
impl CxdbBinaryClient for MockCxdbBackend {
    async fn ctx_create(&self, base_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mutex poisoned".to_string()))?;
        let (head_turn_id, head_depth) = if base_turn_id == 0 {
            (0, 0)
        } else {
            let Some(turn) = state.turns.get(&base_turn_id) else {
                return Err(CxdbClientError::NotFound {
                    resource: "turn",
                    id: base_turn_id.to_string(),
                });
            };
            (turn.turn_id, turn.depth)
        };
        let context_id = state.next_context_id;
        state.next_context_id += 1;
        state.contexts.insert(
            context_id,
            MockContext {
                head_turn_id,
                head_depth,
            },
        );
        Ok(BinaryContextHead {
            context_id,
            head_turn_id,
            head_depth,
        })
    }

    async fn ctx_fork(&self, from_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        self.ctx_create(from_turn_id).await
    }

    async fn append_turn(
        &self,
        request: BinaryAppendTurnRequest,
    ) -> Result<BinaryAppendTurnResponse, CxdbClientError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mutex poisoned".to_string()))?;
        let context = state
            .contexts
            .get(&request.context_id)
            .cloned()
            .ok_or_else(|| CxdbClientError::NotFound {
                resource: "context",
                id: request.context_id.to_string(),
            })?;

        let idempotency_scoped = format!("{}|{}", request.context_id, request.idempotency_key);
        if !request.idempotency_key.is_empty() {
            if let Some((existing_turn_id, created_tick)) =
                state.idempotency.get(&idempotency_scoped)
            {
                let age = state.now_tick.saturating_sub(*created_tick);
                if age <= state.idempotency_ttl_ticks {
                    let existing = state.turns.get(existing_turn_id).ok_or_else(|| {
                        CxdbClientError::Backend("idempotency index corrupted".to_string())
                    })?;
                    return Ok(BinaryAppendTurnResponse {
                        context_id: existing.context_id,
                        new_turn_id: existing.turn_id,
                        new_depth: existing.depth,
                        content_hash: existing.content_hash,
                    });
                }
            }
        }

        let parent_turn_id = if request.parent_turn_id == 0 {
            context.head_turn_id
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
        if content_hash != request.content_hash {
            return Err(CxdbClientError::InvalidInput(
                "content hash mismatch".to_string(),
            ));
        }

        let turn_id = state.next_turn_id;
        state.next_turn_id += 1;
        let turn = BinaryStoredTurn {
            context_id: request.context_id,
            turn_id,
            parent_turn_id,
            depth: parent_depth + 1,
            type_id: request.type_id,
            type_version: request.type_version,
            payload: request.payload,
            idempotency_key: if request.idempotency_key.is_empty() {
                None
            } else {
                Some(request.idempotency_key)
            },
            content_hash,
        };

        state.turns.insert(turn_id, turn.clone());
        if let Some(ctx) = state.contexts.get_mut(&turn.context_id) {
            ctx.head_turn_id = turn.turn_id;
            ctx.head_depth = turn.depth;
        }
        if let Some(key) = turn.idempotency_key.as_ref() {
            let now_tick = state.now_tick;
            state.idempotency.insert(
                format!("{}|{}", turn.context_id, key),
                (turn.turn_id, now_tick),
            );
        }
        state.now_tick = state.now_tick.saturating_add(1);

        Ok(BinaryAppendTurnResponse {
            context_id: turn.context_id,
            new_turn_id: turn.turn_id,
            new_depth: turn.depth,
            content_hash: turn.content_hash,
        })
    }

    async fn get_head(&self, context_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mutex poisoned".to_string()))?;
        let context = state
            .contexts
            .get(&context_id)
            .ok_or_else(|| CxdbClientError::NotFound {
                resource: "context",
                id: context_id.to_string(),
            })?;
        Ok(BinaryContextHead {
            context_id,
            head_turn_id: context.head_turn_id,
            head_depth: context.head_depth,
        })
    }

    async fn get_last(
        &self,
        context_id: u64,
        limit: usize,
        include_payload: bool,
    ) -> Result<Vec<BinaryStoredTurn>, CxdbClientError> {
        if !include_payload {
            return Err(CxdbClientError::InvalidInput(
                "include_payload must be true".to_string(),
            ));
        }
        let state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mutex poisoned".to_string()))?;
        let context = state
            .contexts
            .get(&context_id)
            .ok_or_else(|| CxdbClientError::NotFound {
                resource: "context",
                id: context_id.to_string(),
            })?;

        let mut cursor = context.head_turn_id;
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
        let mut state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mutex poisoned".to_string()))?;
        let hash = blake3::hash(raw_bytes).to_hex().to_string();
        state
            .blobs
            .entry(hash.clone())
            .or_insert_with(|| raw_bytes.to_vec());
        Ok(hash)
    }

    async fn get_blob(&self, content_hash: &String) -> Result<Option<Vec<u8>>, CxdbClientError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mutex poisoned".to_string()))?;
        Ok(state.blobs.get(content_hash).cloned())
    }

    async fn attach_fs(&self, turn_id: u64, fs_root_hash: &String) -> Result<(), CxdbClientError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mutex poisoned".to_string()))?;
        if !state.turns.contains_key(&turn_id) {
            return Err(CxdbClientError::NotFound {
                resource: "turn",
                id: turn_id.to_string(),
            });
        }
        if !state.blobs.contains_key(fs_root_hash) {
            return Err(CxdbClientError::NotFound {
                resource: "blob",
                id: fs_root_hash.clone(),
            });
        }
        state.turn_fs_roots.insert(turn_id, fs_root_hash.clone());
        Ok(())
    }
}

#[async_trait]
impl CxdbHttpClient for MockCxdbBackend {
    async fn list_turns(
        &self,
        context_id: u64,
        before_turn_id: Option<u64>,
        limit: usize,
    ) -> Result<Vec<HttpStoredTurn>, CxdbClientError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mutex poisoned".to_string()))?;
        let context = state
            .contexts
            .get(&context_id)
            .ok_or_else(|| CxdbClientError::NotFound {
                resource: "context",
                id: context_id.to_string(),
            })?;

        let mut cursor = if let Some(before) = before_turn_id {
            if before == 0 {
                return Ok(Vec::new());
            }
            if !state.context_has_turn(context, before) {
                return Err(CxdbClientError::InvalidInput(format!(
                    "turn {before} not reachable from context {context_id}"
                )));
            }
            state
                .turns
                .get(&before)
                .ok_or_else(|| CxdbClientError::NotFound {
                    resource: "turn",
                    id: before.to_string(),
                })?
                .parent_turn_id
        } else {
            context.head_turn_id
        };

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
        bundle_id: &str,
        bundle_json: &[u8],
    ) -> Result<(), CxdbClientError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mutex poisoned".to_string()))?;
        state
            .registry_bundles
            .insert(bundle_id.to_string(), bundle_json.to_vec());
        Ok(())
    }

    async fn get_registry_bundle(
        &self,
        bundle_id: &str,
    ) -> Result<Option<Vec<u8>>, CxdbClientError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mutex poisoned".to_string()))?;
        Ok(state.registry_bundles.get(bundle_id).cloned())
    }
}

fn append_request(context_id: &str, payload: &[u8], key: &str) -> AppendTurnRequest {
    AppendTurnRequest {
        context_id: context_id.to_string(),
        parent_turn_id: None,
        type_id: "forge.agent.user_turn".to_string(),
        type_version: 1,
        payload: payload.to_vec(),
        idempotency_key: key.to_string(),
    }
}

async fn exercise_parity_contract<T: TurnStore + ArtifactStore + TypedTurnStore>(store: &T) {
    let context = store
        .create_context(None)
        .await
        .expect("context should be created");

    let first = store
        .append_turn(append_request(&context.context_id, b"one", "k1"))
        .await
        .expect("append should succeed");
    let second = store
        .append_turn(append_request(&context.context_id, b"two", "k2"))
        .await
        .expect("append should succeed");
    let dupe = store
        .append_turn(append_request(&context.context_id, b"one", "k1"))
        .await
        .expect("idempotent append should succeed");
    assert_eq!(dupe.turn_id, first.turn_id);

    let turns = store
        .list_turns(&context.context_id, None, 10)
        .await
        .expect("list should succeed");
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].turn_id, first.turn_id);
    assert_eq!(turns[1].turn_id, second.turn_id);

    let older = store
        .list_turns(&context.context_id, Some(&second.turn_id), 10)
        .await
        .expect("paged list should succeed");
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].turn_id, first.turn_id);

    let blob_hash = store
        .put_blob(b"blob")
        .await
        .expect("blob put should succeed");
    assert_eq!(
        store
            .get_blob(&blob_hash)
            .await
            .expect("blob get should succeed")
            .as_deref(),
        Some(b"blob".as_slice())
    );
    store
        .attach_fs(&second.turn_id, &blob_hash)
        .await
        .expect("attach fs should succeed");

    store
        .publish_registry_bundle(RegistryBundle {
            bundle_id: "bundle-1".to_string(),
            bundle_json: br#"{"registry_version":1}"#.to_vec(),
        })
        .await
        .expect("publish bundle should succeed");
    assert_eq!(
        store
            .get_registry_bundle("bundle-1")
            .await
            .expect("get bundle should succeed"),
        Some(br#"{"registry_version":1}"#.to_vec())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn parity_memory_fs_cxdb_expected_same_contract_behavior() {
    let memory = MemoryTurnStore::new();
    exercise_parity_contract(&memory).await;

    let temp = tempfile::tempdir().expect("tempdir should create");
    let fs = FsTurnStore::new(temp.path()).expect("fs store should create");
    exercise_parity_contract(&fs).await;

    let backend = MockCxdbBackend::new(24);
    let cxdb = CxdbTurnStore::new(backend.clone(), backend);
    exercise_parity_contract(&cxdb).await;
}

#[tokio::test(flavor = "current_thread")]
async fn protocol_get_last_returns_chronological_order_expected_oldest_to_newest() {
    let backend = MockCxdbBackend::new(24);
    let store = CxdbTurnStore::new(backend.clone(), backend);
    let context = store
        .create_context(None)
        .await
        .expect("context should be created");

    let t1 = store
        .append_turn(append_request(&context.context_id, b"a", "k1"))
        .await
        .expect("append should succeed");
    let t2 = store
        .append_turn(append_request(&context.context_id, b"b", "k2"))
        .await
        .expect("append should succeed");
    let t3 = store
        .append_turn(append_request(&context.context_id, b"c", "k3"))
        .await
        .expect("append should succeed");

    let turns = store
        .list_turns(&context.context_id, None, 3)
        .await
        .expect("list should succeed");
    assert_eq!(
        turns.iter().map(|t| t.turn_id.clone()).collect::<Vec<_>>(),
        vec![t1.turn_id, t2.turn_id, t3.turn_id]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn protocol_http_paging_parity_expected_before_turn_id_returns_older_window() {
    let backend = MockCxdbBackend::new(24);
    let store = CxdbTurnStore::new(backend.clone(), backend);
    let context = store
        .create_context(None)
        .await
        .expect("context should be created");

    let mut ids = Vec::new();
    for i in 0..5_u8 {
        let turn = store
            .append_turn(append_request(
                &context.context_id,
                &[i],
                &format!("k-{}", i),
            ))
            .await
            .expect("append should succeed");
        ids.push(turn.turn_id);
    }

    let paged = store
        .list_turns(&context.context_id, Some(&ids[4]), 2)
        .await
        .expect("paged list should succeed");
    assert_eq!(paged.len(), 2);
    assert_eq!(paged[0].turn_id, ids[2]);
    assert_eq!(paged[1].turn_id, ids[3]);
}

#[tokio::test(flavor = "current_thread")]
async fn protocol_idempotency_ttl_expiry_expected_key_reusable_after_ttl_window() {
    let backend = MockCxdbBackend::new(1);
    let store = CxdbTurnStore::new(backend.clone(), backend.clone());
    let context = store
        .create_context(None)
        .await
        .expect("context should be created");

    let first = store
        .append_turn(append_request(&context.context_id, b"same", "ttl-key"))
        .await
        .expect("first append should succeed");
    let second = store
        .append_turn(append_request(&context.context_id, b"same", "ttl-key"))
        .await
        .expect("deduplicated append should succeed");
    assert_eq!(first.turn_id, second.turn_id);

    backend.advance_ticks(10);

    let third = store
        .append_turn(append_request(&context.context_id, b"same", "ttl-key"))
        .await
        .expect("append after ttl expiry should create new turn");
    assert_ne!(third.turn_id, first.turn_id);
}
