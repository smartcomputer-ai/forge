//! Provider-neutral prompt text for the VFS catalog context entry.

use engine::{BlobRef, ToolExecutionTarget, storage::BlobStore};
use tools::environment::projection::{FsRoute, FsRouteAccess, FsRouteSource, VfsCatalog};

use crate::error::{LlmAdapterError, LlmAdapterResult};

pub(crate) async fn read_vfs_catalog(
    blobs: &dyn BlobStore,
    blob_ref: &BlobRef,
) -> LlmAdapterResult<VfsCatalog> {
    read_projection(blobs, blob_ref).await
}

pub(crate) fn vfs_catalog_text(catalog: &VfsCatalog) -> String {
    let mut text =
        String::from("Filesystem (virtual: file tools only, no shell; wins on path collisions):\n");
    if catalog.routes.is_empty() {
        text.push_str("  No VFS routes are currently mounted.\n");
    } else {
        for route in &catalog.routes {
            text.push_str(&format!("  {}\n", route_line(route)));
        }
    }
    text.push_str("\nThe VFS is not an execution environment. Commands cannot run in VFS paths.");
    text
}

async fn read_projection<T>(blobs: &dyn BlobStore, blob_ref: &BlobRef) -> LlmAdapterResult<T>
where
    T: serde::de::DeserializeOwned,
{
    let bytes = blobs.read_bytes(blob_ref).await?;
    serde_json::from_slice(&bytes).map_err(|error| LlmAdapterError::InvalidJson {
        blob_ref: blob_ref.clone(),
        message: error.to_string(),
    })
}

fn route_line(route: &FsRoute) -> String {
    let same_state = match &route.same_state_as_active_env {
        Some(environment_id) => {
            format!("; same files as shell in {environment_id} for paths resolved here")
        }
        None => "; no shell access".to_owned(),
    };
    format!(
        "{:<12} {} - {}{}",
        route.path,
        route_access(route.access),
        route_source(&route.source),
        same_state
    )
}

fn route_access(access: FsRouteAccess) -> &'static str {
    match access {
        FsRouteAccess::ReadOnly => "read-only",
        FsRouteAccess::ReadWrite => "read/write",
    }
}

fn route_source(source: &FsRouteSource) -> String {
    match source {
        FsRouteSource::VfsSnapshot { snapshot_ref } => {
            format!("VFS snapshot {snapshot_ref}")
        }
        FsRouteSource::VfsWorkspace { workspace_id } => {
            format!("VFS workspace {workspace_id}")
        }
        FsRouteSource::HostFilesystem { target } => {
            format!("environment filesystem {}", target_text(target))
        }
        FsRouteSource::FusedWorkspace { environment_id } => {
            format!("fused workspace for {environment_id}")
        }
    }
}

fn target_text(target: &ToolExecutionTarget) -> String {
    format!("{}:{}", target.namespace, target.id)
}

#[cfg(test)]
mod tests {
    use tools::environment::projection::{
        FsRoute, FsRouteAccess, FsRouteAvailability, FsRouteSource, VfsCatalog,
    };

    use super::*;

    #[test]
    fn vfs_catalog_text_says_no_shell() {
        let catalog = VfsCatalog::new(
            0,
            vec![FsRoute {
                path: tools::fs::FsPath::new("/workspace").unwrap(),
                source_path: None,
                access: FsRouteAccess::ReadWrite,
                source: FsRouteSource::VfsWorkspace {
                    workspace_id: "workspace_1".to_owned(),
                },
                same_state_as_active_env: None,
                availability: FsRouteAvailability::Available,
            }],
        );

        let text = vfs_catalog_text(&catalog);

        assert!(text.contains("/workspace"));
        assert!(text.contains("no shell"));
        assert!(text.contains("Commands cannot run in VFS paths"));
    }
}
