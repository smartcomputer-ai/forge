use async_trait::async_trait;
use engine::{
    BlobRef,
    storage::{BlobEdge, BlobGraphStore, BlobInfo, BlobStore, BlobStoreError},
};
use sqlx::Row;

use crate::{
    PgStore,
    object::direct_blob_key,
    shared::{
        blob_sql_error, blob_store_error, i64_to_u64, sha256_hex, unix_now_ms, usize_to_blob_i64,
    },
};

impl PgStore {
    /// Touch-or-insert. Existing content only moves `touched_at_ms` forward;
    /// bytes are never rewritten and objects never re-uploaded. The touch is
    /// what keeps a deduplicated write safe against a concurrent sweep: the
    /// sweep only considers blobs untouched for longer than its grace, so a
    /// ref handed out here cannot dangle before its holder commits.
    async fn put_single_blob(&self, bytes: Vec<u8>) -> Result<BlobRef, BlobStoreError> {
        let blob_ref = BlobRef::from_bytes(&bytes);
        let digest = sha256_hex(&blob_ref)?;
        let byte_len = usize_to_blob_i64(bytes.len(), "blob byte length")?;
        let now_ms = unix_now_ms();
        // Write-through: the ref derives from these exact bytes, so the
        // hash-verified invariant of the cache holds by construction.
        if let Some(cache) = &self.blob_cache {
            cache.insert(self.config.universe_id, &blob_ref, &bytes);
        }

        let touched = sqlx::query(
            r#"
            UPDATE cas_blobs
            SET touched_at_ms = GREATEST(touched_at_ms, $3)
            WHERE universe_id = $1 AND digest = $2
            RETURNING 1
            "#,
        )
        .bind(self.config.universe_id)
        .bind(digest)
        .bind(now_ms)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| blob_sql_error("touch blob", error))?;
        if touched.is_some() {
            return Ok(blob_ref);
        }

        self.ensure_universe()
            .await
            .map_err(|error| blob_store_error("ensure universe", error))?;

        if bytes.len() <= self.config.inline_threshold_bytes {
            sqlx::query(
                r#"
                INSERT INTO cas_blobs (
                    universe_id,
                    digest,
                    byte_len,
                    storage_kind,
                    inline_bytes,
                    created_at_ms,
                    touched_at_ms
                )
                VALUES ($1, $2, $3, 'inline', $4, $5, $5)
                ON CONFLICT (universe_id, digest) DO UPDATE
                SET touched_at_ms = GREATEST(cas_blobs.touched_at_ms, EXCLUDED.touched_at_ms)
                "#,
            )
            .bind(self.config.universe_id)
            .bind(digest)
            .bind(byte_len)
            .bind(bytes)
            .bind(now_ms)
            .execute(&self.pool)
            .await
            .map_err(|error| blob_sql_error("insert inline blob", error))?;
            return Ok(blob_ref);
        }

        let object_key = direct_blob_key(&self.config, &blob_ref)?;
        let put_result = self.put_object(&object_key, bytes).await?;
        // Two writers racing on a new digest both upload the same object
        // under the same key; the second insert only touches the row.
        sqlx::query(
            r#"
            INSERT INTO cas_blobs (
                universe_id,
                digest,
                byte_len,
                storage_kind,
                object_key,
                object_etag,
                object_version,
                created_at_ms,
                touched_at_ms
            )
            VALUES ($1, $2, $3, 'object', $4, $5, $6, $7, $7)
            ON CONFLICT (universe_id, digest) DO UPDATE
            SET touched_at_ms = GREATEST(cas_blobs.touched_at_ms, EXCLUDED.touched_at_ms)
            "#,
        )
        .bind(self.config.universe_id)
        .bind(digest)
        .bind(byte_len)
        .bind(object_key)
        .bind(put_result.e_tag)
        .bind(put_result.version)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(|error| blob_sql_error("insert object blob", error))?;
        Ok(blob_ref)
    }

    /// Catalog timestamps of one blob: `(created_at_ms, touched_at_ms)`.
    pub async fn blob_timestamps(
        &self,
        blob_ref: &BlobRef,
    ) -> Result<Option<(u64, u64)>, BlobStoreError> {
        let digest = sha256_hex(blob_ref)?;
        let row = sqlx::query(
            r#"
            SELECT created_at_ms, touched_at_ms
            FROM cas_blobs
            WHERE universe_id = $1 AND digest = $2
            "#,
        )
        .bind(self.config.universe_id)
        .bind(digest)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| blob_sql_error("load blob timestamps", error))?;
        row.map(|row| {
            let created_at_ms: i64 = row
                .try_get("created_at_ms")
                .map_err(|error| blob_sql_error("decode blob created time", error))?;
            let touched_at_ms: i64 = row
                .try_get("touched_at_ms")
                .map_err(|error| blob_sql_error("decode blob touched time", error))?;
            Ok((
                i64_to_u64(created_at_ms, "blob created time")
                    .map_err(|message| BlobStoreError::Store { message })?,
                i64_to_u64(touched_at_ms, "blob touched time")
                    .map_err(|message| BlobStoreError::Store { message })?,
            ))
        })
        .transpose()
    }
}

#[async_trait]
impl BlobStore for PgStore {
    async fn put_bytes(&self, bytes: Vec<u8>) -> Result<BlobRef, BlobStoreError> {
        self.put_single_blob(bytes).await
    }

    async fn put_many(&self, blobs: Vec<Vec<u8>>) -> Result<Vec<BlobRef>, BlobStoreError> {
        let mut refs = Vec::with_capacity(blobs.len());
        for bytes in blobs {
            refs.push(self.put_single_blob(bytes).await?);
        }
        Ok(refs)
    }

    async fn read_bytes(&self, blob_ref: &BlobRef) -> Result<Vec<u8>, BlobStoreError> {
        let digest = sha256_hex(blob_ref)?;
        if let Some(cache) = &self.blob_cache
            && let Some(bytes) = cache.get(self.config.universe_id, blob_ref)
        {
            return Ok(bytes.to_vec());
        }
        let row = sqlx::query(
            r#"
            SELECT storage_kind, inline_bytes, object_key
            FROM cas_blobs
            WHERE universe_id = $1 AND digest = $2
            "#,
        )
        .bind(self.config.universe_id)
        .bind(digest)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| blob_sql_error("load blob", error))?;

        let Some(row) = row else {
            return Err(BlobStoreError::NotFound {
                blob_ref: blob_ref.clone(),
            });
        };

        let storage_kind: String = row
            .try_get("storage_kind")
            .map_err(|error| blob_sql_error("decode blob storage kind", error))?;
        let bytes = match storage_kind.as_str() {
            "inline" => row
                .try_get::<Option<Vec<u8>>, _>("inline_bytes")
                .map_err(|error| blob_sql_error("decode inline blob bytes", error))?
                .ok_or_else(|| BlobStoreError::Store {
                    message: format!("inline blob '{blob_ref}' has no inline bytes"),
                })?,
            "object" => {
                let object_key = row
                    .try_get::<Option<String>, _>("object_key")
                    .map_err(|error| blob_sql_error("decode blob object key", error))?
                    .ok_or_else(|| BlobStoreError::Store {
                        message: format!("object blob '{blob_ref}' has no object key"),
                    })?;
                self.get_object(&object_key, blob_ref).await?
            }
            other => {
                return Err(BlobStoreError::Store {
                    message: format!("unsupported blob storage kind '{other}' for {blob_ref}"),
                });
            }
        };

        let actual = BlobRef::from_bytes(&bytes);
        if &actual != blob_ref {
            return Err(BlobStoreError::Store {
                message: format!("blob hash mismatch: expected {blob_ref}, got {actual}"),
            });
        }
        // Only hash-verified bytes enter the cache.
        if let Some(cache) = &self.blob_cache {
            cache.insert(self.config.universe_id, blob_ref, &bytes);
        }
        Ok(bytes)
    }

    async fn has_blob(&self, blob_ref: &BlobRef) -> Result<bool, BlobStoreError> {
        let digest = sha256_hex(blob_ref)?;
        sqlx::query(
            r#"
            SELECT 1
            FROM cas_blobs
            WHERE universe_id = $1 AND digest = $2
            "#,
        )
        .bind(self.config.universe_id)
        .bind(digest)
        .fetch_optional(&self.pool)
        .await
        .map(|row| row.is_some())
        .map_err(|error| blob_sql_error("check blob existence", error))
    }

    async fn stat_blob(&self, blob_ref: &BlobRef) -> Result<BlobInfo, BlobStoreError> {
        let digest = sha256_hex(blob_ref)?;
        let row = sqlx::query(
            r#"
            SELECT byte_len
            FROM cas_blobs
            WHERE universe_id = $1 AND digest = $2
            "#,
        )
        .bind(self.config.universe_id)
        .bind(digest)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| blob_sql_error("stat blob", error))?;

        let Some(row) = row else {
            return Err(BlobStoreError::NotFound {
                blob_ref: blob_ref.clone(),
            });
        };
        let byte_len = row
            .try_get::<i64, _>("byte_len")
            .map_err(|error| blob_sql_error("decode blob byte length", error))?;
        Ok(BlobInfo {
            blob_ref: blob_ref.clone(),
            byte_len: i64_to_u64(byte_len, "blob byte length")
                .map_err(|message| BlobStoreError::Store { message })?,
        })
    }
}

#[async_trait]
impl BlobGraphStore for PgStore {
    async fn record_blob_edges(&self, edges: Vec<BlobEdge>) -> Result<(), BlobStoreError> {
        if edges.is_empty() {
            return Ok(());
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| blob_sql_error("begin blob edges transaction", error))?;
        for edge in edges {
            if edge.edge_kind.is_empty() {
                return Err(BlobStoreError::Store {
                    message: "blob edge kind must not be empty".into(),
                });
            }
            sqlx::query(
                r#"
                INSERT INTO cas_blob_edges (
                    universe_id,
                    parent_digest,
                    child_digest,
                    edge_kind
                )
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (universe_id, parent_digest, child_digest, edge_kind) DO NOTHING
                "#,
            )
            .bind(self.config.universe_id)
            .bind(sha256_hex(&edge.parent)?)
            .bind(sha256_hex(&edge.child)?)
            .bind(edge.edge_kind)
            .execute(&mut *tx)
            .await
            .map_err(|error| blob_sql_error("record blob edge", error))?;
        }
        tx.commit()
            .await
            .map_err(|error| blob_sql_error("commit blob edges transaction", error))
    }
}
