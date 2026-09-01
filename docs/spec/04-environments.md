# Environments

Lightspeed environments are universe-owned live resources backed by external
providers. Sessions neither own nor attach copies of them. A session records
only an optional active universe environment id.

Universe environments define ownership and selection, while explicit VFS and
environment domains define filesystem/tool routing. Lightspeed is greenfield,
so these replace the old
session-binding, environment-catalog, and generic default-target designs rather
than preserving compatibility with them.

## Ownership and extension boundary

The universe owns:

- stable environment identity and lifecycle metadata;
- provider observations, connection information, and capabilities; and
- credentials bound directly to an environment.

The environment provider API and `environment-protocol` are the external extension
seam. Provider implementations may run outside this repository. Lightspeed's
internal `EnvironmentResolver` only centralizes lookup, provider filtering,
liveness checks, and structured errors; it is not another plugin interface.

Provider status and capabilities are live observations. They remain in shared
storage so gateways and workers can coordinate across restarts, but they do not
enter deterministic session state.

## Session policy and state

`SessionConfig.features.environments` grants the capability and retains one
optional policy: a provider allowlist. When it is absent, every universe
provider is allowed. Environment-specific allowlists, tag rules, and capability
predicates are intentionally deferred.

Deterministic state contains only:

```text
EnvironmentState
  active_environment_id: Option<EnvironmentId>
```

`SetActiveEnvironment` and `ClearActiveEnvironment` produce dedicated
environment events. Clone/fork replay therefore carries the selection without
copying any live resource state.

An environment can be closed or disappear while a session still references
it. The reference remains in session state and resolves as unavailable; the
runtime never silently selects a replacement. Closing a session does not close
an environment unless a separate lifecycle ownership policy requires it.

One such policy exists: a profile's `environment: { type: "provision" }`
creates one environment for the session it starts (request id derived from
the session id), activates it while it is still provisioning, and — with the
default `retention: closeWithSession` — the lifecycle reconciler closes it once
that session is closed. The environment records `originSession` as
provenance; it remains an ordinary universe resource that other sessions may
select and the universe may close. Environment-dependent tool calls made
before the environment is ready do not fail: the worker reports the call as
not executed, the session workflow waits in a heartbeated
`await_environment_ready` activity, and re-dispatches the call.

Long-lived clients — a bot's main, routed, rotated, and chat sessions, or
several bots — share an environment by naming the same `existing` id in
their profiles. Nothing coordinates them: idleness is the daemon's fact, so
the reaper powers the environment down when none of its users touch it and
any user's next call wakes it; and use cancels a pending power-down — a
`ready` environment whose desired power was lowered by the reaper is written
back to `running` when a call resolves it, before the reconciler can freeze
it under that call. Such an environment is closed only by an operator or by
its own idle policy, never by a session or a bot.

## Power states and idle policy

A provisioned environment carries a Lightspeed-owned power intent,
`desiredPower ∈ {running, paused, suspended, stopped}` (default `running`),
next to its observed lifecycle `status`, which gains `paused` and `suspended`
beside `offline` (stopped). The lifecycle reconciler converges the provider
target toward the intent through one provider verb,
`controller/setTargetPower`; providers report the states each target supports
(`incarnation.powerStates`) and Lightspeed validates every request against
that list. Incus offers running/paused/stopped (freeze/unfreeze/stop/start);
snapshot-based providers may add `suspended`.

Waking is transparent: when a session selects or uses a paused, suspended, or
stopped provisioned environment whose provider supports power control, the
resolver records desired `running` and reports the environment as not ready,
so the same `await_environment_ready` re-dispatch path used for provisioning
applies. External environments have no power control.

Idle detection is a daemon fact. `lightspeed-envd` keeps a monotonic
activity clock and answers `env/idle` with the idle duration and the number
of running processes and jobs. The power reaper polls that report for
`ready` environments with an `idlePolicy` (`pauseAfterMs`, `suspendAfterMs`,
`stopAfterMs`, `closeAfterMs`; non-decreasing) and records the most escalated
due stage the provider supports, never while anything is executing. No
per-call activity is written to Lightspeed storage.

## Discovery and selection tools

The environment feature permits active-environment use and always installs
`environment_read`. Calling it without `environment_id` reads the session's
active environment; calling it with a known id reads that environment subject
to the provider filter. If no id is supplied and no environment is active, it
returns the ordinary structured tool error `no_active_environment`.

Setting `features.environments.selectionTools` to `true` additionally installs
three separate discovery/selection tools:

- `environment_list` lists allowed universe environments and their current
  observed status;
- `environment_activate` validates provider policy and live selectability,
  then selects the environment for the session; and
- `environment_deactivate` clears the selection without changing the universe
  resource.

List and read are live queries whose answers appear as ordinary tool results.
There is no environment catalog or active-environment CAS context entry.
Activation/deactivation use trusted tool effects so the deterministic engine
records the state change after successful tool completion.

`selectionTools` is default-off and gates list/activate/deactivate as one
surface. When it is absent or false, the model can still inspect a known or
active environment with `environment_read`; clients and profiles may activate
an environment through the session API, and filesystem/process tools use that
active environment. The independent `jobs` flag controls job tools. Future
environment provisioning tools require a separate grant rather than inheriting
selection authority. No environment details are injected into instructions or
model context because the model can query them on demand.

A selection tool cannot share a batch with another selection or with an
environment file/process/new-job tool. Those calls consume the active id
captured for the batch, so the runtime rejects the ambiguous combination and
lets the model continue in a later turn. VFS tools do not depend on environment
selection and may share the batch.

## Routing and replay safety

There are two disjoint filesystem domains:

- dedicated `vfs_*` tools operate only on VFS workspace links declared in
  session config; and
- ordinary file tools and process tools operate only on the active
  environment's real filesystem.

The runtime never mounts, overlays, or synchronizes one domain into the other.
The same path may contain unrelated bytes in each domain. VFS prompt,
instruction, and skill extraction remains based only on workspace links; the
catalog is explicitly `skills.catalog.vfs`.

Environment tools require an active environment and the corresponding live
capability. Missing, closed, stale, disallowed, unreachable, filesystem-less,
or read-only environments produce distinct tool failures. They do not prevent
VFS tools from operating, and environment-only batches do not resolve VFS
workspace links.

The deterministic engine has no generic execution-target model. It copies the
active environment id onto each tool-batch request; retries consume that bounded
id even if session selection changes later. Trusted runtime bindings identify
whether a call is `vfs.*` or `env.*`.

Job handles contain their originating universe environment id. Reads and other
handle-based operations resolve that environment rather than rerouting through
the currently active selection.

## Credentials

Credential bindings are keyed by `(universe_id, environment_id, env_name)`.
Their source may be an auth grant, auth-provider credential, or direct secret.
Every Lightspeed-started process or job targeting the environment receives the
same configured injection at execution time, including bare
`environments/jobs/create` calls and session tool calls. Session state and
activation events never carry credential ids or secret material.

Deleting an environment cascades to its credential bindings. Credential
operations live under universe `environments/credentials/*` APIs.

## Control-plane API

Universe resource operations are:

- `environments/create` (optionally with `idlePolicy`)
- `environments/read`
- `environments/list`
- `environments/close`
- `environments/power/put`
- `environments/idle-policy/put`

Session selection operations are:

- `session/environments/activate`, accepting a universe `environmentId`
- `session/environments/deactivate`

There are no session environment list/read/attach/detach APIs, session-local
aliases, attached/detached records, cwd overrides, or per-binding filesystem
routes.

## Current implementation

- PostgreSQL stores universe providers, environments, and environment
  credentials; there is no `session_environment_bindings` table.
- Deterministic core owns explicit active-environment state and events.
- Gateways and model tools share the internal live resolver and provider-filter
  rules.
- Hosted tool execution and bare environment-job starts resolve live
  connections and universe credentials at invocation time.
- The VFS catalog remains independent of environment projection and drives
  only VFS path framing plus prompt, instruction, and skill discovery.
- Provider-owned durable jobs remain outside a Lightspeed job registry.

## Deferred work

- finer-grained selection policy beyond provider filtering;
- automatic fallback or failover;
- environment-local prompt, instruction, or skill extraction (which should
  explicitly snapshot selected content into CAS if introduced); and
- additional provider capabilities such as browser or computer-use surfaces.
