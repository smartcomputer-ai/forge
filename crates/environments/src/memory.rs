use std::{
    collections::{BTreeMap, BTreeSet},
    sync::RwLock,
};

use async_trait::async_trait;
use host_protocol::{control::targets::HostTargetStatus, shared::HostTargetId};

use super::*;

type CredentialKey = (EnvironmentId, String);
type ProviderTargetKey = (EnvironmentProviderId, HostTargetId);

#[derive(Default)]
struct RegistryState {
    providers: BTreeMap<EnvironmentProviderId, EnvironmentProviderRecord>,
    environments: BTreeMap<EnvironmentId, EnvironmentRecord>,
    provider_targets: BTreeMap<ProviderTargetKey, EnvironmentId>,
    credentials: BTreeMap<CredentialKey, EnvironmentCredentialRecord>,
}

#[derive(Default)]
pub struct InMemoryEnvironmentRegistryStore {
    state: RwLock<RegistryState>,
}

impl InMemoryEnvironmentRegistryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_state(
        &self,
    ) -> Result<std::sync::RwLockReadGuard<'_, RegistryState>, EnvironmentRegistryError> {
        self.state
            .read()
            .map_err(|_| EnvironmentRegistryError::Store {
                message: "environment registry read lock poisoned".to_owned(),
            })
    }

    fn write_state(
        &self,
    ) -> Result<std::sync::RwLockWriteGuard<'_, RegistryState>, EnvironmentRegistryError> {
        self.state
            .write()
            .map_err(|_| EnvironmentRegistryError::Store {
                message: "environment registry write lock poisoned".to_owned(),
            })
    }
}

#[async_trait]
impl EnvironmentProviderStore for InMemoryEnvironmentRegistryStore {
    async fn register_provider(
        &self,
        record: RegisterEnvironmentProvider,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
        let mut record = record.into_record()?;
        let mut state = self.write_state()?;
        if let Some(existing) = state.providers.get(&record.provider_id) {
            record.created_at_ms = existing.created_at_ms;
        }
        state
            .providers
            .insert(record.provider_id.clone(), record.clone());
        Ok(record)
    }

    async fn read_provider(
        &self,
        provider_id: &EnvironmentProviderId,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
        self.read_state()?
            .providers
            .get(provider_id)
            .cloned()
            .ok_or_else(|| not_found("environment_provider", provider_id))
    }

    async fn list_providers(
        &self,
        request: ListEnvironmentProviders,
    ) -> Result<Vec<EnvironmentProviderRecord>, EnvironmentRegistryError> {
        Ok(self
            .read_state()?
            .providers
            .values()
            .filter(|provider| request.status.is_none_or(|value| provider.status == value))
            .filter(|provider| {
                request
                    .provider_kind
                    .is_none_or(|value| provider.provider_kind == value)
            })
            .cloned()
            .collect())
    }

    async fn update_provider_heartbeat(
        &self,
        heartbeat: EnvironmentProviderHeartbeat,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
        validate_nonnegative_i64(heartbeat.observed_at_ms, "observed_at_ms")?;
        if let Some(ttl) = heartbeat.lease_ttl_ms {
            validate_positive_i64(ttl, "lease_ttl_ms")?;
        }
        let mut state = self.write_state()?;
        let provider = state
            .providers
            .get_mut(&heartbeat.provider_id)
            .ok_or_else(|| not_found("environment_provider", &heartbeat.provider_id))?;
        let ttl = heartbeat.lease_ttl_ms.unwrap_or_else(|| {
            provider
                .lease_expires_ms
                .saturating_sub(provider.last_seen_ms)
        });
        validate_positive_i64(ttl, "lease_ttl_ms")?;
        provider.last_seen_ms = heartbeat.observed_at_ms;
        provider.lease_expires_ms = heartbeat.observed_at_ms.checked_add(ttl).ok_or_else(|| {
            EnvironmentRegistryError::InvalidInput {
                message: "lease expiry timestamp overflowed".to_owned(),
            }
        })?;
        provider.updated_at_ms = heartbeat.observed_at_ms;
        provider.status = EnvironmentProviderStatus::Online;
        provider.validate()?;
        Ok(provider.clone())
    }

    async fn update_provider_status(
        &self,
        request: UpdateEnvironmentProviderStatus,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
        validate_nonnegative_i64(request.updated_at_ms, "updated_at_ms")?;
        let mut state = self.write_state()?;
        let provider = state
            .providers
            .get_mut(&request.provider_id)
            .ok_or_else(|| not_found("environment_provider", &request.provider_id))?;
        provider.status = request.status;
        provider.updated_at_ms = request.updated_at_ms;
        provider.validate()?;
        Ok(provider.clone())
    }

    async fn delete_provider(
        &self,
        provider_id: &EnvironmentProviderId,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
        self.write_state()?
            .providers
            .remove(provider_id)
            .ok_or_else(|| not_found("environment_provider", provider_id))
    }
}

#[async_trait]
impl EnvironmentStore for InMemoryEnvironmentRegistryStore {
    async fn observe_environment(
        &self,
        record: ObserveEnvironment,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError> {
        let incoming = record.into_record();
        incoming.validate()?;
        let mut state = self.write_state()?;
        if !state.providers.contains_key(&incoming.provider_id) {
            return Err(not_found("environment_provider", &incoming.provider_id));
        }
        let provider_key = (
            incoming.provider_id.clone(),
            incoming.provider_target_id.clone(),
        );
        let environment_id = state
            .provider_targets
            .get(&provider_key)
            .cloned()
            .unwrap_or_else(|| incoming.environment_id.clone());
        let record = if let Some(existing) = state.environments.get(&environment_id) {
            if incoming.observed_at_ms < existing.observed_at_ms {
                return Ok(existing.clone());
            }
            EnvironmentRecord {
                environment_id: environment_id.clone(),
                origin: if existing.origin == EnvironmentOrigin::Provisioned {
                    EnvironmentOrigin::Provisioned
                } else {
                    incoming.origin
                },
                created_at_ms: existing.created_at_ms,
                ..incoming
            }
        } else {
            EnvironmentRecord {
                environment_id: environment_id.clone(),
                ..incoming
            }
        };
        record.validate()?;
        state
            .provider_targets
            .insert(provider_key, environment_id.clone());
        state.environments.insert(environment_id, record.clone());
        Ok(record)
    }

    async fn read_environment(
        &self,
        environment_id: &EnvironmentId,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError> {
        self.read_state()?
            .environments
            .get(environment_id)
            .cloned()
            .ok_or_else(|| not_found("environment", environment_id))
    }

    async fn read_environment_by_provider_target(
        &self,
        provider_id: &EnvironmentProviderId,
        provider_target_id: &HostTargetId,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError> {
        let state = self.read_state()?;
        let key = (provider_id.clone(), provider_target_id.clone());
        let environment_id =
            state
                .provider_targets
                .get(&key)
                .ok_or_else(|| EnvironmentRegistryError::NotFound {
                    kind: "environment",
                    id: format!("{provider_id}/{provider_target_id}"),
                })?;
        state
            .environments
            .get(environment_id)
            .cloned()
            .ok_or_else(|| not_found("environment", environment_id))
    }

    async fn list_environments(
        &self,
        request: ListEnvironments,
    ) -> Result<Vec<EnvironmentRecord>, EnvironmentRegistryError> {
        Ok(self
            .read_state()?
            .environments
            .values()
            .filter(|record| {
                request
                    .provider_id
                    .as_ref()
                    .is_none_or(|id| id == &record.provider_id)
            })
            .filter(|record| request.status.is_none_or(|status| status == record.status))
            .filter(|record| request.origin.is_none_or(|origin| origin == record.origin))
            .cloned()
            .collect())
    }

    async fn mark_missing_provided_environments_unknown(
        &self,
        provider_id: &EnvironmentProviderId,
        observed_target_ids: &BTreeSet<HostTargetId>,
        observed_at_ms: i64,
    ) -> Result<Vec<EnvironmentRecord>, EnvironmentRegistryError> {
        let mut state = self.write_state()?;
        let mut changed = Vec::new();
        for record in state.environments.values_mut() {
            if &record.provider_id == provider_id
                && record.origin == EnvironmentOrigin::Provided
                && !observed_target_ids.contains(&record.provider_target_id)
                && record.observed_at_ms <= observed_at_ms
            {
                record.status = HostTargetStatus::Unknown;
                record.observed_at_ms = observed_at_ms;
                record.updated_at_ms = observed_at_ms;
                changed.push(record.clone());
            }
        }
        Ok(changed)
    }

    async fn update_environment_status(
        &self,
        request: UpdateEnvironmentStatus,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError> {
        let mut state = self.write_state()?;
        let record = state
            .environments
            .get_mut(&request.environment_id)
            .ok_or_else(|| not_found("environment", &request.environment_id))?;
        record.status = request.status;
        record.observed_at_ms = request.observed_at_ms;
        record.updated_at_ms = request.observed_at_ms;
        record.validate()?;
        Ok(record.clone())
    }

    async fn begin_close_environment(
        &self,
        request: BeginCloseEnvironment,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError> {
        let mut state = self.write_state()?;
        let record = state
            .environments
            .get_mut(&request.environment_id)
            .ok_or_else(|| not_found("environment", &request.environment_id))?;
        record.status = HostTargetStatus::Closing;
        record.updated_at_ms = request.updated_at_ms;
        Ok(record.clone())
    }
}

#[async_trait]
impl EnvironmentCredentialStore for InMemoryEnvironmentRegistryStore {
    async fn bind_credential(
        &self,
        record: PutEnvironmentCredential,
    ) -> Result<EnvironmentCredentialRecord, EnvironmentRegistryError> {
        let record = record.into_record();
        record.validate()?;
        let mut state = self.write_state()?;
        if !state.environments.contains_key(&record.environment_id) {
            return Err(not_found("environment", &record.environment_id));
        }
        let key = (record.environment_id.clone(), record.env_name.clone());
        let record = if let Some(existing) = state.credentials.get(&key) {
            EnvironmentCredentialRecord {
                created_at_ms: existing.created_at_ms,
                ..record
            }
        } else {
            record
        };
        state.credentials.insert(key, record.clone());
        Ok(record)
    }

    async fn list_credentials(
        &self,
        request: ListEnvironmentCredentials,
    ) -> Result<Vec<EnvironmentCredentialRecord>, EnvironmentRegistryError> {
        Ok(self
            .read_state()?
            .credentials
            .values()
            .filter(|record| record.environment_id == request.environment_id)
            .cloned()
            .collect())
    }

    async fn unbind_credential(
        &self,
        environment_id: &EnvironmentId,
        env_name: &str,
    ) -> Result<EnvironmentCredentialRecord, EnvironmentRegistryError> {
        self.write_state()?
            .credentials
            .remove(&(environment_id.clone(), env_name.to_owned()))
            .ok_or_else(|| EnvironmentRegistryError::NotFound {
                kind: "environment_credential",
                id: format!("{environment_id}/{env_name}"),
            })
    }
}

fn not_found(kind: &'static str, id: &impl ToString) -> EnvironmentRegistryError {
    EnvironmentRegistryError::NotFound {
        kind,
        id: id.to_string(),
    }
}
