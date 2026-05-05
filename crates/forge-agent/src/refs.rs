//! Artifact and transcript reference records.
//!
//! Large prompts, responses, tool arguments, outputs, patches, and compaction
//! artifacts are represented by refs plus short optional previews.

use crate::ids::SessionId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCompatibility {
    pub provider: String,
    pub api_kind: String,
    pub model: Option<String>,
    pub model_family: Option<String>,
    pub artifact_type: String,
    pub opaque: bool,
    pub encrypted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    UserPrompt,
    AssistantMessage,
    ReasoningSummary,
    RawLlmResponse,
    ToolArguments,
    ToolOutput,
    FileContent,
    Patch,
    Compaction,
    ProviderNative,
    #[default]
    Custom,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub uri: String,
    pub kind: ArtifactKind,
    pub media_type: Option<String>,
    pub digest: Option<String>,
    pub byte_len: Option<u64>,
    pub preview: Option<String>,
    pub provider_compatibility: Option<ProviderCompatibility>,
    pub metadata: BTreeMap<String, String>,
}

impl ArtifactRef {
    pub fn new(uri: impl Into<String>, kind: ArtifactKind) -> Self {
        Self {
            uri: uri.into(),
            kind,
            media_type: None,
            digest: None,
            byte_len: None,
            preview: None,
            provider_compatibility: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_preview(mut self, preview: impl Into<String>) -> Self {
        self.preview = Some(preview.into());
        self
    }

    pub fn provider_native(uri: impl Into<String>, compatibility: ProviderCompatibility) -> Self {
        Self {
            provider_compatibility: Some(compatibility),
            ..Self::new(uri, ArtifactKind::ProviderNative)
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRefKind {
    #[default]
    Prefix,
    Snapshot,
    CompactedSnapshot,
    ImportedHistory,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptBoundary {
    pub entry_seq: Option<u64>,
    pub event_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptRef {
    pub uri: String,
    pub kind: TranscriptRefKind,
    pub source_session_id: Option<SessionId>,
    pub boundary: Option<TranscriptBoundary>,
    pub artifact_ref: Option<ArtifactRef>,
    pub metadata: BTreeMap<String, String>,
}

impl TranscriptRef {
    pub fn new(uri: impl Into<String>, kind: TranscriptRefKind) -> Self {
        Self {
            uri: uri.into(),
            kind,
            source_session_id: None,
            boundary: None,
            artifact_ref: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_ref_can_carry_provider_compatibility() {
        let artifact = ArtifactRef::provider_native(
            "blob://raw-window",
            ProviderCompatibility {
                provider: "openai".into(),
                api_kind: "responses".into(),
                model: Some("gpt-x".into()),
                model_family: Some("gpt".into()),
                artifact_type: "raw_window".into(),
                opaque: true,
                encrypted: false,
            },
        );

        assert_eq!(artifact.kind, ArtifactKind::ProviderNative);
        assert!(
            artifact
                .provider_compatibility
                .as_ref()
                .is_some_and(|value| value.opaque)
        );
    }

    #[test]
    fn transcript_ref_round_trips_through_msgpack() {
        let transcript = TranscriptRef {
            uri: "transcript://session-a/prefix/3".into(),
            kind: TranscriptRefKind::Prefix,
            source_session_id: Some(SessionId::new("session-a")),
            boundary: Some(TranscriptBoundary {
                entry_seq: Some(3),
                event_id: None,
            }),
            artifact_ref: None,
            metadata: BTreeMap::new(),
        };

        let encoded = rmp_serde::to_vec_named(&transcript).expect("encode transcript ref");
        let decoded: TranscriptRef =
            rmp_serde::from_slice(&encoded).expect("decode transcript ref");
        assert_eq!(decoded, transcript);
    }
}
