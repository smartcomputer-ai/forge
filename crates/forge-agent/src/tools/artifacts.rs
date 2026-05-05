//! Minimal artifact access used by tool handlers and dispatch validation.

use crate::refs::ArtifactRef;
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ArtifactStoreError {
    #[error("artifact not found: {uri}")]
    NotFound { uri: String },

    #[error("artifact store failure: {message}")]
    Store { message: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolArtifactWrite {
    pub uri_hint: Option<String>,
    pub bytes: Vec<u8>,
    pub media_type: Option<String>,
    pub preview: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[async_trait]
pub trait ToolArtifactStore: Send + Sync {
    async fn read_bytes(&self, artifact_ref: &ArtifactRef) -> Result<Vec<u8>, ArtifactStoreError>;

    async fn write_bytes(
        &self,
        artifact: ToolArtifactWrite,
    ) -> Result<ArtifactRef, ArtifactStoreError>;

    async fn read_text(&self, artifact_ref: &ArtifactRef) -> Result<String, ArtifactStoreError> {
        let bytes = self.read_bytes(artifact_ref).await?;
        String::from_utf8(bytes).map_err(|error| ArtifactStoreError::Store {
            message: format!(
                "artifact '{}' is not valid UTF-8: {error}",
                artifact_ref.uri
            ),
        })
    }
}

#[derive(Clone, Default)]
pub struct InMemoryToolArtifactStore {
    inner: Arc<RwLock<InMemoryToolArtifactStoreInner>>,
}

#[derive(Default)]
struct InMemoryToolArtifactStoreInner {
    next_seq: u64,
    bytes_by_uri: BTreeMap<String, Vec<u8>>,
    refs_by_uri: BTreeMap<String, ArtifactRef>,
}

impl InMemoryToolArtifactStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_text(&self, uri: impl Into<String>, text: impl Into<String>) -> ArtifactRef {
        let uri = uri.into();
        let text = text.into();
        let artifact_ref =
            ArtifactRef::new(uri.clone()).with_preview(text.chars().take(120).collect::<String>());
        let mut inner = self.inner.write().expect("artifact store lock poisoned");
        inner.bytes_by_uri.insert(uri.clone(), text.into_bytes());
        inner.refs_by_uri.insert(uri, artifact_ref.clone());
        artifact_ref
    }
}

#[async_trait]
impl ToolArtifactStore for InMemoryToolArtifactStore {
    async fn read_bytes(&self, artifact_ref: &ArtifactRef) -> Result<Vec<u8>, ArtifactStoreError> {
        self.inner
            .read()
            .expect("artifact store lock poisoned")
            .bytes_by_uri
            .get(&artifact_ref.uri)
            .cloned()
            .ok_or_else(|| ArtifactStoreError::NotFound {
                uri: artifact_ref.uri.clone(),
            })
    }

    async fn write_bytes(
        &self,
        artifact: ToolArtifactWrite,
    ) -> Result<ArtifactRef, ArtifactStoreError> {
        let mut inner = self.inner.write().expect("artifact store lock poisoned");
        inner.next_seq = inner.next_seq.saturating_add(1);
        let uri = artifact
            .uri_hint
            .unwrap_or_else(|| format!("mem://tool-artifact/{}", inner.next_seq));
        let mut artifact_ref = ArtifactRef::new(uri.clone());
        artifact_ref.media_type = artifact.media_type;
        artifact_ref.byte_len = Some(artifact.bytes.len() as u64);
        artifact_ref.preview = artifact.preview;
        artifact_ref.metadata = artifact.metadata;
        inner.bytes_by_uri.insert(uri.clone(), artifact.bytes);
        inner.refs_by_uri.insert(uri, artifact_ref.clone());
        Ok(artifact_ref)
    }
}
