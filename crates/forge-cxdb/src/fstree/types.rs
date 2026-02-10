// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

#![allow(non_upper_case_globals)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

pub type EntryKind = u8;

pub const EntryKindFile: EntryKind = 0;
pub const EntryKindDirectory: EntryKind = 1;
pub const EntryKindSymlink: EntryKind = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeEntry {
    #[serde(rename = "1")]
    pub name: String,
    #[serde(rename = "2")]
    pub kind: EntryKind,
    #[serde(rename = "3")]
    pub mode: u32,
    #[serde(rename = "4")]
    pub size: u64,
    #[serde(rename = "5")]
    #[serde(with = "serde_bytes")]
    pub hash: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeObject {
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub root_hash: [u8; 32],
    pub trees: HashMap<[u8; 32], Vec<u8>>,
    pub files: HashMap<[u8; 32], FileRef>,
    pub symlinks: HashMap<[u8; 32], String>,
    pub stats: SnapshotStats,
    pub captured_at: SystemTime,
}

#[derive(Debug, Clone)]
pub struct FileRef {
    pub path: PathBuf,
    pub size: u64,
    pub hash: [u8; 32],
}

#[derive(Debug, Clone, Default)]
pub struct SnapshotStats {
    pub file_count: usize,
    pub dir_count: usize,
    pub symlink_count: usize,
    pub total_bytes: u64,
    pub duration: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct SnapshotDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub modified: Vec<String>,
    pub old_root: [u8; 32],
    pub new_root: [u8; 32],
}
