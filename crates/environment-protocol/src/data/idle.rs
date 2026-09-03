//! Daemon activity report used by idle power policy.
//!
//! The daemon reports a monotonic idle *duration*, never a wall-clock
//! timestamp: after a freeze, stateful stop, or snapshot restore the guest
//! clock is stale until it resyncs, while a monotonic duration stays correct.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleParams {}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleResponse {
    /// Milliseconds since the last data-plane request or since the last
    /// process/job finished, whichever is later. Zero while anything runs.
    pub idle_for_ms: u64,
    /// Processes started through `process/start` that have not exited.
    pub running_processes: u32,
    /// Jobs that are running or cancelling.
    pub running_jobs: u32,
    /// Process groups of finished commands that still have members alive:
    /// services and other leftovers a command started and did not stop.
    /// They are not running work, so they never block idle power policy;
    /// a policy may prefer freezing over stopping while the count is
    /// non-zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub leftover_process_groups: u32,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl IdleResponse {
    /// True when nothing is executing inside the environment.
    pub fn is_quiescent(&self) -> bool {
        self.running_processes == 0 && self.running_jobs == 0
    }
}
