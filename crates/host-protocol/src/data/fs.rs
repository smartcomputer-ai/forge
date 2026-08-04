//! Filesystem method payloads.

use serde::{Deserialize, Serialize};

use crate::shared::{ByteChunk, HostPath};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileParams {
    pub path: HostPath,
    /// Byte offset to start reading from. Hosts advertising
    /// `filesystem_ranged_read` honor it; others ignore it and return the
    /// full file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u64>,
    /// Maximum bytes to return. Hosts advertising `filesystem_ranged_read`
    /// truncate at the source and set `truncated`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileResponse {
    pub data: ByteChunk,
    /// Total size of the file on the host, independent of the returned range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    /// True when the returned data is a strict prefix range of the file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteFileParams {
    pub path: HostPath,
    pub data: ByteChunk,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteFileResponse {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDirectoryParams {
    pub path: HostPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recursive: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDirectoryResponse {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetMetadataParams {
    pub path: HostPath,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetMetadataResponse {
    pub is_directory: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub created_at_ms: i64,
    pub modified_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadDirectoryParams {
    pub path: HostPath,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadDirectoryEntry {
    pub file_name: String,
    pub is_directory: bool,
    pub is_file: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadDirectoryResponse {
    pub entries: Vec<ReadDirectoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveParams {
    pub path: HostPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recursive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveResponse {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyParams {
    pub source_path: HostPath,
    pub destination_path: HostPath,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyResponse {}

/// Bounded recursive text search executed natively by the host.
///
/// The host performs traversal and matching locally and returns only bounded
/// matches plus scan statistics, so a broad search does not become per-file
/// transfer over the data plane. Every limit is mandatory: a host must stop
/// scanning when any bound is reached and report why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTextParams {
    pub root: HostPath,
    /// Regular expression (Rust `regex` dialect, the same dialect the
    /// caller-side fallback uses).
    pub pattern: String,
    /// Optional glob filter applied to the root-relative path or file name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<String>,
    #[serde(default)]
    pub case_sensitive: bool,
    /// Maximum directory depth below the root, matching the generic
    /// traversal's semantics; absent means unbounded depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
    pub limits: SearchTextLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTextLimits {
    pub max_matches: u64,
    /// Maximum number of files searched (not merely visited by traversal).
    pub max_files: u64,
    /// Maximum cumulative bytes searched across files.
    pub max_bytes: u64,
    pub max_duration_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTextMatch {
    pub path: HostPath,
    pub line_number: u64,
    pub line: String,
}

/// Why a search stopped before exhausting the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchTextStop {
    MatchLimit,
    FileLimit,
    ByteLimit,
    TimeLimit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchTextResponse {
    pub matches: Vec<SearchTextMatch>,
    pub files_searched: u64,
    pub bytes_searched: u64,
    pub elapsed_ms: u64,
    /// Absent when the search exhausted the tree within its limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<SearchTextStop>,
}

/// Bounded recursive file enumeration executed natively by the host.
///
/// Pattern semantics mirror the caller-side generic glob tool: a pattern
/// starting with `/` matches the caller-space absolute path; otherwise the
/// root-relative path matches, and a pattern without `/` also matches bare
/// file names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobFilesParams {
    pub root: HostPath,
    pub pattern: String,
    /// Maximum directory depth below the root, matching the generic
    /// traversal's semantics; absent means unbounded depth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,
    pub limits: GlobFilesLimits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobFilesLimits {
    pub max_matches: u64,
    /// Maximum directory entries (files and directories) visited by the
    /// traversal.
    pub max_entries: u64,
    pub max_duration_ms: u64,
}

/// Why an enumeration stopped before exhausting the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GlobFilesStop {
    MatchLimit,
    EntryLimit,
    TimeLimit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobFilesResponse {
    pub matches: Vec<HostPath>,
    pub entries_visited: u64,
    pub elapsed_ms: u64,
    /// Absent when the enumeration exhausted the tree within its limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped: Option<GlobFilesStop>,
}
