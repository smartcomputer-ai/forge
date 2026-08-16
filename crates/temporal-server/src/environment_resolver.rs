//! Internal environment lookup and liveness policy.
//!
//! This is a shared runtime service, not a provider/plugin extension seam.

use std::{collections::BTreeSet, sync::Arc};

use environments::{
    EnvironmentId, EnvironmentProviderStore, EnvironmentRecord, EnvironmentRegistryError,
    EnvironmentStatus, EnvironmentStore, ListEnvironments,
};
use store_pg::PgStore;
use thiserror::Error;

#[derive(Clone)]
pub(crate) struct EnvironmentResolver {
    environments: Arc<dyn EnvironmentStore>,
    providers: Arc<dyn EnvironmentProviderStore>,
    gateway: Option<crate::environment_gateway::EnvironmentGatewayClientConfig>,
    universe_id: uuid::Uuid,
}

impl EnvironmentResolver {
    pub(crate) fn universe_id(&self) -> uuid::Uuid {
        self.universe_id
    }
    pub(crate) fn new(
        environments: Arc<dyn EnvironmentStore>,
        providers: Arc<dyn EnvironmentProviderStore>,
    ) -> Self {
        Self {
            environments,
            providers,
            gateway: None,
            universe_id: uuid::Uuid::nil(),
        }
    }

    pub(crate) fn from_pg_store(store: Arc<PgStore>) -> Self {
        let universe_id = store.config().universe_id;
        let mut resolver = Self::new(store.clone(), store);
        resolver.universe_id = universe_id;
        resolver
    }

    pub(crate) fn with_gateway(
        mut self,
        gateway: crate::environment_gateway::EnvironmentGatewayClientConfig,
    ) -> Self {
        self.gateway = Some(gateway);
        self
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
            environments.retain(|environment| {
                environment
                    .provider_id()
                    .is_some_and(|id| allowed.contains(id.as_str()))
            });
        }
        Ok(environments)
    }

    pub(crate) async fn read_allowed(
        &self,
        environment_id: &EnvironmentId,
        allowed_providers: Option<&BTreeSet<String>>,
    ) -> Result<EnvironmentRecord, EnvironmentResolveError> {
        let environment = self.environments.read_environment(environment_id).await?;
        if allowed_providers.is_some_and(|allowed| {
            environment
                .provider_id()
                .is_none_or(|id| !allowed.contains(id.as_str()))
        }) {
            return Err(EnvironmentResolveError::ProviderNotAllowed {
                provider_id: environment
                    .provider_id()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "external".to_owned()),
            });
        }
        Ok(environment)
    }

    /// Activation admission: like [`Self::selectable`], but a
    /// `provisioning`/`booting` environment is admitted as valid intent and
    /// returned with `ready == false` instead of failing. Environment tools
    /// wait for readiness at call time.
    pub(crate) async fn activatable(
        &self,
        environment_id: &EnvironmentId,
        allowed_providers: Option<&BTreeSet<String>>,
        now_ms: i64,
    ) -> Result<(EnvironmentRecord, bool), EnvironmentResolveError> {
        match self
            .selectable(environment_id, allowed_providers, now_ms)
            .await
        {
            Ok(environment) => Ok((environment, true)),
            Err(EnvironmentResolveError::NotReady { .. }) => Ok((
                self.read_allowed(environment_id, allowed_providers).await?,
                false,
            )),
            Err(error) => Err(error),
        }
    }

    /// Status-aware selection admission. `provisioning`/`booting`
    /// environments are admitted as intent without a route probe (they cannot
    /// be reachable yet) and reported as `NotReady`; `failed`, `closing`, and
    /// `closed` are rejected with typed errors; everything else must prove the
    /// full data-plane route.
    pub(crate) async fn selectable(
        &self,
        environment_id: &EnvironmentId,
        allowed_providers: Option<&BTreeSet<String>>,
        _now_ms: i64,
    ) -> Result<EnvironmentRecord, EnvironmentResolveError> {
        let environment = self.read_allowed(environment_id, allowed_providers).await?;
        if let Some(provider_id) = environment.provider_id() {
            self.providers.read_provider(provider_id).await?;
        }
        match environment.status {
            EnvironmentStatus::Provisioning | EnvironmentStatus::Booting => {
                return Err(EnvironmentResolveError::NotReady {
                    environment_id: environment.environment_id.as_str().to_owned(),
                    status: environment.status,
                });
            }
            EnvironmentStatus::Failed => {
                return Err(EnvironmentResolveError::Failed {
                    environment_id: environment.environment_id.as_str().to_owned(),
                    message: environment
                        .metadata
                        .get(LIFECYCLE_ERROR_METADATA_KEY)
                        .cloned()
                        .unwrap_or_else(|| "environment provisioning failed".to_owned()),
                });
            }
            EnvironmentStatus::Closing | EnvironmentStatus::Closed => {
                return Err(EnvironmentResolveError::Closed {
                    environment_id: environment.environment_id.as_str().to_owned(),
                });
            }
            EnvironmentStatus::Ready | EnvironmentStatus::Offline | EnvironmentStatus::Unknown => {}
        }
        if let Some(gateway) = &self.gateway {
            let connection = gateway.connection_for(self.universe_id, &environment);
            if environment_client::EnvironmentDataClient::connect(
                &connection.endpoint,
                gateway.connect_options("lightspeed-environment-selection"),
            )
            .await
            .is_ok()
            {
                return Ok(environment);
            }
        }
        Err(EnvironmentResolveError::EnvironmentUnavailable {
            environment_id: environment.environment_id.as_str().to_owned(),
            status: "environment endpoint is not reachable".to_owned(),
        })
    }
}

#[derive(Debug, Error)]
pub(crate) enum EnvironmentResolveError {
    #[error(transparent)]
    Store(#[from] EnvironmentRegistryError),

    #[error("environment provider is not allowed by session config: {provider_id}")]
    ProviderNotAllowed { provider_id: String },

    #[error("environment is unavailable: {environment_id} ({status})")]
    EnvironmentUnavailable {
        environment_id: String,
        status: String,
    },

    /// The environment exists and is being provisioned or booted; it is not
    /// yet reachable but selecting it is valid intent.
    #[error("environment is not ready yet: {environment_id} ({status:?})")]
    NotReady {
        environment_id: String,
        status: EnvironmentStatus,
    },

    #[error("environment failed to provision: {environment_id}: {message}")]
    Failed {
        environment_id: String,
        message: String,
    },

    #[error("environment is closed: {environment_id}")]
    Closed { environment_id: String },
}

/// Metadata key under which the lifecycle reconciler records the provider's
/// failure message on a `failed` environment.
pub(crate) const LIFECYCLE_ERROR_METADATA_KEY: &str = "lifecycleError";

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use environment_protocol::shared::{EnvironmentTransport, ProviderTargetId};
    use environments::{
        CreateEnvironment, EnvironmentConnectionSpec, EnvironmentIncarnationId,
        EnvironmentProviderBindingId, EnvironmentProviderBindingStatus,
        EnvironmentProviderBindingStore, EnvironmentProviderId, EnvironmentProvisionRequestId,
        EnvironmentStatus, EnvironmentTemplateId, ObserveProvisionedEnvironment,
        PutEnvironmentProvider, PutEnvironmentProviderBinding,
    };

    use super::*;

    async fn resolver() -> (EnvironmentResolver, EnvironmentId) {
        let store = Arc::new(environments::InMemoryEnvironmentRegistryStore::new());
        let provider_id = EnvironmentProviderId::new("allowed");
        store
            .put_provider(PutEnvironmentProvider {
                provider_id: provider_id.clone(),
                display_name: None,
                controller_connection: EnvironmentConnectionSpec::new(
                    "http://controller.test",
                    EnvironmentTransport::Http,
                ),
                metadata: BTreeMap::new(),
                updated_at_ms: 10,
            })
            .await
            .expect("provider");
        store
            .put_provider_binding(PutEnvironmentProviderBinding {
                universe_id: store.universe_id(),
                binding_id: EnvironmentProviderBindingId::new("primary"),
                provider_id: provider_id.clone(),
                status: EnvironmentProviderBindingStatus::Enabled,
                expected_revision: None,
                metadata: BTreeMap::new(),
                updated_at_ms: 10,
            })
            .await
            .expect("binding");
        let environment_id = EnvironmentId::new("environment-1");
        let target_id = ProviderTargetId::new("target-1");
        store
            .create_environment(CreateEnvironment {
                request_id: EnvironmentProvisionRequestId::new("request-1"),
                environment_id: environment_id.clone(),
                incarnation_id: EnvironmentIncarnationId::new("incarnation-1"),
                binding_id: EnvironmentProviderBindingId::new("primary"),
                template_id: EnvironmentTemplateId::new("test-template"),
                display_name: None,
                metadata: BTreeMap::new(),
                origin_session: None,
                created_at_ms: 10,
            })
            .await
            .expect("create environment");
        store
            .observe_provisioned_environment(ObserveProvisionedEnvironment {
                environment_id: environment_id.clone(),
                provider_target_id: target_id.clone(),
                status: EnvironmentStatus::Offline,
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
    async fn offline_environment_without_gateway_is_unavailable_but_readable() {
        let (resolver, environment_id) = resolver().await;
        assert!(resolver.read_allowed(&environment_id, None).await.is_ok());
        assert!(matches!(
            resolver.selectable(&environment_id, None, 111).await,
            Err(EnvironmentResolveError::EnvironmentUnavailable { .. })
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn selection_is_status_aware() {
        let (resolver, environment_id) = resolver().await;
        let store = resolver.environments.clone();
        let observe = |status: EnvironmentStatus| {
            let store = store.clone();
            let environment_id = environment_id.clone();
            async move {
                store
                    .observe_provisioned_environment(ObserveProvisionedEnvironment {
                        environment_id,
                        provider_target_id: ProviderTargetId::new("target-1"),
                        status,
                        observed_at_ms: 20,
                    })
                    .await
                    .expect("observe");
            }
        };

        // A provisioning/booting environment is admitted as intent without a
        // probe and reported as not ready.
        observe(EnvironmentStatus::Provisioning).await;
        assert!(matches!(
            resolver.selectable(&environment_id, None, 30).await,
            Err(EnvironmentResolveError::NotReady {
                status: EnvironmentStatus::Provisioning,
                ..
            })
        ));
        observe(EnvironmentStatus::Booting).await;
        assert!(matches!(
            resolver.selectable(&environment_id, None, 30).await,
            Err(EnvironmentResolveError::NotReady {
                status: EnvironmentStatus::Booting,
                ..
            })
        ));
        let (record, ready) = resolver
            .activatable(&environment_id, None, 30)
            .await
            .expect("activation admits a booting environment");
        assert!(!ready);
        assert_eq!(record.status, EnvironmentStatus::Booting);

        store
            .fail_environment_lifecycle(environments::FailEnvironmentLifecycle {
                environment_id: environment_id.clone(),
                message: "no capacity".to_owned(),
                observed_at_ms: 40,
            })
            .await
            .expect("fail");
        assert!(matches!(
            resolver.selectable(&environment_id, None, 50).await,
            Err(EnvironmentResolveError::Failed { message, .. }) if message == "no capacity"
        ));

        store
            .begin_close_environment(environments::BeginCloseEnvironment {
                environment_id: environment_id.clone(),
                updated_at_ms: 60,
            })
            .await
            .expect("close");
        assert!(matches!(
            resolver.selectable(&environment_id, None, 70).await,
            Err(EnvironmentResolveError::Closed { .. })
        ));
    }
}
