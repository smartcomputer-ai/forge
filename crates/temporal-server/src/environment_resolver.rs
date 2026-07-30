//! Internal environment lookup and liveness policy.
//!
//! This is a shared runtime service, not a provider/plugin extension seam.

use std::{collections::BTreeSet, sync::Arc};

use environments::{
    EnvironmentId, EnvironmentProviderStore, EnvironmentRecord, EnvironmentRegistryError,
    EnvironmentStore, ListEnvironments,
};
use store_pg::PgStore;
use thiserror::Error;

#[derive(Clone)]
pub(crate) struct EnvironmentResolver {
    environments: Arc<dyn EnvironmentStore>,
    providers: Arc<dyn EnvironmentProviderStore>,
}

impl EnvironmentResolver {
    pub(crate) fn new(
        environments: Arc<dyn EnvironmentStore>,
        providers: Arc<dyn EnvironmentProviderStore>,
    ) -> Self {
        Self {
            environments,
            providers,
        }
    }

    pub(crate) fn from_pg_store(store: Arc<PgStore>) -> Self {
        Self::new(store.clone(), store)
    }

    pub(crate) async fn list_allowed(
        &self,
        allowed_providers: Option<&BTreeSet<String>>,
    ) -> Result<Vec<EnvironmentRecord>, EnvironmentResolveError> {
        let mut environments = self
            .environments
            .list_environments(ListEnvironments::default())
            .await?;
        if let Some(allowed) = allowed_providers {
            environments.retain(|environment| allowed.contains(environment.provider_id.as_str()));
        }
        Ok(environments)
    }

    pub(crate) async fn read_allowed(
        &self,
        environment_id: &EnvironmentId,
        allowed_providers: Option<&BTreeSet<String>>,
    ) -> Result<EnvironmentRecord, EnvironmentResolveError> {
        let environment = self.environments.read_environment(environment_id).await?;
        if allowed_providers
            .is_some_and(|allowed| !allowed.contains(environment.provider_id.as_str()))
        {
            return Err(EnvironmentResolveError::ProviderNotAllowed {
                provider_id: environment.provider_id.as_str().to_owned(),
            });
        }
        Ok(environment)
    }

    pub(crate) async fn selectable(
        &self,
        environment_id: &EnvironmentId,
        allowed_providers: Option<&BTreeSet<String>>,
        now_ms: i64,
    ) -> Result<EnvironmentRecord, EnvironmentResolveError> {
        let environment = self.read_allowed(environment_id, allowed_providers).await?;
        let provider = self
            .providers
            .read_provider(&environment.provider_id)
            .await?;
        if !provider.is_live_at(now_ms) {
            return Err(EnvironmentResolveError::ProviderUnavailable {
                provider_id: provider.provider_id.as_str().to_owned(),
            });
        }
        if !environment.is_attachable() {
            return Err(EnvironmentResolveError::EnvironmentUnavailable {
                environment_id: environment.environment_id.as_str().to_owned(),
                status: format!("{:?}", environment.status).to_lowercase(),
            });
        }
        Ok(environment)
    }
}

#[derive(Debug, Error)]
pub(crate) enum EnvironmentResolveError {
    #[error(transparent)]
    Store(#[from] EnvironmentRegistryError),

    #[error("environment provider is not allowed by session config: {provider_id}")]
    ProviderNotAllowed { provider_id: String },

    #[error("environment provider is unavailable: {provider_id}")]
    ProviderUnavailable { provider_id: String },

    #[error("environment is unavailable: {environment_id} ({status})")]
    EnvironmentUnavailable {
        environment_id: String,
        status: String,
    },
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use environments::{
        EnvironmentOrigin, EnvironmentProviderCapabilities, EnvironmentProviderId,
        EnvironmentProviderKind, HostControllerConnectionSpec, ObserveEnvironment,
        RegisterEnvironmentProvider,
    };
    use host_protocol::{
        control::targets::HostTargetStatus,
        shared::{
            HostCapabilities, HostConnectionSpec, HostPath, HostScope, HostTargetId, HostTransport,
            ImplementationInfo,
        },
    };

    use super::*;

    async fn resolver() -> (EnvironmentResolver, EnvironmentId) {
        let store = Arc::new(environments::InMemoryEnvironmentRegistryStore::new());
        let provider_id = EnvironmentProviderId::new("allowed");
        store
            .register_provider(RegisterEnvironmentProvider {
                provider_id: provider_id.clone(),
                provider_kind: EnvironmentProviderKind::Bridge,
                display_name: None,
                controller_connection: HostControllerConnectionSpec::new(
                    "http://controller.test",
                    HostTransport::Http,
                ),
                capabilities: EnvironmentProviderCapabilities {
                    list_targets: true,
                    ..EnvironmentProviderCapabilities::default()
                },
                implementation: ImplementationInfo {
                    name: "test".to_owned(),
                    version: None,
                },
                lease_ttl_ms: 100,
                metadata: BTreeMap::new(),
                observed_at_ms: 10,
            })
            .await
            .expect("provider");
        let environment_id = EnvironmentId::new("environment-1");
        let target_id = HostTargetId::new("target-1");
        let capabilities = HostCapabilities::filesystem(true, true).with_process();
        store
            .observe_environment(ObserveEnvironment {
                environment_id: environment_id.clone(),
                provider_id,
                provider_target_id: target_id.clone(),
                origin: EnvironmentOrigin::Provided,
                display_name: None,
                status: HostTargetStatus::Ready,
                scope: HostScope::Default,
                capabilities: capabilities.clone(),
                connection: HostConnectionSpec {
                    target_id,
                    endpoint: "http://host.test".to_owned(),
                    transport: HostTransport::Http,
                    scope: HostScope::Default,
                    default_cwd: Some(HostPath::new("/workspace").expect("cwd")),
                    capabilities,
                },
                default_cwd: Some(HostPath::new("/workspace").expect("cwd")),
                metadata: BTreeMap::new(),
                observed_at_ms: 10,
            })
            .await
            .expect("environment");
        (
            EnvironmentResolver::new(store.clone(), store),
            environment_id,
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn provider_filter_applies_to_list_read_and_selection() {
        let (resolver, environment_id) = resolver().await;
        let denied = BTreeSet::from(["other".to_owned()]);
        assert!(
            resolver
                .list_allowed(Some(&denied))
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            resolver.read_allowed(&environment_id, Some(&denied)).await,
            Err(EnvironmentResolveError::ProviderNotAllowed { .. })
        ));
        assert!(matches!(
            resolver
                .selectable(&environment_id, Some(&denied), 20)
                .await,
            Err(EnvironmentResolveError::ProviderNotAllowed { .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn selection_checks_provider_lease_but_read_still_explores() {
        let (resolver, environment_id) = resolver().await;
        assert!(resolver.read_allowed(&environment_id, None).await.is_ok());
        assert!(matches!(
            resolver.selectable(&environment_id, None, 111).await,
            Err(EnvironmentResolveError::ProviderUnavailable { .. })
        ));
    }
}
