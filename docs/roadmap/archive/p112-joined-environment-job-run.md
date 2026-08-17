# P112: Joined Environment Job Run

**Status**

- Implemented 2026-08-01. Offline workspace tests and the serial
  Temporal/host-bridge proof pass.
- Adopts Issue 2 from
  [Environment Execution Usability](later/pNNN-environment-job-model-usability.md).
- Builds on [P106 joined workflow tools](p106-joined-workflow-tools.md) and
  [P111 Promise-result materialization](p111-promise-result-materialization.md).
- Greenfield change. The existing asynchronous `job_start` surface is renamed
  to `job_submit`; no compatibility alias or dynamic completion-mode argument
  is added.

## Summary

Add `job_run`, a single-job environment tool backed by P106 `Start + Joined`
completion. It starts one durable provider job, parks the original tool call
durably, and completes that call with P111's normalized terminal job result.
It exposes no Promise and requires no model-authored `await`.

Use `job_submit` as the asynchronous group surface:

```text
job_run     one job, Joined, terminal result returned directly
job_submit  one or more jobs, Promises, caller controls waiting and ownership
```

Completion mode remains part of the immutable trusted binding. One tool does
not switch between Joined and Promises according to whether its arguments
contain one job or an array.

## Motivation

`job_submit` is the right primitive for dependency graphs, parallel jobs,
selective waiting, cancellation, and detach. It is unnecessary ceremony when
the model needs one command's result before it can continue:

```text
job_submit -> Promise acknowledgement -> model round -> await -> result
```

`job_run` makes that common path look like an ordinary tool while retaining
Temporal durability:

```text
job_run -> child workflow -> provider job -> terminal result -> original call
```

The session workflow does not block on a host RPC or long-running activity.
P106's runtime-owned `reply` Promise parks and resumes the batch across worker
restart, replay, continue-as-new, deadline, and cancellation.

## Tool Contracts

### `job_submit`

The existing asynchronous group shape remains:

```json
{
  "jobs": [
    {
      "job_id": "build",
      "name": "Build project",
      "argv": ["cargo", "build"],
      "cwd": "/workspace",
      "env": { "CARGO_TERM_COLOR": "always" },
      "stdin": null,
      "timeout_ms": 1800000,
      "depends_on": [],
      "dependency_policy": "allSucceeded",
      "queue_key": "project-build"
    }
  ]
}
```

`jobs`, each `job_id`, and each `argv` are required. The binding uses
`Start + Promises` with one `job-{index}` completion key per array element.

### `job_run`

The joined single-job shape is flat and process-like:

```json
{
  "name": "Run tests",
  "argv": ["cargo", "test"],
  "cwd": "/workspace",
  "env": { "CARGO_TERM_COLOR": "always" },
  "stdin": null,
  "timeout_ms": 1800000,
  "queue_key": "project-tests"
}
```

Only `argv` is required. The runtime derives a deterministic provider job id
from the workflow-tool invocation. `job_run` does not accept `jobs`,
`job_id`, `depends_on`, or `dependency_policy`; callers needing a dependency
group use `job_submit`.

Several independent joined jobs may be emitted as several `job_run` calls in
one model tool batch. P106 starts them independently, parks the batch until all
are terminal, and maps each result to its own original call id.

## Binding And Workflow Contract

The runtime admits two reserved system bindings when
`features.environments.jobs` is granted:

| Tool | Target | Completion | Completion keys |
| --- | --- | --- | --- |
| `job_submit` | `Start(EnvironmentJobWorkflow)` | `Promises` | `job-{index}` |
| `job_run` | `Start(EnvironmentJobWorkflow)` | `Joined` | `reply` |

Both bindings pin the active environment id and allowed-provider policy in
the invocation's runtime-owned execution context. The receiving preparation
activity validates the tool id, semantic type, argument shape, and exact
completion-key shape before constructing host parameters.

For `job_run`, preparation:

1. decodes the flat single-job arguments;
2. derives one stable job id from the invocation id;
3. constructs one `StartJobsParams` entry;
4. maps the reserved `reply` Promise to that job id; and
5. starts the existing `EnvironmentJobWorkflow` under P106's deterministic
   execution id.

The core environment-job workflow and fixed source-resolution signal remain
unchanged. A terminal provider result resolves or fails the one subscription,
and P106 maps it back to the parked original call.

## Result And Failure Semantics

Successful `job_run` returns the same `ModelJobResult` root used by P111 for
an asynchronous environment-job Promise:

```json
{
  "handle": {
    "environment_id": "env_123",
    "job_id": "job-..."
  },
  "summary": {
    "jobId": "job-...",
    "status": "succeeded",
    "exitCode": 0
  },
  "output": [
    { "stream": "stdout", "text": "tests passed\n" }
  ],
  "outputNextSeq": 4,
  "truncated": false,
  "artifacts": []
}
```

The handle is mandatory for workflow-produced job results so a truncated
result can be continued with `job_read`. Terminal polling requests artifact
metadata and retains binary output as typed CAS references, never inline
Base64.

The existing environment policy remains:

- `Succeeded` resolves the Promise and succeeds `job_run`;
- `Failed`, `Cancelled`, `TimedOut`, `DependencyFailed`, `Interrupted`, and
  `Lost` fail the Promise and therefore fail `job_run`; and
- a non-success error root is a structured `ModelJobResult`, preserving the
  handle, terminal summary, readable output, cursor, truncation, and artifact
  metadata rather than reducing the result to a string.

Explicit `await` continues to report a failed asynchronous job as one failed
Promise entry. Joined completion maps the same structured error root to an
ordinary failed tool result.

## Deadline And Cancellation

P106 requires every Joined binding to declare a non-zero trusted hard
deadline. P112 defines a fixed `job_run` total-operation ceiling. The
provider-side `timeout_ms` remains a job execution timeout; the joined
deadline includes workflow dispatch, provider admission, queueing, execution,
polling, and result delivery.

The initial constants are:

```text
job_run default provider timeout = 30 minutes
job_run maximum provider timeout = 60 minutes
job_run Joined hard deadline     = 65 minutes
```

The input schema rejects a provider timeout above the maximum. An omitted
timeout receives the default during preparation. Callers needing longer,
indefinite, queued, or detachable work use asynchronous `job_submit`.

The internal reply Promise remains run-scoped and runtime-owned:

- model-facing `await`, `cancel`, and `detach` cannot address it;
- run or session cancellation cancels the Promise and the start-on-call child
  workflow;
- child workflow cancellation performs best-effort cancellation of the
  provider job; and
- hard-deadline expiry fails the joined call and causes the terminal child
  execution to be cancelled through the existing P106 cleanup path.

`job_run` has no detach mode.

## API And Durable State

P112 adds no public JSON-RPC environment-job method and no durable engine
event. `job_run` is a model tool derived from the existing environment jobs
feature grant.

The new tool has its own reserved tool id, semantic type, schema, and binding
fingerprint. Both system bindings are add-only session metadata and are hidden
from managed-session declaration readback just like the existing core
`job_submit` binding.

Previously admitted greenfield sessions are not retrofitted in place. Normal
toolset configuration admits any missing system binding before exposing the
effective toolset.

## Non-Goals

P112 does not:

- dynamically choose completion mode from model arguments;
- add a Joined job-group aggregate;
- add compatibility aliases for the former `job_start` name;
- change the raw environment protocol or control-plane job APIs;
- add streaming job output or progress events;
- make binary artifacts model context automatically;
- add environment-level non-secret variables; or
- change the generic P106 or Promise state machines.

## Implementation Plan

1. Add the `job_run` constants, arguments, schema, and canonical tool spec.
2. Admit and materialize both reserved environment-job system bindings.
3. Pin the active environment for both tool invocations.
4. Teach environment-job preparation the exact Promises and Joined shapes.
5. Include handles and artifact metadata in workflow-produced results and
   store structured terminal failures.
6. Add unit coverage for schema, source resolution, and generic Joined
   completion/cancellation behavior.
7. Extend the serial Temporal/host-bridge environment-jobs proof with direct
   readable Joined completion and the absence of a model-visible
   Promise/`await`.

## Done When

- a jobs-enabled session exposes both `job_submit` and `job_run`;
- `job_run` starts exactly one durable job and directly completes its original
  tool call with a normalized terminal result;
- the result contains a usable handle and no Base64 transport chunks;
- no model-owned Promise or explicit `await` is created for `job_run`;
- async `job_submit` retains its job-group and keyed-Promise behavior;
- terminal failures are structured and consistent across async and Joined;
- cancellation reaches the provider job through the existing workflow path;
  and
- targeted unit suites plus the serial live proof pass.
