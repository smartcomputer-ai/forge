// Copyright 2025 StrongDM Inc
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, RwLock};

use super::capture::{capture, Result as FstreeResult};
use super::options::SnapshotOption;
use super::types::{Snapshot, SnapshotDiff};

pub struct Tracker {
    root: String,
    opts: Vec<SnapshotOption>,
    last_snapshot: Arc<RwLock<Option<Snapshot>>>,
}

impl Tracker {
    pub fn new(root: impl Into<String>, opts: impl IntoIterator<Item = SnapshotOption>) -> Self {
        Self {
            root: root.into(),
            opts: opts.into_iter().collect(),
            last_snapshot: Arc::new(RwLock::new(None)),
        }
    }

    pub fn snapshot(&self) -> FstreeResult<(Snapshot, bool)> {
        let snap = capture(&self.root, self.opts.clone())?;
        let mut guard = self.last_snapshot.write().unwrap();
        let changed = guard
            .as_ref()
            .map(|prev| prev.root_hash != snap.root_hash)
            .unwrap_or(true);
        *guard = Some(snap.clone());
        Ok((snap, changed))
    }

    pub fn last_snapshot(&self) -> Option<Snapshot> {
        self.last_snapshot.read().unwrap().clone()
    }

    pub fn snapshot_if_changed(&self) -> FstreeResult<(Option<Snapshot>, bool)> {
        let (snap, changed) = self.snapshot()?;
        if changed {
            Ok((Some(snap), true))
        } else {
            Ok((None, false))
        }
    }

    pub fn diff_from_last(&self, current: &Snapshot) -> FstreeResult<SnapshotDiff> {
        let last = self.last_snapshot.read().unwrap().clone();
        current.diff(last.as_ref())
    }
}
