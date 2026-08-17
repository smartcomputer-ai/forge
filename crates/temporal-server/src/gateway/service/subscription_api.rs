//! `auth/subscriptions/import` (P127 S1/S2): coding-agent subscription
//! credentials as auth grants. Parsing/rendering lives in `auth::subscriptions`;
//! this module maps pasted credentials onto grant + secret records.

use super::*;

pub(super) const SUBSCRIPTION_METADATA_SOURCE: &str = "source";
pub(super) const SUBSCRIPTION_METADATA_CREDENTIAL: &str = "credential";
pub(super) const SUBSCRIPTION_METADATA_ID_TOKEN_SECRET_ID: &str = "idTokenSecretId";
pub(super) const SUBSCRIPTION_METADATA_LAST_REFRESH_MS: &str = "lastRefreshMs";
pub(super) const SUBSCRIPTION_CREDENTIAL_TOKEN: &str = "token";
pub(super) const SUBSCRIPTION_CREDENTIAL_TOKEN_SET: &str = "tokenSet";
/// Marks a `static_bearer` grant as a Claude Code subscription token so the
/// Integrations page can list it without a dedicated grant kind.
pub(super) const SUBSCRIPTION_METADATA_SUBSCRIPTION: &str = "subscription";
pub(super) const SUBSCRIPTION_CLAUDE_CODE: &str = "claudeCode";

#[derive(Debug)]
pub(super) struct SubscriptionImportDraft {
    pub(super) secrets: Vec<auth::PutSecretRecord>,
    pub(super) grant: auth::CreateAuthGrantRecord,
    pub(super) shape: SubscriptionCredentialShape,
}

fn new_secret_id() -> Result<auth::SecretId, AgentApiError> {
    auth::SecretId::try_new(format!("authsec_{}", uuid::Uuid::new_v4().simple()))
        .map_err(|error| AgentApiError::internal(format!("generate secret id: {error}")))
}

fn map_credential_error(error: auth::SubscriptionCredentialError) -> AgentApiError {
    AgentApiError::invalid_request(format!("subscription credential: {error}"))
}

pub(super) fn subscription_import_draft(
    params: AuthSubscriptionImportParams,
    now_ms: i64,
) -> Result<SubscriptionImportDraft, AgentApiError> {
    let grant_id = match params.grant_id {
        Some(grant_id) => parse_auth_grant_id(grant_id)?,
        None => auth::AuthGrantId::try_new(format!("authgrant_{}", uuid::Uuid::new_v4().simple()))
            .map_err(|error| AgentApiError::internal(format!("generate auth grant id: {error}")))?,
    };
    let mut secrets = Vec::new();
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        SUBSCRIPTION_METADATA_SOURCE.to_owned(),
        serde_json::Value::String("pasted".to_owned()),
    );

    let (
        provider_id,
        provider_kind,
        access_secret,
        refresh_secret,
        expires_at_ms,
        subject_hint,
        shape,
    ) = match params.provider {
        SubscriptionProvider::Anthropic => {
            let token = auth::parse_anthropic_claude_code_token(&params.credential)
                .map_err(map_credential_error)?;
            let secret_id = new_secret_id()?;
            secrets.push(auth::PutSecretRecord {
                secret_id: secret_id.clone(),
                secret_kind: auth::SECRET_KIND_ANTHROPIC_CLAUDE_CODE_TOKEN.to_owned(),
                value: auth::SecretValue::new(token),
                created_at_ms: now_ms,
            });
            metadata.insert(
                SUBSCRIPTION_METADATA_CREDENTIAL.to_owned(),
                serde_json::Value::String(SUBSCRIPTION_CREDENTIAL_TOKEN.to_owned()),
            );
            metadata.insert(
                SUBSCRIPTION_METADATA_SUBSCRIPTION.to_owned(),
                serde_json::Value::String(SUBSCRIPTION_CLAUDE_CODE.to_owned()),
            );
            (
                "anthropic".to_owned(),
                auth::AuthProviderKind::StaticBearer,
                secret_id,
                None,
                Some(now_ms.saturating_add(auth::ANTHROPIC_CLAUDE_CODE_TOKEN_TTL_MS)),
                None,
                SubscriptionCredentialShape::Token,
            )
        }
        SubscriptionProvider::OpenAi => {
            match auth::parse_openai_chatgpt_credential(&params.credential)
                .map_err(map_credential_error)?
            {
                auth::OpenAiChatGptCredential::AccessToken(token) => {
                    let secret_id = new_secret_id()?;
                    secrets.push(auth::PutSecretRecord {
                        secret_id: secret_id.clone(),
                        secret_kind: auth::SECRET_KIND_OPENAI_CHATGPT_ACCESS_TOKEN.to_owned(),
                        value: auth::SecretValue::new(token),
                        created_at_ms: now_ms,
                    });
                    metadata.insert(
                        SUBSCRIPTION_METADATA_CREDENTIAL.to_owned(),
                        serde_json::Value::String(SUBSCRIPTION_CREDENTIAL_TOKEN.to_owned()),
                    );
                    (
                        "openai".to_owned(),
                        auth::AuthProviderKind::OpenAiChatGpt,
                        secret_id,
                        None,
                        None,
                        None,
                        SubscriptionCredentialShape::Token,
                    )
                }
                auth::OpenAiChatGptCredential::TokenSet {
                    tokens,
                    metadata: facts,
                } => {
                    let access_id = new_secret_id()?;
                    let refresh_id = new_secret_id()?;
                    let id_token_id = new_secret_id()?;
                    secrets.push(auth::PutSecretRecord {
                        secret_id: access_id.clone(),
                        secret_kind: auth::SECRET_KIND_OPENAI_CHATGPT_ACCESS_TOKEN.to_owned(),
                        value: auth::SecretValue::new(tokens.access_token),
                        created_at_ms: now_ms,
                    });
                    secrets.push(auth::PutSecretRecord {
                        secret_id: refresh_id.clone(),
                        secret_kind: auth::SECRET_KIND_OPENAI_CHATGPT_REFRESH_TOKEN.to_owned(),
                        value: auth::SecretValue::new(tokens.refresh_token),
                        created_at_ms: now_ms,
                    });
                    secrets.push(auth::PutSecretRecord {
                        secret_id: id_token_id.clone(),
                        secret_kind: auth::SECRET_KIND_OPENAI_CHATGPT_ID_TOKEN.to_owned(),
                        value: auth::SecretValue::new(tokens.id_token),
                        created_at_ms: now_ms,
                    });
                    metadata.insert(
                        SUBSCRIPTION_METADATA_CREDENTIAL.to_owned(),
                        serde_json::Value::String(SUBSCRIPTION_CREDENTIAL_TOKEN_SET.to_owned()),
                    );
                    metadata.insert(
                        SUBSCRIPTION_METADATA_ID_TOKEN_SECRET_ID.to_owned(),
                        serde_json::Value::String(id_token_id.as_str().to_owned()),
                    );
                    metadata.insert(
                        SUBSCRIPTION_METADATA_LAST_REFRESH_MS.to_owned(),
                        serde_json::Value::from(now_ms),
                    );
                    if let serde_json::Value::Object(facts) =
                        serde_json::to_value(&facts).map_err(|error| {
                            AgentApiError::internal(format!(
                                "encode subscription metadata: {error}"
                            ))
                        })?
                    {
                        for (key, value) in facts {
                            metadata.insert(key, value);
                        }
                    }
                    (
                        "openai".to_owned(),
                        auth::AuthProviderKind::OpenAiChatGpt,
                        access_id,
                        Some(refresh_id),
                        facts.access_token_expires_at_ms,
                        facts.email.clone(),
                        SubscriptionCredentialShape::CodexTokenSet,
                    )
                }
            }
        }
    };

    let grant = auth::CreateAuthGrantRecord {
        grant_id,
        provider_id,
        provider_kind,
        principal: crate::gateway::principal::request_principal(),
        display_name: params.display_name,
        subject_hint,
        scopes: Vec::new(),
        audience: None,
        access_token_secret: Some(access_secret),
        refresh_token_secret: refresh_secret,
        oauth_client: None,
        metadata: serde_json::Value::Object(metadata),
        expires_at_ms,
        status: auth::AuthGrantStatus::Active,
        created_at_ms: now_ms,
    };
    for secret in &secrets {
        secret.validate().map_err(map_auth_error)?;
    }
    grant
        .clone()
        .into_record()
        .validate()
        .map_err(map_auth_error)?;
    Ok(SubscriptionImportDraft {
        secrets,
        grant,
        shape,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_import_produces_token_grant_with_expiry() {
        let draft = subscription_import_draft(
            AuthSubscriptionImportParams {
                provider: SubscriptionProvider::Anthropic,
                credential: "sk-ant-oat01-abc".to_owned(),
                display_name: Some("Lukas Max".to_owned()),
                grant_id: None,
            },
            1_000,
        )
        .unwrap();
        assert_eq!(draft.secrets.len(), 1);
        assert_eq!(
            draft.grant.provider_kind,
            auth::AuthProviderKind::StaticBearer
        );
        assert_eq!(draft.grant.provider_id, "anthropic");
        assert_eq!(draft.grant.metadata["subscription"], "claudeCode");
        assert_eq!(
            draft.grant.expires_at_ms,
            Some(1_000 + auth::ANTHROPIC_CLAUDE_CODE_TOKEN_TTL_MS)
        );
        assert_eq!(draft.shape, SubscriptionCredentialShape::Token);
        assert!(draft.grant.refresh_token_secret.is_none());
    }

    #[test]
    fn openai_access_token_import_is_a_plain_token_grant() {
        let draft = subscription_import_draft(
            AuthSubscriptionImportParams {
                provider: SubscriptionProvider::OpenAi,
                credential: "codex_pat_xyz".to_owned(),
                display_name: None,
                grant_id: Some("authgrant_codex".to_owned()),
            },
            5,
        )
        .unwrap();
        assert_eq!(draft.grant.grant_id.as_str(), "authgrant_codex");
        assert_eq!(
            draft.grant.provider_kind,
            auth::AuthProviderKind::OpenAiChatGpt
        );
        assert_eq!(draft.shape, SubscriptionCredentialShape::Token);
        assert_eq!(draft.grant.metadata["credential"], "token");
    }

    #[test]
    fn openai_auth_json_import_stores_three_secrets() {
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let jwt = |payload: serde_json::Value| {
            format!(
                "{}.{}.{}",
                engine.encode(br#"{"alg":"none"}"#),
                engine.encode(payload.to_string()),
                engine.encode(b"s")
            )
        };
        let credential = serde_json::json!({
            "tokens": {
                "id_token": jwt(serde_json::json!({
                    "email": "a@b.c",
                    "https://api.openai.com/auth": {"chatgpt_account_id": "acct", "chatgpt_plan_type": "plus"}
                })),
                "access_token": jwt(serde_json::json!({"exp": 2_000_000_000})),
                "refresh_token": "rt",
                "account_id": "acct"
            }
        })
        .to_string();
        let draft = subscription_import_draft(
            AuthSubscriptionImportParams {
                provider: SubscriptionProvider::OpenAi,
                credential,
                display_name: None,
                grant_id: None,
            },
            7,
        )
        .unwrap();
        assert_eq!(draft.secrets.len(), 3);
        assert_eq!(draft.shape, SubscriptionCredentialShape::CodexTokenSet);
        assert_eq!(draft.grant.expires_at_ms, Some(2_000_000_000_000));
        assert_eq!(draft.grant.metadata["accountId"], "acct");
        assert_eq!(draft.grant.metadata["planType"], "plus");
        assert_eq!(draft.grant.subject_hint.as_deref(), Some("a@b.c"));
        assert!(draft.grant.metadata["idTokenSecretId"].is_string());
    }

    #[test]
    fn invalid_credentials_are_invalid_requests() {
        let error = subscription_import_draft(
            AuthSubscriptionImportParams {
                provider: SubscriptionProvider::Anthropic,
                credential: "sk-ant-api03-key".to_owned(),
                display_name: None,
                grant_id: None,
            },
            1,
        )
        .unwrap_err();
        assert_eq!(error.kind, api::AgentApiErrorKind::InvalidRequest);
    }
}
