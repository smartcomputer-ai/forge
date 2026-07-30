//! Runtime resolution products for one active universe environment.

use std::{collections::BTreeMap, sync::Arc};

use engine::{ToolExecutionTarget, WorkspaceLinkAccess, storage::BlobStore};
use environments::EnvironmentRecord;
use thiserror::Error;
use tools::{
    environment::{
        EnvironmentToolContext,
        projection::{
            EnvironmentCapabilities, EnvironmentKind, EnvironmentRecord as RuntimeRecord,
            EnvironmentStatus, FsRoute, FsRouteAccess, FsRouteAvailability, FsRouteSource,
        },
    },
    fs::{
        FileSystem, FsError, FsPath, FsToolContext, ScopedFileSystem, SessionFileSystem,
        SessionFileSystemRoute, SessionFileSystemRouteSource,
    },
    targets::{ENV_TARGET_NAMESPACE, SESSION_FS_TARGET_ID, ToolTargets},
};
use vfs::{ResolvedWorkspaceLink, ResolvedWorkspaceLinkTarget};

#[derive(Clone)]
pub struct RuntimeEnvironment {
    resource: EnvironmentRecord,
    record: RuntimeRecord,
    tool_context: EnvironmentToolContext,
    fs_context: Option<FsToolContext>,
    fs_routes: Vec<FsRoute>,
}

impl RuntimeEnvironment {
    pub fn from_resource(
        resource: EnvironmentRecord,
        tool_context: EnvironmentToolContext,
        fs_context: FsToolContext,
    ) -> Result<Self, RuntimeEnvironmentError> {
        let environment_id = resource.environment_id.as_str().to_owned();
        let target = ToolExecutionTarget::new(ENV_TARGET_NAMESPACE, &environment_id);
        let cwd = resource
            .default_cwd
            .as_ref()
            .map(|cwd| FsPath::new(cwd.as_str()))
            .transpose()
            .map_err(|error| RuntimeEnvironmentError::InvalidCwd(error.to_string()))?;
        let fs_root = resource
            .metadata
            .get("fsRoot")
            .map(FsPath::new)
            .transpose()
            .map_err(|error| RuntimeEnvironmentError::InvalidFsRoot(error.to_string()))?;
        let capabilities = environment_capabilities_from_host(&resource.capabilities);
        let status = environment_status(resource.status);
        let record = RuntimeRecord {
            environment_id: environment_id.clone(),
            kind: EnvironmentKind::AttachedHost,
            capabilities: capabilities.clone(),
            exec_target: Some(target.clone()),
            cwd: cwd.clone(),
            status,
        };
        let fs_routes = if capabilities.fs_read {
            vec![FsRoute {
                path: FsPath::new("/").expect("root path is valid"),
                source_path: fs_root,
                access: if capabilities.fs_write {
                    FsRouteAccess::ReadWrite
                } else {
                    FsRouteAccess::ReadOnly
                },
                source: FsRouteSource::HostFilesystem { target },
                availability: FsRouteAvailability::Available,
                same_state_as_active_env: Some(environment_id),
            }]
        } else {
            Vec::new()
        };
        Ok(Self {
            resource,
            record,
            tool_context,
            fs_context: capabilities.fs_read.then_some(fs_context),
            fs_routes,
        })
    }

    pub fn environment_id(&self) -> &str {
        self.resource.environment_id.as_str()
    }

    pub fn resource(&self) -> &EnvironmentRecord {
        &self.resource
    }

    pub fn record(&self) -> &RuntimeRecord {
        &self.record
    }

    pub fn tool_context(&self) -> &EnvironmentToolContext {
        &self.tool_context
    }

    pub fn fs_context(&self) -> Option<&FsToolContext> {
        self.fs_context.as_ref()
    }
}

#[derive(Clone)]
pub struct SessionEnvironmentManager {
    blobs: Arc<dyn BlobStore>,
    environments: BTreeMap<String, RuntimeEnvironment>,
}

impl SessionEnvironmentManager {
    pub fn new(blobs: Arc<dyn BlobStore>) -> Self {
        Self {
            blobs,
            environments: BTreeMap::new(),
        }
    }

    pub fn insert_environment(&mut self, environment: RuntimeEnvironment) {
        self.environments
            .insert(environment.environment_id().to_owned(), environment);
    }

    pub fn environment(&self, environment_id: &str) -> Option<&RuntimeEnvironment> {
        self.environments.get(environment_id)
    }

    pub fn has_process_environment(&self) -> bool {
        self.environments
            .values()
            .any(|environment| environment.record.capabilities.process_exec)
    }

    pub fn has_job_environment(&self) -> bool {
        self.environments.values().any(|environment| {
            let capabilities = &environment.record.capabilities;
            capabilities.job_start
                || capabilities.job_list
                || capabilities.job_read
                || capabilities.job_cancel
        })
    }

    pub fn tool_targets(
        &self,
        session_fs: Option<FsToolContext>,
        workspace_links: &[ResolvedWorkspaceLink],
        active_environment_target: Option<&ToolExecutionTarget>,
    ) -> Result<ToolTargets, RuntimeEnvironmentError> {
        let mut targets = ToolTargets::new();
        if let Some(session_fs) =
            self.composed_session_fs(session_fs, workspace_links, active_environment_target)?
        {
            targets.insert_fs_context(SESSION_FS_TARGET_ID, session_fs);
        }
        for environment in self.environments.values() {
            targets.insert_environment_context(
                environment.environment_id(),
                environment.tool_context.clone(),
            );
        }
        Ok(targets)
    }

    fn composed_session_fs(
        &self,
        session_fs: Option<FsToolContext>,
        workspace_links: &[ResolvedWorkspaceLink],
        active_environment_target: Option<&ToolExecutionTarget>,
    ) -> Result<Option<FsToolContext>, RuntimeEnvironmentError> {
        let active = active_environment_target
            .filter(|target| target.namespace == ENV_TARGET_NAMESPACE)
            .and_then(|target| self.environments.get(&target.id))
            .filter(|environment| environment.fs_context.is_some());
        let Some(active) = active else {
            return Ok(session_fs);
        };
        let active_fs = active.fs_context.as_ref().expect("checked above");
        let mut routes = Vec::new();
        if let Some(session_fs) = session_fs.as_ref() {
            for link in workspace_links {
                routes.push(vfs_session_route(link, session_fs.fs.clone())?);
            }
        }
        for route in &active.fs_routes {
            let route_fs = scoped_route_fs(route, active_fs.fs.clone())?;
            routes.push(SessionFileSystemRoute::new(
                route.path.clone(),
                route_fs,
                SessionFileSystemRouteSource::EnvironmentFilesystem {
                    environment_id: active.environment_id().to_owned(),
                },
                true,
            )?);
        }
        if routes.is_empty() {
            return Ok(session_fs);
        }
        let mut context = FsToolContext::new(
            Arc::new(SessionFileSystem::new(routes)?),
            self.blobs.clone(),
        );
        if let Some(cwd) = active_fs.fs_cwd.clone().or_else(|| {
            session_fs
                .as_ref()
                .and_then(|context| context.fs_cwd.clone())
        }) {
            context = context.with_cwd(cwd);
        }
        Ok(Some(context))
    }
}

fn vfs_session_route(
    link: &ResolvedWorkspaceLink,
    fs: Arc<dyn FileSystem>,
) -> Result<SessionFileSystemRoute, FsError> {
    let path = FsPath::new(link.path.as_str())?;
    let route_fs = match link.access {
        WorkspaceLinkAccess::ReadOnly => ScopedFileSystem::read_only_from_arc(path.clone(), fs)?,
        WorkspaceLinkAccess::ReadWrite => ScopedFileSystem::read_write_from_arc(path.clone(), fs)?,
    };
    let source = match &link.target {
        ResolvedWorkspaceLinkTarget::AvailableSnapshot { .. }
        | ResolvedWorkspaceLinkTarget::Unavailable {
            declared_target: engine::WorkspaceLinkTarget::Snapshot { .. },
            ..
        } => SessionFileSystemRouteSource::VfsSnapshot,
        ResolvedWorkspaceLinkTarget::AvailableWorkspace { .. }
        | ResolvedWorkspaceLinkTarget::Unavailable {
            declared_target: engine::WorkspaceLinkTarget::Workspace { .. },
            ..
        } => SessionFileSystemRouteSource::VfsWorkspace,
    };
    Ok(SessionFileSystemRoute::new(
        path,
        Arc::new(route_fs),
        source,
        false,
    )?)
}

fn scoped_route_fs(
    route: &FsRoute,
    fs: Arc<dyn FileSystem>,
) -> Result<Arc<dyn FileSystem>, FsError> {
    let source_path = route.source_path.as_ref().unwrap_or(&route.path);
    let scoped = match route.access {
        FsRouteAccess::ReadOnly => ScopedFileSystem::read_only_from_arc(source_path.clone(), fs)?,
        FsRouteAccess::ReadWrite => ScopedFileSystem::read_write_from_arc(source_path.clone(), fs)?,
    };
    Ok(Arc::new(scoped))
}

fn environment_capabilities_from_host(
    capabilities: &host_protocol::shared::HostCapabilities,
) -> EnvironmentCapabilities {
    EnvironmentCapabilities {
        fs_read: capabilities.filesystem_read,
        fs_write: capabilities.filesystem_write,
        process_exec: capabilities.process_start,
        process_stdin: capabilities.process_stdin,
        job_start: capabilities.job_start,
        job_list: capabilities.job_list,
        job_read: capabilities.job_read,
        job_cancel: capabilities.job_cancel,
        job_wait_hint: capabilities.job_wait_hint,
        job_dependencies: capabilities.job_dependencies,
        job_queue_keys: capabilities.job_queue_keys,
        network: capabilities.network,
        persistent: true,
    }
}

fn environment_status(
    status: host_protocol::control::targets::HostTargetStatus,
) -> EnvironmentStatus {
    use host_protocol::control::targets::HostTargetStatus;
    match status {
        HostTargetStatus::Ready => EnvironmentStatus::Ready,
        HostTargetStatus::Creating | HostTargetStatus::Starting => EnvironmentStatus::Attaching,
        HostTargetStatus::Stopped
        | HostTargetStatus::Closing
        | HostTargetStatus::Closed
        | HostTargetStatus::Failed
        | HostTargetStatus::Unknown => EnvironmentStatus::Degraded,
    }
}

#[derive(Debug, Error)]
pub enum RuntimeEnvironmentError {
    #[error("invalid environment cwd: {0}")]
    InvalidCwd(String),

    #[error("invalid environment filesystem root: {0}")]
    InvalidFsRoot(String),

    #[error(transparent)]
    Fs(#[from] FsError),
}
