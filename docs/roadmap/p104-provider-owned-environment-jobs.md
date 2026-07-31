# P104: Provider-Owned Environment Jobs

**Status**
- Completed 2026-07-29.
- Builds on P100b environment-job workflow adoption.

## Decision

Remove Lightspeed's PostgreSQL job registry. The provider owns job execution,
retained output, and close-versus-active-job behavior; Temporal owns workflow
orchestration, polling, Promises, and structured cancellation. A job handle is
only `(instance_id, job_id)`.

Environment close first marks the instance `closing`, preventing new starts,
and then asks the provider to close it. The provider may reject the close or
cancel/interrupt active jobs. A start that races after the local state check is
resolved by the provider. Do not replace the deleted tables with an occupancy
counter or another job registry.

Session agents receive the job system only when
`features.environments.jobs` is explicitly `true`; enabling environments alone
defaults jobs off. That grant installs the session workflow binding even before
an environment is attached. The model tools are exposed separately and only
while a ready attached environment advertises the matching job capability. An
already-admitted binding remains dormant if the grant is later disabled.

## Changes

- Delete `environment_jobs`, `environment_job_groups`, `JobHandleStore`, and
  their migrations and implementations.
- Delete the model-facing `job_list` tool and public
  `environments/jobs/list` method.
- Make job read and public cancellation provider-direct using the canonical
  handle. P100b cancellation continues to target the owning workflow.
- Build create responses from the workflow/provider result without rereading
  stored handles.
- Require explicit job ids for model-visible `job_start`, since generated ids
  would no longer be discoverable while a Promise is pending. Public creates
  may still derive ids because their synchronous acceptance response returns
  the resolved handles.
- Rely on deterministic Temporal execution identity plus provider request/job
  idempotency. Idempotency is bounded by Temporal/provider retention rather
  than persisted forever in PostgreSQL.
- Gate session workflow admission on the explicit jobs sub-grant and gate
  model-tool exposure on both that grant and live instance capabilities.
- P108 subsequently made credentials universe-environment-owned. Resolve and
  inject those bindings at provider start for both bare public creates and
  session-supervised starts; session identity is not credential scope.

## Done When

- [x] No PostgreSQL row is created for an environment job or job group.
- [x] Environment close is proven for provider reject and active-job interruption.
- [x] Start/close races and retry-idempotent starts have live coverage.
- [x] API contracts, generated clients, P86/P96/P102 docs, and tests reflect the
  provider-owned lifecycle.
