//! Content-addressed blob collection primitives.
//!
//! Liveness is reachability over rows that already exist, never reference
//! counting: a blob is live while a session event embeds it, a checkpoint, a
//! VFS snapshot or workspace head, or a bot event names it, an edge from
//! another blob points at it, or it is pinned. The sweep evaluates only one level of
//! edges: a child with any incoming edge is skipped, and deleting its parent
//! cascades the edge so the child becomes a candidate on a later pass.
//!
//! Every statement repeats the age guard and the full liveness predicate in
//! its `WHERE`, so a holder or a touch that appears between selecting
//! candidates and deleting them wins. Rows go before objects: a failure in
//! between leaves an unreadable object behind (a harmless leak), whereas the
//! reverse order would leave a row whose reads fail.

use engine::{BlobRef, storage::BlobStoreError};
use object_store::{ObjectStoreExt, path::Path as ObjectPath};
use sqlx::Row;
use thiserror::Error;

use crate::{
    PgStore,
    shared::{blob_sql_error, i64_to_u64, sha256_hex, usize_to_blob_i64},
};

/// One catalog row eligible for deletion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CasSweepCandidate {
    pub blob_ref: BlobRef,
    pub byte_len: u64,
    /// Set for object-backed blobs; the object is deleted after the row.
    pub object_key: Option<String>,
}

#[derive(Debug, Error)]
pub enum CasSweepError {
    /// The delete statement violated a foreign key from a holder table the
    /// liveness predicate does not cover. Nothing was deleted; the sweeper
    /// reports it and moves on rather than retrying.
    #[error("blob deletion conflicts with holder constraint {constraint}: {message}")]
    HolderConflict { constraint: String, message: String },

    #[error(transparent)]
    Blob(#[from] BlobStoreError),
}

/// Outcome of deleting object-store keys; failures are counted, not fatal,
/// because the rows are already gone and the objects are unreachable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CasObjectDeletion {
    pub deleted: usize,
    pub failures: Vec<(String, String)>,
}

/// `b` is the `cas_blobs` row under consideration. `$1` is the universe,
/// `$2` the age cutoff, `$3` the pinned digests.
const DEAD_BLOB_PREDICATE: &str = r#"
    b.universe_id = $1
    AND b.touched_at_ms < $2
    AND b.digest <> ALL($3::text[])
    AND NOT EXISTS (
        SELECT 1 FROM session_events AS e
        WHERE e.universe_id = b.universe_id
          AND e.blob_refs @> jsonb_build_array(b.blob_ref)
    )
    AND NOT EXISTS (
        SELECT 1 FROM session_checkpoints AS c
        WHERE c.universe_id = b.universe_id AND c.state_digest = b.digest
    )
    AND NOT EXISTS (
        SELECT 1 FROM vfs_snapshots AS s
        WHERE s.universe_id = b.universe_id AND s.digest = b.digest
    )
    AND NOT EXISTS (
        SELECT 1 FROM vfs_workspaces AS w
        WHERE w.universe_id = b.universe_id
          AND (w.head_snapshot_digest = b.digest OR w.base_snapshot_digest = b.digest)
    )
    AND NOT EXISTS (
        SELECT 1 FROM bot_events AS e
        WHERE e.universe_id = b.universe_id
          AND (e.document_ref = b.blob_ref OR e.prompt_ref = b.blob_ref)
    )
    AND NOT EXISTS (
        SELECT 1 FROM bot_events AS e
        WHERE e.universe_id = b.universe_id
          AND e.media_json @> jsonb_build_array(jsonb_build_object('blobRef', b.blob_ref))
    )
    AND NOT EXISTS (
        SELECT 1 FROM cas_blob_edges AS g
        WHERE g.universe_id = b.universe_id AND g.child_digest = b.digest
    )
"#;

impl PgStore {
    /// The oldest `limit` blobs of this universe that are dead and untouched
    /// since before `cutoff_ms`, excluding `pinned`.
    pub async fn list_sweep_candidates(
        &self,
        cutoff_ms: u64,
        pinned: &[BlobRef],
        limit: usize,
    ) -> Result<Vec<CasSweepCandidate>, CasSweepError> {
        let query = format!(
            r#"
            SELECT b.digest, b.byte_len, b.object_key
            FROM cas_blobs AS b
            WHERE {DEAD_BLOB_PREDICATE}
            ORDER BY b.touched_at_ms, b.digest
            LIMIT $4
            "#
        );
        let rows = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(cutoff_to_i64(cutoff_ms)?)
            .bind(digests(pinned)?)
            .bind(usize_to_blob_i64(limit, "sweep limit")?)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| blob_sql_error("list sweep candidates", error))?;
        rows.iter().map(candidate_from_row).collect()
    }

    /// Delete the catalog rows of `candidates` that are still dead and still
    /// older than `cutoff_ms` at deletion time. Returns what was deleted so
    /// the caller can remove the objects and account for the bytes.
    pub async fn delete_dead_blobs(
        &self,
        candidates: &[BlobRef],
        cutoff_ms: u64,
        pinned: &[BlobRef],
    ) -> Result<Vec<CasSweepCandidate>, CasSweepError> {
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        let query = format!(
            r#"
            DELETE FROM cas_blobs AS b
            WHERE b.digest = ANY($4::text[])
              AND {DEAD_BLOB_PREDICATE}
            RETURNING b.digest, b.byte_len, b.object_key
            "#
        );
        let rows = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(cutoff_to_i64(cutoff_ms)?)
            .bind(digests(pinned)?)
            .bind(digests(candidates)?)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| match holder_constraint(&error) {
                Some(constraint) => CasSweepError::HolderConflict {
                    constraint,
                    message: error.to_string(),
                },
                None => blob_sql_error("delete dead blobs", error).into(),
            })?;
        rows.iter().map(candidate_from_row).collect()
    }

    /// Remove object-store objects whose catalog rows are already gone. A
    /// missing object counts as deleted; other failures are collected.
    pub async fn delete_blob_objects(&self, keys: &[String]) -> CasObjectDeletion {
        let mut outcome = CasObjectDeletion::default();
        if keys.is_empty() {
            return outcome;
        }
        let Some(object_store) = self.object_store.as_ref() else {
            outcome.failures.extend(
                keys.iter()
                    .map(|key| (key.clone(), "no object store is configured".to_owned())),
            );
            return outcome;
        };
        for key in keys {
            match object_store.delete(&ObjectPath::from(key.as_str())).await {
                Ok(()) | Err(object_store::Error::NotFound { .. }) => outcome.deleted += 1,
                Err(error) => outcome.failures.push((key.clone(), error.to_string())),
            }
        }
        outcome
    }
}

fn candidate_from_row(row: &sqlx::postgres::PgRow) -> Result<CasSweepCandidate, CasSweepError> {
    let digest: String = row
        .try_get("digest")
        .map_err(|error| blob_sql_error("decode sweep digest", error))?;
    let byte_len: i64 = row
        .try_get("byte_len")
        .map_err(|error| blob_sql_error("decode sweep byte length", error))?;
    let object_key: Option<String> = row
        .try_get("object_key")
        .map_err(|error| blob_sql_error("decode sweep object key", error))?;
    Ok(CasSweepCandidate {
        blob_ref: BlobRef::parse(format!("sha256:{digest}")).map_err(|error| {
            BlobStoreError::Store {
                message: format!("decode sweep blob ref: {error}"),
            }
        })?,
        byte_len: i64_to_u64(byte_len, "sweep byte length")
            .map_err(|message| BlobStoreError::Store { message })?,
        object_key,
    })
}

fn digests(blob_refs: &[BlobRef]) -> Result<Vec<String>, CasSweepError> {
    blob_refs
        .iter()
        .map(|blob_ref| sha256_hex(blob_ref).map(str::to_owned))
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn cutoff_to_i64(cutoff_ms: u64) -> Result<i64, CasSweepError> {
    i64::try_from(cutoff_ms)
        .map_err(|_| BlobStoreError::Store {
            message: format!("sweep cutoff is too large for Postgres bigint: {cutoff_ms}"),
        })
        .map_err(Into::into)
}

/// The constraint name of a foreign-key violation (SQLSTATE 23503), if that
/// is what the error is.
fn holder_constraint(error: &sqlx::Error) -> Option<String> {
    let database_error = error.as_database_error()?;
    (database_error.code().as_deref() == Some("23503")).then(|| {
        database_error
            .constraint()
            .unwrap_or("<unknown>")
            .to_owned()
    })
}
