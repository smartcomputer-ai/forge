//! Coding-agent subscription credentials (P127 S1/S2): the Claude Code
//! `setup-token` and the ChatGPT credential Codex uses. Pure parsing and
//! rendering only; storage and injection live in the gateway and worker.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SECRET_KIND_ANTHROPIC_CLAUDE_CODE_TOKEN: &str = "auth.anthropic.claude_code_token";
pub const SECRET_KIND_OPENAI_CHATGPT_ACCESS_TOKEN: &str = "auth.openai.chatgpt.access_token";
pub const SECRET_KIND_OPENAI_CHATGPT_REFRESH_TOKEN: &str = "auth.openai.chatgpt.refresh_token";
pub const SECRET_KIND_OPENAI_CHATGPT_ID_TOKEN: &str = "auth.openai.chatgpt.id_token";

/// `claude setup-token` mints one-year tokens; Lightspeed records the paste
/// time plus this TTL as a best-effort expiry.
pub const ANTHROPIC_CLAUDE_CODE_TOKEN_TTL_MS: i64 = 365 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SubscriptionCredentialError {
    #[error("credential is empty")]
    Empty,
    #[error(
        "credential is not a Claude Code token (expected `sk-ant-oat…` from `claude setup-token`)"
    )]
    NotClaudeCodeToken,
    #[error("auth.json is not valid JSON: {message}")]
    InvalidAuthJson { message: String },
    #[error(
        "auth.json has no ChatGPT tokens (`tokens.access_token`); API-key-only files are not a subscription credential"
    )]
    AuthJsonWithoutTokens,
    #[error("auth.json tokens are missing `{field}`")]
    AuthJsonMissing { field: &'static str },
    #[error("id_token is not a decodable JWT: {message}")]
    InvalidIdToken { message: String },
}

/// Validated Claude Code subscription token. Anthropic's tokens are opaque;
/// only the documented prefix is checked.
pub fn parse_anthropic_claude_code_token(
    input: &str,
) -> Result<String, SubscriptionCredentialError> {
    let token = input.trim();
    if token.is_empty() {
        return Err(SubscriptionCredentialError::Empty);
    }
    if !token.starts_with("sk-ant-oat") || token.contains(char::is_whitespace) {
        return Err(SubscriptionCredentialError::NotClaudeCodeToken);
    }
    Ok(token.to_owned())
}

/// The ChatGPT token set Codex keeps in `$CODEX_HOME/auth.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatGptTokenSet {
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
    pub account_id: Option<String>,
}

/// Non-secret facts derived from the token set for grant metadata and UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiChatGptMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    /// Access-token expiry from its `exp` claim, when decodable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token_expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenAiChatGptCredential {
    /// ChatGPT Enterprise "Codex access token" (`CODEX_ACCESS_TOKEN`).
    AccessToken(String),
    /// Full token set pasted from a local `auth.json` (Plus/Pro/Team).
    TokenSet {
        tokens: ChatGptTokenSet,
        metadata: OpenAiChatGptMetadata,
    },
}

/// Accepts either a pasted `auth.json` document or a bare access token.
pub fn parse_openai_chatgpt_credential(
    input: &str,
) -> Result<OpenAiChatGptCredential, SubscriptionCredentialError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(SubscriptionCredentialError::Empty);
    }
    if !trimmed.starts_with('{') {
        if trimmed.contains(char::is_whitespace) {
            return Err(SubscriptionCredentialError::InvalidAuthJson {
                message: "expected a JSON object or a single access token".to_owned(),
            });
        }
        return Ok(OpenAiChatGptCredential::AccessToken(trimmed.to_owned()));
    }
    let document: AuthDotJson = serde_json::from_str(trimmed).map_err(|error| {
        SubscriptionCredentialError::InvalidAuthJson {
            message: error.to_string(),
        }
    })?;
    let Some(tokens) = document.tokens else {
        return Err(SubscriptionCredentialError::AuthJsonWithoutTokens);
    };
    let access_token = required(tokens.access_token, "access_token")?;
    let refresh_token = required(tokens.refresh_token, "refresh_token")?;
    let id_token = required(tokens.id_token, "id_token")?;
    let claims = decode_id_token_claims(&id_token)?;
    let account_id = tokens
        .account_id
        .filter(|v| !v.is_empty())
        .or(claims.account_id.clone());
    let metadata = OpenAiChatGptMetadata {
        email: claims.email,
        account_id: account_id.clone(),
        plan_type: claims.plan_type,
        access_token_expires_at_ms: jwt_exp_ms(&access_token),
    };
    Ok(OpenAiChatGptCredential::TokenSet {
        tokens: ChatGptTokenSet {
            id_token,
            access_token,
            refresh_token,
            account_id,
        },
        metadata,
    })
}

fn required(
    value: Option<String>,
    field: &'static str,
) -> Result<String, SubscriptionCredentialError> {
    value
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .ok_or(SubscriptionCredentialError::AuthJsonMissing { field })
}

/// `$CODEX_HOME/auth.json` as Codex reads it (subset; unknown fields ignored).
#[derive(Debug, Default, Deserialize)]
struct AuthDotJson {
    #[serde(default)]
    tokens: Option<AuthDotJsonTokens>,
}

#[derive(Debug, Default, Deserialize)]
struct AuthDotJsonTokens {
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Default)]
struct IdTokenClaims {
    email: Option<String>,
    account_id: Option<String>,
    plan_type: Option<String>,
}

fn decode_jwt_payload(token: &str) -> Result<serde_json::Value, SubscriptionCredentialError> {
    let mut parts = token.split('.');
    let (_header, payload) = match (parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(p), Some(_sig)) => (h, p),
        _ => {
            return Err(SubscriptionCredentialError::InvalidIdToken {
                message: "expected three dot-separated segments".to_owned(),
            });
        }
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .map_err(|error| SubscriptionCredentialError::InvalidIdToken {
            message: format!("payload is not base64url: {error}"),
        })?;
    serde_json::from_slice(&bytes).map_err(|error| SubscriptionCredentialError::InvalidIdToken {
        message: format!("payload is not JSON: {error}"),
    })
}

fn decode_id_token_claims(id_token: &str) -> Result<IdTokenClaims, SubscriptionCredentialError> {
    let payload = decode_jwt_payload(id_token)?;
    let auth = payload.get("https://api.openai.com/auth");
    Ok(IdTokenClaims {
        email: payload
            .get("email")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        account_id: auth
            .and_then(|a| a.get("chatgpt_account_id"))
            .and_then(|v| v.as_str())
            .map(str::to_owned),
        plan_type: auth
            .and_then(|a| a.get("chatgpt_plan_type"))
            .and_then(|v| v.as_str())
            .map(str::to_owned),
    })
}

/// `exp` (seconds) of a JWT as epoch milliseconds; `None` when the token is
/// opaque or has no `exp`.
fn jwt_exp_ms(token: &str) -> Option<i64> {
    decode_jwt_payload(token)
        .ok()?
        .get("exp")?
        .as_i64()
        .map(|seconds| seconds.saturating_mul(1000))
}

/// The document rendered into `CODEX_AUTH_JSON` for the environment bootstrap
/// to write to `$CODEX_HOME/auth.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodexAuthJson {
    pub auth_mode: &'static str,
    #[serde(rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
    pub tokens: CodexAuthJsonTokens,
    pub last_refresh: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodexAuthJsonTokens {
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

/// Renders the Codex `auth.json` document. `last_refresh_ms` is when
/// Lightspeed last obtained these tokens (RFC 3339 UTC in the file).
pub fn render_codex_auth_json(tokens: &ChatGptTokenSet, last_refresh_ms: i64) -> String {
    let document = CodexAuthJson {
        auth_mode: "chatgpt",
        openai_api_key: None,
        tokens: CodexAuthJsonTokens {
            id_token: tokens.id_token.clone(),
            access_token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone(),
            account_id: tokens.account_id.clone(),
        },
        last_refresh: rfc3339_utc(last_refresh_ms),
    };
    serde_json::to_string(&document).expect("codex auth.json serializes")
}

fn rfc3339_utc(epoch_ms: i64) -> String {
    let secs = epoch_ms.div_euclid(1000);
    let millis = epoch_ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

// Howard Hinnant's days-from-civil inverse; avoids a chrono dependency.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(payload: serde_json::Value) -> String {
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        format!(
            "{}.{}.{}",
            engine.encode(br#"{"alg":"RS256","typ":"JWT"}"#),
            engine.encode(payload.to_string()),
            engine.encode(b"sig")
        )
    }

    #[test]
    fn anthropic_token_requires_setup_token_prefix() {
        assert_eq!(
            parse_anthropic_claude_code_token("  sk-ant-oat01-abc  ").unwrap(),
            "sk-ant-oat01-abc"
        );
        assert_eq!(
            parse_anthropic_claude_code_token("sk-ant-api03-key"),
            Err(SubscriptionCredentialError::NotClaudeCodeToken)
        );
        assert_eq!(
            parse_anthropic_claude_code_token("  "),
            Err(SubscriptionCredentialError::Empty)
        );
    }

    #[test]
    fn bare_openai_token_is_an_access_token() {
        assert_eq!(
            parse_openai_chatgpt_credential(" oat_enterprise_token ").unwrap(),
            OpenAiChatGptCredential::AccessToken("oat_enterprise_token".to_owned())
        );
        assert!(matches!(
            parse_openai_chatgpt_credential("two words"),
            Err(SubscriptionCredentialError::InvalidAuthJson { .. })
        ));
    }

    #[test]
    fn auth_json_token_set_extracts_claims_and_expiry() {
        let id_token = jwt(serde_json::json!({
            "email": "lukas@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct_123",
                "chatgpt_plan_type": "pro"
            }
        }));
        let access_token = jwt(serde_json::json!({ "exp": 1_800_000_000 }));
        let input = serde_json::json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": id_token,
                "access_token": access_token,
                "refresh_token": "rt_1",
                "account_id": ""
            },
            "last_refresh": "2026-08-17T00:00:00Z"
        })
        .to_string();
        let OpenAiChatGptCredential::TokenSet { tokens, metadata } =
            parse_openai_chatgpt_credential(&input).unwrap()
        else {
            panic!("expected token set");
        };
        assert_eq!(tokens.refresh_token, "rt_1");
        assert_eq!(tokens.account_id.as_deref(), Some("acct_123"));
        assert_eq!(metadata.email.as_deref(), Some("lukas@example.com"));
        assert_eq!(metadata.plan_type.as_deref(), Some("pro"));
        assert_eq!(metadata.access_token_expires_at_ms, Some(1_800_000_000_000));
    }

    #[test]
    fn auth_json_without_tokens_is_rejected() {
        let input = r#"{"OPENAI_API_KEY":"sk-proj-x"}"#;
        assert_eq!(
            parse_openai_chatgpt_credential(input),
            Err(SubscriptionCredentialError::AuthJsonWithoutTokens)
        );
        let input = r#"{"tokens":{"access_token":"a","refresh_token":"r"}}"#;
        assert_eq!(
            parse_openai_chatgpt_credential(input),
            Err(SubscriptionCredentialError::AuthJsonMissing { field: "id_token" })
        );
    }

    #[test]
    fn codex_auth_json_matches_codex_layout() {
        let rendered = render_codex_auth_json(
            &ChatGptTokenSet {
                id_token: "id".into(),
                access_token: "at".into(),
                refresh_token: "rt".into(),
                account_id: Some("acct".into()),
            },
            1_755_388_800_123,
        );
        let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(value["auth_mode"], "chatgpt");
        assert!(value["OPENAI_API_KEY"].is_null());
        assert_eq!(value["tokens"]["access_token"], "at");
        assert_eq!(value["tokens"]["account_id"], "acct");
        assert_eq!(value["last_refresh"], "2025-08-17T00:00:00.123Z");
    }

    #[test]
    fn rfc3339_handles_epoch_and_leap_years() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_utc(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }
}
