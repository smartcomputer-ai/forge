//! Registration keys: the universe-management surface of key-based outbound
//! `envd` registration. Keys are minted here; admission itself happens on
//! the environment gateway's connect route.

use ::environments::{
    BeginCloseEnvironment, CreateEnvironmentRegistrationKey, EnvironmentRegistrationKeyRecord,
    EnvironmentRegistrationKeyStatus, EnvironmentRegistrationKeyStore, EnvironmentStatus,
    EnvironmentStore, ListEnvironments, RegistrationKeyPolicy, RevokeEnvironmentRegistrationKey,
    mint_registration_key,
};

use super::*;
use super::{
    environment_lifecycle::parse_registration_key_id,
    environment_providers::{identity_mode_view, registry_identity_mode},
};

impl GatewayAgentApi {
    pub(super) async fn create_environment_registration_key_record(
        &self,
        params: EnvironmentRegistrationKeyCreateParams,
    ) -> Result<EnvironmentRegistrationKeyCreateResponse, AgentApiError> {
        let now = now_ms()?;
        let minted = mint_registration_key(
            allocate_registration_key_id(),
            RegistrationKeyPolicy {
                display_name: params.display_name,
                identity_mode: registry_identity_mode(params.identity_mode),
                max_active_environments: params.max_active_environments,
                ephemeral_disconnect_grace_ms: params.ephemeral_disconnect_grace_ms,
                expires_at_ms: params.expires_at_ms,
            },
            now,
        )
        .map_err(map_environments_error)?;
        let record = EnvironmentRegistrationKeyStore::create_registration_key(
            self.store.as_ref(),
            CreateEnvironmentRegistrationKey {
                secret_hash: minted.secret_hash,
                record: minted.record,
            },
        )
        .await
        .map_err(map_environments_error)?;
        tracing::info!(
            target: "temporal_server",
            registration_key_id = %record.registration_key_id,
            display_name = %record.display_name,
            identity_mode = record.identity_mode.as_str(),
            "environment registration key created"
        );
        Ok(EnvironmentRegistrationKeyCreateResponse {
            registration_key: self.registration_key_view(&record, now).await?,
            secret: EnvironmentRegistrationSecretView(minted.secret.expose().to_owned()),
        })
    }

    pub(super) async fn read_environment_registration_key_record(
        &self,
        params: EnvironmentRegistrationKeyReadParams,
    ) -> Result<EnvironmentRegistrationKeyReadResponse, AgentApiError> {
        let registration_key_id = parse_registration_key_id(params.registration_key_id)?;
        let record = EnvironmentRegistrationKeyStore::read_registration_key(
            self.store.as_ref(),
            &registration_key_id,
        )
        .await
        .map_err(map_environments_error)?;
        Ok(EnvironmentRegistrationKeyReadResponse {
            registration_key: self.registration_key_view(&record, now_ms()?).await?,
        })
    }

    pub(super) async fn list_environment_registration_key_records(
        &self,
        _params: EnvironmentRegistrationKeyListParams,
    ) -> Result<EnvironmentRegistrationKeyListResponse, AgentApiError> {
        let now = now_ms()?;
        let records = EnvironmentRegistrationKeyStore::list_registration_keys(self.store.as_ref())
            .await
            .map_err(map_environments_error)?;
        let mut registration_keys = Vec::with_capacity(records.len());
        for record in &records {
            registration_keys.push(self.registration_key_view(record, now).await?);
        }
        Ok(EnvironmentRegistrationKeyListResponse { registration_keys })
    }

    pub(super) async fn revoke_environment_registration_key_record(
        &self,
        params: EnvironmentRegistrationKeyRevokeParams,
    ) -> Result<EnvironmentRegistrationKeyRevokeResponse, AgentApiError> {
        let registration_key_id = parse_registration_key_id(params.registration_key_id)?;
        let now = now_ms()?;
        let record = EnvironmentRegistrationKeyStore::revoke_registration_key(
            self.store.as_ref(),
            RevokeEnvironmentRegistrationKey {
                registration_key_id: registration_key_id.clone(),
                revoked_at_ms: now,
            },
        )
        .await
        .map_err(map_environments_error)?;
        let mut closed_environment_ids = Vec::new();
        if params.close_environments {
            let environments = EnvironmentStore::list_environments(
                self.store.as_ref(),
                ListEnvironments {
                    registration_key_id: Some(registration_key_id.clone()),
                    ..ListEnvironments::default()
                },
            )
            .await
            .map_err(map_environments_error)?;
            for environment in environments {
                if matches!(
                    environment.status,
                    EnvironmentStatus::Closing | EnvironmentStatus::Closed
                ) {
                    continue;
                }
                EnvironmentStore::begin_close_environment(
                    self.store.as_ref(),
                    BeginCloseEnvironment {
                        environment_id: environment.environment_id.clone(),
                        updated_at_ms: now,
                    },
                )
                .await
                .map_err(map_environments_error)?;
                closed_environment_ids.push(environment.environment_id.to_string());
            }
        }
        tracing::info!(
            target: "temporal_server",
            registration_key_id = %record.registration_key_id,
            closed = closed_environment_ids.len(),
            "environment registration key revoked"
        );
        Ok(EnvironmentRegistrationKeyRevokeResponse {
            registration_key: self.registration_key_view(&record, now).await?,
            closed_environment_ids,
        })
    }

    async fn registration_key_view(
        &self,
        record: &EnvironmentRegistrationKeyRecord,
        now_ms: i64,
    ) -> Result<EnvironmentRegistrationKeyView, AgentApiError> {
        let usage = EnvironmentRegistrationKeyStore::registration_key_usage(
            self.store.as_ref(),
            &record.registration_key_id,
        )
        .await
        .map_err(map_environments_error)?;
        Ok(EnvironmentRegistrationKeyView {
            registration_key_id: record.registration_key_id.to_string(),
            display_name: record.display_name.clone(),
            key_prefix: record.key_prefix.clone(),
            identity_mode: identity_mode_view(record.identity_mode),
            max_active_environments: record.max_active_environments,
            ephemeral_disconnect_grace_ms: record.ephemeral_disconnect_grace_ms(),
            expires_at_ms: record.expires_at_ms,
            status: match record.status(now_ms) {
                EnvironmentRegistrationKeyStatus::Active => {
                    EnvironmentRegistrationKeyStatusView::Active
                }
                EnvironmentRegistrationKeyStatus::Revoked => {
                    EnvironmentRegistrationKeyStatusView::Revoked
                }
                EnvironmentRegistrationKeyStatus::Expired => {
                    EnvironmentRegistrationKeyStatusView::Expired
                }
            },
            registered_environment_count: usage.registered,
            active_environment_count: usage.active,
            last_registered_at_ms: usage.last_registered_at_ms,
            created_at_ms: record.created_at_ms,
            revoked_at_ms: record.revoked_at_ms,
        })
    }
}

fn allocate_registration_key_id() -> ::environments::EnvironmentRegistrationKeyId {
    ::environments::EnvironmentRegistrationKeyId::new(format!(
        "registration_key_{}",
        uuid::Uuid::new_v4().simple()
    ))
}
