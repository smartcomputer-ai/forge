//! Runtime-owned environment and VFS context projection snapshots.

use std::collections::BTreeSet;

use engine::{
    BlobRef, ContextEntryInput, ContextEntryKey, ContextEntryKind, CoreAgentCommand,
    CoreAgentState, VFS_CATALOG_CONTEXT_KEY, WorkspaceLinkAccess, WorkspaceLinkTarget,
    storage::{BlobGraphStore, BlobStore, BlobStoreError, record_contains_edges},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use vfs::{ResolvedWorkspaceLink, ResolvedWorkspaceLinkTarget};

use crate::fs::FsPath;

pub const VFS_CATALOG_SCHEMA_VERSION: &str = "lightspeed.environment.vfs_catalog.v1";
pub const ENVIRONMENT_PROJECTION_MEDIA_TYPE: &str = "application/json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VfsCatalog {
    pub schema_version: String,
    pub revision: u64,
    pub routes: Vec<FsRoute>,
}

impl VfsCatalog {
    pub fn new(revision: u64, routes: Vec<FsRoute>) -> Self {
        Self {
            schema_version: VFS_CATALOG_SCHEMA_VERSION.to_owned(),
            revision,
            routes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsRoute {
    pub path: FsPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<FsPath>,
    pub access: FsRouteAccess,
    pub source: FsRouteSource,
    pub availability: FsRouteAvailability,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FsRouteAvailability {
    Available,
    Unavailable { reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsRouteAccess {
    ReadOnly,
    ReadWrite,
}

impl From<WorkspaceLinkAccess> for FsRouteAccess {
    fn from(value: WorkspaceLinkAccess) -> Self {
        match value {
            WorkspaceLinkAccess::ReadOnly => Self::ReadOnly,
            WorkspaceLinkAccess::ReadWrite => Self::ReadWrite,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FsRouteSource {
    VfsSnapshot { snapshot_ref: BlobRef },
    VfsWorkspace { workspace_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentProjectionPublication<T> {
    pub snapshot_ref: BlobRef,
    pub snapshot: T,
    pub snapshot_bytes: Vec<u8>,
    pub command: Option<CoreAgentCommand>,
}

#[derive(Debug, Error)]
pub enum EnvironmentProjectionError {
    #[error(transparent)]
    BlobStore(#[from] BlobStoreError),

    #[error("failed to encode environment projection: {message}")]
    Encode { message: String },

    #[error("invalid environment projection path {path}: {message}")]
    InvalidPath { path: String, message: String },
}

pub async fn prepare_vfs_catalog_publication(
    blobs: &dyn BlobStore,
    blob_graph: Option<&dyn BlobGraphStore>,
    state: &CoreAgentState,
    catalog: VfsCatalog,
) -> Result<EnvironmentProjectionPublication<VfsCatalog>, EnvironmentProjectionError> {
    prepare_projection_publication(
        blobs,
        blob_graph,
        state,
        catalog,
        VFS_CATALOG_CONTEXT_KEY,
        vfs_catalog_context_input,
        vfs_catalog_blob_refs,
    )
    .await
}

/// Every snapshot manifest a catalog routes to. The catalog write records one
/// `contains` edge per ref so linked snapshots stay reachable while the
/// catalog is.
pub fn vfs_catalog_blob_refs(catalog: &VfsCatalog) -> BTreeSet<BlobRef> {
    catalog
        .routes
        .iter()
        .filter_map(|route| match &route.source {
            FsRouteSource::VfsSnapshot { snapshot_ref } => Some(snapshot_ref.clone()),
            FsRouteSource::VfsWorkspace { .. } => None,
        })
        .collect()
}

pub fn vfs_catalog_from_workspace_links(
    links: &[ResolvedWorkspaceLink],
) -> Result<VfsCatalog, EnvironmentProjectionError> {
    let mut routes = links
        .iter()
        .map(fs_route_from_workspace_link)
        .collect::<Result<Vec<_>, _>>()?;
    routes.sort_by(|left, right| left.path.cmp(&right.path));
    let revision = stable_revision(&encode_json(&routes)?);
    Ok(VfsCatalog::new(revision, routes))
}

pub fn vfs_catalog_context_input(catalog_ref: BlobRef) -> ContextEntryInput {
    projection_context_input(ContextEntryKind::VfsCatalog, catalog_ref, "VFS catalog")
}

pub fn current_vfs_catalog_ref(state: &CoreAgentState) -> Option<BlobRef> {
    current_context_ref(state, VFS_CATALOG_CONTEXT_KEY, ContextEntryKind::VfsCatalog)
}

async fn prepare_projection_publication<T>(
    blobs: &dyn BlobStore,
    blob_graph: Option<&dyn BlobGraphStore>,
    state: &CoreAgentState,
    snapshot: T,
    key: &'static str,
    context_input: fn(BlobRef) -> ContextEntryInput,
    embedded_refs: fn(&T) -> BTreeSet<BlobRef>,
) -> Result<EnvironmentProjectionPublication<T>, EnvironmentProjectionError>
where
    T: Clone + PartialEq + Serialize,
{
    let snapshot_bytes = encode_json(&snapshot)?;
    let snapshot_ref = blobs.put_bytes(snapshot_bytes.clone()).await?;
    record_contains_edges(blob_graph, &snapshot_ref, embedded_refs(&snapshot)).await?;
    let command = if current_key_ref(state, key).as_ref() == Some(&snapshot_ref) {
        None
    } else {
        Some(CoreAgentCommand::UpsertContext {
            expected_revision: None,
            key: ContextEntryKey::new(key),
            entry: context_input(snapshot_ref.clone()),
        })
    };

    Ok(EnvironmentProjectionPublication {
        snapshot_ref,
        snapshot,
        snapshot_bytes,
        command,
    })
}

fn fs_route_from_workspace_link(
    link: &ResolvedWorkspaceLink,
) -> Result<FsRoute, EnvironmentProjectionError> {
    let path = FsPath::new(link.path.as_str()).map_err(|error| {
        EnvironmentProjectionError::InvalidPath {
            path: link.path.as_str().to_owned(),
            message: error.to_string(),
        }
    })?;
    let (source, availability) = match &link.target {
        ResolvedWorkspaceLinkTarget::AvailableSnapshot { snapshot_ref } => (
            FsRouteSource::VfsSnapshot {
                snapshot_ref: snapshot_ref.clone(),
            },
            FsRouteAvailability::Available,
        ),
        ResolvedWorkspaceLinkTarget::AvailableWorkspace { workspace } => (
            FsRouteSource::VfsWorkspace {
                workspace_id: workspace.workspace_id.as_str().to_owned(),
            },
            FsRouteAvailability::Available,
        ),
        ResolvedWorkspaceLinkTarget::Unavailable {
            declared_target,
            reason,
        } => {
            let source = match declared_target {
                WorkspaceLinkTarget::Snapshot { snapshot_ref } => FsRouteSource::VfsSnapshot {
                    snapshot_ref: BlobRef::parse(snapshot_ref.clone()).map_err(|error| {
                        EnvironmentProjectionError::Encode {
                            message: error.to_string(),
                        }
                    })?,
                },
                WorkspaceLinkTarget::Workspace { workspace_id } => FsRouteSource::VfsWorkspace {
                    workspace_id: workspace_id.clone(),
                },
            };
            (
                source,
                FsRouteAvailability::Unavailable {
                    reason: reason.clone(),
                },
            )
        }
    };
    Ok(FsRoute {
        path,
        source_path: None,
        access: link.access.into(),
        source,
        availability,
    })
}

fn projection_context_input(
    kind: ContextEntryKind,
    content_ref: BlobRef,
    preview: &'static str,
) -> ContextEntryInput {
    ContextEntryInput {
        kind,
        content: engine::ContentRef {
            content_ref,
            media_type: Some(ENVIRONMENT_PROJECTION_MEDIA_TYPE.to_owned()),
            provider_kind: None,
        },
        preview: Some(preview.to_owned()),
        origin: None,
        provenance_ref: None,
        token_estimate: None,
    }
}

fn current_context_ref(
    state: &CoreAgentState,
    key: &'static str,
    kind: ContextEntryKind,
) -> Option<BlobRef> {
    state
        .context
        .entries
        .iter()
        .rev()
        .find(|entry| {
            entry
                .key
                .as_ref()
                .is_some_and(|entry_key| entry_key.as_str() == key)
                && entry.kind == kind
        })
        .map(|entry| entry.content.content_ref.clone())
}

fn current_key_ref(state: &CoreAgentState, key: &'static str) -> Option<BlobRef> {
    state
        .context
        .entries
        .iter()
        .rev()
        .find(|entry| {
            entry
                .key
                .as_ref()
                .is_some_and(|entry_key| entry_key.as_str() == key)
        })
        .map(|entry| entry.content.content_ref.clone())
}

fn encode_json<T: Serialize>(value: &T) -> Result<Vec<u8>, EnvironmentProjectionError> {
    serde_json::to_vec(value).map_err(|error| EnvironmentProjectionError::Encode {
        message: error.to_string(),
    })
}

fn stable_revision(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use engine::{WorkspaceLinkAccess, storage::InMemoryBlobStore};
    use vfs::{ResolvedWorkspaceLink, ResolvedWorkspaceLinkTarget, VfsPath, VfsWorkspaceId};

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn vfs_catalog_publication_skips_unchanged_catalog() {
        let blobs = InMemoryBlobStore::new();
        let catalog = VfsCatalog::new(0, Vec::new());
        let state = CoreAgentState::new();

        let first = prepare_vfs_catalog_publication(&blobs, None, &state, catalog.clone())
            .await
            .expect("first publication");
        assert!(first.command.is_some());

        let mut state = CoreAgentState::new();
        state.context.entries = vec![engine::ContextEntry {
            entry_id: engine::ContextEntryId::new(1),
            key: Some(ContextEntryKey::new(VFS_CATALOG_CONTEXT_KEY)),
            kind: ContextEntryKind::VfsCatalog,
            source: engine::ContextEntrySource::Runtime {
                label: "environment.projection".to_owned(),
            },
            content: engine::ContentRef {
                content_ref: first.snapshot_ref.clone(),
                media_type: Some(ENVIRONMENT_PROJECTION_MEDIA_TYPE.to_owned()),
                provider_kind: None,
            },
            preview: Some("VFS catalog".to_owned()),
            origin: None,
            provenance_ref: None,
            token_estimate: None,
            supersedes: None,
        }];

        let second = prepare_vfs_catalog_publication(&blobs, None, &state, catalog)
            .await
            .expect("second publication");
        assert!(second.command.is_none());
    }

    /// The recorded edge set must equal every ref the projection embeds:
    /// the snapshot manifests its routes point at.
    #[tokio::test(flavor = "current_thread")]
    async fn projection_writes_record_an_edge_for_every_embedded_ref() {
        let blobs = InMemoryBlobStore::new();
        let snapshot_ref = engine::storage::BlobStore::put_bytes(&blobs, b"snapshot".to_vec())
            .await
            .expect("put snapshot");
        let catalog = VfsCatalog::new(
            7,
            vec![
                FsRoute {
                    path: FsPath::new("/docs").expect("path"),
                    source_path: None,
                    access: FsRouteAccess::ReadOnly,
                    source: FsRouteSource::VfsSnapshot {
                        snapshot_ref: snapshot_ref.clone(),
                    },
                    availability: FsRouteAvailability::Available,
                },
                FsRoute {
                    path: FsPath::new("/workspace").expect("path"),
                    source_path: None,
                    access: FsRouteAccess::ReadWrite,
                    source: FsRouteSource::VfsWorkspace {
                        workspace_id: "workspace_1".to_owned(),
                    },
                    availability: FsRouteAvailability::Available,
                },
            ],
        );
        let publication =
            prepare_vfs_catalog_publication(&blobs, Some(&blobs), &CoreAgentState::new(), catalog)
                .await
                .expect("publication");

        let embedded = engine::storage::collect_blob_refs(
            &serde_json::from_slice(&publication.snapshot_bytes).expect("catalog json"),
        );
        let recorded: BTreeSet<BlobRef> = blobs
            .edges()
            .into_iter()
            .inspect(|edge| assert_eq!(edge.parent, publication.snapshot_ref))
            .map(|edge| edge.child)
            .collect();
        assert_eq!(recorded, embedded);
        assert_eq!(recorded, vfs_catalog_blob_refs(&publication.snapshot));
        assert_eq!(embedded, BTreeSet::from([snapshot_ref]));
    }

    #[test]
    fn vfs_catalog_from_workspace_links_projects_routes() {
        let link = workspace_link();

        let catalog = vfs_catalog_from_workspace_links(&[link]).expect("catalog");

        assert_ne!(catalog.revision, 0);
        assert_eq!(catalog.routes.len(), 1);
        assert_eq!(catalog.routes[0].path.as_str(), "/workspace");
        assert_eq!(catalog.routes[0].access, FsRouteAccess::ReadWrite);
        assert!(matches!(
            catalog.routes[0].source,
            FsRouteSource::VfsWorkspace { .. }
        ));
    }

    fn workspace_link() -> ResolvedWorkspaceLink {
        ResolvedWorkspaceLink {
            path: VfsPath::parse("/workspace").expect("link path"),
            target: ResolvedWorkspaceLinkTarget::AvailableWorkspace {
                workspace: vfs::VfsWorkspaceRecord {
                    workspace_id: VfsWorkspaceId::new("workspace_1"),
                    display_name: None,
                    base_snapshot_ref: None,
                    head_snapshot_ref: BlobRef::from_bytes(b"head"),
                    head_totals: vfs::VfsTotals::default(),
                    revision: 0,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            },
            access: WorkspaceLinkAccess::ReadWrite,
        }
    }
}
