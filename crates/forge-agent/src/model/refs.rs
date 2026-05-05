//! Artifact reference records.
//!
//! Large prompts, responses, tool arguments, outputs, patches, and compaction
//! artifacts are represented by refs plus short optional previews.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub uri: String,
    pub media_type: Option<String>,
    pub digest: Option<String>,
    pub byte_len: Option<u64>,
    pub preview: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

impl ArtifactRef {
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            media_type: None,
            digest: None,
            byte_len: None,
            preview: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_preview(mut self, preview: impl Into<String>) -> Self {
        self.preview = Some(preview.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_ref_round_trips_through_json() {
        let artifact = ArtifactRef::new("blob://payload").with_preview("payload");

        let encoded = serde_json::to_string(&artifact).expect("serialize artifact ref");
        let decoded: ArtifactRef = serde_json::from_str(&encoded).expect("decode artifact ref");
        assert_eq!(decoded, artifact);
    }
}
