use super::*;

use ::environments::{
    EnvironmentCredentialRecord, EnvironmentCredentialSource, EnvironmentCredentialStore,
    ListEnvironmentCredentials, PutEnvironmentCredential,
};
use auth::{AuthGrantId, AuthGrantStatus, AuthProviderId, AuthProviderStatus, SecretId};

/// Claude Code picks `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN` over
/// `CLAUDE_CODE_OAUTH_TOKEN`; binding both into one environment silently
/// disables the subscription, so the pair is rejected (P127 D2).
const CLAUDE_CODE_OAUTH_TOKEN_ENV: &str = "CLAUDE_CODE_OAUTH_TOKEN";
const ANTHROPIC_KEY_ENVS: [&str; 2] = ["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];

pub(super) fn conflicting_anthropic_env(new_name: &str, existing: &[String]) -> Option<String> {
    if new_name == CLAUDE_CODE_OAUTH_TOKEN_ENV {
        return existing
            .iter()
            .find(|name| ANTHROPIC_KEY_ENVS.contains(&name.as_str()))
            .cloned();
    }
    if ANTHROPIC_KEY_ENVS.contains(&new_name) {
        return existing
            .iter()
            .find(|name| name.as_str() == CLAUDE_CODE_OAUTH_TOKEN_ENV)
            .cloned();
    }
    None
}

impl GatewayAgentApi {
    pub(super) async fn bind_environment_credential_record(
        &self,
        params: EnvironmentCredentialBindParams,
    ) -> Result<EnvironmentCredentialBindResponse, AgentApiError> {
        let environment_id = parse_registry_environment_id(params.environment_id)?;
        validate_credential_env_name(&params.env_name)?;
        let source = self.credential_source_from_api(params.source).await?;
        let existing = EnvironmentCredentialStore::list_credentials(
            self.store.as_ref(),
            ListEnvironmentCredentials {
                environment_id: environment_id.clone(),
            },
        )
        .await
        .map_err(map_environments_error)?
        .into_iter()
        .map(|credential| credential.env_name)
        .collect::<Vec<_>>();
        if let Some(conflict) = conflicting_anthropic_env(&params.env_name, &existing) {
            return Err(AgentApiError::rejected(format!(
                "{} cannot be bound alongside {conflict}: Claude Code prefers the API key and \
                 would ignore the subscription token; unbind one first",
                params.env_name
            )));
        }
        let credential = EnvironmentCredentialStore::bind_credential(
            self.store.as_ref(),
            PutEnvironmentCredential {
                environment_id,
                env_name: params.env_name,
                source,
                created_at_ms: now_ms()?,
            },
        )
        .await
        .map_err(map_environments_error)?;
        Ok(EnvironmentCredentialBindResponse {
            credential: environment_credential_view(credential),
        })
    }

    pub(super) async fn list_environment_credential_records(
        &self,
        params: EnvironmentCredentialListParams,
    ) -> Result<EnvironmentCredentialListResponse, AgentApiError> {
        let environment_id = parse_registry_environment_id(params.environment_id)?;
        let credentials = EnvironmentCredentialStore::list_credentials(
            self.store.as_ref(),
            ListEnvironmentCredentials { environment_id },
        )
        .await
        .map_err(map_environments_error)?;
        Ok(EnvironmentCredentialListResponse {
            credentials: credentials
                .into_iter()
                .map(environment_credential_view)
                .collect(),
        })
    }

    pub(super) async fn unbind_environment_credential_record(
        &self,
        params: EnvironmentCredentialUnbindParams,
    ) -> Result<EnvironmentCredentialUnbindResponse, AgentApiError> {
        let environment_id = parse_registry_environment_id(params.environment_id)?;
        validate_credential_env_name(&params.env_name)?;
        let credential = EnvironmentCredentialStore::unbind_credential(
            self.store.as_ref(),
            &environment_id,
            &params.env_name,
        )
        .await
        .map_err(map_environments_error)?;
        Ok(EnvironmentCredentialUnbindResponse {
            credential: environment_credential_view(credential),
        })
    }

    async fn credential_source_from_api(
        &self,
        source: EnvironmentCredentialSourceView,
    ) -> Result<EnvironmentCredentialSource, AgentApiError> {
        match source {
            EnvironmentCredentialSourceView::AuthGrant { grant_id } => {
                let grant_id = AuthGrantId::try_new(grant_id).map_err(|error| {
                    AgentApiError::invalid_request(format!("invalid grant_id: {error}"))
                })?;
                let grant = auth::AuthGrantStore::read_grant(self.store.as_ref(), &grant_id)
                    .await
                    .map_err(map_auth_error)?;
                if grant.status != AuthGrantStatus::Active {
                    return Err(AgentApiError::rejected(format!(
                        "auth grant is not active: {grant_id}"
                    )));
                }
                Ok(EnvironmentCredentialSource::AuthGrant { grant_id })
            }
            EnvironmentCredentialSourceView::AuthProviderCredential { provider_id } => {
                let provider_id = AuthProviderId::try_new(provider_id).map_err(|error| {
                    AgentApiError::invalid_request(format!("invalid provider_id: {error}"))
                })?;
                let provider =
                    auth::AuthProviderStore::read_auth_provider(self.store.as_ref(), &provider_id)
                        .await
                        .map_err(map_auth_error)?;
                if provider.status != AuthProviderStatus::Active {
                    return Err(AgentApiError::rejected(format!(
                        "auth provider is not active: {provider_id}"
                    )));
                }
                if provider.credential_secret.is_none() {
                    return Err(AgentApiError::rejected(format!(
                        "auth provider has no exportable credential secret: {provider_id}"
                    )));
                }
                Ok(EnvironmentCredentialSource::AuthProviderCredential { provider_id })
            }
            EnvironmentCredentialSourceView::DirectSecret { secret_id } => {
                let secret_id = SecretId::try_new(secret_id).map_err(|error| {
                    AgentApiError::invalid_request(format!("invalid secret_id: {error}"))
                })?;
                let _ = auth::SecretStore::read_secret(self.store.as_ref(), &secret_id)
                    .await
                    .map_err(map_auth_error)?;
                Ok(EnvironmentCredentialSource::DirectSecret { secret_id })
            }
        }
    }
}

pub(super) fn environment_credential_view(
    record: EnvironmentCredentialRecord,
) -> EnvironmentCredentialView {
    EnvironmentCredentialView {
        environment_id: record.environment_id.as_str().to_owned(),
        env_name: record.env_name,
        source: credential_source_view(record.source),
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
    }
}

fn credential_source_view(source: EnvironmentCredentialSource) -> EnvironmentCredentialSourceView {
    match source {
        EnvironmentCredentialSource::AuthGrant { grant_id } => {
            EnvironmentCredentialSourceView::AuthGrant {
                grant_id: grant_id.as_str().to_owned(),
            }
        }
        EnvironmentCredentialSource::AuthProviderCredential { provider_id } => {
            EnvironmentCredentialSourceView::AuthProviderCredential {
                provider_id: provider_id.as_str().to_owned(),
            }
        }
        EnvironmentCredentialSource::DirectSecret { secret_id } => {
            EnvironmentCredentialSourceView::DirectSecret {
                secret_id: secret_id.as_str().to_owned(),
            }
        }
    }
}

pub(super) fn validate_credential_env_name(value: &str) -> Result<(), AgentApiError> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(AgentApiError::invalid_request(
            "credential env_name must not be empty",
        ));
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return Err(AgentApiError::invalid_request(format!(
            "invalid credential env_name: {value}"
        )));
    }
    let len = 1 + chars
        .try_fold(0usize, |count, ch| {
            if ch == '_' || ch.is_ascii_alphanumeric() {
                Ok(count + 1)
            } else {
                Err(())
            }
        })
        .map_err(|()| {
            AgentApiError::invalid_request(format!("invalid credential env_name: {value}"))
        })?;
    if len > 128 {
        return Err(AgentApiError::invalid_request(format!(
            "credential env_name is too long: {len} bytes, max 128"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::conflicting_anthropic_env;

    #[test]
    fn anthropic_key_and_subscription_token_conflict_both_ways() {
        let existing = vec!["ANTHROPIC_API_KEY".to_owned(), "GH_TOKEN".to_owned()];
        assert_eq!(
            conflicting_anthropic_env("CLAUDE_CODE_OAUTH_TOKEN", &existing).as_deref(),
            Some("ANTHROPIC_API_KEY")
        );
        let existing = vec!["CLAUDE_CODE_OAUTH_TOKEN".to_owned()];
        assert_eq!(
            conflicting_anthropic_env("ANTHROPIC_AUTH_TOKEN", &existing).as_deref(),
            Some("CLAUDE_CODE_OAUTH_TOKEN")
        );
        assert_eq!(conflicting_anthropic_env("OPENAI_API_KEY", &existing), None);
        assert_eq!(
            conflicting_anthropic_env("CLAUDE_CODE_OAUTH_TOKEN", &[]),
            None
        );
    }
}
