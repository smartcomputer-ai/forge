use std::collections::BTreeSet;

use async_trait::async_trait;
use auth::{AuthGrantId, AuthProviderId, SecretId};
use environments::{
    BeginCloseEnvironment, EnvironmentCredentialRecord, EnvironmentCredentialSource,
    EnvironmentCredentialStore, EnvironmentId, EnvironmentOrigin, EnvironmentProviderHeartbeat,
    EnvironmentProviderId, EnvironmentProviderKind, EnvironmentProviderRecord,
    EnvironmentProviderStatus, EnvironmentProviderStore, EnvironmentRecord,
    EnvironmentRegistryError, EnvironmentStore, ListEnvironmentCredentials,
    ListEnvironmentProviders, ListEnvironments, ObserveEnvironment, PutEnvironmentCredential,
    RegisterEnvironmentProvider, UpdateEnvironmentProviderStatus, UpdateEnvironmentStatus,
};
use host_protocol::{
    control::targets::HostTargetStatus,
    shared::{HostPath, HostTargetId},
};
use sqlx::Row;

use crate::PgStore;

const PROVIDER_COLUMNS: &str = r#"
    provider_id, provider_kind, display_name, status,
    controller_connection_json, capabilities_json, implementation_json,
    last_seen_ms, lease_expires_ms, metadata_json, created_at_ms, updated_at_ms
"#;

const ENVIRONMENT_COLUMNS: &str = r#"
    environment_id, provider_id, provider_target_id, origin, display_name, status,
    scope_json, capabilities_json, connection_json, default_cwd, metadata_json,
    observed_at_ms, created_at_ms, updated_at_ms
"#;

const CREDENTIAL_COLUMNS: &str = r#"
    environment_id, env_name, source_kind, grant_id, auth_provider_id,
    secret_id, created_at_ms, updated_at_ms
"#;

#[async_trait]
impl EnvironmentProviderStore for PgStore {
    async fn register_provider(
        &self,
        request: RegisterEnvironmentProvider,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
        self.ensure_universe()
            .await
            .map_err(|error| store_error("ensure universe", error))?;
        let record = request.into_record()?;
        let query = format!(
            r#"
            INSERT INTO environment_providers (
                universe_id, provider_id, provider_kind, display_name, status,
                controller_connection_json, capabilities_json, implementation_json,
                last_seen_ms, lease_expires_ms, metadata_json, created_at_ms, updated_at_ms
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
            ON CONFLICT (universe_id, provider_id) DO UPDATE SET
                provider_kind = EXCLUDED.provider_kind,
                display_name = EXCLUDED.display_name,
                status = EXCLUDED.status,
                controller_connection_json = EXCLUDED.controller_connection_json,
                capabilities_json = EXCLUDED.capabilities_json,
                implementation_json = EXCLUDED.implementation_json,
                last_seen_ms = EXCLUDED.last_seen_ms,
                lease_expires_ms = EXCLUDED.lease_expires_ms,
                metadata_json = EXCLUDED.metadata_json,
                updated_at_ms = EXCLUDED.updated_at_ms
            RETURNING {PROVIDER_COLUMNS}
            "#
        );
        let row = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(record.provider_id.as_str())
            .bind(provider_kind_to_str(record.provider_kind))
            .bind(record.display_name.as_deref())
            .bind(provider_status_to_str(record.status))
            .bind(json_value(
                "encode provider controller connection",
                &record.controller_connection,
            )?)
            .bind(json_value(
                "encode provider capabilities",
                &record.capabilities,
            )?)
            .bind(json_value(
                "encode provider implementation",
                &record.implementation,
            )?)
            .bind(record.last_seen_ms)
            .bind(record.lease_expires_ms)
            .bind(json_value("encode provider metadata", &record.metadata)?)
            .bind(record.created_at_ms)
            .bind(record.updated_at_ms)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| sql_error("register environment provider", error))?;
        provider_from_row(&row)
    }

    async fn read_provider(
        &self,
        provider_id: &EnvironmentProviderId,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
        let query = format!(
            "SELECT {PROVIDER_COLUMNS} FROM environment_providers WHERE universe_id = $1 AND provider_id = $2"
        );
        let row = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(provider_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| sql_error("read environment provider", error))?
            .ok_or_else(|| not_found("environment_provider", provider_id))?;
        provider_from_row(&row)
    }

    async fn list_providers(
        &self,
        request: ListEnvironmentProviders,
    ) -> Result<Vec<EnvironmentProviderRecord>, EnvironmentRegistryError> {
        let mut query =
            format!("SELECT {PROVIDER_COLUMNS} FROM environment_providers WHERE universe_id = $1");
        let mut next = 2;
        if request.status.is_some() {
            query.push_str(&format!(" AND status = ${next}"));
            next += 1;
        }
        if request.provider_kind.is_some() {
            query.push_str(&format!(" AND provider_kind = ${next}"));
        }
        query.push_str(" ORDER BY provider_id");
        let mut sql = sqlx::query(&query).bind(self.config.universe_id);
        if let Some(status) = request.status {
            sql = sql.bind(provider_status_to_str(status));
        }
        if let Some(kind) = request.provider_kind {
            sql = sql.bind(provider_kind_to_str(kind));
        }
        let rows = sql
            .fetch_all(&self.pool)
            .await
            .map_err(|error| sql_error("list environment providers", error))?;
        rows.iter().map(provider_from_row).collect()
    }

    async fn update_provider_heartbeat(
        &self,
        heartbeat: EnvironmentProviderHeartbeat,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
        let current = self.read_provider(&heartbeat.provider_id).await?;
        let ttl = heartbeat.lease_ttl_ms.unwrap_or_else(|| {
            current
                .lease_expires_ms
                .saturating_sub(current.last_seen_ms)
        });
        if heartbeat.observed_at_ms < 0 || ttl <= 0 {
            return invalid("invalid provider heartbeat time or lease ttl");
        }
        let lease_expires_ms = heartbeat.observed_at_ms.checked_add(ttl).ok_or_else(|| {
            EnvironmentRegistryError::InvalidInput {
                message: "lease expiry timestamp overflowed".to_owned(),
            }
        })?;
        let query = format!(
            r#"
            UPDATE environment_providers
            SET status = 'online', last_seen_ms = $3, lease_expires_ms = $4, updated_at_ms = $3
            WHERE universe_id = $1 AND provider_id = $2
            RETURNING {PROVIDER_COLUMNS}
            "#
        );
        let row = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(heartbeat.provider_id.as_str())
            .bind(heartbeat.observed_at_ms)
            .bind(lease_expires_ms)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| sql_error("heartbeat environment provider", error))?;
        provider_from_row(&row)
    }

    async fn update_provider_status(
        &self,
        request: UpdateEnvironmentProviderStatus,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
        let query = format!(
            r#"
            UPDATE environment_providers SET status = $3, updated_at_ms = $4
            WHERE universe_id = $1 AND provider_id = $2
            RETURNING {PROVIDER_COLUMNS}
            "#
        );
        let row = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(request.provider_id.as_str())
            .bind(provider_status_to_str(request.status))
            .bind(request.updated_at_ms)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| sql_error("update environment provider", error))?
            .ok_or_else(|| not_found("environment_provider", &request.provider_id))?;
        provider_from_row(&row)
    }

    async fn delete_provider(
        &self,
        provider_id: &EnvironmentProviderId,
    ) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
        let query = format!(
            "DELETE FROM environment_providers WHERE universe_id = $1 AND provider_id = $2 RETURNING {PROVIDER_COLUMNS}"
        );
        let row = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(provider_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| sql_error("delete environment provider", error))?
            .ok_or_else(|| not_found("environment_provider", provider_id))?;
        provider_from_row(&row)
    }
}

#[async_trait]
impl EnvironmentStore for PgStore {
    async fn observe_environment(
        &self,
        request: ObserveEnvironment,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError> {
        self.ensure_universe()
            .await
            .map_err(|error| store_error("ensure universe", error))?;
        let record = request.into_record();
        record.validate()?;
        let query = format!(
            r#"
            INSERT INTO environments (
                universe_id, environment_id, provider_id, provider_target_id, origin,
                display_name, status, scope_json, capabilities_json, connection_json,
                default_cwd, metadata_json, observed_at_ms, created_at_ms, updated_at_ms
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
            ON CONFLICT (universe_id, provider_id, provider_target_id) DO UPDATE SET
                origin = CASE WHEN environments.origin = 'provisioned' THEN 'provisioned' ELSE EXCLUDED.origin END,
                display_name = EXCLUDED.display_name,
                status = EXCLUDED.status,
                scope_json = EXCLUDED.scope_json,
                capabilities_json = EXCLUDED.capabilities_json,
                connection_json = EXCLUDED.connection_json,
                default_cwd = EXCLUDED.default_cwd,
                metadata_json = EXCLUDED.metadata_json,
                observed_at_ms = EXCLUDED.observed_at_ms,
                updated_at_ms = EXCLUDED.updated_at_ms
            WHERE environments.observed_at_ms <= EXCLUDED.observed_at_ms
            RETURNING {ENVIRONMENT_COLUMNS}
            "#
        );
        let row = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(record.environment_id.as_str())
            .bind(record.provider_id.as_str())
            .bind(record.provider_target_id.as_str())
            .bind(environment_origin_to_str(record.origin))
            .bind(record.display_name.as_deref())
            .bind(target_status_to_str(record.status))
            .bind(json_value("encode environment scope", &record.scope)?)
            .bind(json_value(
                "encode environment capabilities",
                &record.capabilities,
            )?)
            .bind(json_value(
                "encode environment connection",
                &record.connection,
            )?)
            .bind(record.default_cwd.as_ref().map(HostPath::as_str))
            .bind(json_value("encode environment metadata", &record.metadata)?)
            .bind(record.observed_at_ms)
            .bind(record.created_at_ms)
            .bind(record.updated_at_ms)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| sql_error("observe environment instance", error))?;
        match row {
            Some(row) => environment_from_row(&row),
            None => {
                self.read_environment_by_provider_target(
                    &record.provider_id,
                    &record.provider_target_id,
                )
                .await
            }
        }
    }

    async fn read_environment(
        &self,
        environment_id: &EnvironmentId,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError> {
        let query = format!(
            "SELECT {ENVIRONMENT_COLUMNS} FROM environments WHERE universe_id = $1 AND environment_id = $2"
        );
        let row = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(environment_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| sql_error("read environment instance", error))?
            .ok_or_else(|| not_found("environment_instance", environment_id))?;
        environment_from_row(&row)
    }

    async fn read_environment_by_provider_target(
        &self,
        provider_id: &EnvironmentProviderId,
        provider_target_id: &HostTargetId,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError> {
        let query = format!(
            "SELECT {ENVIRONMENT_COLUMNS} FROM environments WHERE universe_id = $1 AND provider_id = $2 AND provider_target_id = $3"
        );
        let row = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(provider_id.as_str())
            .bind(provider_target_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| sql_error("read environment by provider target", error))?
            .ok_or_else(|| EnvironmentRegistryError::NotFound {
                kind: "environment_instance",
                id: format!("{provider_id}/{provider_target_id}"),
            })?;
        environment_from_row(&row)
    }

    async fn list_environments(
        &self,
        request: ListEnvironments,
    ) -> Result<Vec<EnvironmentRecord>, EnvironmentRegistryError> {
        let mut query =
            format!("SELECT {ENVIRONMENT_COLUMNS} FROM environments WHERE universe_id = $1");
        let mut next = 2;
        if request.provider_id.is_some() {
            query.push_str(&format!(" AND provider_id = ${next}"));
            next += 1;
        }
        if request.status.is_some() {
            query.push_str(&format!(" AND status = ${next}"));
            next += 1;
        }
        if request.origin.is_some() {
            query.push_str(&format!(" AND origin = ${next}"));
        }
        query.push_str(" ORDER BY environment_id");
        let mut sql = sqlx::query(&query).bind(self.config.universe_id);
        if let Some(provider_id) = request.provider_id {
            sql = sql.bind(provider_id.as_str().to_owned());
        }
        if let Some(status) = request.status {
            sql = sql.bind(target_status_to_str(status));
        }
        if let Some(origin) = request.origin {
            sql = sql.bind(environment_origin_to_str(origin));
        }
        let rows = sql
            .fetch_all(&self.pool)
            .await
            .map_err(|error| sql_error("list environment instances", error))?;
        rows.iter().map(environment_from_row).collect()
    }

    async fn mark_missing_provided_environments_unknown(
        &self,
        provider_id: &EnvironmentProviderId,
        observed_target_ids: &BTreeSet<HostTargetId>,
        observed_at_ms: i64,
    ) -> Result<Vec<EnvironmentRecord>, EnvironmentRegistryError> {
        let observed = observed_target_ids
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect::<Vec<_>>();
        let query = format!(
            r#"
            UPDATE environments
            SET status = 'unknown', observed_at_ms = $4, updated_at_ms = $4
            WHERE universe_id = $1 AND provider_id = $2 AND origin = 'provided'
              AND NOT (provider_target_id = ANY($3)) AND observed_at_ms <= $4
            RETURNING {ENVIRONMENT_COLUMNS}
            "#
        );
        let rows = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(provider_id.as_str())
            .bind(observed)
            .bind(observed_at_ms)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| sql_error("mark missing environment instances unknown", error))?;
        rows.iter().map(environment_from_row).collect()
    }

    async fn update_environment_status(
        &self,
        request: UpdateEnvironmentStatus,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError> {
        let query = format!(
            r#"
            UPDATE environments SET status = $3, observed_at_ms = $4, updated_at_ms = $4
            WHERE universe_id = $1 AND environment_id = $2
            RETURNING {ENVIRONMENT_COLUMNS}
            "#
        );
        let row = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(request.environment_id.as_str())
            .bind(target_status_to_str(request.status))
            .bind(request.observed_at_ms)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| sql_error("update environment instance status", error))?
            .ok_or_else(|| not_found("environment_instance", &request.environment_id))?;
        environment_from_row(&row)
    }

    async fn begin_close_environment(
        &self,
        request: BeginCloseEnvironment,
    ) -> Result<EnvironmentRecord, EnvironmentRegistryError> {
        let update = format!(
            "UPDATE environments SET status = 'closing', updated_at_ms = $3 WHERE universe_id = $1 AND environment_id = $2 RETURNING {ENVIRONMENT_COLUMNS}"
        );
        let row = sqlx::query(&update)
            .bind(self.config.universe_id)
            .bind(request.environment_id.as_str())
            .bind(request.updated_at_ms)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| sql_error("begin closing environment", error))?
            .ok_or_else(|| not_found("environment", &request.environment_id))?;
        environment_from_row(&row)
    }
}

#[async_trait]
impl EnvironmentCredentialStore for PgStore {
    async fn bind_credential(
        &self,
        request: PutEnvironmentCredential,
    ) -> Result<EnvironmentCredentialRecord, EnvironmentRegistryError> {
        let record = request.into_record();
        record.validate()?;
        self.read_environment(&record.environment_id).await?;
        let (source_kind, grant_id, auth_provider_id, secret_id) =
            credential_source_columns(&record.source);
        let query = format!(
            r#"
            INSERT INTO environment_credentials (
                universe_id, environment_id, env_name, source_kind,
                grant_id, auth_provider_id, secret_id, created_at_ms, updated_at_ms
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$8)
            ON CONFLICT (universe_id, environment_id, env_name) DO UPDATE SET
                source_kind = EXCLUDED.source_kind, grant_id = EXCLUDED.grant_id,
                auth_provider_id = EXCLUDED.auth_provider_id, secret_id = EXCLUDED.secret_id,
                updated_at_ms = EXCLUDED.updated_at_ms
            RETURNING {CREDENTIAL_COLUMNS}
            "#
        );
        let row = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(record.environment_id.as_str())
            .bind(&record.env_name)
            .bind(source_kind)
            .bind(grant_id)
            .bind(auth_provider_id)
            .bind(secret_id)
            .bind(record.created_at_ms)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| sql_error("bind environment credential", error))?;
        credential_from_row(&row)
    }

    async fn list_credentials(
        &self,
        request: ListEnvironmentCredentials,
    ) -> Result<Vec<EnvironmentCredentialRecord>, EnvironmentRegistryError> {
        let query = format!(
            "SELECT {CREDENTIAL_COLUMNS} FROM environment_credentials WHERE universe_id = $1 AND environment_id = $2 ORDER BY env_name"
        );
        let rows = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(request.environment_id.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(|error| sql_error("list environment credentials", error))?;
        rows.iter().map(credential_from_row).collect()
    }

    async fn unbind_credential(
        &self,
        environment_id: &EnvironmentId,
        env_name: &str,
    ) -> Result<EnvironmentCredentialRecord, EnvironmentRegistryError> {
        let query = format!(
            "DELETE FROM environment_credentials WHERE universe_id = $1 AND environment_id = $2 AND env_name = $3 RETURNING {CREDENTIAL_COLUMNS}"
        );
        let row = sqlx::query(&query)
            .bind(self.config.universe_id)
            .bind(environment_id.as_str())
            .bind(env_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| sql_error("unbind environment credential", error))?
            .ok_or_else(|| EnvironmentRegistryError::NotFound {
                kind: "environment_credential",
                id: format!("{environment_id}/{env_name}"),
            })?;
        credential_from_row(&row)
    }
}

fn provider_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<EnvironmentProviderRecord, EnvironmentRegistryError> {
    let record = EnvironmentProviderRecord {
        provider_id: parse_id(row, "provider_id", EnvironmentProviderId::try_new)?,
        provider_kind: provider_kind_from_str(&column(row, "provider_kind")?)?,
        display_name: row
            .try_get("display_name")
            .map_err(|error| sql_error("decode provider display_name", error))?,
        status: provider_status_from_str(&column(row, "status")?)?,
        controller_connection: json_column(row, "controller_connection_json")?,
        capabilities: json_column(row, "capabilities_json")?,
        implementation: json_column(row, "implementation_json")?,
        last_seen_ms: row
            .try_get("last_seen_ms")
            .map_err(|error| sql_error("decode provider last_seen_ms", error))?,
        lease_expires_ms: row
            .try_get("lease_expires_ms")
            .map_err(|error| sql_error("decode provider lease_expires_ms", error))?,
        metadata: json_column(row, "metadata_json")?,
        created_at_ms: row
            .try_get("created_at_ms")
            .map_err(|error| sql_error("decode provider created_at_ms", error))?,
        updated_at_ms: row
            .try_get("updated_at_ms")
            .map_err(|error| sql_error("decode provider updated_at_ms", error))?,
    };
    record.validate()?;
    Ok(record)
}

fn environment_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<EnvironmentRecord, EnvironmentRegistryError> {
    let default_cwd: Option<String> = row
        .try_get("default_cwd")
        .map_err(|error| sql_error("decode environment default_cwd", error))?;
    let record = EnvironmentRecord {
        environment_id: parse_id(row, "environment_id", EnvironmentId::try_new)?,
        provider_id: parse_id(row, "provider_id", EnvironmentProviderId::try_new)?,
        provider_target_id: HostTargetId::new(column(row, "provider_target_id")?),
        origin: environment_origin_from_str(&column(row, "origin")?)?,
        display_name: row
            .try_get("display_name")
            .map_err(|error| sql_error("decode environment display_name", error))?,
        status: target_status_from_str(&column(row, "status")?)?,
        scope: json_column(row, "scope_json")?,
        capabilities: json_column(row, "capabilities_json")?,
        connection: json_column(row, "connection_json")?,
        default_cwd: default_cwd
            .as_deref()
            .map(HostPath::new)
            .transpose()
            .map_err(|error| EnvironmentRegistryError::Store {
                message: format!("decode environment cwd: {error}"),
            })?,
        metadata: json_column(row, "metadata_json")?,
        observed_at_ms: row
            .try_get("observed_at_ms")
            .map_err(|error| sql_error("decode environment observed_at_ms", error))?,
        created_at_ms: row
            .try_get("created_at_ms")
            .map_err(|error| sql_error("decode environment created_at_ms", error))?,
        updated_at_ms: row
            .try_get("updated_at_ms")
            .map_err(|error| sql_error("decode environment updated_at_ms", error))?,
    };
    record.validate()?;
    Ok(record)
}

fn credential_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<EnvironmentCredentialRecord, EnvironmentRegistryError> {
    let source_kind = column(row, "source_kind")?;
    let grant_id: Option<String> = row
        .try_get("grant_id")
        .map_err(|error| sql_error("decode credential grant_id", error))?;
    let provider_id: Option<String> = row
        .try_get("auth_provider_id")
        .map_err(|error| sql_error("decode credential provider_id", error))?;
    let secret_id: Option<String> = row
        .try_get("secret_id")
        .map_err(|error| sql_error("decode credential secret_id", error))?;
    let source = match source_kind.as_str() {
        "auth_grant" => EnvironmentCredentialSource::AuthGrant {
            grant_id: AuthGrantId::try_new(
                grant_id.ok_or_else(|| store_message("missing grant_id"))?,
            )
            .map_err(|error| store_message(format!("decode grant id: {error}")))?,
        },
        "auth_provider_credential" => EnvironmentCredentialSource::AuthProviderCredential {
            provider_id: AuthProviderId::try_new(
                provider_id.ok_or_else(|| store_message("missing auth_provider_id"))?,
            )
            .map_err(|error| store_message(format!("decode auth provider id: {error}")))?,
        },
        "direct_secret" => EnvironmentCredentialSource::DirectSecret {
            secret_id: SecretId::try_new(
                secret_id.ok_or_else(|| store_message("missing secret_id"))?,
            )
            .map_err(|error| store_message(format!("decode secret id: {error}")))?,
        },
        other => return Err(store_message(format!("unknown credential source: {other}"))),
    };
    let record = EnvironmentCredentialRecord {
        environment_id: parse_id(row, "environment_id", EnvironmentId::try_new)?,
        env_name: column(row, "env_name")?,
        source,
        created_at_ms: row
            .try_get("created_at_ms")
            .map_err(|error| sql_error("decode credential created_at_ms", error))?,
        updated_at_ms: row
            .try_get("updated_at_ms")
            .map_err(|error| sql_error("decode credential updated_at_ms", error))?,
    };
    record.validate()?;
    Ok(record)
}

fn credential_source_columns(
    source: &EnvironmentCredentialSource,
) -> (&'static str, Option<&str>, Option<&str>, Option<&str>) {
    match source {
        EnvironmentCredentialSource::AuthGrant { grant_id } => {
            ("auth_grant", Some(grant_id.as_str()), None, None)
        }
        EnvironmentCredentialSource::AuthProviderCredential { provider_id } => (
            "auth_provider_credential",
            None,
            Some(provider_id.as_str()),
            None,
        ),
        EnvironmentCredentialSource::DirectSecret { secret_id } => {
            ("direct_secret", None, None, Some(secret_id.as_str()))
        }
    }
}

fn provider_kind_to_str(value: EnvironmentProviderKind) -> &'static str {
    match value {
        EnvironmentProviderKind::Sandbox => "sandbox",
        EnvironmentProviderKind::Bridge => "bridge",
        EnvironmentProviderKind::Custom => "custom",
    }
}
fn provider_kind_from_str(
    value: &str,
) -> Result<EnvironmentProviderKind, EnvironmentRegistryError> {
    match value {
        "sandbox" => Ok(EnvironmentProviderKind::Sandbox),
        "bridge" => Ok(EnvironmentProviderKind::Bridge),
        "custom" => Ok(EnvironmentProviderKind::Custom),
        other => Err(store_message(format!("unknown provider kind: {other}"))),
    }
}
fn provider_status_to_str(value: EnvironmentProviderStatus) -> &'static str {
    match value {
        EnvironmentProviderStatus::Online => "online",
        EnvironmentProviderStatus::Offline => "offline",
    }
}
fn provider_status_from_str(
    value: &str,
) -> Result<EnvironmentProviderStatus, EnvironmentRegistryError> {
    match value {
        "online" => Ok(EnvironmentProviderStatus::Online),
        "offline" => Ok(EnvironmentProviderStatus::Offline),
        other => Err(store_message(format!("unknown provider status: {other}"))),
    }
}
fn environment_origin_to_str(value: EnvironmentOrigin) -> &'static str {
    match value {
        EnvironmentOrigin::Provided => "provided",
        EnvironmentOrigin::Provisioned => "provisioned",
    }
}
fn environment_origin_from_str(value: &str) -> Result<EnvironmentOrigin, EnvironmentRegistryError> {
    match value {
        "provided" => Ok(EnvironmentOrigin::Provided),
        "provisioned" => Ok(EnvironmentOrigin::Provisioned),
        other => Err(store_message(format!(
            "unknown environment origin: {other}"
        ))),
    }
}
fn target_status_to_str(value: HostTargetStatus) -> &'static str {
    match value {
        HostTargetStatus::Creating => "creating",
        HostTargetStatus::Starting => "starting",
        HostTargetStatus::Ready => "ready",
        HostTargetStatus::Stopped => "stopped",
        HostTargetStatus::Closing => "closing",
        HostTargetStatus::Closed => "closed",
        HostTargetStatus::Failed => "failed",
        HostTargetStatus::Unknown => "unknown",
    }
}
fn target_status_from_str(value: &str) -> Result<HostTargetStatus, EnvironmentRegistryError> {
    match value {
        "creating" => Ok(HostTargetStatus::Creating),
        "starting" => Ok(HostTargetStatus::Starting),
        "ready" => Ok(HostTargetStatus::Ready),
        "stopped" => Ok(HostTargetStatus::Stopped),
        "closing" => Ok(HostTargetStatus::Closing),
        "closed" => Ok(HostTargetStatus::Closed),
        "failed" => Ok(HostTargetStatus::Failed),
        "unknown" => Ok(HostTargetStatus::Unknown),
        other => Err(store_message(format!(
            "unknown environment status: {other}"
        ))),
    }
}

fn parse_id<T, E>(
    row: &sqlx::postgres::PgRow,
    name: &str,
    parse: impl FnOnce(String) -> Result<T, E>,
) -> Result<T, EnvironmentRegistryError>
where
    E: std::fmt::Display,
{
    let value = column(row, name)?;
    parse(value).map_err(|error| store_message(format!("decode {name}: {error}")))
}

fn column(row: &sqlx::postgres::PgRow, name: &str) -> Result<String, EnvironmentRegistryError> {
    row.try_get(name)
        .map_err(|error| sql_error("decode environment column", error))
}

fn json_value<T: serde::Serialize>(
    action: &str,
    value: &T,
) -> Result<serde_json::Value, EnvironmentRegistryError> {
    serde_json::to_value(value).map_err(|error| EnvironmentRegistryError::Store {
        message: format!("{action}: {error}"),
    })
}

fn json_column<T: serde::de::DeserializeOwned>(
    row: &sqlx::postgres::PgRow,
    name: &str,
) -> Result<T, EnvironmentRegistryError> {
    let value: serde_json::Value = row
        .try_get(name)
        .map_err(|error| sql_error("decode json column", error))?;
    serde_json::from_value(value).map_err(|error| store_message(format!("decode {name}: {error}")))
}

fn not_found(kind: &'static str, id: &impl ToString) -> EnvironmentRegistryError {
    EnvironmentRegistryError::NotFound {
        kind,
        id: id.to_string(),
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, EnvironmentRegistryError> {
    Err(EnvironmentRegistryError::InvalidInput {
        message: message.into(),
    })
}

fn store_error(action: &str, error: crate::PgStoreError) -> EnvironmentRegistryError {
    EnvironmentRegistryError::Store {
        message: format!("{action}: {error}"),
    }
}

fn sql_error(action: &str, error: sqlx::Error) -> EnvironmentRegistryError {
    EnvironmentRegistryError::Store {
        message: format!("{action}: {error}"),
    }
}

fn store_message(message: impl Into<String>) -> EnvironmentRegistryError {
    EnvironmentRegistryError::Store {
        message: message.into(),
    }
}
