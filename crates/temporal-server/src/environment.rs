//! Runtime resolution products for active universe environments.

use std::collections::BTreeMap;

use environments::EnvironmentRecord;
use tools::environment::EnvironmentToolContext;

#[derive(Clone)]
pub struct RuntimeEnvironment {
    resource: EnvironmentRecord,
    tool_context: EnvironmentToolContext,
}

impl RuntimeEnvironment {
    /// Wraps a negotiated tool context for one registry environment. The
    /// filesystem, process, and job capabilities were already gated by the
    /// data-plane capability negotiation that produced `tool_context`; this
    /// only stamps the environment identity.
    pub fn from_resource(
        resource: EnvironmentRecord,
        tool_context: EnvironmentToolContext,
    ) -> Self {
        let environment_id = resource.environment_id.as_str().to_owned();
        let tool_context = tool_context.with_environment_id(environment_id);

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

/// Why the session's active environment could not be resolved into a tool
/// context for this invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActiveEnvironmentBlocker {
    /// The environment exists and is provisioning or booting; the call must
    /// wait rather than fail.
    NotReady {
        environment_id: String,
        status: environments::EnvironmentStatus,
    },
    /// The environment cannot serve calls (failed, closed, unreachable, or
    /// not allowed); the call fails with this message.
    Unavailable { message: String },
}

#[derive(Clone, Default)]
pub struct SessionEnvironmentManager {
    environments: BTreeMap<String, RuntimeEnvironment>,
    active_blocker: Option<ActiveEnvironmentBlocker>,
}

impl SessionEnvironmentManager {
    pub fn new(_blobs: std::sync::Arc<dyn engine::storage::BlobStore>) -> Self {
        Self::default()
    }

    pub fn with_active_blocker(mut self, blocker: ActiveEnvironmentBlocker) -> Self {
        self.active_blocker = Some(blocker);
        self
    }

    /// The reason the active environment has no tool context in this manager,
    /// if resolution found one.
    pub fn active_blocker(&self) -> Option<&ActiveEnvironmentBlocker> {
        self.active_blocker.as_ref()
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
    use environment_protocol::shared::ProviderTargetId;
    use environments::{
        EnvironmentIncarnationId, EnvironmentIncarnationRecord, EnvironmentProviderBindingId,
        EnvironmentProviderId, EnvironmentProvisionRequestId, EnvironmentSource, EnvironmentStatus,
        EnvironmentTemplateId,
    };
    use tools::fs::{FsPath, FsToolContext, InMemoryFileSystem};

    use super::*;

    fn resource() -> EnvironmentRecord {
        let target_id = ProviderTargetId::new("target-a");
        EnvironmentRecord {
            environment_id: engine::EnvironmentId::new("environment-a"),
            request_id: EnvironmentProvisionRequestId::new("request-a"),
            source: EnvironmentSource::Provisioned {
                provider_id: EnvironmentProviderId::new("provider-a"),
                binding_id: EnvironmentProviderBindingId::new("binding-a"),
            },
            display_name: None,
            status: EnvironmentStatus::Offline,
            desired_power: environments::PowerState::Running,
            idle_policy: None,
            incarnation: EnvironmentIncarnationRecord {
                incarnation_id: EnvironmentIncarnationId::new("incarnation-a"),
                provision_request_id: Some(EnvironmentProvisionRequestId::new("request-a")),
                provider_target_id: Some(target_id.clone()),
                template_id: Some(EnvironmentTemplateId::new("template-a")),
                adoption_source_target: None,
                power_states: Vec::new(),
                created_at_ms: 1,
                updated_at_ms: 1,
            },
            public_ingress_enabled: false,
            public_endpoint: None,
            origin_session: None,
            metadata: BTreeMap::from([("fsRoot".to_owned(), "/sandbox".to_owned())]),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn environment_keeps_negotiated_filesystem_context() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let fs = FsToolContext::new(Arc::new(InMemoryFileSystem::full_access()), blobs.clone())
            .with_cwd(FsPath::new("/sandbox/project").expect("cwd"));
        let context = EnvironmentToolContext::new(None, blobs).with_filesystem(fs);

        let environment = RuntimeEnvironment::from_resource(resource(), context);
        let filesystem = environment
            .tool_context()
            .filesystem
            .as_ref()
            .expect("filesystem context survives runtime wrapping");
        assert_eq!(
            filesystem.fs_cwd,
            Some(FsPath::new("/sandbox/project").expect("cwd"))
        );
        assert_eq!(
            environment.tool_context().environment_id.as_deref(),
            Some("environment-a")
        );
    }

    #[test]
    fn environment_without_negotiated_filesystem_stays_without_one() {
        let blobs = Arc::new(InMemoryBlobStore::new());
        let context = EnvironmentToolContext::new(None, blobs);

        let environment = RuntimeEnvironment::from_resource(resource(), context);
        assert!(environment.tool_context().filesystem.is_none());
    }
}
