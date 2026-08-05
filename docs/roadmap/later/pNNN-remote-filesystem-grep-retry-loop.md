# Remote Filesystem Grep Can Trap a Session in an Infinite Retry Loop

**Status**

- Later / reliability and resource-bounding follow-up.
- Discovered in production on 2026-08-04 while a session searched a remote
  environment attached through `host-bridge`.
- This is not primarily a model-provider failure or a host availability
  problem. It is the interaction between an unbounded remote filesystem scan,
  a coarse Temporal activity timeout, and Temporal's default unlimited retry
  policy.

## Incident

Production session `session_9b0aa37836f9401aa49e3c4163f74fc4`
stopped making visible progress at event sequence 477. Run 5, turn 28, tool
batch 24 had started five environment filesystem calls:

- two `read_file` calls;
- three recursive `grep` calls.

The grep roots were in
`/home/agent/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f` on the
`ls-dev` environment served from `hz02`. At the time of inspection that tree
was about 733 MiB and contained 15,983 Rust source files. The three searches
had no `max_depth`; two searched the full `temporalio-sdk-0.4.0` subtree and
one searched the full Cargo registry source tree.

The session workflow remained open, but its only pending activity was
`WorkflowActivities::tool_invoke_batch`. Temporal reported repeated
`StartToClose` timeouts, attempt 28, and `MaximumAttempts: 0` (unlimited).
Consequently the UI continued to show the session as running while the same
batch was restarted after every timeout.

Both hosts were lightly loaded. The Lightspeed worker, Temporal server, and
host bridge were running, and the environment registry continued to report
the target as ready. The host bridge logged repeated WebSocket resets during
the same period, consistent with remote clients being torn down; there was no
evidence that the bridge daemon itself was unavailable.

## Failure Mechanism

The canonical `grep` implementation is implemented above the generic
`FileSystem` interface rather than by a provider-side search operation. It:

1. recursively enumerates every file below the root with
   `collect_file_paths`;
2. filters the collected paths by the optional `include` glob;
3. reads every included file through `FileSystem::read_file`;
4. applies the regular expression in the Lightspeed worker.

For a remote host filesystem, every metadata, directory, and file read is a
host-protocol request. `RemoteHostFileSystem` protects its single
`HostDataClient` with an async mutex, serializing requests through that client.
A recursive grep that is cheap with a native tool such as `rg` can therefore
become thousands of network round trips plus transfer of all searched file
contents. Running several such greps in one tool batch compounds the cost.

The session workflow schedules the entire tool batch as one Temporal activity
with a 360-second start-to-close timeout. Its `ActivityOptions` do not declare
a bounded retry policy, so Temporal applies its default retry behavior. When
the batch exceeds 360 seconds, Temporal times out the activity and retries the
whole batch from the beginning. No per-call progress is durable, so a retry
cannot resume after directory enumeration or preserve successful calls from
the previous attempt. If the workload consistently takes longer than the
timeout, the session cannot advance on its own.

Relevant code:

- `crates/tools/src/fs/tools/grep.rs`: recursive collection and per-file reads;
- `crates/tools/src/fs/tools/shared.rs`: `collect_file_paths` traversal;
- `crates/tools/src/host_protocol/remote.rs`: the mutex-guarded remote host
  data client;
- `crates/temporal-workflow/src/config.rs`: the 360-second activity timeout
  and activity options without an explicit retry bound;
- `crates/temporal-workflow/src/workflows/session/activity_calls.rs`: the
  entire tool batch is submitted as one activity.

## Impact

- A single broad environment `grep` can make an interactive session appear to
  spin forever.
- Retries repeat expensive filesystem and network work instead of making
  progress.
- Because retries apply to the whole tool batch, the same policy is risky for
  batches containing non-idempotent tools, even though this incident involved
  only reads.
- The environment can remain healthy and heartbeating, so readiness alone does
  not expose the stuck session.
- Repeated attempts consume worker, bridge, network, and Temporal history
  resources until an operator or user cancels the run.

## Areas for a Later Fix

The fix is now planned in
[P114: Per-Call Tool Activities And Host-Side Search](../p114-per-call-tool-activities-and-host-search.md);
the list below is the incident-time assessment, kept for the record.

The eventual fix should address both halves of the failure rather than merely
raising the activity timeout:

1. Bound recursive filesystem tools by files visited, bytes read, depth,
   elapsed time, or a combination, and return a typed truncated/error outcome.
2. Prefer a provider-side grep/search capability for remote environments so a
   host can use an efficient native implementation and return only matches.
3. Give tool activities an explicit retry policy and a schedule-to-close or
   equivalent total deadline. A workload that deterministically exceeds its
   execution budget must eventually produce a tool failure, not retry forever.
4. Reconsider retrying a heterogeneous tool batch as one activity. Any retry
   design must account for partial completion and non-idempotent tool calls.
5. Add observability for activity attempt count, timeout cause, scan progress,
   and the active tool names so the UI or operator tooling can distinguish a
   slow scan from healthy agent progress.

Regression coverage should include a remote filesystem with enough entries to
exceed a small test budget and prove that the run reaches a bounded terminal
tool outcome without retrying indefinitely.

## Operational Guidance Until Fixed

- Avoid broad built-in `grep` calls over dependency caches, repository roots,
  or other large remote trees. Narrow the path and set `max_depth` where
  possible.
- When process execution is available, use a native search command such as
  `rg` in the environment for large trees.
- Interrupt a run that is repeatedly timing out in `tool_invoke_batch`; waiting
  longer will not help when each retry restarts the same over-budget scan.
