//! Runtime-owned environment and VFS context projection snapshots.

use engine::{
    BlobRef, ContextEntryInput, ContextEntryKey, ContextEntryKind, CoreAgentCommand,
    CoreAgentState, ToolExecutionTarget, VFS_CATALOG_CONTEXT_KEY, WorkspaceLinkAccess,
    WorkspaceLinkTarget,
    storage::{BlobStore, BlobStoreError},
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
pub struct EnvironmentRecord {
    pub environment_id: String,
    pub kind: EnvironmentKind,
    pub capabilities: EnvironmentCapabilities,
    pub exec_target: Option<ToolExecutionTarget>,
    pub cwd: Option<FsPath>,
    pub status: EnvironmentStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentKind {
    Sandbox,
    AttachedHost,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentCapabilities {
    #[serde(default)]
    pub fs_read: bool,
    #[serde(default)]
    pub fs_write: bool,
    #[serde(default)]
    pub process_exec: bool,
    #[serde(default)]
    pub process_stdin: bool,
    #[serde(default)]
    pub job_start: bool,
    #[serde(default)]
    pub job_list: bool,
    #[serde(default)]
    pub job_read: bool,
    #[serde(default)]
    pub job_cancel: bool,
    #[serde(default)]
    pub job_wait_hint: bool,
    #[serde(default)]
    pub job_dependencies: bool,
    #[serde(default)]
    pub job_queue_keys: bool,
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub persistent: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentStatus {
    Attaching,
    Ready,
    Degraded,
    Detached,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsRoute {
    pub path: FsPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<FsPath>,
    pub access: FsRouteAccess,
    pub source: FsRouteSource,
    pub availability: FsRouteAvailability,
    pub same_state_as_active_env: Option<String>,
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
    HostFilesystem { target: ToolExecutionTarget },
    FusedWorkspace { environment_id: String },
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
    state: &CoreAgentState,
    catalog: VfsCatalog,
) -> Result<EnvironmentProjectionPublication<VfsCatalog>, EnvironmentProjectionError> {
    prepare_projection_publication(
        blobs,
        state,
        catalog,
        VFS_CATALOG_CONTEXT_KEY,
        vfs_catalog_context_input,
    )
    .await
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
    state: &CoreAgentState,
    snapshot: T,
    key: &'static str,
    context_input: fn(BlobRef) -> ContextEntryInput,
) -> Result<EnvironmentProjectionPublication<T>, EnvironmentProjectionError>
where
    T: Clone + PartialEq + Serialize,
{
    let snapshot_bytes = encode_json(&snapshot)?;
    let snapshot_ref = blobs.put_bytes(snapshot_bytes.clone()).await?;
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
        same_state_as_active_env: None,
    })
}

fn projection_context_input(
    kind: ContextEntryKind,
    content_ref: BlobRef,
    preview: &'static str,
) -> ContextEntryInput {
    ContextEntryInput {
        kind,
        content_ref,
        media_type: Some(ENVIRONMENT_PROJECTION_MEDIA_TYPE.to_owned()),
        preview: Some(preview.to_owned()),
        provider_kind: None,
        provider_item_id: None,
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
        .find(|entry| {
            entry
                .key
                .as_ref()
                .is_some_and(|entry_key| entry_key.as_str() == key)
                && entry.kind == kind
        })
        .map(|entry| entry.content_ref.clone())
}

fn current_key_ref(state: &CoreAgentState, key: &'static str) -> Option<BlobRef> {
    state
        .context
        .entries
        .iter()
        .find(|entry| {
            entry
                .key
                .as_ref()
                .is_some_and(|entry_key| entry_key.as_str() == key)
        })
        .map(|entry| entry.content_ref.clone())
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

        let first = prepare_vfs_catalog_publication(&blobs, &state, catalog.clone())
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
            content_ref: first.snapshot_ref.clone(),
            media_type: Some(ENVIRONMENT_PROJECTION_MEDIA_TYPE.to_owned()),
            preview: Some("VFS catalog".to_owned()),
            provider_kind: None,
            provider_item_id: None,
            token_estimate: None,
        }];

        let second = prepare_vfs_catalog_publication(&blobs, &state, catalog)
            .await
            .expect("second publication");
        assert!(second.command.is_none());
    }

    #[test]
    fn vfs_catalog_from_workspace_links_projects_routes() {
        let link = workspace_link();

        let catalog = vfs_catalog_from_workspace_links(&[link]).expect("catalog");

        assert_ne!(catalog.revision, 0);
        assert_eq!(catalog.routes.len(), 1);
        assert_eq!(catalog.routes[0].path.as_str(), "/workspace");
        assert_eq!(catalog.routes[0].access, FsRouteAccess::ReadWrite);
        assert_eq!(catalog.routes[0].same_state_as_active_env, None);
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
