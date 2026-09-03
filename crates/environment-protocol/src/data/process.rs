//! Process method payloads.
//!
//! A process is started as the leader of its own process group and read
//! through a daemon-owned cursor: `process/read` without `afterSeq` returns
//! the output produced since the previous cursor read and advances the
//! cursor, so a handle used from a later connection never re-reads output.
//! The root's natural exit never sweeps the group; whatever it left running
//! is reported and keeps the environment's lifetime.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::shared::{ByteChunk, EnvironmentPath, ProcessId, SecretString};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartProcessParams {
    pub process_id: ProcessId,
    pub argv: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<EnvironmentPath>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secret_env: BTreeMap<String, SecretString>,
    /// One-shot standard input, written and closed at start. Plain-pipe
    /// processes otherwise read end of file from `/dev/null`; interactive
    /// input requires `tty`, whose master is the process's input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<ByteChunk>,
    /// Kill deadline. Absent means the call never kills a running process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub tty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartProcessResponse {
    pub process_id: ProcessId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadProcessParams {
    pub process_id: ProcessId,
    /// Explicit re-read from a sequence number over the retained output. It
    /// does not move the daemon's cursor; omit it to read after the last
    /// delivered chunk and advance the cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<usize>,
    /// Collect output for this long, returning early only when the process
    /// exits or `max_bytes` is reached. Absent blocks until exit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOutputChunk {
    pub seq: u64,
    pub stream: ProcessOutputStream,
    pub chunk: ByteChunk,
}

/// A process still running in the command's group after the root exited.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeftoverProcess {
    pub pid: u32,
    /// Command line when the platform exposes it, otherwise the executable
    /// name; may be empty when neither could be read.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
}

/// Why a process that did not exit on its own stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessTermination {
    /// The start deadline (`timeoutMs`) expired and the group was killed.
    TimedOut,
    /// `process/terminate` with `kill` ended the group.
    Killed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadProcessResponse {
    pub chunks: Vec<ProcessOutputChunk>,
    pub next_seq: u64,
    pub exited: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub closed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// Set when the daemon stopped the process; absent on a natural exit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination: Option<ProcessTermination>,
    /// OS pid of the root process, which is also its process group id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Bytes dropped from the middle of the retained output since the last
    /// cursor read because the retained buffer overflowed. The gap sits at
    /// the sequence discontinuity between two returned chunks.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_bytes: u64,
    /// Processes still running in the command's group after the root
    /// exited, sampled when this read was answered. Empty until the exit is
    /// observed and on platforms without process enumeration.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leftover_processes: Vec<LeftoverProcess>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteProcessParams {
    pub process_id: ProcessId,
    /// Bytes for the process's input. Requires an open input (a PTY); an
    /// absent or empty chunk with `closeStdin: false` is a wait and is
    /// accepted whether or not the process has exited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk: Option<ByteChunk>,
    #[serde(default)]
    pub close_stdin: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WriteProcessStatus {
    Accepted,
    /// The entry is gone: the process finished and its retention elapsed.
    UnknownProcess,
    /// Input was given but the process has no open input, or has exited.
    StdinClosed,
    Starting,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteProcessResponse {
    pub status: WriteProcessStatus,
}

/// Signal delivered by `process/terminate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessSignal {
    /// `SIGINT` to the process group; changes no daemon state, the next
    /// read observes whatever the process did.
    Interrupt,
    /// Kill the whole group; the process is recorded as killed.
    Kill,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminateProcessParams {
    pub process_id: ProcessId,
    pub signal: ProcessSignal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminateProcessResponse {
    /// True when the process was still running when the signal was sent.
    pub running: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeProcessParams {
    pub process_id: ProcessId,
    pub size: TerminalSize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResizeProcessResponse {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcessOutputStream {
    Stdout,
    Stderr,
    Pty,
}
