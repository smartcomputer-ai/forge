use crate::memory::{MemoryState, MemoryTurnStore};
use crate::store::{ArtifactStore, TurnStore, TurnStoreError, TurnStoreResult, TypedTurnStore};
use crate::types::{
    AppendTurnRequest, BlobHash, ContextId, RegistryBundle, StoreContext, StoredTurn,
    StoredTurnRef, TurnId,
};
use std::fs;
use std::path::{Path, PathBuf};

const STATE_FILE_NAME: &str = "turnstore-state.json";

#[derive(Clone, Debug)]
pub struct FsTurnStore {
    state_file: PathBuf,
    inner: MemoryTurnStore,
}

impl FsTurnStore {
    pub fn new<P: AsRef<Path>>(root: P) -> TurnStoreResult<Self> {
        fs::create_dir_all(root.as_ref()).map_err(|err| {
            TurnStoreError::Backend(format!("create fs store root failed: {err}"))
        })?;
        let state_file = root.as_ref().join(STATE_FILE_NAME);
        let state = if state_file.exists() {
            let raw = fs::read(&state_file)
                .map_err(|err| TurnStoreError::Backend(format!("read state file failed: {err}")))?;
            serde_json::from_slice::<MemoryState>(&raw)
                .map_err(|err| TurnStoreError::Serialization(err.to_string()))?
        } else {
            MemoryState::default()
        };

        Ok(Self {
            state_file,
            inner: MemoryTurnStore::from_state(state),
        })
    }

    fn persist(&self) -> TurnStoreResult<()> {
        let snapshot = self.inner.snapshot();
        let raw = serde_json::to_vec_pretty(&snapshot)
            .map_err(|err| TurnStoreError::Serialization(err.to_string()))?;
        let tmp = self.state_file.with_extension("json.tmp");
        fs::write(&tmp, raw)
            .map_err(|err| TurnStoreError::Backend(format!("write state file failed: {err}")))?;
        fs::rename(&tmp, &self.state_file)
            .map_err(|err| TurnStoreError::Backend(format!("rename state file failed: {err}")))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl TurnStore for FsTurnStore {
    async fn create_context(&self, base_turn_id: Option<TurnId>) -> TurnStoreResult<StoreContext> {
        let created = self.inner.create_context(base_turn_id).await?;
        self.persist()?;
        Ok(created)
    }

    async fn append_turn(&self, request: AppendTurnRequest) -> TurnStoreResult<StoredTurn> {
        let turn = self.inner.append_turn(request).await?;
        self.persist()?;
        Ok(turn)
    }

    async fn fork_context(&self, from_turn_id: TurnId) -> TurnStoreResult<StoreContext> {
        let context = self.inner.fork_context(from_turn_id).await?;
        self.persist()?;
        Ok(context)
    }

    async fn get_head(&self, context_id: &ContextId) -> TurnStoreResult<StoredTurnRef> {
        self.inner.get_head(context_id).await
    }

    async fn list_turns(
        &self,
        context_id: &ContextId,
        before_turn_id: Option<&TurnId>,
        limit: usize,
    ) -> TurnStoreResult<Vec<StoredTurn>> {
        self.inner
            .list_turns(context_id, before_turn_id, limit)
            .await
    }
}

#[async_trait::async_trait]
impl TypedTurnStore for FsTurnStore {
    async fn publish_registry_bundle(&self, bundle: RegistryBundle) -> TurnStoreResult<()> {
        self.inner.publish_registry_bundle(bundle).await?;
        self.persist()
    }

    async fn get_registry_bundle(&self, bundle_id: &str) -> TurnStoreResult<Option<Vec<u8>>> {
        self.inner.get_registry_bundle(bundle_id).await
    }
}

#[async_trait::async_trait]
impl ArtifactStore for FsTurnStore {
    async fn put_blob(&self, raw_bytes: &[u8]) -> TurnStoreResult<BlobHash> {
        let hash = self.inner.put_blob(raw_bytes).await?;
        self.persist()?;
        Ok(hash)
    }

    async fn get_blob(&self, content_hash: &BlobHash) -> TurnStoreResult<Option<Vec<u8>>> {
        self.inner.get_blob(content_hash).await
    }

    async fn attach_fs(&self, turn_id: &TurnId, fs_root_hash: &BlobHash) -> TurnStoreResult<()> {
        self.inner.attach_fs(turn_id, fs_root_hash).await?;
        self.persist()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn fs_store_reopen_restores_previous_head() {
        let tmp = tempfile::tempdir().expect("tempdir should be created");
        let store = FsTurnStore::new(tmp.path()).expect("fs store should initialize");

        let context = store
            .create_context(None)
            .await
            .expect("context should be created");
        let appended = store
            .append_turn(AppendTurnRequest {
                context_id: context.context_id.clone(),
                parent_turn_id: None,
                type_id: "forge.agent.user_turn".to_string(),
                type_version: 1,
                payload: b"state".to_vec(),
                idempotency_key: "k1".to_string(),
            })
            .await
            .expect("append should succeed");
        drop(store);

        let reopened = FsTurnStore::new(tmp.path()).expect("fs store should reopen");
        let head = reopened
            .get_head(&context.context_id)
            .await
            .expect("head lookup should succeed");
        assert_eq!(head.turn_id, appended.turn_id);
    }
}
