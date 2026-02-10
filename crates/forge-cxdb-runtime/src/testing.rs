use crate::{
    BinaryAppendTurnRequest, BinaryAppendTurnResponse, BinaryContextHead, BinaryStoredTurn,
    CxdbBinaryClient, CxdbClientError, CxdbHttpClient, HttpStoredTurn,
};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
pub struct MockCxdb {
    inner: Arc<Mutex<MockCxdbState>>,
}

#[derive(Clone, Debug, Default)]
struct MockCxdbState {
    next_context_id: u64,
    next_turn_id: u64,
    contexts: BTreeMap<u64, MockContextState>,
    turns: BTreeMap<u64, BinaryStoredTurn>,
    idempotency: BTreeMap<String, u64>,
    blobs: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug)]
struct MockContextState {
    head_turn_id: u64,
    head_depth: u32,
}

impl Default for MockContextState {
    fn default() -> Self {
        Self {
            head_turn_id: 0,
            head_depth: 0,
        }
    }
}

impl MockCxdbState {
    fn allocate_context_id(&mut self) -> u64 {
        if self.next_context_id == 0 {
            self.next_context_id = 1;
        }
        let id = self.next_context_id;
        self.next_context_id += 1;
        id
    }

    fn allocate_turn_id(&mut self) -> u64 {
        if self.next_turn_id == 0 {
            self.next_turn_id = 1;
        }
        let id = self.next_turn_id;
        self.next_turn_id += 1;
        id
    }

    fn turn_depth(&self, turn_id: u64) -> Option<u32> {
        self.turns.get(&turn_id).map(|turn| turn.depth)
    }

    fn context_has_turn(&self, context: &MockContextState, turn_id: u64) -> bool {
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

#[async_trait]
impl CxdbBinaryClient for MockCxdb {
    async fn ctx_create(&self, base_turn_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mock backend mutex poisoned".to_string()))?;

        let (head_turn_id, head_depth) = if base_turn_id == 0 {
            (0, 0)
        } else {
            let Some(depth) = state.turn_depth(base_turn_id) else {
                return Err(CxdbClientError::NotFound {
                    resource: "turn",
                    id: base_turn_id.to_string(),
                });
            };
            (base_turn_id, depth)
        };

        let context_id = state.allocate_context_id();
        state.contexts.insert(
            context_id,
            MockContextState {
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
            .map_err(|_| CxdbClientError::Backend("mock backend mutex poisoned".to_string()))?;

        let context_snapshot = state
            .contexts
            .get(&request.context_id)
            .cloned()
            .ok_or_else(|| CxdbClientError::NotFound {
                resource: "context",
                id: request.context_id.to_string(),
            })?;

        if !request.idempotency_key.is_empty() {
            let key = format!("{}|{}", request.context_id, request.idempotency_key);
            if let Some(existing_turn_id) = state.idempotency.get(&key).copied() {
                let existing_turn = state.turns.get(&existing_turn_id).ok_or_else(|| {
                    CxdbClientError::Backend("idempotency index corrupted".to_string())
                })?;
                return Ok(BinaryAppendTurnResponse {
                    context_id: existing_turn.context_id,
                    new_turn_id: existing_turn.turn_id,
                    new_depth: existing_turn.depth,
                    content_hash: existing_turn.content_hash,
                });
            }
        }

        let parent_turn_id = if request.parent_turn_id == 0 {
            context_snapshot.head_turn_id
        } else {
            request.parent_turn_id
        };

        let parent_depth = if parent_turn_id == 0 {
            0
        } else {
            state
                .turn_depth(parent_turn_id)
                .ok_or_else(|| CxdbClientError::NotFound {
                    resource: "turn",
                    id: parent_turn_id.to_string(),
                })?
        };

        let content_hash = *blake3::hash(&request.payload).as_bytes();
        if content_hash != request.content_hash {
            return Err(CxdbClientError::InvalidInput(
                "content hash mismatch for append payload".to_string(),
            ));
        }

        if parent_turn_id != 0 && !state.context_has_turn(&context_snapshot, parent_turn_id) {
            return Err(CxdbClientError::Conflict(
                "parent turn is not reachable from context head".to_string(),
            ));
        }

        let turn_id = state.allocate_turn_id();
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
                Some(request.idempotency_key.clone())
            },
            content_hash,
        };

        state.turns.insert(turn_id, turn.clone());
        if !request.idempotency_key.is_empty() {
            let key = format!("{}|{}", request.context_id, request.idempotency_key);
            state.idempotency.insert(key, turn_id);
        }

        let context = state.contexts.get_mut(&request.context_id).ok_or_else(|| {
            CxdbClientError::NotFound {
                resource: "context",
                id: request.context_id.to_string(),
            }
        })?;
        context.head_turn_id = turn.turn_id;
        context.head_depth = turn.depth;

        Ok(BinaryAppendTurnResponse {
            context_id: turn.context_id,
            new_turn_id: turn.turn_id,
            new_depth: turn.depth,
            content_hash,
        })
    }

    async fn get_head(&self, context_id: u64) -> Result<BinaryContextHead, CxdbClientError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mock backend mutex poisoned".to_string()))?;

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
                "mock backend requires include_payload=true".to_string(),
            ));
        }

        let state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mock backend mutex poisoned".to_string()))?;

        let context = state
            .contexts
            .get(&context_id)
            .ok_or_else(|| CxdbClientError::NotFound {
                resource: "context",
                id: context_id.to_string(),
            })?;

        let mut turns = Vec::new();
        let mut cursor = context.head_turn_id;
        while cursor != 0 && turns.len() < limit {
            let turn = state
                .turns
                .get(&cursor)
                .ok_or_else(|| CxdbClientError::Backend("mock turn chain corrupted".to_string()))?;
            turns.push(turn.clone());
            cursor = turn.parent_turn_id;
        }

        turns.reverse();
        Ok(turns)
    }

    async fn put_blob(&self, raw_bytes: &[u8]) -> Result<String, CxdbClientError> {
        let hash = blake3::hash(raw_bytes).to_hex().to_string();
        let mut state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mock backend mutex poisoned".to_string()))?;
        state.blobs.insert(hash.clone(), raw_bytes.to_vec());
        Ok(hash)
    }

    async fn get_blob(&self, content_hash: &String) -> Result<Option<Vec<u8>>, CxdbClientError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mock backend mutex poisoned".to_string()))?;
        Ok(state.blobs.get(content_hash).cloned())
    }

    async fn attach_fs(&self, turn_id: u64, fs_root_hash: &String) -> Result<(), CxdbClientError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mock backend mutex poisoned".to_string()))?;
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
        let state = self
            .inner
            .lock()
            .map_err(|_| CxdbClientError::Backend("mock backend mutex poisoned".to_string()))?;

        let context = state
            .contexts
            .get(&context_id)
            .ok_or_else(|| CxdbClientError::NotFound {
                resource: "context",
                id: context_id.to_string(),
            })?;

        let mut turns = Vec::new();
        let mut cursor = before_turn_id
            .and_then(|before| state.turns.get(&before).map(|turn| turn.parent_turn_id))
            .unwrap_or(context.head_turn_id);

        while cursor != 0 && turns.len() < limit {
            let turn = state
                .turns
                .get(&cursor)
                .ok_or_else(|| CxdbClientError::Backend("mock turn chain corrupted".to_string()))?;
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
