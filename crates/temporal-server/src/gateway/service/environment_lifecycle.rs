use super::environment_providers::{environment_instance_view, registry_target_status};
use super::*;

use ::environments::{
    BeginCloseEnvironment, EnvironmentId, EnvironmentOrigin, EnvironmentProviderRecord,
    ListEnvironments, ObserveEnvironment, ObservedEnvironmentTarget, UpdateEnvironmentStatus,
};
use host_protocol::{
    control::targets::{
        AttachedHostSpec, CloseTargetParams, CreateTargetParams, HostTargetCreateRequest,
        HostTargetStatus, SandboxTargetSpec,
    },
    shared::HostPath,
};

impl GatewayAgentApi {
    pub(super) async fn create_environment_record(
        &self,
        params: EnvironmentCreateParams,
    ) -> Result<EnvironmentCreateResponse, AgentApiError> {
        let provider_id = parse_environment_provider_id(params.provider_id)?;
        let provider = self.read_live_environment_provider(&provider_id).await?;
        if !provider.capabilities.create_target {
            return Err(AgentApiError::rejected(format!(
                "environment provider does not support target creation: {provider_id}"
            )));
        }
        let mut controller = self
            .host_controller_connector
            .connect(&provider.controller_connection)
            .await?;
        let response = controller
            .create_target(&CreateTargetParams {
                request: host_target_create_request(params.request)?,
            })
            .await?;
        let observed_at_ms = now_ms()?;
        let environment = ::environments::EnvironmentStore::observe_environment(
            self.store.as_ref(),
            ObserveEnvironment::from_observation(
                allocate_environment_id(),
                provider_id,
                EnvironmentOrigin::Provisioned,
                ObservedEnvironmentTarget {
                    target: response.target,
                    connection: response.connection,
                },
                observed_at_ms,
            ),
        )
        .await
        .map_err(map_environments_error)?;
        Ok(EnvironmentCreateResponse {
            environment: environment_instance_view(&environment),
        })
    }

    pub(super) async fn read_environment_record(
        &self,
        params: EnvironmentReadParams,
    ) -> Result<EnvironmentReadResponse, AgentApiError> {
        let environment_id = parse_registry_environment_id(params.environment_id)?;
        let environment = ::environments::EnvironmentStore::read_environment(
            self.store.as_ref(),
            &environment_id,
        )
        .await
        .map_err(map_environments_error)?;
        Ok(EnvironmentReadResponse {
            environment: environment_instance_view(&environment),
        })
    }

    pub(super) async fn list_environment_records(
        &self,
        params: EnvironmentListParams,
    ) -> Result<EnvironmentListResponse, AgentApiError> {
        let provider_id = params
            .provider_id
            .map(parse_environment_provider_id)
            .transpose()?;
        let environments = ::environments::EnvironmentStore::list_environments(
            self.store.as_ref(),
            ListEnvironments {
                provider_id,
                status: params.status.map(registry_target_status),
                origin: None,
            },
        )
        .await
        .map_err(map_environments_error)?;
        Ok(EnvironmentListResponse {
            environments: environments.iter().map(environment_instance_view).collect(),
        })
    }

    pub(super) async fn close_environment_record(
        &self,
        params: EnvironmentCloseParams,
    ) -> Result<EnvironmentCloseResponse, AgentApiError> {
        let environment_id = parse_registry_environment_id(params.environment_id)?;
        let previous = ::environments::EnvironmentStore::read_environment(
            self.store.as_ref(),
            &environment_id,
        )
        .await
        .map_err(map_environments_error)?;
        let closing = ::environments::EnvironmentStore::begin_close_environment(
            self.store.as_ref(),
            BeginCloseEnvironment {
                environment_id: environment_id.clone(),
                updated_at_ms: now_ms()?,
            },
        )
        .await
        .map_err(map_environments_error)?;
        let result = async {
            let provider = self
                .read_live_environment_provider(&closing.provider_id)
                .await?;
            if !provider.capabilities.close_target {
                return Err(AgentApiError::rejected(format!(
                    "environment provider does not support target close: {}",
                    provider.provider_id
                )));
            }
            let mut controller = self
                .host_controller_connector
                .connect(&provider.controller_connection)
                .await?;
            controller
                .close_target(&CloseTargetParams {
                    target_id: closing.provider_target_id.clone(),
                    force: false,
                })
                .await
        }
        .await;
        let (status, error) = match result {
            Ok(response) => (response.status, None),
            Err(error) if error.kind == AgentApiErrorKind::Rejected => {
                (previous.status, Some(error))
            }
            Err(error) => (HostTargetStatus::Unknown, Some(error)),
        };
        let environment = ::environments::EnvironmentStore::update_environment_status(
            self.store.as_ref(),
            UpdateEnvironmentStatus {
                environment_id,
                status,
                observed_at_ms: now_ms()?,
            },
        )
        .await
        .map_err(map_environments_error)?;
        if let Some(error) = error {
            return Err(error);
        }
        Ok(EnvironmentCloseResponse {
            environment: environment_instance_view(&environment),
        })
    }

    pub(super) async fn read_live_environment_provider(
        &self,
        provider_id: &::environments::EnvironmentProviderId,
    ) -> Result<EnvironmentProviderRecord, AgentApiError> {
        let provider = ::environments::EnvironmentProviderStore::read_provider(
            self.store.as_ref(),
            provider_id,
        )
        .await
        .map_err(map_environments_error)?;
        if !provider.is_live_at(now_ms()?) {
            return Err(AgentApiError::rejected(format!(
                "environment provider lease is not live: {provider_id}"
            )));
        }
        Ok(provider)
    }
}

pub(super) fn parse_registry_environment_id(value: String) -> Result<EnvironmentId, AgentApiError> {
    EnvironmentId::try_new(value)
        .map_err(|error| AgentApiError::invalid_request(format!("invalid environment id: {error}")))
}

pub(super) fn allocate_environment_id() -> EnvironmentId {
    EnvironmentId::new(format!("environment_{}", uuid::Uuid::new_v4().simple()))
}

fn host_target_create_request(
    value: HostTargetCreateRequestView,
) -> Result<HostTargetCreateRequest, AgentApiError> {
    Ok(match value {
        HostTargetCreateRequestView::Sandbox { spec } => HostTargetCreateRequest::Sandbox {
            spec: SandboxTargetSpec {
                template: spec.template,
                image: spec.image,
                cwd: optional_host_path(spec.cwd, "cwd")?,
                env: spec.env,
                labels: spec.labels,
                provider_options: spec.provider_options,
            },
        },
        HostTargetCreateRequestView::AttachedHost { spec } => {
            HostTargetCreateRequest::AttachedHost {
                spec: AttachedHostSpec {
                    name: spec.name,
                    endpoint: spec.endpoint,
                    cwd: optional_host_path(spec.cwd, "cwd")?,
                    labels: spec.labels,
                    provider_options: spec.provider_options,
                },
            }
        }
        HostTargetCreateRequestView::Provider {
            provider_type,
            spec,
        } => {
            if provider_type.is_empty() {
                return Err(AgentApiError::invalid_request(
                    "provider type must not be empty",
                ));
            }
            HostTargetCreateRequest::Provider {
                provider_type,
                spec,
            }
        }
    })
}

fn optional_host_path(
    value: Option<String>,
    name: &str,
) -> Result<Option<HostPath>, AgentApiError> {
    value
        .map(HostPath::new)
        .transpose()
        .map_err(|error| AgentApiError::invalid_request(format!("invalid {name}: {error}")))
}
