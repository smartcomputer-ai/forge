// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

mod capture;
mod options;
mod snapshot;
mod tracker;
mod types;
mod upload;

pub use capture::{
    capture, deserialize_tree, ErrCyclicLink, ErrFileTooLarge, ErrTooManyFiles, FstreeError,
    FstreeErrorKind,
};
pub use options::{
    with_exclude, with_exclude_func, with_follow_symlinks, with_max_file_size, with_max_files,
    Options, SnapshotOption,
};
pub use tracker::Tracker;
pub use types::{
    EntryKind, EntryKindDirectory, EntryKindFile, EntryKindSymlink, FileRef, Snapshot,
    SnapshotDiff, SnapshotStats, TreeEntry, TreeObject,
};
pub use upload::{capture_and_upload, upload_and_attach, UploadResult};

/// Go-parity alias for snapshot option type.
pub type Option = SnapshotOption;

#[cfg(test)]
mod tests;
