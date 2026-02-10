use crate::types::{
    AppendTurnRequest, BlobHash, ContextId, RegistryBundle, StoreContext, StoredTurn,
    StoredTurnRef, TurnId,
};

#[derive(Debug, thiserror::Error)]
pub enum TurnStoreError {
    #[error("resource not found: {resource} ({id})")]
    NotFound { resource: &'static str, id: String },

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("unsupported operation: {0}")]
    Unsupported(String),

    #[error("serialization failed: {0}")]
    Serialization(String),

    #[error("backend failure: {0}")]
    Backend(String),
}

pub type TurnStoreResult<T> = Result<T, TurnStoreError>;

#[async_trait::async_trait]
pub trait TurnStore: Send + Sync {
    /// Cross-check:
    /// - CXDB binary protocol `CTX_CREATE` (`spec/cxdb/protocol.md`)
    async fn create_context(&self, base_turn_id: Option<TurnId>) -> TurnStoreResult<StoreContext>;

    /// Cross-check:
    /// - CXDB binary protocol `APPEND_TURN` (`spec/cxdb/protocol.md`)
    async fn append_turn(&self, request: AppendTurnRequest) -> TurnStoreResult<StoredTurn>;

    /// Cross-check:
    /// - CXDB binary protocol `CTX_FORK` (`spec/cxdb/protocol.md`)
    async fn fork_context(&self, from_turn_id: TurnId) -> TurnStoreResult<StoreContext>;

    /// Cross-check:
    /// - CXDB binary protocol `GET_HEAD` (`spec/cxdb/protocol.md`)
    async fn get_head(&self, context_id: &ContextId) -> TurnStoreResult<StoredTurnRef>;

    /// Cross-check:
    /// - CXDB binary protocol `GET_LAST` (`spec/cxdb/protocol.md`)
    /// - CXDB HTTP paging/projection (`spec/cxdb/http-api.md`)
    async fn list_turns(
        &self,
        context_id: &ContextId,
        before_turn_id: Option<&TurnId>,
        limit: usize,
    ) -> TurnStoreResult<Vec<StoredTurn>>;
}

#[async_trait::async_trait]
pub trait TypedTurnStore: TurnStore {
    /// Cross-check:
    /// - CXDB HTTP registry publish API (`spec/cxdb/http-api.md`)
    async fn publish_registry_bundle(&self, bundle: RegistryBundle) -> TurnStoreResult<()>;

    /// Cross-check:
    /// - CXDB HTTP registry read API (`spec/cxdb/http-api.md`)
    async fn get_registry_bundle(&self, bundle_id: &str) -> TurnStoreResult<Option<Vec<u8>>>;
}

#[async_trait::async_trait]
pub trait ArtifactStore: Send + Sync {
    /// Cross-check:
    /// - CXDB binary protocol `PUT_BLOB` (`spec/cxdb/protocol.md`)
    async fn put_blob(&self, raw_bytes: &[u8]) -> TurnStoreResult<BlobHash>;

    /// Cross-check:
    /// - CXDB binary protocol `GET_BLOB` (`spec/cxdb/protocol.md`)
    async fn get_blob(&self, content_hash: &BlobHash) -> TurnStoreResult<Option<Vec<u8>>>;

    /// Cross-check:
    /// - CXDB binary protocol `ATTACH_FS` (`spec/cxdb/protocol.md`)
    async fn attach_fs(&self, turn_id: &TurnId, fs_root_hash: &BlobHash) -> TurnStoreResult<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turnstore_error_not_found_expected_metadata() {
        let error = TurnStoreError::NotFound {
            resource: "context",
            id: "ctx-1".to_string(),
        };

        assert!(matches!(
            error,
            TurnStoreError::NotFound {
                resource: "context",
                ..
            }
        ));
        assert_eq!(error.to_string(), "resource not found: context (ctx-1)");
    }
}
