// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use std::fs::File;
use std::path::{Path, PathBuf};

use super::capture::deserialize_tree;
use super::types::{
    EntryKindDirectory, EntryKindFile, EntryKindSymlink, Snapshot, SnapshotDiff, TreeEntry,
};
use super::{FstreeError, FstreeErrorKind};

impl Snapshot {
    pub fn get_file(&self, hash: [u8; 32]) -> Result<File, FstreeError> {
        let file_ref = self.files.get(&hash).ok_or_else(|| {
            FstreeError::new(
                FstreeErrorKind::Other,
                format!("file not found: {}", hash_prefix(&hash)),
            )
        })?;
        File::open(&file_ref.path)
            .map_err(|err| FstreeError::new(FstreeErrorKind::Io, err.to_string()))
    }

    pub fn get_tree(&self, hash: [u8; 32]) -> Result<Vec<TreeEntry>, FstreeError> {
        let data = self.trees.get(&hash).ok_or_else(|| {
            FstreeError::new(
                FstreeErrorKind::Other,
                format!("tree not found: {}", hash_prefix(&hash)),
            )
        })?;
        deserialize_tree(data)
    }

    pub fn get_root_entries(&self) -> Result<Vec<TreeEntry>, FstreeError> {
        self.get_tree(self.root_hash)
    }

    pub fn walk<F>(&self, mut f: F) -> Result<(), FstreeError>
    where
        F: FnMut(&str, &TreeEntry) -> Result<(), FstreeError>,
    {
        self.walk_tree(self.root_hash, Path::new(""), &mut f)
    }

    fn walk_tree<F>(&self, hash: [u8; 32], prefix: &Path, f: &mut F) -> Result<(), FstreeError>
    where
        F: FnMut(&str, &TreeEntry) -> Result<(), FstreeError>,
    {
        let entries = self.get_tree(hash)?;
        for entry in entries {
            let path = if prefix.as_os_str().is_empty() {
                PathBuf::from(&entry.name)
            } else {
                prefix.join(&entry.name)
            };
            let path_str = path.to_string_lossy();
            f(&path_str, &entry)?;
            if entry.kind == EntryKindDirectory {
                self.walk_tree(entry.hash, &path, f)?;
            }
        }
        Ok(())
    }

    pub fn list_files(&self) -> Result<Vec<String>, FstreeError> {
        let mut paths = Vec::new();
        self.walk(|path, entry| {
            if entry.kind == EntryKindFile {
                paths.push(path.to_string());
            }
            Ok(())
        })?;
        Ok(paths)
    }

    pub fn get_file_at_path(
        &self,
        path: &str,
    ) -> Result<Option<(TreeEntry, Option<File>)>, FstreeError> {
        let parts = split_path(path);
        if parts.is_empty() {
            return Err(FstreeError::new(FstreeErrorKind::Other, "empty path"));
        }

        let mut current_hash = self.root_hash;
        for (idx, part) in parts.iter().enumerate() {
            let entries = self.get_tree(current_hash)?;
            let mut found = None;
            for entry in entries {
                if entry.name == *part {
                    found = Some(entry);
                    break;
                }
            }
            let found = match found {
                Some(entry) => entry,
                None => {
                    return Err(FstreeError::new(
                        FstreeErrorKind::Other,
                        format!("path not found: {path}"),
                    ))
                }
            };

            if idx == parts.len() - 1 {
                if found.kind == EntryKindFile {
                    let file = self.get_file(found.hash)?;
                    return Ok(Some((found, Some(file))));
                }
                return Ok(Some((found, None)));
            }

            if found.kind != EntryKindDirectory {
                return Err(FstreeError::new(
                    FstreeErrorKind::Other,
                    format!("not a directory: {}", part),
                ));
            }
            current_hash = found.hash;
        }

        Ok(None)
    }

    pub fn diff(&self, old: Option<&Snapshot>) -> Result<SnapshotDiff, FstreeError> {
        let mut diff = SnapshotDiff {
            new_root: self.root_hash,
            ..SnapshotDiff::default()
        };

        if let Some(old) = old {
            diff.old_root = old.root_hash;
            if self.root_hash == old.root_hash {
                return Ok(diff);
            }
        }

        let mut new_paths = std::collections::HashMap::new();
        self.walk(|path, entry| {
            if entry.kind == EntryKindFile || entry.kind == EntryKindSymlink {
                new_paths.insert(path.to_string(), entry.hash);
            }
            Ok(())
        })?;

        if old.is_none() {
            diff.added = new_paths.keys().cloned().collect();
            return Ok(diff);
        }

        let old = old.unwrap();
        let mut old_paths = std::collections::HashMap::new();
        old.walk(|path, entry| {
            if entry.kind == EntryKindFile || entry.kind == EntryKindSymlink {
                old_paths.insert(path.to_string(), entry.hash);
            }
            Ok(())
        })?;

        for (path, new_hash) in &new_paths {
            match old_paths.get(path) {
                None => diff.added.push(path.clone()),
                Some(old_hash) => {
                    if old_hash != new_hash {
                        diff.modified.push(path.clone());
                    }
                }
            }
        }

        for path in old_paths.keys() {
            if !new_paths.contains_key(path) {
                diff.removed.push(path.clone());
            }
        }

        Ok(diff)
    }
}

impl SnapshotDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }

    pub fn total_changes(&self) -> usize {
        self.added.len() + self.removed.len() + self.modified.len()
    }
}

fn split_path(path: &str) -> Vec<String> {
    let normalized = path.replace('\\', "/");
    let normalized = Path::new(&normalized);
    let mut parts = Vec::new();
    for part in normalized.components() {
        if let std::path::Component::Normal(os_str) = part {
            if let Some(part) = os_str.to_str() {
                parts.push(part.to_string());
            }
        }
    }
    parts
}

fn hash_prefix(hash: &[u8; 32]) -> String {
    hash[..4].iter().map(|b| format!("{:02x}", b)).collect()
}
