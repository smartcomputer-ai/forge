//! Bounded inline filesystem copies. No VFS identities or storage access are required.
use crate::shared::{ByteChunk, EnvironmentPath};
use serde::{Deserialize, Serialize};

/// Hard ceilings for one operation, before base64/JSON framing.
pub const MAX_TRANSFER_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_TRANSFER_ENTRIES: u32 = 1024;
pub const MAX_TRANSFER_DEPTH: u32 = 32;
pub const MAX_TRANSFER_DURATION_MS: u64 = 30_000;
pub const MAX_TRANSFER_PATH_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferLimits {
    /// Counts the selected root, including an empty directory.
    pub max_entries: u32,
    /// Selected root has depth zero.
    pub max_depth: u32,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    /// Cooperative deadline checked between filesystem operations and byte chunks.
    /// Does not interrupt an in-flight kernel syscall.
    pub max_duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TransferContent {
    Directory,
    File { data: ByteChunk, executable: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferEntry {
    /// Root is "". Others are strict relative '/' paths; parents must precede children.
    /// UTF-8 names only; no empty, '.', '..', NUL or backslash components.
    pub path: String,
    pub content: TransferContent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferOnExisting {
    Error,
    #[default]
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterializeParams {
    pub destination: EnvironmentPath,
    pub entries: Vec<TransferEntry>,
    pub limits: TransferLimits,
    #[serde(default)]
    pub on_existing: TransferOnExisting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterializeResponse {
    pub destination: EnvironmentPath,
    pub entries: u32,
    pub bytes: u64,
    /// Reserved for compatibility with early daemons. Current daemons clean retired
    /// staging trees asynchronously and return None.
    pub retired_directory: Option<EnvironmentPath>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureParams {
    pub source: EnvironmentPath,
    pub limits: TransferLimits,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResponse {
    pub source: EnvironmentPath,
    /// Complete selected tree only; failures never return a partial inventory.
    pub entries: Vec<TransferEntry>,
    pub bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binary_inventory_roundtrips_and_legacy_capabilities_default_false() {
        let entry = TransferEntry {
            path: "".into(),
            content: TransferContent::File {
                data: ByteChunk::from(vec![0, 255, 128]),
                executable: true,
            },
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["content"]["data"], "AP+A");
        assert_eq!(
            serde_json::from_value::<TransferEntry>(json).unwrap(),
            entry
        );
        let capabilities: crate::shared::EnvironmentCapabilities =
            serde_json::from_str("{}").unwrap();
        assert!(!capabilities.filesystem_capture);
        assert!(!capabilities.filesystem_materialize);
        assert!(serde_json::from_str::<CaptureParams>(r#"{"source":"."}"#).is_err());
    }
}
