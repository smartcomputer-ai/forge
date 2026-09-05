use engine::{BlobRef, storage::BlobStoreError};
use futures_util::TryStreamExt as _;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload, path::Path as ObjectPath};
use uuid::Uuid;

use crate::{
    PgStore, PgStoreConfig,
    shared::{object_store_error, sha256_hex},
};

impl PgStore {
    pub(crate) async fn put_object(
        &self,
        key: &str,
        bytes: Vec<u8>,
    ) -> Result<object_store::PutResult, BlobStoreError> {
        let object_store = self
            .object_store
            .as_ref()
            .ok_or_else(|| BlobStoreError::Store {
                message: format!(
                    "blob exceeds inline threshold ({} bytes) but no object store is configured",
                    self.config.inline_threshold_bytes
                ),
            })?;
        object_store
            .put(&ObjectPath::from(key), PutPayload::from(bytes))
            .await
            .map_err(|error| object_store_error("put object", key, error))
    }

    pub(crate) async fn get_object(
        &self,
        key: &str,
        blob_ref: &BlobRef,
    ) -> Result<Vec<u8>, BlobStoreError> {
        let object_store = self
            .object_store
            .as_ref()
            .ok_or_else(|| BlobStoreError::Store {
                message: format!(
                    "blob '{blob_ref}' is object-backed but no object store is configured"
                ),
            })?;
        match object_store.get(&ObjectPath::from(key)).await {
            Ok(result) => result
                .bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(|error| object_store_error("read object body", key, error)),
            Err(object_store::Error::NotFound { .. }) => Err(BlobStoreError::NotFound {
                blob_ref: blob_ref.clone(),
            }),
            Err(error) => Err(object_store_error("get object", key, error)),
        }
    }
}

pub(crate) fn direct_blob_key(
    config: &PgStoreConfig,
    blob_ref: &BlobRef,
) -> Result<String, BlobStoreError> {
    let digest = sha256_hex(blob_ref)?;
    let prefix = &digest[..2];
    Ok(prefixed_key(
        config,
        &format!(
            "universes/{}/cas/blobs/sha256/{prefix}/{digest}/{}.bin",
            config.universe_id,
            Uuid::new_v4().simple()
        ),
    ))
}

fn prefixed_key(config: &PgStoreConfig, suffix: &str) -> String {
    prefix_key(&config.object_prefix, suffix)
}

fn prefix_key(object_prefix: &str, suffix: &str) -> String {
    let prefix = object_prefix.trim_matches('/');
    if prefix.is_empty() {
        suffix.to_owned()
    } else {
        format!("{prefix}/{suffix}")
    }
}

/// Object-store prefix under which every CAS object of one universe lives,
/// including the deployment's configured object prefix. Universe deletion
/// clears it wholesale so objects whose rows were already swept, or whose
/// deletion failed after the row went, do not outlive the universe.
pub fn universe_cas_object_prefix(object_prefix: &str, universe_id: Uuid) -> String {
    prefix_key(object_prefix, &format!("universes/{universe_id}/cas/"))
}

/// Delete every object under `prefix`; returns how many were removed.
pub async fn delete_objects_under_prefix(
    object_store: &dyn ObjectStore,
    prefix: &str,
) -> Result<u64, object_store::Error> {
    let prefix = ObjectPath::from(prefix.trim_end_matches('/'));
    let mut listing = object_store.list(Some(&prefix));
    let mut deleted = 0u64;
    while let Some(meta) = listing.try_next().await? {
        match object_store.delete(&meta.location).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => deleted += 1,
            Err(error) => return Err(error),
        }
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PgStoreConfig;
    use uuid::Uuid;

    #[test]
    fn universe_cas_prefix_contains_every_direct_blob_key() {
        let universe_id = Uuid::new_v4();
        let config = PgStoreConfig::new(universe_id)
            .with_inline_threshold_bytes(8)
            .with_object_prefix("/prefix/");
        let key = direct_blob_key(&config, &BlobRef::from_bytes(b"hello")).expect("blob key");
        let prefix = universe_cas_object_prefix("/prefix/", universe_id);
        assert_eq!(prefix, format!("prefix/universes/{universe_id}/cas/"));
        assert!(key.starts_with(&prefix));
        assert_eq!(
            universe_cas_object_prefix("", universe_id),
            format!("universes/{universe_id}/cas/")
        );
    }

    #[test]
    fn direct_blob_keys_are_scoped_by_universe() {
        let config = PgStoreConfig::new(Uuid::new_v4())
            .with_inline_threshold_bytes(8)
            .with_object_prefix("prefix");
        let blob_ref = BlobRef::from_bytes(b"hello");
        let key = direct_blob_key(&config, &blob_ref).expect("blob key");

        assert!(key.starts_with(&format!(
            "prefix/universes/{}/cas/blobs/sha256/",
            config.universe_id
        )));
        assert!(key.ends_with(".bin"));
    }
}
