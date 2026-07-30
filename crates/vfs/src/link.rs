use std::sync::Arc;

use engine::{
    BlobRef, WorkspaceLink, WorkspaceLinkAccess, WorkspaceLinkTarget, storage::BlobStore,
};

use crate::{
    VfsCatalogError, VfsPath, VfsWorkspaceId, VfsWorkspaceRecord, VfsWorkspaceStore,
    read_snapshot_manifest,
};

/// A session workspace link resolved against one coherent catalog view.
/// This value is transient and must never be persisted as session authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedWorkspaceLink {
    pub path: VfsPath,
    pub target: ResolvedWorkspaceLinkTarget,
    pub access: WorkspaceLinkAccess,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedWorkspaceLinkTarget {
    AvailableSnapshot {
        snapshot_ref: BlobRef,
    },
    AvailableWorkspace {
        workspace: VfsWorkspaceRecord,
    },
    Unavailable {
        declared_target: WorkspaceLinkTarget,
        reason: String,
    },
}

impl ResolvedWorkspaceLink {
    pub fn is_available(&self) -> bool {
        !matches!(self.target, ResolvedWorkspaceLinkTarget::Unavailable { .. })
    }

    pub fn unavailable_reason(&self) -> Option<&str> {
        match &self.target {
            ResolvedWorkspaceLinkTarget::Unavailable { reason, .. } => Some(reason),
            _ => None,
        }
    }

    pub fn is_writable(&self) -> bool {
        self.access == WorkspaceLinkAccess::ReadWrite
    }
}

/// Resolve declarations without turning missing catalog resources into a
/// global failure. Invalid durable identifiers remain request errors; missing
/// or unreadable targets become per-link unavailable projections.
pub async fn resolve_workspace_links(
    blobs: Arc<dyn BlobStore>,
    workspace_store: Arc<dyn VfsWorkspaceStore>,
    links: &[WorkspaceLink],
) -> Result<Vec<ResolvedWorkspaceLink>, VfsCatalogError> {
    let mut resolved = Vec::with_capacity(links.len());
    for link in links {
        let path = VfsPath::parse(&link.path).map_err(|error| VfsCatalogError::InvalidInput {
            message: format!("invalid workspace link path {:?}: {error}", link.path),
        })?;
        let target = match &link.target {
            WorkspaceLinkTarget::Snapshot { snapshot_ref } => {
                let snapshot_ref = BlobRef::parse(snapshot_ref.clone()).map_err(|error| {
                    VfsCatalogError::InvalidInput {
                        message: format!("invalid workspace link snapshot ref: {error}"),
                    }
                })?;
                match read_snapshot_manifest(blobs.as_ref(), &snapshot_ref).await {
                    Ok(_) => ResolvedWorkspaceLinkTarget::AvailableSnapshot { snapshot_ref },
                    Err(error) => ResolvedWorkspaceLinkTarget::Unavailable {
                        declared_target: link.target.clone(),
                        reason: error.to_string(),
                    },
                }
            }
            WorkspaceLinkTarget::Workspace { workspace_id } => {
                let workspace_id =
                    VfsWorkspaceId::try_new(workspace_id.clone()).map_err(|error| {
                        VfsCatalogError::InvalidInput {
                            message: format!("invalid workspace link workspace id: {error}"),
                        }
                    })?;
                match workspace_store.read_workspace(&workspace_id).await {
                    Ok(workspace) => {
                        match read_snapshot_manifest(blobs.as_ref(), &workspace.head_snapshot_ref)
                            .await
                        {
                            Ok(_) => ResolvedWorkspaceLinkTarget::AvailableWorkspace { workspace },
                            Err(error) => ResolvedWorkspaceLinkTarget::Unavailable {
                                declared_target: link.target.clone(),
                                reason: error.to_string(),
                            },
                        }
                    }
                    Err(error) => ResolvedWorkspaceLinkTarget::Unavailable {
                        declared_target: link.target.clone(),
                        reason: error.to_string(),
                    },
                }
            }
        };
        resolved.push(ResolvedWorkspaceLink {
            path,
            target,
            access: link.access,
        });
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use engine::storage::InMemoryBlobStore;

    use super::*;
    use crate::{
        CompareAndSetVfsWorkspaceHead, CreateInlineSnapshotRequest, CreateVfsWorkspaceRecord,
        InlineFile, VfsTotals, create_inline_snapshot,
    };

    #[derive(Default)]
    struct TestWorkspaceStore {
        workspace: Mutex<Option<VfsWorkspaceRecord>>,
    }

    #[async_trait]
    impl VfsWorkspaceStore for TestWorkspaceStore {
        async fn create_workspace(
            &self,
            record: CreateVfsWorkspaceRecord,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            let workspace = VfsWorkspaceRecord {
                workspace_id: record.workspace_id,
                display_name: record.display_name,
                base_snapshot_ref: record.base_snapshot_ref,
                head_snapshot_ref: record.head_snapshot_ref,
                head_totals: record.head_totals,
                revision: 0,
                created_at_ms: record.created_at_ms,
                updated_at_ms: record.created_at_ms,
            };
            *self.workspace.lock().unwrap() = Some(workspace.clone());
            Ok(workspace)
        }

        async fn read_workspace(
            &self,
            workspace_id: &VfsWorkspaceId,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            self.workspace
                .lock()
                .unwrap()
                .clone()
                .filter(|workspace| &workspace.workspace_id == workspace_id)
                .ok_or_else(|| VfsCatalogError::NotFound {
                    kind: "workspace",
                    id: workspace_id.to_string(),
                })
        }

        async fn list_workspaces(&self) -> Result<Vec<VfsWorkspaceRecord>, VfsCatalogError> {
            Ok(self.workspace.lock().unwrap().clone().into_iter().collect())
        }

        async fn compare_and_set_head(
            &self,
            _request: CompareAndSetVfsWorkspaceHead,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            unreachable!()
        }

        async fn delete_workspace(
            &self,
            workspace_id: &VfsWorkspaceId,
        ) -> Result<VfsWorkspaceRecord, VfsCatalogError> {
            let workspace =
                self.workspace
                    .lock()
                    .unwrap()
                    .take()
                    .ok_or_else(|| VfsCatalogError::NotFound {
                        kind: "workspace",
                        id: workspace_id.to_string(),
                    })?;
            Ok(workspace)
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deleting_a_linked_workspace_preserves_the_declaration_as_unavailable() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let store = Arc::new(TestWorkspaceStore::default());
        let snapshot = create_inline_snapshot(
            blobs.as_ref(),
            CreateInlineSnapshotRequest::new(vec![
                InlineFile::new("README.md", b"hello".to_vec()).unwrap(),
            ]),
        )
        .await
        .unwrap();
        let workspace_id = VfsWorkspaceId::new("workspace-linked");
        store
            .create_workspace(CreateVfsWorkspaceRecord {
                workspace_id: workspace_id.clone(),
                display_name: None,
                base_snapshot_ref: None,
                head_snapshot_ref: snapshot.snapshot_ref,
                head_totals: VfsTotals { files: 1, bytes: 5 },
                created_at_ms: 1,
            })
            .await
            .unwrap();
        let declaration = WorkspaceLink {
            path: "/workspace".to_owned(),
            target: WorkspaceLinkTarget::Workspace {
                workspace_id: workspace_id.to_string(),
            },
            access: WorkspaceLinkAccess::ReadWrite,
        };

        let available = resolve_workspace_links(
            blobs.clone(),
            store.clone(),
            std::slice::from_ref(&declaration),
        )
        .await
        .unwrap();
        assert!(available[0].is_available());

        store.delete_workspace(&workspace_id).await.unwrap();
        let unavailable = resolve_workspace_links(blobs, store, std::slice::from_ref(&declaration))
            .await
            .unwrap();
        assert!(!unavailable[0].is_available());
        assert_eq!(
            unavailable[0].target,
            ResolvedWorkspaceLinkTarget::Unavailable {
                declared_target: declaration.target,
                reason: format!("vfs catalog workspace not found: {workspace_id}"),
            }
        );
    }
}
