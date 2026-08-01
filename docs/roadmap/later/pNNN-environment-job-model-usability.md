# Environment Execution Usability: Readable Job Output, Joined Starts, And Variables

**Status**

- Later / agent-usability follow-up.
- The readable-job-output work in Issue 1 was completed by
  [P111](../p111-promise-result-materialization.md), together with generic
  Promise-result materialization through `await`. This document retains Issue
  2 (a Joined job-start surface) and Issue 3 (non-secret environment-level
  variables).
- Written 2026-07-31 after inspecting
  `session_91992f6a60524ee9ace5c50369326af8`, where a successful environment job
  returned Base64-encoded output chunks to the model through an awaited
  Promise.
- Contains three related but independently shippable issues: model-readable
  job output, joined job starts, and non-secret environment-level variables.
- Builds on [P106 joined workflow tools](../p106-joined-workflow-tools.md). P106
  implements durable `Joined` completion for workflow-backed tools, but
  deliberately left environment tools out of its initial adoption scope.

## Goal

Make the common environment-job path feel like an ordinary model tool:

1. When a job produces text, the model receives readable text rather than a
   transport encoding.
2. When the model starts work whose result it immediately needs, it can use a
   joined form that completes the original tool call without a separate
   model-authored `await` call.
3. An environment may optionally provide ordinary, non-secret variables as
   defaults for the processes and jobs executed in it.
4. The existing asynchronous Promise form remains available when the model
   genuinely wants concurrency, selective waiting, cancellation, or detach.

These are model-interface changes. They must not weaken the byte-safe host
protocol or replace durable workflow suspension with a blocking RPC.

## Issue 1: Transport Chunks Leak Into Model Context (Adopted By P111)

### Observed behavior

The inspected session started `demo_job_001`, awaited its terminal Promise,
and received a payload shaped like:

```json
{
  "summary": {
    "jobId": "demo_job_001",
    "status": "succeeded",
    "exitCode": 0
  },
  "outputChunks": [
    {
      "seq": 0,
      "stream": "stdout",
      "chunk": "Sm9iIHN0YXJ0ZWQK"
    }
  ],
  "outputNextSeq": 5
}
```

The `chunk` field is Base64. Decoding all five chunks in sequence produced
ordinary terminal text beginning with `Job started` and ending with
`Job finished`.

Base64 is correct in `host-protocol`: process output is arbitrary bytes, JSON
cannot carry arbitrary bytes directly, and a host chunk may contain binary
data, control bytes, or only part of a UTF-8 code point. The problem is that
this lossless transport representation crosses the model-facing boundary.

Today there are two paths to consider:

- `job_read` serializes `JobReadResultSet`, including raw `JobOutputChunk`
  values, even though `crates/tools/src/environment/jobs.rs` already has a
  separate visible-text formatter.
- The environment-job polling activity serializes the raw host
  `JobReadResult` into the terminal Promise payload. Consequently, `await`
  exposes the same Base64 chunks to the model.

### Why this matters

- The model cannot directly read or reason about the output it requested.
- Every agent must notice the encoding and perform an avoidable decode step.
- Base64 expands the bytes by roughly one third, then adds chunk-level JSON
  overhead, consuming context and tool-result tokens.
- Arbitrary transport chunk boundaries distract from the semantic output.
- Session transcripts and live debugging become difficult for humans to
  follow.
- Tool descriptions may claim that a job result is available while the useful
  part remains hidden behind an undocumented transport detail.

The existing visible formatter does not fully solve the problem if the
structured content and Promise payload still contain raw chunks. The actual
model-visible result should use the readable representation consistently.

## Desired Model-Facing Job Result

Keep the host protocol unchanged and normalize output at the Lightspeed tool
or workflow-result boundary. A typical result should look conceptually like:

```json
{
  "summary": {
    "jobId": "demo_job_001",
    "name": "demo environment job",
    "status": "succeeded",
    "exitCode": 0
  },
  "output": [
    {
      "stream": "stdout",
      "text": "Job started\n...\nJob finished\n"
    }
  ],
  "outputNextSeq": 5
}
```

Required behavior:

1. Decode valid UTF-8 before the result enters model context.
2. Preserve observed `stdout`/`stderr` order, while merging adjacent chunks
   from the same stream so transport fragmentation is not exposed.
3. Preserve `outputNextSeq` for incremental `job_read` calls. Its model-facing
   description should clearly say to use the returned value as the next
   `after_seq` cursor.
4. Preserve summaries, failures, timestamps, artifacts, truncation state, and
   other semantically useful job data.
5. Make truncation explicit. A byte limit must not silently look like complete
   output.
6. Handle non-UTF-8 data deliberately. A rare binary segment may use an
   explicitly named Base64 fallback such as `dataBase64`, but valid text must
   never be returned that way. Large binary results should normally become
   artifacts rather than model context.
7. Apply the same normalization to direct `job_read` results and terminal
   results delivered through Promises or joined completion.

The normalizer should work from bytes, not by independently decoding every
transport chunk. A UTF-8 code point may span chunks. Stream identity and
sequence order must remain available while constructing readable segments.

The control-plane and host APIs may continue exposing the lossless wire form
for programmatic clients and debugging. The important rule is:

```text
host protocol       -> lossless byte chunks
model-facing tools  -> compact readable text segments
raw diagnostics     -> byte chunks on demand
```

## Issue 2: Starting Joined Jobs

### Current behavior

The built-in hosted `job_start` binding uses `Start + Promises`, with one keyed
Promise per submitted job. This is the right form when the model wants to:

- start several jobs and overlap them with other work;
- await only a subset;
- keep durable handles for later reads;
- cancel individual jobs; or
- deliberately detach from completion.

It is ceremony in the common case where the model starts a job and immediately
needs its result. That path currently requires:

```text
job_start
  -> Promise acknowledgement
  -> another model step
  -> await
  -> terminal job result
  -> model continues
```

The extra step adds a model round, another tool call, more transcript content,
and another opportunity to forget the await or mishandle its payload. There is
no useful scheduling decision between start and await when the result is
needed unconditionally.

### Desired behavior

Consider adding a joined environment-job start form using P106's existing
durable `Joined` completion:

```text
joined job start
  -> durable job workflow starts
  -> original tool call remains durably parked
  -> job reaches a terminal state
  -> original tool call completes with the normalized job result
  -> model continues
```

This must be an engine-native durable join, not a long-running
`tool_invoke_batch` activity or a synchronous wait on the host bridge. Worker
restart, Temporal replay, continue-as-new, deadline handling, and cancellation
must use the same machinery already established by P106.

The current Promise form must remain available. Joined completion is an
ergonomic option for work the caller needs immediately, not a replacement for
asynchronous jobs.

### Tool-surface question

P106 makes completion mode part of the immutable trusted binding rather than a
model-selected argument. Reusing that rule suggests two explicit tool
surfaces, for example:

- `job_start`: asynchronous, returns keyed Promises as today;
- `job_run` or `job_start_joined`: joined, returns terminal results directly.

This is preferable to a `join: true` argument if that argument would make one
binding dynamically switch its durable completion semantics. The final name
is a product/tool-design choice, but the distinction should be obvious in the
tool descriptions.

The first joined form could accept exactly one job, matching P106's
single-semantic-reply model. Alternatively, it could accept a job group and
produce one aggregate reply after all jobs are terminal. That choice needs an
explicit result and cancellation contract; it should not fall out accidentally
from the current per-array-index Promise implementation.

Other design questions:

- What hard joined deadline should the trusted binding declare, and how does
  it relate to each job's `timeout_ms`?
- Does cancellation of the calling run cancel the joined job or whole joined
  group? The default should preserve structured cancellation unless the tool
  explicitly offers detach semantics.
- Should a failed provider job complete the tool call as a structured job
  result or as a failed tool call? The async and joined forms should agree on
  the facts exposed to the model.
- How should artifacts be returned when a joined result is too large for
  model context?

## Issue 3: Non-Secret Environment-Level Variables

### Motivation

An environment may need stable execution configuration that is not secret and
should not be repeated in every process or job request. Examples include:

- `CI=true`;
- a project or toolchain selector;
- ordinary feature flags;
- default logging configuration; or
- a service base URL that is safe to inspect.

Today environment credential bindings already map an environment variable name
to an auth grant, auth-provider credential, or direct secret. Using that
mechanism for ordinary values would be misleading: it would treat harmless
configuration as secret material, hide values that should be inspectable, and
couple simple execution defaults to the credential broker and secret store.

We should consider allowing a universe environment to own a separate map of
plain environment variables. These values would be stored and exposed as
ordinary environment configuration and automatically applied when Lightspeed
starts a process or durable job in that environment.

### Required separation from credentials

Plain variables and credential bindings may both ultimately become child
process environment variables, but they have different contracts:

| Property | Environment variable | Credential binding |
| --- | --- | --- |
| Value sensitivity | explicitly non-secret | secret or minted credential |
| Storage | ordinary environment configuration | secret/auth stores plus binding metadata |
| Readback | value may be shown to authorized clients and models | secret value is never returned |
| Resolution | available directly | resolved or minted at execution time |
| Output redaction | no secret guarantee | injected values participate in redaction |
| Typical use | flags, paths, safe URLs, tool defaults | tokens, passwords, API keys |

The product and API should not imply that a plain variable is protected. The
editor should say that its value is readable and direct users to a credential
binding for sensitive material.

### Precedence and collision rules

A reasonable initial precedence model is:

```text
host/provider process environment
  < environment-level plain variables
  < explicit per-process or per-job plain variables
```

Explicit call arguments can therefore override an environment default without
mutating the environment. Secret bindings remain outside this override chain.
A plain variable—whether environment-level or explicit—must not share a name
with an environment credential binding. That collision should fail clearly,
matching the current rule that explicit job/process variables cannot override
secret injection.

The control plane should ideally prevent an environment from simultaneously
storing a plain variable and credential binding with the same name, rather than
discovering the conflict only when a process starts.

### Scope and lifecycle questions

- Are variables mutable fields on the universe environment, or a separate
  put/list/delete collection keyed by `(environment_id, env_name)`?
- Should updates use whole-document replacement with an expected revision, or
  per-name operations like credential bindings?
- Are values exposed through `environment_read`, or only through a dedicated
  control-plane read to avoid injecting a potentially large map into model
  context?
- Does an execution use the variables current at dispatch time, or a snapshot
  pinned when the durable job is accepted? Durable job retry and idempotency
  need an explicit answer.
- Should provider-advertised variables and universe-configured variables be
  separate layers if providers later expose defaults of their own?
- Do variables apply only to process/job capabilities, or also to future
  environment-backed browser and computer-use capabilities where relevant?

Environment variables should remain universe-owned configuration. They should
not enter deterministic session state or activation events, just as live
credential resolution does not.

## Implementation Direction

P111 owns the shared job-result normalizer, the text-first output-segment
shape, binary CAS references, direct `job_read` normalization, and normalized
terminal Promise payloads. The remaining implementation direction here is:

1. Decide the joined tool name and whether its unit is one job or one job
   group.
2. Declare the joined tool through the existing P106 `Start + Joined`
   machinery and extend `EnvironmentJobWorkflow` to produce its one completion
   reply.
3. Reuse existing environment-job cancellation handling for joined-call
   cancellation.

If environment-level variables are adopted:

1. Add a distinct non-secret environment-variable record/API rather than a new
   credential source variant.
2. Validate names with the same environment-variable-name rules used by job
   and process requests.
3. Reject name collisions with credential bindings at control-plane writes and
   again at execution as defense in depth.
4. Merge environment defaults into process and job requests before secret
   resolution, using one shared precedence implementation.
5. Define whether durable jobs pin the merged plain environment at acceptance
   or resolve current environment defaults at dispatch.

## Regression Coverage

Readable output:

- stdout-only UTF-8 is returned as ordinary text with no Base64 fields;
- interleaved stdout/stderr preserves stream identity and observed order;
- a UTF-8 code point split across transport chunks decodes correctly;
- non-UTF-8 output uses the explicit binary fallback;
- truncated output is marked and `outputNextSeq` resumes without duplication;
- direct `job_read` and awaited terminal results expose the same normalized
  representation.

Joined starts:

- the model-visible call completes with the job's readable terminal result and
  creates no model-addressable Promise or synthetic `await` call;
- the session can park across worker restart and continue-as-new, then resume
  exactly once;
- cancellation of the calling run reaches the correct provider job scope;
- timeout and provider failure produce deterministic terminal behavior;
- retry/recovery does not start the provider job twice;
- ordinary asynchronous `job_start` still returns keyed Promises and supports
  selective await/cancel unchanged.

Environment-level variables:

- configured plain variables appear in both process and durable-job execution;
- explicit plain variables override environment defaults;
- plain/credential name collisions fail without exposing secret values;
- reads clearly distinguish inspectable variables from credential-binding
  metadata;
- environment variable changes follow the chosen durable-job snapshot rule;
- deleting an environment removes its configured variables along with its
  credential bindings.

## Done When

- A text-producing environment job can be started, read, and awaited without
  exposing Base64 to the model.
- Raw chunks remain available where byte fidelity or protocol diagnostics are
  required.
- Lightspeed has an explicit decision and tested path for joined job starts,
  built on P106 rather than on blocking host or workflow calls.
- Agents use explicit Promises only when they need asynchronous control, not
  as mandatory ceremony for every environment job.
- Lightspeed has an explicit decision on environment-level non-secret
  variables, including visibility, precedence, collision, and durable-job
  snapshot semantics, without weakening credential-binding secrecy.
