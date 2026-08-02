//! Runtime resolution products for active universe environments.

use std::collections::BTreeMap;

use environments::EnvironmentRecord;
use tools::{
    environment::EnvironmentToolContext,
    fs::{FsToolContext, ReadOnlyFileSystem},
};

#[derive(Clone)]
pub struct RuntimeEnvironment {
    resource: EnvironmentRecord,
    tool_context: EnvironmentToolContext,
}

impl RuntimeEnvironment {
    pub fn from_resource(
        resource: EnvironmentRecord,
        mut tool_context: EnvironmentToolContext,
        fs_context: FsToolContext,
    ) -> Self {
        let environment_id = resource.environment_id.as_str().to_owned();
        tool_context = tool_context.with_environment_id(environment_id);
        if resource.capabilities.filesystem_read {
            let filesystem = if resource.capabilities.filesystem_write {
                fs_context
            } else {
                FsToolContext {
                    fs: std::sync::Arc::new(ReadOnlyFileSystem::from_arc(fs_context.fs)),
                    blobs: fs_context.blobs,
                    limits: fs_context.limits,
                    fs_cwd: fs_context.fs_cwd,
                }
            };
            tool_context = tool_context.with_filesystem(filesystem);
        } else {
            tool_context.filesystem = None;
        }

        Self {
            resource,
            tool_context,
        }
    }

    pub fn environment_id(&self) -> &str {
        self.resource.environment_id.as_str()
    }

    pub fn resource(&self) -> &EnvironmentRecord {
        &self.resource
    }

    pub fn tool_context(&self) -> &EnvironmentToolContext {
        &self.tool_context
    }
}

#[derive(Clone, Default)]
pub struct SessionEnvironmentManager {
    environments: BTreeMap<String, RuntimeEnvironment>,
}

impl SessionEnvironmentManager {
    pub fn new(_blobs: std::sync::Arc<dyn engine::storage::BlobStore>) -> Self {
        Self::default()
    }

    pub fn insert_environment(&mut self, environment: RuntimeEnvironment) {
        self.environments
            .insert(environment.environment_id().to_owned(), environment);
    }

    pub fn environment(&self, environment_id: &str) -> Option<&RuntimeEnvironment> {
        self.environments.get(environment_id)
    }

    pub fn active_tool_context(&self, environment_id: &str) -> Option<EnvironmentToolContext> {
        self.environment(environment_id)
            .map(|environment| environment.tool_context.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use engine::storage::InMemoryBlobStore;
    use environments::{EnvironmentOrigin, EnvironmentProviderId};
    use host_protocol::{
        control::targets::HostTargetStatus,
        shared::{
            HostCapabilities, HostConnectionSpec, HostPath, HostScope, HostTargetId, HostTransport,
        },
    };
    use tools::fs::{FileAccessPolicy, FsPath, FsToolContext, InMemoryFileSystem};

    use super::*;

    fn resource(capabilities: HostCapabilities) -> EnvironmentRecord {
        let target_id = HostTargetId::new("target-a");
        EnvironmentRecord {
            environment_id: engine::EnvironmentId::new("environment-a"),
            provider_id: EnvironmentProviderId::new("provider-a"),
            provider_target_id: target_id.clone(),
            origin: EnvironmentOrigin::Provided,
            display_name: None,
            status: HostTargetStatus::Ready,
            scope: HostScope::Default,
            capabilities: capabilities.clone(),
            connection: HostConnectionSpec {
                target_id,
                endpoint: "http://environment.test".to_owned(),
                transport: HostTransport::Http,
                scope: HostScope::Default,
                default_cwd: Some(HostPath::new("/sandbox/project").expect("cwd")),
                capabilities,
            },
            default_cwd: Some(HostPath::new("/sandbox/project").expect("cwd")),
            metadata: BTreeMap::from([("fsRoot".to_owned(), "/sandbox".to_owned())]),
            observed_at_ms: 1,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn environment_filesystem_keeps_native_cwd_and_enforces_record_read_only_capability() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let fs = FsToolContext::new(Arc::new(InMemoryFileSystem::full_access()), blobs.clone())
            .with_cwd(FsPath::new("/sandbox/project").expect("cwd"));
        let context = EnvironmentToolContext::new(None, blobs);

        let environment = RuntimeEnvironment::from_resource(
            resource(HostCapabilities::filesystem(true, false)),
            context,
            fs,
        );
        let filesystem = environment
            .tool_context()
            .filesystem
            .as_ref()
            .expect("filesystem");

        assert_eq!(
            filesystem.fs_cwd.as_ref().map(FsPath::as_str),
            Some("/sandbox/project")
        );
        assert_eq!(
            filesystem.fs.access_policy(),
            FileAccessPolicy::FullReadOnly
        );
    }

    #[test]
    fn environment_without_read_capability_has_no_file_context() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let fs = FsToolContext::new(Arc::new(InMemoryFileSystem::full_access()), blobs.clone());
        let context = EnvironmentToolContext::new(None, blobs);

        let environment = RuntimeEnvironment::from_resource(
            resource(HostCapabilities::filesystem(false, false)),
            context,
            fs,
        );

        assert!(environment.tool_context().filesystem.is_none());
    }
}
