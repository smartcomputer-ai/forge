# P114: Per-Call Tool Activities And Host-Side Search

**Status**

- Proposed.
- Greenfield runtime change; compatibility with the batch activity shape is not
  required.
- Addresses the production failure recorded in
  [Remote Filesystem Grep Can Trap a Session in an Infinite Retry Loop](later/pNNN-remote-filesystem-grep-retry-loop.md).

## Problem

The hosted runtime executes a complete tool batch in one Temporal activity.
Calls run sequentially inside that activity, share one 360-second
start-to-close timeout, and inherit Temporal's unlimited retry policy. A slow
call therefore delays its siblings, a timeout discards completed sibling
results, and Temporal retries the whole batch. This is unsafe for mixed or
non-idempotent batches and can prevent a session from ever advancing.

Remote recursive filesystem tools amplify the problem. `grep` and `glob`
enumerate through the generic `FileSystem` interface, turning one logical
search into thousands of serialized host-protocol directory and file calls.

## Decision 1: One Activity Per Tool Call

Keep provider-observed batch identity and batch completion semantics, but
schedule each executable tool call as its own Temporal activity. Batch-level
validation runs before any call is scheduled. Each call result is accepted and
appended independently; the batch becomes complete only when every call is
terminal.

```text
tool batch
  -> validate batch constraints
  -> call A activity -> durable call result
  -> call B activity -> durable call result
  -> call C activity -> durable call result
  -> batch terminal when A, B, and C are terminal
```

Provider context materialization preserves original call order even when calls
finish in another order. Execution concurrency follows the admitted tool
parallelism policy. Batch-specific rules for `await`, environment selection,
Fleet, and workflow tools remain explicit preflight/orchestration rules; they
must not force ordinary filesystem and process calls back into one activity.

The activity input carries the stable session, run, turn, batch, and call
identity plus the bounded runtime facts needed for that call. This identity is
also the future idempotency key for side-effecting host operations.

## Decision 2: Tool-Specific Activity Policies

Replace the shared activity options with a small set of runtime-owned tool
execution classes. The policy is selected from the admitted logical binding,
not from model-controlled input. Initial classes are:

- **interactive filesystem** — approximately 90 seconds of operation time;
- **process** — derived from the validated process timeout plus bounded
  transport/completion grace;
- **remote interactive** — bounded network, MCP, and environment-control
  calls; and
- **long-running** — return a process handle, Promise, or job instead of
  occupying a long-lived activity.

Each policy defines an operation soft deadline, Temporal start-to-close,
schedule-to-close, retry safety, and parallelism. The invariant is:

```text
operation soft deadline < start-to-close <= schedule-to-close
```

An operation deadline or resource limit returns an ordinary terminal tool
failure and is never retried. A Temporal activity failure is converted at the
workflow boundary into a terminal call result rather than failing the session
workflow. Cancellation remains cancellation.

Read-only calls may receive a small bounded retry allowance for infrastructure
failure. Mutations and process starts use one attempt until the downstream
operation deduplicates the stable call identity. No tool activity has an
unlimited retry policy.

Long builds and tests remain supported. `run_process` may use an activity
deadline derived from its admitted timeout, but the preferred long-running
shape is to yield a durable process handle and poll or await it.

## Decision 3: Execute Expensive Search At The Host

Extend `host-protocol` with an optional filesystem text-search capability and
operation. `RemoteHostFileSystem` prefers it for environment grep; hosts
without the capability use the bounded generic fallback.

The host search request includes the root, regular expression, include filter,
case sensitivity, maximum depth, match limit, scan limits, and timeout. The
bridge performs the traversal and matching locally, using a safe native
implementation such as fixed-argv `rg`, and returns only bounded matches and
scan statistics.

The response reports whether and why it stopped, for example match, file,
byte, or time limit. Recursive glob/find is the next candidate for a host-side
operation. Ranged or capped host reads should follow so an oversized file is
not transferred in full before the worker rejects it.

## Narrow Resource Bounds

P114 does not introduce a universal execution-budget framework. Activity
deadlines bound runtime occupancy; expensive search operations additionally
receive only the limits needed to bound their amplification:

- entries or files visited;
- cumulative bytes searched;
- matches returned; and
- elapsed search time.

Deployment configuration owns the maxima. A caller may request a lower limit
but cannot raise it. The same bounds apply in the host implementation and the
generic fallback. Generalize these counters into a shared execution-budget
abstraction only after additional operations demonstrate the same need.

## Delivery Order

1. Add per-call activity requests/results and incremental engine completion.
2. Introduce tool execution classes and bounded Temporal policies.
3. Convert activity deadline/failure into terminal call results.
4. Add bounded host-side text search and the generic bounded fallback.
5. Add host-side recursive glob and capped/ranged reads where justified.
6. Add downstream idempotency for side-effecting calls before enabling their
   retries.

## Acceptance

- A slow filesystem call cannot restart completed sibling calls.
- Every tool call has a total deadline and bounded Temporal attempts.
- Filesystem and process calls may use different timeout classes.
- A deadline produces a visible terminal tool result, not a failed or
  indefinitely running session workflow.
- A remote broad search performs bounded work at the host when supported.
- A regression test with tiny time and scan limits reaches a terminal call and
  run outcome without repeating a non-idempotent sibling.

## Non-Goals

- A universal resource-accounting framework for all tools.
- Making every tool call parallel.
- Retrying side effects without downstream idempotency.
- Raising the existing batch timeout as a substitute for decomposition.
