//! Key-based outbound registration: the control-channel messages an `envd`
//! exchanges with the environment gateway after dialing out.
//!
//! The control connection carries registration, liveness, and requests to
//! open data connections. It never carries environment-protocol frames:
//! every worker route is served by a separate, reverse-dialed data socket
//! that runs the ordinary data protocol.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::shared::{CURRENT_PROTOCOL_VERSION, ImplementationInfo, SecretString};

/// Public route an `envd` dials to register and hold its control connection.
pub const CONNECT_PATH: &str = "/environment-gateway/connect";

/// Public route an `envd` dials, presenting a one-time token as a bearer
/// header, to serve one worker route.
pub const DATA_PATH: &str = "/environment-gateway/data";

/// Domain separator the daemon signs together with the gateway's nonce, so a
/// signature over a nonce can never be confused with any other Ed25519 use
/// of the same key.
pub const REGISTRATION_SIGNATURE_DOMAIN: &[u8] = b"lightspeed-envd-registration/v1";

/// Bytes of the gateway's per-connection challenge nonce.
pub const REGISTRATION_NONCE_BYTES: usize = 32;

pub const MAX_DISPLAY_NAME_BYTES: usize = 128;
pub const MAX_METADATA_ENTRIES: usize = 32;
pub const MAX_METADATA_KEY_BYTES: usize = 64;
pub const MAX_METADATA_VALUE_BYTES: usize = 256;
/// Metadata keys under this prefix are written by Lightspeed only.
pub const RESERVED_METADATA_PREFIX: &str = "lightspeed.";

/// Messages the gateway sends on the control connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GatewayControlMessage {
    /// First frame after the upgrade. `nonce` is hex; the daemon signs
    /// [`signed_registration_message`] of its raw bytes.
    Challenge {
        protocol_version: u32,
        nonce: String,
    },
    /// The daemon is admitted; the receipt identifies the environment it
    /// now serves. `heartbeatIntervalMs` tells the daemon how often the
    /// gateway pings.
    Accepted {
        #[serde(flatten)]
        receipt: RegistrationReceipt,
        heartbeat_interval_ms: u64,
    },
    /// Registration failed. The gateway closes the socket after this frame.
    Rejected {
        code: RegistrationRejectionCode,
        message: String,
    },
    /// A worker route is waiting: dial `dataUrl` with `token` as a bearer
    /// header and run the data protocol on that socket.
    OpenData {
        token: SecretString,
        data_url: String,
    },
}

/// Messages the daemon sends on the control connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DaemonControlMessage {
    Register(RegisterParams),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterParams {
    #[serde(default = "default_protocol_version")]
    pub protocol_version: u32,
    /// Required for a first-seen daemon public key; ignored for a known one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration_key: Option<SecretString>,
    /// Lowercase hex of the raw 32-byte Ed25519 public key.
    pub daemon_public_key: String,
    /// Lowercase hex of the 64-byte Ed25519 signature over
    /// [`signed_registration_message`] of the challenge nonce.
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    pub implementation: ImplementationInfo,
}

/// What the daemon learns about the environment it serves. Written to the
/// receipt file and logged once; contains no secret material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationReceipt {
    pub environment_id: String,
    pub incarnation_id: String,
    pub daemon_id: String,
    pub connection_id: String,
    pub identity_mode: String,
}

/// Why a registration was refused. Terminal codes mean the daemon must not
/// retry with the same identity and configuration; the others are transient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegistrationRejectionCode {
    UnsupportedProtocol,
    InvalidRequest,
    InvalidSignature,
    InvalidRegistrationKey,
    RegistrationKeyRevoked,
    RegistrationKeyExpired,
    /// A daemon identity the gateway has never seen presented no key.
    UnknownDaemon,
    /// The identity's environment was closed; the identity is spent.
    EnvironmentClosed,
    /// The identity is already bound in another universe.
    IdentityInUse,
    CapacityExhausted,
    RateLimited,
    Unavailable,
}

impl RegistrationRejectionCode {
    pub fn is_terminal(self) -> bool {
        !matches!(
            self,
            Self::CapacityExhausted | Self::RateLimited | Self::Unavailable
        )
    }
}

/// The bytes a daemon signs for one challenge.
pub fn signed_registration_message(nonce: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(REGISTRATION_SIGNATURE_DOMAIN.len() + nonce.len());
    message.extend_from_slice(REGISTRATION_SIGNATURE_DOMAIN);
    message.extend_from_slice(nonce);
    message
}

/// Check the bounds on caller-supplied correlation metadata and display
/// name. Metadata is descriptive only; it never selects or authenticates.
/// Caller-supplied maps may not use the reserved prefix; stored records
/// may, because Lightspeed annotates them itself (see
/// [`validate_metadata_bounds`]). Sessions share this validator so a
/// session and the environment it ran in accept the same keys.
pub fn validate_registration_metadata(
    display_name: Option<&str>,
    metadata: &BTreeMap<String, String>,
) -> Result<(), String> {
    if let Some(name) = display_name
        && (name.is_empty()
            || name.len() > MAX_DISPLAY_NAME_BYTES
            || name.chars().any(char::is_control))
    {
        return Err(format!(
            "display name must be 1..={MAX_DISPLAY_NAME_BYTES} bytes without control characters"
        ));
    }
    validate_metadata_bounds(metadata)?;
    if let Some(key) = metadata
        .keys()
        .find(|key| key.starts_with(RESERVED_METADATA_PREFIX))
    {
        return Err(format!(
            "metadata key {key:?} uses the reserved {RESERVED_METADATA_PREFIX} prefix"
        ));
    }
    Ok(())
}

/// Entry count, key and value bytes, and control characters: the bounds
/// every stored metadata map obeys, including the keys under the reserved
/// prefix that Lightspeed writes itself.
pub fn validate_metadata_bounds(metadata: &BTreeMap<String, String>) -> Result<(), String> {
    if metadata.len() > MAX_METADATA_ENTRIES {
        return Err(format!(
            "metadata has more than {MAX_METADATA_ENTRIES} entries"
        ));
    }
    for (key, value) in metadata {
        validate_metadata_entry(key, value)?;
    }
    Ok(())
}

/// Key and value bytes and control characters for one entry, independent of
/// how many entries a map may hold.
pub fn validate_metadata_entry(key: &str, value: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > MAX_METADATA_KEY_BYTES || key.chars().any(char::is_control) {
        return Err(format!(
            "metadata key {key:?} must be 1..={MAX_METADATA_KEY_BYTES} bytes without control characters"
        ));
    }
    if value.is_empty()
        || value.len() > MAX_METADATA_VALUE_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(format!(
            "metadata value for {key:?} must be 1..={MAX_METADATA_VALUE_BYTES} bytes without control characters"
        ));
    }
    Ok(())
}

pub fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

pub fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

fn default_protocol_version() -> u32 {
    CURRENT_PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_messages_are_tagged_and_flatten_the_receipt() {
        let accepted = GatewayControlMessage::Accepted {
            receipt: RegistrationReceipt {
                environment_id: "environment_a".to_owned(),
                incarnation_id: "incarnation_a".to_owned(),
                daemon_id: "daemon_a".to_owned(),
                connection_id: "connection_a".to_owned(),
                identity_mode: "ephemeral".to_owned(),
            },
            heartbeat_interval_ms: 30_000,
        };
        let json = serde_json::to_value(&accepted).expect("serialize");
        assert_eq!(json["type"], "accepted");
        assert_eq!(json["environmentId"], "environment_a");
        assert_eq!(json["heartbeatIntervalMs"], 30_000);
        let parsed: GatewayControlMessage = serde_json::from_value(json).expect("parse");
        assert_eq!(parsed, accepted);

        let open = serde_json::json!({"type": "openData", "token": "t", "dataUrl": "wss://g/d"});
        let parsed: GatewayControlMessage = serde_json::from_value(open).expect("parse");
        assert!(matches!(parsed, GatewayControlMessage::OpenData { .. }));
        assert!(!format!("{parsed:?}").contains("\"t\""));
    }

    #[test]
    fn rejection_codes_split_terminal_from_retryable() {
        assert!(RegistrationRejectionCode::EnvironmentClosed.is_terminal());
        assert!(RegistrationRejectionCode::InvalidSignature.is_terminal());
        assert!(RegistrationRejectionCode::RegistrationKeyRevoked.is_terminal());
        assert!(!RegistrationRejectionCode::CapacityExhausted.is_terminal());
        assert!(!RegistrationRejectionCode::RateLimited.is_terminal());
        assert!(!RegistrationRejectionCode::Unavailable.is_terminal());
    }

    #[test]
    fn metadata_bounds_and_reserved_prefix_are_enforced() {
        let mut metadata = BTreeMap::new();
        metadata.insert("harbor.trialId".to_owned(), "trial-1".to_owned());
        assert!(validate_registration_metadata(Some("worker"), &metadata).is_ok());
        metadata.insert("lightspeed.x".to_owned(), "y".to_owned());
        assert!(validate_registration_metadata(None, &metadata).is_err());
        metadata.clear();
        metadata.insert("k".to_owned(), "a".repeat(MAX_METADATA_VALUE_BYTES + 1));
        assert!(validate_registration_metadata(None, &metadata).is_err());
        assert!(validate_registration_metadata(Some("bad\nname"), &BTreeMap::new()).is_err());
        assert!(validate_registration_metadata(Some(""), &BTreeMap::new()).is_err());
    }

    #[test]
    fn hex_round_trips_and_signed_message_is_domain_separated() {
        let bytes = [0u8, 1, 0xab, 0xff];
        assert_eq!(encode_hex(&bytes), "0001abff");
        assert_eq!(decode_hex("0001abff").as_deref(), Some(&bytes[..]));
        assert_eq!(decode_hex("abc"), None);
        assert_eq!(decode_hex("zz"), None);
        let message = signed_registration_message(&[9u8; 4]);
        assert!(message.starts_with(REGISTRATION_SIGNATURE_DOMAIN));
        assert!(message.ends_with(&[9u8; 4]));
    }
}
