// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::test_util::decode_hex;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use tempfile::TempDir;

#[derive(Debug, Deserialize)]
struct FstreeFixture {
    root_hash_hex: String,
    trees: HashMap<String, String>,
    files: HashMap<String, String>,
    blake3: HashMap<String, String>,
}

fn load_fixture() -> FstreeFixture {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fstree_basic.json");
    let data = fs::read_to_string(&path).expect("read fstree fixture");
    serde_json::from_str(&data).expect("parse fstree fixture")
}

fn decode_hash(hex_str: &str) -> [u8; 32] {
    let bytes = decode_hex(hex_str);
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    hash
}

fn seed_workspace(root: &std::path::Path) {
    fs::create_dir_all(root.join("src")).unwrap();
    write_file(root.join("README.md"), b"# Test", 0o644);
    write_file(root.join("src").join("main.go"), b"package main", 0o644);
    write_file(
        root.join("src").join("lib.go"),
        b"package main\n\nfunc foo() {}",
        0o644,
    );
    write_file(root.join("script.sh"), b"#!/bin/bash\necho hi", 0o755);
}

fn write_file(path: std::path::PathBuf, data: &[u8], mode: u32) {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(mode)
            .open(&path)
            .unwrap();
        file.write_all(data).unwrap();
    }

    #[cfg(not(unix))]
    {
        let _ = mode;
        fs::write(path, data).unwrap();
    }
}

#[test]
fn blake3_hashes_match_fixture() {
    let fixture = load_fixture();
    let empty = blake3::hash(&[]);
    let hello = blake3::hash(b"hello");
    assert_eq!(
        fixture.blake3.get("empty").unwrap(),
        &hex::encode(empty.as_bytes())
    );
    assert_eq!(
        fixture.blake3.get("hello").unwrap(),
        &hex::encode(hello.as_bytes())
    );
}

#[test]
fn capture_matches_fstree_fixture() {
    let fixture = load_fixture();
    let dir = TempDir::new().unwrap();
    seed_workspace(dir.path());

    let snap = capture(dir.path(), Vec::<SnapshotOption>::new()).unwrap();
    let expected_root = decode_hash(&fixture.root_hash_hex);
    assert_eq!(snap.root_hash, expected_root);

    let mut rust_trees = HashMap::new();
    for (hash, data) in &snap.trees {
        rust_trees.insert(hex::encode(hash), hex::encode(data));
    }
    assert_eq!(rust_trees, fixture.trees);

    let root = fs::canonicalize(dir.path()).unwrap_or_else(|_| dir.path().to_path_buf());
    let mut rust_files = HashMap::new();
    for (hash, file_ref) in &snap.files {
        let rel = file_ref.path.strip_prefix(&root).unwrap_or(&file_ref.path);
        rust_files.insert(rel.to_string_lossy().replace('\\', "/"), hex::encode(hash));
    }
    assert_eq!(rust_files, fixture.files);
}

#[test]
fn capture_basic_tree_stats() {
    let dir = TempDir::new().unwrap();
    seed_workspace(dir.path());

    let snap = capture(dir.path(), Vec::<SnapshotOption>::new()).unwrap();
    assert_eq!(snap.stats.file_count, 4);
    assert_eq!(snap.stats.dir_count, 2);

    let files = snap.list_files().unwrap();
    assert_eq!(files.len(), 4);
}

#[test]
fn capture_deterministic_hash() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();
    fs::write(dir.path().join("b.txt"), "world").unwrap();

    let snap1 = capture(dir.path(), Vec::<SnapshotOption>::new()).unwrap();
    let snap2 = capture(dir.path(), Vec::<SnapshotOption>::new()).unwrap();
    assert_eq!(snap1.root_hash, snap2.root_hash);
}

#[test]
fn capture_content_addressing() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("file1.txt"), "same").unwrap();
    fs::write(dir.path().join("file2.txt"), "same").unwrap();

    let snap = capture(dir.path(), Vec::<SnapshotOption>::new()).unwrap();
    assert_eq!(snap.stats.file_count, 2);
    assert_eq!(snap.files.len(), 1);
}

#[test]
fn capture_exclude_patterns() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("node_modules").join("pkg")).unwrap();
    fs::write(dir.path().join("main.js"), "console.log('hi')").unwrap();
    fs::write(dir.path().join("debug.log"), "debug info").unwrap();
    fs::write(
        dir.path().join("node_modules").join("pkg").join("index.js"),
        "module",
    )
    .unwrap();

    let snap = capture(
        dir.path(),
        vec![with_exclude(vec!["*.log", "node_modules"])],
    )
    .unwrap();
    let files = snap.list_files().unwrap();
    assert_eq!(files.len(), 1);
}

#[cfg(unix)]
#[test]
fn capture_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("target.txt"), "target").unwrap();
    symlink("target.txt", dir.path().join("link.txt")).unwrap();

    let snap = capture(dir.path(), Vec::<SnapshotOption>::new()).unwrap();
    assert_eq!(snap.stats.file_count, 1);
    assert_eq!(snap.stats.symlink_count, 1);
    assert_eq!(snap.symlinks.len(), 1);
}

#[cfg(unix)]
#[test]
fn capture_mode_bits() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("script.sh"), "#!/bin/bash").unwrap();
    fs::write(dir.path().join("data.txt"), "data").unwrap();
    fs::set_permissions(
        dir.path().join("script.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    fs::set_permissions(
        dir.path().join("data.txt"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let snap = capture(dir.path(), Vec::<SnapshotOption>::new()).unwrap();
    let entries = snap.get_root_entries().unwrap();
    let mut modes = HashMap::new();
    for entry in entries {
        modes.insert(entry.name, entry.mode);
    }
    assert_eq!(modes.get("script.sh").copied().unwrap_or(0), 0o755);
    assert_eq!(modes.get("data.txt").copied().unwrap_or(0), 0o644);
}

#[test]
fn snapshot_diff_tracks_changes() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("keep.txt"), "keep").unwrap();
    fs::write(dir.path().join("modify.txt"), "original").unwrap();
    fs::write(dir.path().join("delete.txt"), "delete me").unwrap();

    let snap1 = capture(dir.path(), Vec::<SnapshotOption>::new()).unwrap();

    fs::write(dir.path().join("modify.txt"), "modified").unwrap();
    fs::write(dir.path().join("new.txt"), "new file").unwrap();
    fs::remove_file(dir.path().join("delete.txt")).unwrap();

    let snap2 = capture(dir.path(), Vec::<SnapshotOption>::new()).unwrap();
    let diff = snap2.diff(Some(&snap1)).unwrap();

    let mut added = diff.added.clone();
    let mut modified = diff.modified.clone();
    let mut removed = diff.removed.clone();
    added.sort();
    modified.sort();
    removed.sort();

    assert_eq!(added, vec!["new.txt".to_string()]);
    assert_eq!(modified, vec!["modify.txt".to_string()]);
    assert_eq!(removed, vec!["delete.txt".to_string()]);
}

#[test]
fn snapshot_get_file_at_path() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("src").join("pkg")).unwrap();
    fs::write(
        dir.path().join("src").join("pkg").join("main.go"),
        "package main",
    )
    .unwrap();

    let snap = capture(dir.path(), Vec::<SnapshotOption>::new()).unwrap();
    let (entry, file) = snap
        .get_file_at_path("src/pkg/main.go")
        .unwrap()
        .expect("entry");
    assert_eq!(entry.name, "main.go");
    assert_eq!(entry.kind, EntryKindFile);
    let mut contents = String::new();
    file.unwrap().read_to_string(&mut contents).unwrap();
    assert!(contents.contains("package main"));
}

#[cfg(unix)]
#[test]
fn capture_follow_symlinks_detects_cycle() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("loop")).unwrap();
    symlink(dir.path(), dir.path().join("loop").join("self")).unwrap();

    let err = capture(dir.path(), vec![with_follow_symlinks()]).unwrap_err();
    assert_eq!(err.kind, ErrCyclicLink);
}

#[test]
fn capture_max_file_size_is_enforced() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("big.txt"), "0123456789").unwrap();

    let snap = capture(dir.path(), vec![with_max_file_size(4)]).unwrap();
    assert_eq!(snap.stats.file_count, 0);
}

#[test]
fn capture_max_files_is_enforced() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("one.txt"), "1").unwrap();
    fs::write(dir.path().join("two.txt"), "2").unwrap();

    let err = capture(dir.path(), vec![with_max_files(1)]).unwrap_err();
    assert_eq!(err.kind, ErrTooManyFiles);
}

#[test]
fn tracker_snapshot_if_changed() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("file.txt"), "content").unwrap();

    let tracker = Tracker::new(
        dir.path().to_string_lossy().to_string(),
        Vec::<SnapshotOption>::new(),
    );
    let (snap1, changed1) = tracker.snapshot_if_changed().unwrap();
    assert!(changed1);
    assert!(snap1.is_some());

    let (snap2, changed2) = tracker.snapshot_if_changed().unwrap();
    assert!(!changed2);
    assert!(snap2.is_none());
}
