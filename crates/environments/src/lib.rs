//! Runtime environment registry contracts.
//!
//! Providers advertise presence and universe environments own machine lifetime,
//! connection observations, and credential bindings.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr,
};

use async_trait::async_trait;
use auth::{AuthGrantId, AuthProviderId, SecretId};
pub use engine::EnvironmentId;
use engine::{StringIdError, validate_general_string_id};
use host_protocol::{
    control::{
        handshake::ControllerCapabilities,
        targets::{HostTargetStatus, HostTargetSummary},
    },
    shared::{
        HostCapabilities, HostConnectionSpec, HostPath, HostScope, HostTargetId, HostTransport,
        ImplementationInfo,
    },
};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

macro_rules! registry_string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                let value = value.into();
                Self::try_new(value)
                    .unwrap_or_else(|error| panic!("invalid {}: {error}", stringify!($name)))
            }

            pub fn try_new(value: impl Into<String>) -> Result<Self, StringIdError> {
                let value = value.into();
                validate_general_string_id(stringify!($name), &value)?;
                Ok(Self(value))
            }

            pub fn parse(value: impl Into<String>) -> Result<Self, StringIdError> {
                Self::try_new(value)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = StringIdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::try_new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = StringIdError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::try_new(value)
            }
        }

        impl FromStr for $name {
            type Err = StringIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::try_new(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::try_new(value).map_err(de::Error::custom)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

registry_string_id!(EnvironmentProviderId);
registry_string_id!(EnvironmentJobGroupId);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EnvironmentRegistryError {
    #[error("environment registry {kind} already exists: {id}")]
    AlreadyExists { kind: &'static str, id: String },

    #[error("environment registry {kind} not found: {id}")]
    NotFound { kind: &'static str, id: String },

    #[error("invalid environment registry request: {message}")]
    InvalidInput { message: String },

    #[error("environment registry store failure: {message}")]
    Store { message: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentProviderRecord {
    pub provider_id: EnvironmentProviderId,
    pub provider_kind: EnvironmentProviderKind,
    pub display_name: Option<String>,
    pub status: EnvironmentProviderStatus,
    pub controller_connection: HostControllerConnectionSpec,
    pub capabilities: EnvironmentProviderCapabilities,
    pub implementation: ImplementationInfo,
    pub last_seen_ms: i64,
    pub lease_expires_ms: i64,
    pub metadata: BTreeMap<String, String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl EnvironmentProviderRecord {
    pub fn is_live_at(&self, now_ms: i64) -> bool {
        self.status == EnvironmentProviderStatus::Online && self.lease_expires_ms > now_ms
    }

    pub fn presence_at(&self, now_ms: i64) -> EnvironmentProviderPresence {
        match self.status {
            EnvironmentProviderStatus::Offline => EnvironmentProviderPresence::Offline,
            EnvironmentProviderStatus::Online if self.lease_expires_ms > now_ms => {
                EnvironmentProviderPresence::Online
            }
            EnvironmentProviderStatus::Online => EnvironmentProviderPresence::Stale,
        }
    }

    pub fn validate(&self) -> Result<(), EnvironmentRegistryError> {
        validate_nonempty_optional("display_name", self.display_name.as_deref())?;
        self.controller_connection.validate()?;
        self.capabilities.validate()?;
        validate_nonempty_string("implementation name", &self.implementation.name)?;
        validate_metadata(&self.metadata)?;
        validate_nonnegative_i64(self.last_seen_ms, "last_seen_ms")?;
        validate_nonnegative_i64(self.lease_expires_ms, "lease_expires_ms")?;
        validate_timestamps(self.created_at_ms, self.updated_at_ms)?;
        if self.lease_expires_ms < self.last_seen_ms {
            return invalid("lease_expires_ms must be >= last_seen_ms");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterEnvironmentProvider {
    pub provider_id: EnvironmentProviderId,
    pub provider_kind: EnvironmentProviderKind,
    pub display_name: Option<String>,
    pub controller_connection: HostControllerConnectionSpec,
    pub capabilities: EnvironmentProviderCapabilities,
    pub implementation: ImplementationInfo,
    pub lease_ttl_ms: i64,
    pub metadata: BTreeMap<String, String>,
    pub observed_at_ms: i64,
}

impl RegisterEnvironmentProvider {
    pub fn into_record(self) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
        validate_nonnegative_i64(self.observed_at_ms, "observed_at_ms")?;
        validate_positive_i64(self.lease_ttl_ms, "lease_ttl_ms")?;
        let lease_expires_ms = self
            .observed_at_ms
            .checked_add(self.lease_ttl_ms)
            .ok_or_else(|| EnvironmentRegistryError::InvalidInput {
                message: "lease expiry timestamp overflowed".to_owned(),
            })?;
        let record = EnvironmentProviderRecord {
            provider_id: self.provider_id,
            provider_kind: self.provider_kind,
            display_name: self.display_name,
            status: EnvironmentProviderStatus::Online,
            controller_connection: self.controller_connection,
            capabilities: self.capabilities,
            implementation: self.implementation,
            last_seen_ms: self.observed_at_ms,
            lease_expires_ms,
            metadata: self.metadata,
            created_at_ms: self.observed_at_ms,
            updated_at_ms: self.observed_at_ms,
        };
        record.validate()?;
        Ok(record)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservedEnvironmentTarget {
    pub target: HostTargetSummary,
    pub connection: HostConnectionSpec,
}

impl ObservedEnvironmentTarget {
    pub fn validate(&self) -> Result<(), EnvironmentRegistryError> {
        if self.target.target_id != self.connection.target_id {
            return invalid("observed target and connection target ids must match");
        }
        validate_host_connection(&self.connection)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentProviderHeartbeat {
    pub provider_id: EnvironmentProviderId,
    pub observed_at_ms: i64,
    pub lease_ttl_ms: Option<i64>,
    pub observed_targets: Vec<ObservedEnvironmentTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateEnvironmentProviderStatus {
    pub provider_id: EnvironmentProviderId,
    pub status: EnvironmentProviderStatus,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListEnvironmentProviders {
    pub status: Option<EnvironmentProviderStatus>,
    pub provider_kind: Option<EnvironmentProviderKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentProviderKind {
    Sandbox,
    Bridge,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentProviderStatus {
    Online,
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentProviderPresence {
    Online,
    Stale,
    Offline,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostControllerConnectionSpec {
    pub endpoint: String,
    pub transport: HostTransport,
}

impl HostControllerConnectionSpec {
    pub fn new(endpoint: impl Into<String>, transport: HostTransport) -> Self {
        Self {
            endpoint: endpoint.into(),
            transport,
        }
    }

    pub fn validate(&self) -> Result<(), EnvironmentRegistryError> {
        validate_endpoint("host controller endpoint", &self.endpoint)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentProviderCapabilities {
    #[serde(default)]
    pub list_targets: bool,
    #[serde(default)]
    pub create_target: bool,
    #[serde(default)]
    pub get_target: bool,
    #[serde(default)]
    pub close_target: bool,
}

impl EnvironmentProviderCapabilities {
    pub fn from_controller(value: ControllerCapabilities) -> Self {
        Self {
            list_targets: value.list_targets,
            create_target: value.create_target,
            get_target: value.get_target,
            close_target: value.close_target,
        }
    }

    pub fn validate(&self) -> Result<(), EnvironmentRegistryError> {
        if !self.list_targets && !self.create_target && !self.get_target && !self.close_target {
            return invalid("environment provider must expose at least one controller capability");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentOrigin {
    Provided,
    Provisioned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentRecord {
    pub environment_id: EnvironmentId,
    pub provider_id: EnvironmentProviderId,
    pub provider_target_id: HostTargetId,
    pub origin: EnvironmentOrigin,
    pub display_name: Option<String>,
    pub status: HostTargetStatus,
    pub scope: HostScope,
    pub capabilities: HostCapabilities,
    pub connection: HostConnectionSpec,
    pub default_cwd: Option<HostPath>,
    pub metadata: BTreeMap<String, String>,
    pub observed_at_ms: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl EnvironmentRecord {
    pub fn validate(&self) -> Result<(), EnvironmentRegistryError> {
        validate_host_target_id(&self.provider_target_id)?;
        if self.connection.target_id != self.provider_target_id {
            return invalid("instance connection target id must equal provider_target_id");
        }
        validate_host_connection(&self.connection)?;
        validate_nonempty_optional("display_name", self.display_name.as_deref())?;
        validate_metadata(&self.metadata)?;
        validate_nonnegative_i64(self.observed_at_ms, "observed_at_ms")?;
        validate_timestamps(self.created_at_ms, self.updated_at_ms)
    }

    pub fn is_attachable(&self) -> bool {
        matches!(self.status, HostTargetStatus::Ready)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObserveEnvironment {
    pub environment_id: EnvironmentId,
    pub provider_id: EnvironmentProviderId,
    pub provider_target_id: HostTargetId,
    pub origin: EnvironmentOrigin,
    pub display_name: Option<String>,
    pub status: HostTargetStatus,
    pub scope: HostScope,
    pub capabilities: HostCapabilities,
    pub connection: HostConnectionSpec,
    pub default_cwd: Option<HostPath>,
    pub metadata: BTreeMap<String, String>,
    pub observed_at_ms: i64,
}

impl ObserveEnvironment {
    pub fn from_observation(
        environment_id: EnvironmentId,
        provider_id: EnvironmentProviderId,
        origin: EnvironmentOrigin,
        observation: ObservedEnvironmentTarget,
        observed_at_ms: i64,
    ) -> Self {
        Self {
            environment_id,
            provider_id,
            provider_target_id: observation.target.target_id,
            origin,
            display_name: observation.target.display_name,
            status: observation.target.status,
            scope: observation.target.scope,
            capabilities: observation.connection.capabilities.clone(),
            default_cwd: observation
                .connection
                .default_cwd
                .clone()
                .or(observation.target.default_cwd),
            connection: observation.connection,
            metadata: observation.target.metadata,
            observed_at_ms,
        }
    }

    pub fn into_record(self) -> EnvironmentRecord {
        EnvironmentRecord {
            environment_id: self.environment_id,
            provider_id: self.provider_id,
            provider_target_id: self.provider_target_id,
            origin: self.origin,
            display_name: self.display_name,
            status: self.status,
            scope: self.scope,
            capabilities: self.capabilities,
            connection: self.connection,
            default_cwd: self.default_cwd,
            metadata: self.metadata,
            observed_at_ms: self.observed_at_ms,
            created_at_ms: self.observed_at_ms,
            updated_at_ms: self.observed_at_ms,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListEnvironments {
    pub provider_id: Option<EnvironmentProviderId>,
    pub status: Option<HostTargetStatus>,
    pub origin: Option<EnvironmentOrigin>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateEnvironmentStatus {
    pub environment_id: EnvironmentId,
    pub status: HostTargetStatus,
    pub observed_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeginCloseEnvironment {
    pub environment_id: EnvironmentId,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentCredentialRecord {
    pub environment_id: EnvironmentId,
    pub env_name: String,
    pub source: EnvironmentCredentialSource,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl EnvironmentCredentialRecord {
    pub fn validate(&self) -> Result<(), EnvironmentRegistryError> {
        validate_env_name(&self.env_name)?;
        validate_timestamps(self.created_at_ms, self.updated_at_ms)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PutEnvironmentCredential {
    pub environment_id: EnvironmentId,
    pub env_name: String,
    pub source: EnvironmentCredentialSource,
    pub created_at_ms: i64,
}

impl PutEnvironmentCredential {
    pub fn into_record(self) -> EnvironmentCredentialRecord {
        EnvironmentCredentialRecord {
            environment_id: self.environment_id,
            env_name: self.env_name,
            source: self.source,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.created_at_ms,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EnvironmentCredentialSource {
    AuthGrant { grant_id: AuthGrantId },
    AuthProviderCredential { provider_id: AuthProviderId },
    DirectSecret { secret_id: SecretId },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListEnvironmentCredentials {
    pub environment_id: EnvironmentId,
}

#[async_trait]
pub trait EnvironmentProviderStore: Send + Sync {
    async fn register_provider(
        &self,
        record: RegisterEnvironmentProvider,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError>;
    async fn read_provider(
        &self,
        provider_id: &EnvironmentProviderId,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError>;
    async fn list_providers(
        &self,
        request: ListEnvironmentProviders,
    ) -> Result<Vec<EnvironmentProviderRecord>, EnvironmentRegistryError>;
    async fn update_provider_heartbeat(
        &self,
        heartbeat: EnvironmentProviderHeartbeat,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError>;
    async fn update_provider_status(
        &self,
        request: UpdateEnvironmentProviderStatus,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError>;
    async fn delete_provider(
        &self,
        provider_id: &EnvironmentProviderId,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError>;
}

#[async_trait]
pub trait EnvironmentStore: Send + Sync {
    async fn observe_environment(
        &self,
        record: ObserveEnvironment,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError>;
    async fn read_environment(
        &self,
        environment_id: &EnvironmentId,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError>;
    async fn read_environment_by_provider_target(
        &self,
        provider_id: &EnvironmentProviderId,
        provider_target_id: &HostTargetId,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError>;
    async fn list_environments(
        &self,
        request: ListEnvironments,
    ) -> Result<Vec<EnvironmentRecord>, EnvironmentRegistryError>;
    async fn mark_missing_provided_environments_unknown(
        &self,
        provider_id: &EnvironmentProviderId,
        observed_target_ids: &BTreeSet<HostTargetId>,
        observed_at_ms: i64,
    ) -> Result<Vec<EnvironmentRecord>, EnvironmentRegistryError>;
    async fn update_environment_status(
        &self,
        request: UpdateEnvironmentStatus,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError>;
    async fn begin_close_environment(
        &self,
        request: BeginCloseEnvironment,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError>;
}

#[async_trait]
pub trait EnvironmentCredentialStore: Send + Sync {
    async fn bind_credential(
        &self,
        record: PutEnvironmentCredential,
    ) -> Result<EnvironmentCredentialRecord, EnvironmentRegistryError>;
    async fn list_credentials(
        &self,
        request: ListEnvironmentCredentials,
    ) -> Result<Vec<EnvironmentCredentialRecord>, EnvironmentRegistryError>;
    async fn unbind_credential(
        &self,
        environment_id: &EnvironmentId,
        env_name: &str,
    ) -> Result<EnvironmentCredentialRecord, EnvironmentRegistryError>;
}

mod memory;
pub use memory::InMemoryEnvironmentRegistryStore;

fn invalid<T>(message: impl Into<String>) -> Result<T, EnvironmentRegistryError> {
    Err(EnvironmentRegistryError::InvalidInput {
        message: message.into(),
    })
}

fn validate_timestamps(
    created_at_ms: i64,
    updated_at_ms: i64,
) -> Result<(), EnvironmentRegistryError> {
    validate_nonnegative_i64(created_at_ms, "created_at_ms")?;
    validate_nonnegative_i64(updated_at_ms, "updated_at_ms")?;
    if updated_at_ms < created_at_ms {
        return invalid("updated_at_ms must be >= created_at_ms");
    }
    Ok(())
}

fn validate_endpoint(name: &'static str, value: &str) -> Result<(), EnvironmentRegistryError> {
    validate_nonempty_string(name, value)?;
    if value.chars().any(char::is_whitespace) {
        return invalid(format!("{name} must not contain whitespace"));
    }
    Ok(())
}

fn validate_host_connection(value: &HostConnectionSpec) -> Result<(), EnvironmentRegistryError> {
    validate_host_target_id(&value.target_id)?;
    validate_endpoint("host connection endpoint", &value.endpoint)
}

fn validate_host_target_id(value: &HostTargetId) -> Result<(), EnvironmentRegistryError> {
    validate_general_string_id("target_id", value.as_str()).map_err(|error| {
        EnvironmentRegistryError::InvalidInput {
            message: error.to_string(),
        }
    })
}

fn validate_metadata(value: &BTreeMap<String, String>) -> Result<(), EnvironmentRegistryError> {
    for (key, value) in value {
        validate_nonempty_string("metadata key", key)?;
        validate_nonempty_string("metadata value", value)?;
    }
    Ok(())
}

fn validate_nonempty_optional(
    name: &'static str,
    value: Option<&str>,
) -> Result<(), EnvironmentRegistryError> {
    if value.is_some_and(str::is_empty) {
        return invalid(format!("{name} must not be empty"));
    }
    Ok(())
}

fn validate_nonempty_string(
    name: &'static str,
    value: &str,
) -> Result<(), EnvironmentRegistryError> {
    if value.is_empty() {
        return invalid(format!("{name} must not be empty"));
    }
    Ok(())
}

pub(crate) fn validate_nonnegative_i64(
    value: i64,
    name: &'static str,
) -> Result<(), EnvironmentRegistryError> {
    if value < 0 {
        return invalid(format!("{name} must be nonnegative"));
    }
    Ok(())
}

pub(crate) fn validate_positive_i64(
    value: i64,
    name: &'static str,
) -> Result<(), EnvironmentRegistryError> {
    if value <= 0 {
        return invalid(format!("{name} must be positive"));
    }
    Ok(())
}

fn validate_env_name(value: &str) -> Result<(), EnvironmentRegistryError> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return invalid("credential env_name must not be empty");
    };
    if !(first == '_' || first.is_ascii_alphabetic())
        || chars.any(|value| !(value == '_' || value.is_ascii_alphanumeric()))
    {
        return invalid("credential env_name must match [A-Za-z_][A-Za-z0-9_]*");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
