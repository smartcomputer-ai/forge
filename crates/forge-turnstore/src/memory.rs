use crate::store::{ArtifactStore, TurnStore, TurnStoreError, TurnStoreResult, TypedTurnStore};
use crate::types::{
    AppendTurnRequest, BlobHash, ContextId, RegistryBundle, StoreContext, StoredTurn,
    StoredTurnRef, TurnId,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct MemoryState {
    pub next_context_id: u64,
    pub next_turn_id: u64,
    pub contexts: BTreeMap<ContextId, ContextState>,
    pub turns: BTreeMap<TurnId, StoredTurn>,
    pub idempotency: BTreeMap<String, TurnId>,
    pub registry_bundles: BTreeMap<String, Vec<u8>>,
    pub blobs: BTreeMap<BlobHash, Vec<u8>>,
    pub turn_fs_roots: BTreeMap<TurnId, BlobHash>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct ContextState {
    pub head_turn_id: TurnId,
    pub head_depth: u32,
}

impl Default for ContextState {
    fn default() -> Self {
        Self {
            head_turn_id: "0".to_string(),
            head_depth: 0,
        }
    }
}

impl MemoryState {
    fn allocate_context_id(&mut self) -> ContextId {
        if self.next_context_id == 0 {
            self.next_context_id = 1;
        }
        let id = self.next_context_id;
        self.next_context_id += 1;
        id.to_string()
    }

    fn allocate_turn_id(&mut self) -> TurnId {
        if self.next_turn_id == 0 {
            self.next_turn_id = 1;
        }
        let id = self.next_turn_id;
        self.next_turn_id += 1;
        id.to_string()
    }

    fn turn_depth(&self, turn_id: &str) -> Option<u32> {
        self.turns.get(turn_id).map(|turn| turn.depth)
    }

    fn context_has_turn(&self, context: &ContextState, turn_id: &str) -> bool {
        if turn_id == "0" {
            return true;
        }
        let mut cursor = context.head_turn_id.as_str();
        while cursor != "0" {
            if cursor == turn_id {
                return true;
            }
            let Some(turn) = self.turns.get(cursor) else {
                return false;
            };
            cursor = turn.parent_turn_id.as_str();
        }
        false
    }

    fn content_hash(payload: &[u8]) -> BlobHash {
        blake3::hash(payload).to_hex().to_string()
    }
}

#[derive(Clone, Debug, Default)]
pub struct MemoryTurnStore {
    inner: Arc<Mutex<MemoryState>>,
}

impl MemoryTurnStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_state(state: MemoryState) -> Self {
        Self {
            inner: Arc::new(Mutex::new(state)),
        }
    }

    pub(crate) fn snapshot(&self) -> MemoryState {
        self.inner
            .lock()
            .expect("memory turnstore mutex poisoned")
            .clone()
    }
}

#[async_trait::async_trait]
impl TurnStore for MemoryTurnStore {
    async fn create_context(&self, base_turn_id: Option<TurnId>) -> TurnStoreResult<StoreContext> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| TurnStoreError::Backend("memory turnstore mutex poisoned".to_string()))?;

        let (head_turn_id, head_depth) = match base_turn_id {
            Some(turn_id) if turn_id != "0" => {
                let Some(depth) = state.turn_depth(&turn_id) else {
                    return Err(TurnStoreError::NotFound {
                        resource: "turn",
                        id: turn_id,
                    });
                };
                (turn_id, depth)
            }
            _ => ("0".to_string(), 0),
        };

        let context_id = state.allocate_context_id();
        state.contexts.insert(
            context_id.clone(),
            ContextState {
                head_turn_id: head_turn_id.clone(),
                head_depth,
            },
        );

        Ok(StoreContext {
            context_id,
            head_turn_id,
            head_depth,
        })
    }

    async fn append_turn(&self, request: AppendTurnRequest) -> TurnStoreResult<StoredTurn> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| TurnStoreError::Backend("memory turnstore mutex poisoned".to_string()))?;

        let context_snapshot = state
            .contexts
            .get(&request.context_id)
            .cloned()
            .ok_or_else(|| TurnStoreError::NotFound {
                resource: "context",
                id: request.context_id.clone(),
            })?;

        let key = if request.idempotency_key.is_empty() {
            None
        } else {
            Some(format!(
                "{}|{}",
                request.context_id, request.idempotency_key
            ))
        };

        if let Some(existing_key) = &key {
            if let Some(turn_id) = state.idempotency.get(existing_key) {
                if let Some(turn) = state.turns.get(turn_id) {
                    return Ok(turn.clone());
                }
            }
        }

        let parent_turn_id = request
            .parent_turn_id
            .clone()
            .unwrap_or_else(|| context_snapshot.head_turn_id.clone());

        let parent_depth = if parent_turn_id == "0" {
            0
        } else {
            state
                .turn_depth(&parent_turn_id)
                .ok_or_else(|| TurnStoreError::NotFound {
                    resource: "turn",
                    id: parent_turn_id.clone(),
                })?
        };

        let turn_id = state.allocate_turn_id();
        let turn = StoredTurn {
            context_id: request.context_id.clone(),
            turn_id: turn_id.clone(),
            parent_turn_id: parent_turn_id.clone(),
            depth: parent_depth + 1,
            type_id: request.type_id,
            type_version: request.type_version,
            payload: request.payload,
            idempotency_key: key
                .as_ref()
                .map(|_| request.idempotency_key)
                .filter(|value| !value.is_empty()),
            content_hash: None,
        };

        let mut turn = turn;
        turn.content_hash = Some(MemoryState::content_hash(&turn.payload));

        state.turns.insert(turn_id.clone(), turn.clone());
        if let Some(existing_key) = key {
            state.idempotency.insert(existing_key, turn_id.clone());
        }

        if let Some(context) = state.contexts.get_mut(&request.context_id) {
            context.head_turn_id = turn_id;
            context.head_depth = turn.depth;
        }

        Ok(turn)
    }

    async fn fork_context(&self, from_turn_id: TurnId) -> TurnStoreResult<StoreContext> {
        self.create_context(Some(from_turn_id)).await
    }

    async fn get_head(&self, context_id: &ContextId) -> TurnStoreResult<StoredTurnRef> {
        let state = self
            .inner
            .lock()
            .map_err(|_| TurnStoreError::Backend("memory turnstore mutex poisoned".to_string()))?;
        let context = state
            .contexts
            .get(context_id)
            .ok_or_else(|| TurnStoreError::NotFound {
                resource: "context",
                id: context_id.clone(),
            })?;
        Ok(StoredTurnRef {
            context_id: context_id.clone(),
            turn_id: context.head_turn_id.clone(),
            depth: context.head_depth,
        })
    }

    async fn list_turns(
        &self,
        context_id: &ContextId,
        before_turn_id: Option<&TurnId>,
        limit: usize,
    ) -> TurnStoreResult<Vec<StoredTurn>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let state = self
            .inner
            .lock()
            .map_err(|_| TurnStoreError::Backend("memory turnstore mutex poisoned".to_string()))?;
        let context = state
            .contexts
            .get(context_id)
            .ok_or_else(|| TurnStoreError::NotFound {
                resource: "context",
                id: context_id.clone(),
            })?;

        let mut cursor = if let Some(before) = before_turn_id {
            if before == "0" {
                return Ok(Vec::new());
            }
            if !state.context_has_turn(context, before) {
                return Err(TurnStoreError::InvalidInput(format!(
                    "turn {} is not reachable from context {} head",
                    before, context_id
                )));
            }
            let turn = state
                .turns
                .get(before)
                .ok_or_else(|| TurnStoreError::NotFound {
                    resource: "turn",
                    id: before.clone(),
                })?;
            turn.parent_turn_id.clone()
        } else {
            context.head_turn_id.clone()
        };

        let mut turns = Vec::new();
        while cursor != "0" && turns.len() < limit {
            let turn = state
                .turns
                .get(&cursor)
                .ok_or_else(|| TurnStoreError::NotFound {
                    resource: "turn",
                    id: cursor.clone(),
                })?;
            turns.push(turn.clone());
            cursor = turn.parent_turn_id.clone();
        }
        turns.reverse();
        Ok(turns)
    }
}

#[async_trait::async_trait]
impl TypedTurnStore for MemoryTurnStore {
    async fn publish_registry_bundle(&self, bundle: RegistryBundle) -> TurnStoreResult<()> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| TurnStoreError::Backend("memory turnstore mutex poisoned".to_string()))?;
        state
            .registry_bundles
            .insert(bundle.bundle_id, bundle.bundle_json);
        Ok(())
    }

    async fn get_registry_bundle(&self, bundle_id: &str) -> TurnStoreResult<Option<Vec<u8>>> {
        let state = self
            .inner
            .lock()
            .map_err(|_| TurnStoreError::Backend("memory turnstore mutex poisoned".to_string()))?;
        Ok(state.registry_bundles.get(bundle_id).cloned())
    }
}

#[async_trait::async_trait]
impl ArtifactStore for MemoryTurnStore {
    async fn put_blob(&self, raw_bytes: &[u8]) -> TurnStoreResult<BlobHash> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| TurnStoreError::Backend("memory turnstore mutex poisoned".to_string()))?;
        let hash = MemoryState::content_hash(raw_bytes);
        state
            .blobs
            .entry(hash.clone())
            .or_insert_with(|| raw_bytes.to_vec());
        Ok(hash)
    }

    async fn get_blob(&self, content_hash: &BlobHash) -> TurnStoreResult<Option<Vec<u8>>> {
        let state = self
            .inner
            .lock()
            .map_err(|_| TurnStoreError::Backend("memory turnstore mutex poisoned".to_string()))?;
        Ok(state.blobs.get(content_hash).cloned())
    }

    async fn attach_fs(&self, turn_id: &TurnId, fs_root_hash: &BlobHash) -> TurnStoreResult<()> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| TurnStoreError::Backend("memory turnstore mutex poisoned".to_string()))?;
        if !state.turns.contains_key(turn_id) {
            return Err(TurnStoreError::NotFound {
                resource: "turn",
                id: turn_id.clone(),
            });
        }
        if !state.blobs.contains_key(fs_root_hash) {
            return Err(TurnStoreError::NotFound {
                resource: "blob",
                id: fs_root_hash.clone(),
            });
        }
        state
            .turn_fs_roots
            .insert(turn_id.clone(), fs_root_hash.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn append_turn_with_same_idempotency_key_expected_single_turn() {
        let store = MemoryTurnStore::new();
        let context = store
            .create_context(None)
            .await
            .expect("context should be created");

        let request = AppendTurnRequest {
            context_id: context.context_id.clone(),
            parent_turn_id: None,
            type_id: "forge.agent.user_turn".to_string(),
            type_version: 1,
            payload: b"hello".to_vec(),
            idempotency_key: "k1".to_string(),
        };

        let first = store
            .append_turn(request.clone())
            .await
            .expect("append should succeed");
        let second = store
            .append_turn(request)
            .await
            .expect("idempotent append should succeed");

        assert_eq!(first.turn_id, second.turn_id);
    }
}
