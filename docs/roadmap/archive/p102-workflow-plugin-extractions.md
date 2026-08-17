# P102: Workflow Plugin Extractions — Shrinking The Stable Lightspeed Worker

**Status**
- Completed 2026-07-29. Environment-job dispatch adopted the generic P100b
  machinery while its domain stayed core-owned. Channel transport and
  delivery moved to the external Channels application, so Lightspeed's
  built-in messaging feature, tool family, outbox storage, and delivery APIs
  were deleted instead of migrated behind another internal workflow.
- Originally deferred 2026-07-25.
- Depends on the P100b workflow-backed-tool primitives being designed,
  implemented, and proven with minimal generic workflows first.
- Updated 2026-07-25 after the P100/P100b design review: P100b's completion
  contract is now a **keyed promise set** with a unified
  `PromiseSource::Workflow { producer, invocation_id, key }`. Environment
  jobs stressed the earlier one-invocation/one-promise draft and proved the
  generic shape even though the environment subsystem is no longer a plugin
  extraction candidate. Vocabulary follows the P100 rename: "workflow tool",
  not "port".
- This document is a migration boundary and candidate inventory, not current
  implementation authorization.
- Environment-job compatibility audit completed 2026-07-28. The P100b
  preparation addendum added array-index completion keys and fixed the
  asynchronous acceptance/cancellation contract. The subsequent boundary
  review found that jobs are inseparable from the core environment registry,
  bindings, credentials, environment protocol, public API, and storage without
  inventing a much broader plugin framework. Environment jobs are therefore
  removed from the plugin-extraction candidate list. The model-visible
  `job_submit` path now uses P100b internally while the environment/job domain
  remains a core subsystem.

## Goal

Use proven workflow-plugin primitives to remove feature-specific tool routing,
Promise sources, workflow registrations, activities, and domain dependencies
from the stable Lightspeed session core and worker.

The desired direction is:

```text
stable Lightspeed session core/worker
  tool declarations
  generic workflow bindings
  emissions
  Promises
  generic workflow start/reply/cancel

plugin workers
  domain workflow state machines
  domain activities
  provider/store integration
  domain validation and result interpretation
```

P102 judges success by code and dependency deletion from the stable worker, not
by routing an existing feature through an extra workflow while retaining all
of its old special cases.

## Why This Is Separate From P100b

P100b answers generic questions:

- how a tool targets a bound workflow execution;
- how a tool starts a new workflow execution;
- how accepted and Promise completion differ;
- how push, reply authorization, cancellation, retry, and deduplication work;
- where plugin code executes without a second protocol.

P102 answers domain migration questions:

- whether an existing feature actually fits those primitives;
- how its current behavior and public API are preserved;
- which core/worker branches and dependencies can be deleted;
- whether the added workflow hop is operationally justified.

Combining them would let current implementation accidents distort the
primitive design and would make it unclear whether a failing migration exposed
a bad primitive or merely an incomplete compatibility adapter.

## Migration Law

Every candidate migration must satisfy all of the following:

1. **Behavioral equivalence first.** Existing durable state ownership, return
   values, retries, cancellation, security, and public APIs remain unchanged
   unless a separate product decision explicitly changes them.
2. **One authority per fact.** A plugin workflow must not duplicate business
   state already owned by a durable store or provider.
3. **Actual deletion.** The migration removes plugin-specific branches and
   types from the session core/worker; adding a workflow in front of the old
   path is not completion.
4. **No new execution protocol.** Plugin workflows and their private
   activities run in plugin Temporal workers using P100/P100b contracts.
5. **No compiled plugin registry.** Adding the next plugin does not extend a
   feature-kind `match` in the session worker.
6. **Trusted immutable binding.** The model and ordinary public config cannot
   select workflow ids, workflow types, task queues, or completion mode.
7. **Operational proof.** Retry, duplicate delivery, worker absence,
   continue-as-new, cancellation, and upgrade behavior are tested live.

## Core Internal Consumer: Environment Jobs

P86 is a strong internal start-on-call consumer because it already has a
durable `EnvironmentJobWorkflow`:

- the session worker resolves environment state and starts the workflow;
- the workflow owns provider start, poll, cancel, and terminal emission;
- before the internal adoption, the engine knew `PromiseSource::EnvJob`;
- before the internal adoption, the session workflow and worker contained environment-job-specific
  subscribe/check/cancel branches.

The environment subsystem, including jobs, remains core. Job execution is
deeply coupled to environment instances, session bindings, credentials,
provider presence, the environment protocol, public `environments/jobs/*` methods,
and the core PostgreSQL environment schema. Moving only the workflow to a
plugin leaves those dependencies behind; moving all of them requires plugins
to contribute API namespaces, schemas, credentials, resources, and lifecycle
hooks, far beyond P100b's workflow-tool boundary.

The model-visible `job_submit` path was moved onto generic P100b start-on-call
and keyed Promise machinery on 2026-07-28. This is a core refactor, not a
plugin extraction and not a separate ownership boundary. The job workflow's
short, idempotent environment-adapter calls are local activities on the
co-located core worker, so this design does not introduce a second worker or
task-queue ownership boundary.

Compatibility gates:

- multiple jobs per start call;
- one Promise per job;
- multiple completion keys targeting one job-group workflow;
- dependency DAGs and queue keys;
- asynchronous generic start acknowledgement, with stable handles and final
  results returned in per-job Promise payloads and available through job
  list/read after provider acceptance (an explicit product change from the
  former synchronous tool result);
- provider-owned status, output, and artifacts;
- environment binding and credential enforcement;
- public `environments/jobs/*` create/list/read/cancel behavior.

An internal adoption must not collapse per-job Promises into one group
Promise. P100b's keyed promise set exists to make that unnecessary: one `job_submit`
invocation derives one completion key per validated job, the group workflow
resolves each keyed Promise as its job completes, and every Promise remains
individually awaitable, cancellable, and detachable under P92. If full
`job_submit` adoption cannot preserve the surface cleanly, Promise-source
generalization and tool-dispatch cleanup become separate reviewed slices.

The internal `job_submit` binding uses
`ArrayIndices { pointer: "/jobs", prefix: "job-" }`. The job workflow maps those
stable invocation-local keys to provider job ids after resolving the selected
environment. Per-key cancellation uses the same map. The immediate tool
result remains P100b's generic `{accepted, invocationId, executionId,
promises}` acknowledgement; no synchronous callback or second
execution protocol is added.

Bare API starts and session-supervised starts use separate core workflow entry
types sharing one internal job-group state machine and activities. Each group
still has exactly one workflow owner; P104 removed the stored controller index
and made public cancellation provider-direct.

Internal-adoption deletion target:

- `PromiseSource::EnvJob` and its effect vocabulary;
- environment-job-specific Promise routing in `AgentSessionWorkflow`;
- environment-job-specific check/subscribe/cancel dispatch in the generic
  session worker;
- the `is_environment_job_tool_name` routing branch and direct environment-job
  workflow start code in the generic session tool executor.

This deletion target was completed 2026-07-28. P104 subsequently deleted
`job_list`; `job_read` remains an ordinary provider-direct query tool and only
`job_submit` uses the workflow binding.

Follow-up correction: the core `job_submit` binding is not part of managed
session creation. Ordinary sessions open without workflow-tool state. When a
ready attached environment specifically supports `job_submit`, the gateway
idempotently admits one add-only system workflow binding and then materializes
the tool. Detaching the environment removes the model-visible tool but keeps
the durable binding for in-flight invocations. System bindings neither assign
lifecycle ownership nor participate in the managed-session creation
fingerprint, and config-only clones omit them.

The following explicitly remain core-owned:

- environment providers, instances, bindings, and credentials;
- the fs/process/job environment protocol and client;
- `EnvironmentJobWorkflow` and its activities; and
- public `environments/jobs/create|read|cancel` API methods.

A dedicated task queue or worker process may isolate job load operationally,
but that does not make the environment subsystem a plugin.

## Completed Externalization: Channels

The post-P100 product boundary no longer treats channel delivery as a
Lightspeed feature to migrate. The external Channels application owns channel
accounts, provider clients, routing, delivery state, idempotency, rate limits,
and media behavior. It uses the generic managed-session and promise-bearing
workflow-tool contracts proven by P100/P100b.

Completed deletion target:

- `FeaturesConfig::messaging` and its API/engine projection;
- built-in `message_send`, `message_edit`, `message_react`, and `message_noop`
  materialization;
- messaging-specific dispatch branches and executor ownership in
  `SessionTools`;
- the `messaging` crate and PostgreSQL outbox implementation/migration;
- universe `outbox/read|ack` and deployment `operator/outbox/read` APIs;
- generated OpenRPC, JSON Schema, TypeScript-client, and Configurator MCP
  surfaces for those types and methods.

The generic Channels-shaped P100b live proof remains in Lightspeed because it
tests the workflow-plugin substrate, not a built-in messaging subsystem.

## Other Candidates

No remaining production feature is presumed to be a plugin migration.

- Fleet remains admission-based session-to-session control unless a separate
  design proves otherwise.
- Timers remain session-owned Promise sources unless extraction removes real
  core complexity without weakening deterministic deadlines.
- MCP is already a provider-facing remote-tool mechanism and must not be
  conflated with workflow plugins.
- P101 Work is the lifecycle controller and first P100 pull consumer, not a
  migration target.
- Environment jobs remain a core subsystem; internal P100b adoption is not a
  plugin migration.

Future candidates are added only with a named deletion target and compatibility
contract.

## Sequencing

P102 does not begin migration work until P100b proves:

1. bound push with retry and invocation-id idempotency;
2. bound promise-bearing completion (keyed promise sets) with
   producer-authorized Promise resolution;
3. deterministic start-on-call workflow execution resolving keyed promises;
4. generic workflow cancellation, deadline, and recovery behavior;
5. plugin code running outside the session worker; and
6. no regression to existing production systems.

After that proof:

1. audit current feature-specific coupling and record the exact deletion set;
2. choose one candidate;
3. write its behavior-equivalence test before changing routing;
4. migrate one vertical slice;
5. delete the replaced path in the same change;
6. run live retry/restart/continue-as-new coverage;
7. only then choose the next candidate.

There is no current production extraction candidate. New capabilities should
be designed against P100b from inception when they need an external workflow
boundary.

## Non-Goals

P102 does not:

- redesign P100b primitives to fit one migration shortcut;
- change existing product behavior without a separate decision;
- create a generic HTTP, activity, dynamic-library, or WASM executor;
- move provider or store state into the session log;
- require every bounded tool to become a workflow;
- merge plugin discovery/deployment into the session worker;
- migrate environments, jobs, Fleet, timers, MCP, or P101 by default; or
- count an adapter-only indirection as core simplification.

## Acceptance Boundary

This deferred roadmap becomes an implementation plan only after P100b is
accepted and one candidate has:

- an exact before/after dependency and branch inventory;
- a behavior-equivalence suite;
- a named plugin worker topology;
- a rollback/upgrade plan for live workflow executions; and
- a deletion list demonstrating that the stable core/worker becomes smaller.
