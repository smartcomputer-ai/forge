# Lightspeed Roadmap

## Work
- [ ] [P156](p156-cas-blob-garbage-collection.md) — CAS blob garbage
  collection (proposed 2026-09-03): session deletion leaves every blob
  behind and the roots table has never been written. Store-derived roots at
  event append, `touched_at_ms` on every put with a grace period instead of
  put coordination, edges for nested formats, a bounded worker-role sweeper
  with a dry-run CLI pass, and removal of the write-only raw provider
  request/response dumps that are more than half of the bytes written today.
- [x] [P155](p155-models-list-cache.md) — `models/list` discovery cache
  (implemented 2026-09-03): a process-local, per-universe and per-provider
  single-flight cache with a 10-second success TTL, 2-second failure TTL, and
  generation-safe invalidation when credentials or endpoint configuration are
  replaced.
- [x] [P154](p154-session-retention.md) — session-tree retention: an opt-in
  `deleteAfterCloseMs` on the retention root, inherited ownership for history
  forks and delegated children but not config-only clones, effective retention
  views, explicit manual cascade, and an always-cascading worker-role reaper.
  Unset means keep until manual deletion; descendants never have competing
  policies. Also settles the clock vocabulary and fixes and renames the bot
  routed-session idle timer.
- [x] [P153](p153-session-metadata.md) — session metadata (implemented
  2026-09-03): the bounded
  string map registered environments already carry, set at `session/start`
  and replaced with `session/metadata/put`, a containment filter on
  `session/list` and `environments/list`, chips and a filter bar with a
  selection model in the Platform sessions page, and the Harbor adapter and
  bots stamping the same keys on sessions and environments. No patch, no
  bulk endpoints, no usage roll-up. Motivated by evaluation jobs that create
  hundreds of sessions per universe.
- [ ] [P152](p152-envd-release-and-distribution.md) — envd release and
  distribution: install the rustls provider at startup (the workspace
  release binary panics on its first TLS connection), publish static musl
  builds so any sandbox image can run it, a discovery document beside each
  bundle and at `/.well-known/lightspeed-envd` on the gateway, and
  `initialize` reporting the build's git sha and matching envd. Manual and
  opt-in automatic self-upgrade are implemented; deployment-side serving and
  its protocol-change notice remain open in ls.bot.
- [ ] [P151](p151-exec-leftover-processes.md) — leftover processes survive a
  normal `exec_command` exit (timeouts, termination, and durable jobs keep
  sweeping), leftover output is drained instead of abandoned, the tool
  reports what is still running, and the handle becomes usable: a
  two-operation substrate (`run_process`, `continue_process` with optional
  input or signal) with a daemon-owned read cursor, a window wait, and a
  capped retained buffer, presented on three surfaces: Canonical as
  Lightspeed's neutral shape, Codex-like (`exec_command`/`write_stdin`, the
  OpenAI default, copied from Codex `728cb12fe5`), and Claude-Code-like
  (`Bash`/`BashOutput`/`KillShell`, the Anthropic default, which today has
  no handle path). Greenfield, no compatibility shims. The idle report
  counts leftover groups without treating them as busy, and a server-only
  slice exposes the engine's failure kind on `runFailed` plus output size
  and truncation on `toolCallCompleted`. Motivated by seven Terminal-Bench
  tasks in the first P149 run.
- [ ] [P150](p150-scalable-mcp-discovery-and-tool-search.md) — scalable MCP
  discovery and tool search: exempt management discovery from the gateway
  response budget instead of truncating descriptions, expand truncated
  transcript entries from CAS in the UI, and reshape `mcp_find_tools` into one
  8 KiB-windowed hit shape with 64 KiB byte-paged results and a `names` mode
  for full definitions.
- [ ] [P149](p149-harbor-end-to-end-agent-evaluation.md) — Harbor-driven
  end-to-end agent evaluation, implemented in a separate adapter repository:
  a version-pinned external `BaseAgent` uploads and starts the canonical
  `lightspeed-envd` in each Harbor-owned sandbox, correlates its P148 receipt,
  runs the unchanged task through a hosted Lightspeed session, and leaves
  Harbor to verify and destroy the sandbox. A paired Terminal-Bench matrix
  compares Lightspeed and Codex on the same exact model, reasoning effort,
  task/image/verifier, resources, and timeout while retaining each harness's
  native prompt/context/tool loop; local Docker and remote sandbox compute,
  failure accounting, provenance, artifacts, and paired reporting are
  included, but harness-attribution ablations are not.
- [x] [P148](p148-key-based-outbound-environment-registration.md) — reusable
  universe-scoped registration keys for outbound `lightspeed-envd`
  connections: one shared key may admit many independently identified VMs,
  containers, or pods; `envd` mints only its daemon key while Lightspeed
  assigns and fences the environment/incarnation ids. Reviewed 2026-09-02:
  the outbound socket is a control channel and each worker route gets a
  reverse-dialed data socket through the unchanged gateway proxy; identity
  mode is key policy copied onto the environment, with persistent state
  reconnecting the same environment and volatile state creating a new one
  that auto-closes after disconnect grace; the daemon key is a column on the
  environment row; the registration key is the group, with a grant allowlist
  and list filter; bounded correlation metadata and a registration receipt
  let external orchestrators match their compute to the resulting
  environment. No one-time enrollment tickets, frame multiplexing, pool-claim
  intents, or Harbor-specific adapter in this item.
- [ ] [P147](p147-session-checkpoints-and-bounded-reads.md) — session
  checkpoints and bounded reads: keep the event log authoritative while one
  CAS-backed reducer checkpoint per session (advance-only pointer row) makes
  current-state reads and bootstrap replay only a bounded tail; run records in
  reducer state carry sequence bounds and blob references, so summary pages
  come from loaded state and run detail is a primary-key range scan — no SQL
  run read model unless run counts later demand one. Also bounds
  `session/read`, slims mutation responses, fixes quadratic projection and
  sequential blob resolution, removes duplicated Configurator MCP results, and
  distinguishes discovery from tool-call response limits.
- [ ] [P143](p143-mcp-tool-discovery.md) — MCP tool discovery and catalog
  diagnostics (implementation slices 1–3 and first-party `single`-mode live
  acceptance complete 2026-08-31; authenticated live acceptance remains): a bounded
  control-plane MCP client uses the official Rust SDK for current Streamable
  HTTP `initialize` + paginated `tools/list`, resolves server credentials through
  the existing broker without exposing them, returns a typed request-local
  inventory without storing or caching it, and populates the Platform server editor
  with explicit All/Selected allowlist controls. Model-time MCP execution stays
  provider-native; sessions/profiles select only a server id, and discovery
  never invokes a remote tool.
- [ ] [P142](p142-bots-and-channels-core-in-rust.md) — bots and Channels
  core in the Rust runtime (slices 1–3 implemented 2026-08-30, live-tested
  incl. a real-model resolve; slice 4 Platform cut-over open): `crates/bots` and
  `crates/channels` domain crates, `bots` / `bot_triggers` / `bot_events`
  / `channel_accounts` / `channel_pairings` in `store-pg`, `bots/*` and
  `channels/*` folded into the core API plus a gateway hook route,
  `BotControllerWorkflow` / `BotTriggerFireWorkflow` /
  `ChannelConversationWorkflow` in one process with per-subsystem roles
  and queues (`--roles gateway|sessions|bots|channels`, then
  `--task-types`); a TypeScript multi-account connector host over
  `channels/inbound/admit` and per-account activity queues; the
  Platform keeps people and passthroughs and stops being a Temporal
  client. Reverses P124's non-goal for this scope.
- [x] [P141](p141-bot-console.md) — bots as the product (implemented
  2026-08-27): a three-tab bot page (Chat with the main conversation and
  threads inline, Activity timeline, one Setup page with per-section
  saves), a four-step creation wizard that writes a profile named after
  the bot plus the bot and its triggers in one go, the composer as plain
  input on the main session, a roster-style list fed from `bot_events`,
  trigger cards with plain-language summaries, and person-facing labels.
  Profile ownership is deliberately not tracked.
- [x] [P140](p140-bot-environments-and-bot-lifecycle.md) — bot environments
  and bot lifecycle (implemented 2026-08-27, v2; live dogfood of
  close/delete still open): a bot's
  environment is the profile's `existing` environment — idle policy and
  wake already apply to it (P126), so nothing is added to the core's
  environment model (per-session `provision` stays for sandbox-per-event
  bots); one core hardening ("use cancels a pending power-down" in the
  resolver); an idle-policy editor on the Environments page, bot page
  environment card, exec pollers defaulting to the bot's environment; bots
  get `close` (terminal:
  archives pending events, force-closes sessions, drops schedules, keeps
  history) and `delete` (closes first, erases, frees the name). A first
  version with `provision { scope: controller }` and generations was
  implemented and reverted the same day.
- [x] [P139](p139-channels-as-bot-triggers.md) — Channels as bot triggers
  (implemented 2026-08-26): a chat connection is a `chat` trigger on a bot,
  `channel_bindings` is deleted, every message is an admitted event
  (`perKey` session per conversation, trigger coalescing, media refs),
  the conversation workflow stays the receiver for `message_*` through
  declarations carried on the event, and the controller sends
  `started`/`finished` receipts to the endpoint for typing and the
  no-tool-used reply fallback. Chat sessions never expire by default.
  Replaces P131 §5. Same-day follow-up: `bot_activity` removed in favour
  of a write-once `outcome` on event rows, filter misses never stored,
  `bot_filter_test` takes a payload.
- [x] [P138](p138-model-facing-ids.md) — model-facing ids (implemented
  2026-08-26): `PromiseId` is a session counter (`promise_7`) numbered by
  executors from the tool batch's `promise_id_base` and checked by the
  reducer, with the producer token and the `sourceResolution` emission id
  (now holder-scoped, contract v2) following; workflow-tool
  acknowledgements show only the promise(s) the model needs; `job_submit`
  promises keyed by the model's own `job_id` (`ArrayItemField` key
  source); job handles default to the active environment. Environment
  names left as a separate decision; bot-side ids are P135 §8.
- [x] [P137](p137-prompt-caching.md) — prompt caching (implemented 2026-08-26):
  adapter-placed Anthropic breakpoints (system / last tool / moving
  last-message, `prompt_cache_ttl` param for `1h`), `prompt_cache_key` =
  session id on both OpenAI adapters, `LlmUsage` on `RunView` and
  `turnGenerationCompleted` (web run marker, bot delivery detail), a
  broken-prefix warning in the LLM activity, and per-provider caching live
  suites proving ≥ 80 % cache reads across turns, a tool round trip, and a
  superseded catalog.
- [x] [P136](p136-context-catalogs.md) — context catalogs (implemented
  2026-08-26): the VFS, skill, and sub-agent catalogs are keyed entries at
  the front of the message list, and a keyed replace removes the old entry
  mid-context and pushes the new one at the tail — every catalog change
  re-reads a long-lived session uncached. Append-with-supersede for catalog
  kinds: the old entry stays rendered byte-for-byte, the successor carries a
  "supersedes" header, superseded entries are the first thing compaction
  drops, capped per key; instructions and client keyed appends unchanged.
  The same mechanism is exposed to clients as `InputItem::Catalog` on
  `session/context/append` (external catalogs; P135's bot directory is
  the first consumer).
- [x] [P135](p135-bot-federation.md) — bot federation (proposed and
  implemented 2026-08-26): bot ↔ bot as events through admission only —
  one `bot` inbox trigger per bot (`from` allowlist; filter, route,
  coalesce, deliver as for webhooks), `bot_emit { to, reply }` joined
  with typed refusals, deterministic per-receiver ids, `hops` bound,
  sender rate cap, a `bot:directory` catalog through P136's external
  catalog, deterministic resolution receipts (`bot.reply`) with a logical
  return route, one shared admission pipeline for every event path;
  bots addressed by an authored `botId` plus `displayName`, model-facing
  tool results without digests. No cross-bot authority (the `manage`
  grant is a later note), no `bot_ask`, no publish/subscribe until a use
  case asks.
- [x] [P134](p134-subagents.md) — sub-agents (all slices done 2026-08-25): the fleet control plane is
  replaced by a governed, profile-grantable delegation kernel. Two tools
  shaped like the job pair (`agent_run` joined, `agent_spawn` promise) over
  a start-on-call `SubagentExecutionWorkflow`, so the session workflow and
  engine gain no delegation code; `features.subagents` with an allowlisted
  agent menu (a refreshed catalog context entry) and root-scoped,
  attenuating limits; typed `SessionOrigin` lineage on session views and
  `session/list` (replacing `session_links`); one-shot children closed by
  the execution; `inherit` profile environment intent. Removes fleet,
  `PromiseSource::Run`, and the mailbox.
- [ ] [P133](p133-retrievable-grant-leases.md) — retrievable grants: an
  immutable creation-time `exposure: brokered | retrievable` on auth grants,
  a broker-backed `auth/grants/lease` returning `{token, expiresAtMs}`
  (never refresh tokens), a new `service` method scope admitted only for
  service-account callers (structurally hidden from Configurator MCP and
  browser sessions), lease audit counters, and an in-memory caching
  contract; bots poll/webhook credentials and Channels move onto grant ids
  and the platform's plaintext secret fields are removed.
- [x] [P132](p132-workflow-contract-export.md) — workflow contract export:
  publish the `deliver_emission` protocol (envelope, producer types, id
  derivations with known-answer vectors, workflow-id scheme, start-on-call
  recipe/recovery types) from `temporal-workflow` as committed artifacts
  with a staleness test, generate `@lightspeed-ai/agent-client/workflow`, and
  delete the hand-mirrored Bots/Channels `contracts/emissions.ts`. The
  Temporal transport and producer authorization stay; an HTTP reply method
  is deferred.
- [ ] [P131](p131-bot-trigger-long-tail.md) — bot trigger long tail: `poll`
  primitive (interval + cursor over the schedule machinery), inbound email
  trigger, guardrailed agent-authored pollers as daemon jobs in the bot's
  provisioned environment (Gumloop guardrail set, approval-gated), thin
  webhook presets (Slack, Linear, Stripe, Sentry); the Channels bridge
  (chat platforms as event sources) shipped as P139. Extracted from P130's slice 5;
  recommended order: poll first, pollers after real usage.
- [ ] [P130](p130-bots.md) — Bots: a proactive layer over managed sessions.
  Bot = record (brief, profile, triggers, routing/coalescing policy, budgets)
  plus one controller workflow owning its managed sessions. Slices 1-4
  implemented 2026-08-20..23: controller with dedupe/budget/serial lanes,
  schedule (Temporal Schedules) + webhook + endpoint triggers with CEL
  filters, perKey/perEvent routed sessions, coalescing with full-batch
  delivery, steer/append busy policies, flood breaker, replay, routed-session
  retention, `bot_*` self-configuration tools, and a platform web UI. Later
  additions (2026-08-24): event-input redesign (#N seqs, rendered prompts,
  run-scoped resolve, bot_event_read), `selfConfig`/`selfEmit` capability
  grants, schedule-flood breaker, CEL save-time validation, `bot:self` rate
  cap, routed-session declaration rotation. Open: webhook secret sealing
  (deferred by decision; P133), tier-2 per-trigger prompt projections, and a
  model-supplied trigger secret (2026-08-25 review; the same review's
  non-durable-admission finding was fixed that day); the trigger long tail
  moved to P131.
- [x] [P129](p129-active-run-control.md) — active-run control: make
  cancel, steer, and queue work end to end (both phases done and
  live-validated 2026-08-19). Phase 1: the session workflow
  drains admissions at every drive boundary and races in-flight LLM/tool
  activities against them (Temporal activity cancellation + worker
  heartbeat/abort), the engine drops the inert cancellation grace turn,
  `session/runs/steer` is added, `session/runs/start` returns at `queued`.
  Phase 2: platform web UI (and CLI) get a working stop, a steer/queue
  composer, queued-run display, and run state reconciled from
  `session/read` rather than the event tail alone.
- [ ] [P128](p128-openai-completions-runtime.md) — OpenAI Chat Completions
  runtime: phase 1 registers a full `openai:completions` adapter (native
  context round-trip, standalone compaction, admission rejection of MCP/web
  search, `models/list` expansion, live suites) on OpenAI's endpoint; phase 2
  makes the endpoint configurable per model-provider row (`endpoint.baseUrl`
  + headers + api kinds on `model:<provider_id>`), resolved with the
  credential at send time, so any OpenAI-compatible server works.
- [x] [P125](p125-profile-provisioned-environments.md) — let a profile
  provision a fresh environment for the session it starts (`environment:
  existing | provision`), record origin-session provenance on the
  environment, close-with-session by default for provisioned ones, derived
  request identity, and admission of not-yet-ready environments with a
  workflow-level readiness wait around unchanged environment tool activities;
  also restores environment filesystem tools (pre-P119 gate removed).
  Implemented and live-validated 2026-08-16; runtime deployment pending.
- [ ] [P124](p124-first-party-platform-monorepo.md) — import the complete
  first-party TypeScript platform into Lightspeed, neutralize product branding
  with a greenfield durable-identity reset, enforce atomic cross-language contract
  checks, extend P123 to publish every selected component in one coherent
  release, and reduce the private repository to deployment operations.
- [x] [P123](p123-build-and-release.md) — Lightspeed-owned release artifacts,
  embedded ledgered migrations, one-build packaging, automatic coherent
  snapshots after successful `main` CI, and independent exact-commit SemVer
  releases. Lightspeed side complete 2026-08-14 except for the intentionally
  deferred deployment-repository dispatch; isolated-runner provisioning and consumer cutover
  remain external.
- [x] [P100](p100-workflow-tool-ports.md) — the workflow emission substrate:
  one envelope and one fixed `deliver_emission` signal for all cross-workflow
  facts (run-terminal notifications and env-job source resolutions folded in
  a single push, deleting both promise-specific signals), plus workflow-bound
  tools — schema-validated function tools whose calls become log-backed
  emissions for one fixed opaque receiver per binding, declared by trusted
  workflow plugins at managed-session creation (no built-in endpoint
  registry, no plugin-specific session-worker code); Accepted tools are
  pull-consumed at run boundaries, with promise-bearing push delivery owned by
  P100b. Complete 2026-07-28: generic `WorkflowTool*` vocabulary, immutable
  managed bindings, atomic emissions, receiver-authorized paginated pull,
  retry/restart/continue-as-new coverage, and compatibility gates are closed
- [x] [P100b](p100b-workflow-backed-tools.md) — workflow-backed tool
  interactions over P100/P92: push to bound workflow executions (derived from
  completion, no per-binding delivery mode), keyed promise-set completion
  (request/reply as the single-key case), deterministic start-on-call
  workflow tools, one unified `PromiseSource::Workflow`, and plugin-owned
  Temporal workers/activities with no second executor protocol;
  primitive-only, with no production feature migrations. B1-B6 completed
  2026-07-26 incl. the live plugin-worker suite (workflow_tool_plugins_live,
  11 scenarios), hard-promise-deadline enforcement, and concurrency-toolset
  derivation for promise-bearing bindings. B7 completed 2026-07-28: one
  workflow may be both lifecycle controller and promise-bearing receiver only
  with a non-zero hard deadline; the 13-scenario live suite proves the
  Channels-shaped enqueue/reply-before-terminal flow, deduplication,
  controller continue-as-new, stalled-controller deadline, and absent receiver
- [ ] [P101](p101-durable-work-workflow.md) — durable Work as a Temporal-owned
  goal loop over one managed session and many execution runs; explicit
  completion/blockage reports over P100 workflow tools, automatic
  continuation, caller
  input, and reuse of Fleet promises/run notifications without Work-specific
  transport
- [x] [P102](p102-workflow-plugin-extractions.md) — completed 2026-07-29:
  environment-job dispatch adopted P100b internally while remaining core;
  channel integration moved to the external Channels application, allowing
  the built-in messaging feature, tools, outbox store, and delivery APIs to be
  deleted from Lightspeed
- [x] [P103](p103-managed-session-api.md) — expose the existing trusted
  managed-session admission through the universe-authenticated main API by
  adding `session/managed/start` with the complete bound/start and
  Accepted/keyed-Promise workflow-tool vocabulary, and let
  `session/runs/start` optionally notify the session's immutable lifecycle
  controller; keep ordinary session creation unchanged and reuse the existing
  emission, Promise, event, and blob protocols
- [x] [P104](p104-provider-owned-environment-jobs.md) — completed 2026-07-29:
  remove the PostgreSQL environment-job registry and job listing surfaces;
  keep provider-direct
  read/cancel, let providers reject close or interrupt active jobs, and rely
  on Temporal plus provider idempotency instead of stored job/group rows
- [x] [P106](p106-joined-workflow-tools.md) — completed 2026-07-30: add Joined as the ordinary
  single-result workflow-tool form: durably park and resume the original tool
  call without exposing a Promise or requiring model-authored `await`; make
  bound pull/push dispatch independent of Accepted/Joined/Promises completion,
  motivated by the first production Channels sessions where every provider
  receipt currently costs an otherwise unnecessary await tool round. Advanced
  2026-07-30 through the greenfield v4 dispatch foundation, pushed Accepted,
  engine-native Joined, shared declaration readback, durable event diagnostics,
  generated contracts, and passing serial live proofs
- [x] [P111](p111-promise-result-materialization.md) — completed 2026-08-01:
  make explicit `await` return one structured tool result containing every observed Promise value,
  remove Promise-derived synthetic user messages, retain one root ref per
  Promise with structured child refs, and normalize environment-job byte
  chunks into readable text or typed CAS references before Promise resolution
- [x] [P112](p112-joined-environment-job-run.md) — add `job_run` as the
  single-job `Start + Joined` environment surface, returning P111's normalized
  terminal result directly while retaining `job_submit` as the asynchronous
  dependency-group and keyed-Promise form

## Core
- [x] [P91](p91-core-agent-structure-cleanup.md) — cleanup of CoreAgent structures: delete the SDK-era open-kernel layer, commit to a closed event vocabulary and core FSM
- [x] [P95](p95-config-redesign.md) — config redesign: full-document puts with expected revisions, feature-oriented capability config (secure by default), feature versioning, derived toolset; removes patch semantics and the unused `session/messages/submit` RPC surface
- [x] [P98](p98-context-revisions-and-instruction-reconciliation.md) — optional context-edit revision guards and atomic effective-instruction reconciliation, with the product default active only as a true fallback
- [x] [P107](p107-session-workspace-links.md) — move session VFS bindings into
  `features.vfs.workspaceLinks`, derive filesystem/runtime projection from
  config plus the VFS catalog, remove the `vfs_mounts` table and mount APIs,
  and preserve dangling links when referenced workspaces are deleted

## Hosted Runtime
- [ ] [P105](p105-unbounded-hosted-runs.md) — remove the hosted
  `max_steps_per_input = 128` ceiling; let one logical run execute for hours
  across history-driven Temporal continue-as-new boundaries, preserving
  durable progress and transient transport state without fixed-step rollover
  or workflow failure
- [ ] [P109](p109-runtime-state-handoff.md) — remove owning-session log replay
  from ordinary tool, Promise, environment, environment-job, and Fleet runtime
  paths by carrying bounded facts from the Temporal workflow's current
  `CoreAgentState`; retain replay for bootstrap, continue-as-new, recovery, and
  explicit API/history reads

## Fleet (sub-agents)

Superseded by [P134](p134-subagents.md); the entries below are history.
- [x] [P82](p82-session-graph-fork-clone.md) — session graph foundation: clone, fork (by-reference), and links in the store
- [x] [P83](p83-fleet-subagent-control-plane.md) — agent-facing Fleet control plane (spawn/task/read/list/cancel) on top of P82
- [x] [P84](p84-fleet-wait-and-callbacks.md) — first cut complete: `agent_send`, generic deferred tool batches, `RunSubscription` workflow primitives, `agent_wait` DTO/preflight/parking/resume, and live Mode I/Mode W coverage
- [x] [Appendix: Fleet one-off child lifecycle](appendix-fleet-one-off-lifecycle.md) — `agent_spawn.lifecycle.close_on_terminal` for ephemeral delegation sessions
- [x] agent profiles
- [x] start new sessions with profiles and ad-hoc profiles
- [x] [P92](p92-unified-suspension.md) — unified suspension: promises + one `await`, cancellation-as-resolution, watchdogs, force-close, mailbox unification; motivated by the 2026-07-06 stuck-`cancelling` incident

## Provider Integrations
- [x] [P97](p97-model-discovery.md) — direct provider model discovery for
  `models/list` (OpenAI Responses and Anthropic Messages)
- [ ] support and test completions api
   - test with OAI
   - test with open router
   - test with self-hosted model
- [ ] incremental tool discovery support (at least OAI)

## Environments & Sandboxes
- [ ] Local process environment provider for development: a provider binary
  implementing the environment provider protocol by spawning one
  `lightspeed-envd` per environment on the developer machine (per-target
  ports, relay routing, adopt/power semantics), so profile-provisioned
  environments (P125) can be exercised on machines without Incus. The
  provider server in `environment-provider-incus` is already generic over an
  `IncusBackend`; today `./dev.sh full` starts one daemon that is attached as
  an external environment instead (2026-08-18).
- [x] [P117](p117-environment-compute-plan.md) — agreed target architecture for
  operator-registered reachable compute providers, lightweight universe
  bindings, provider-wide offering/ingress policy, stable environment
  identity, reachable external environments, on-demand data connections,
  managed Incus VMs, public ingress, and standalone/native-cluster Incus modes;
  delivery is split across P118-P122
- [ ] [P118](p118-environment-domain-and-lifecycle.md) — replace the
  provider-as-singleton model with operator providers, revisioned universe
  routing/admission bindings, transient controller initialization,
  incarnation-scoped physical facts, idempotent asynchronous lifecycle, and a
  Lightspeed-owned reconciler over a provider-owned policy boundary
- [ ] [P119](p119-environment-daemon-gateway-enrollment.md) — core passive
  `lightspeed-envd` and on-demand gateway data plane implemented; packaging,
  deployment authentication, and product UX remain follow-ups
- [ ] [P120](p120-incus-environment-provider.md) — core standalone stateless
  Incus controller, passive on-demand data endpoint, immutable image recipe, provider-wide policy,
  and durable VM provisioning are implemented; hz01/hz02 deployment, image
  publication, platform UX, and live isolation/acceptance proofs remain
- [ ] [P121](p121-environment-public-ingress.md) — add provider-authorized
  per-environment HTTPS ingress through a shared node-edge proxy with wildcard
  DNS/TLS and no agent or platform credential inside the VM; core API,
  protocol, Incus-provider, and stateless proxy are implemented, while
  deployment and live acceptance remain
- [ ] [P122](p122-incus-multi-node-pool.md) — run the stateless Incus provider
  against either one standalone server or one native Incus cluster, with API
  endpoint failover, cluster-group placement, bounded topology health, and
  explicit non-destructive member-loss behavior; live cluster acceptance and
  deployment remain
- [x] [P113](p113-explicit-vfs-and-environment-tool-domains.md) — separate
  dedicated VFS tools from ordinary active-environment file/process tools,
  remove generic execution targets and the fused filesystem, and make prompts
  and the existing skill catalog explicitly VFS-owned
- [x] [P108](p108-universe-environments.md) — make environments and their
  credentials universe resources, replace session attachment/catalog state
  with one event-sourced active environment, add focused model discovery and
  selection tools, and remove generic default-target routing
- [ ] [P96](p96-environment-api.md) — environment API review: machines as universe resources vs session bindings, real presence leases, machine-keyed durable jobs, occupancy-checked teardown
- [x] Stop externally re-scoping the host-bridge filesystem by its `fsRoot`;
      ordinary environment file tools now pass environment-native absolute
      paths directly to the bridge, which enforces its own root boundary
- [ ] Finalize sandbox protocol (look at Codex's protocol)
- [ ] Write first sandbox integration
- [ ] Allow agent to request new sandbox/env
- [ ] [P86](p86-durable-environment-jobs.md) — durable environment jobs for long VM/sandbox work, including parallel jobs, serial lanes, dependency DAGs, and wait/cancel/read primitives
- [ ] run coding agent (CC or Codex) on sandbox wrappers

## External Channels Integration
- [x] [P88](p88-media-aware-context-append-and-activation.md) — media-aware
  `context/append`, context-triggered runs, and eager bridge ingest/activation
  for current supported media types
- [x] [P89](p89-room-context-retention.md) — room context retention and
  compaction: watermarked drop-oldest pruning via `context/remove`, then
  summarize-and-replace, so always-on group sessions stay bounded
- [x] Channel transport, account sessions, delivery state, media handling,
  authentication, and provider-specific behavior now live in the external
  Channels application; Lightspeed exposes only generic P100/P100b managed
  session and workflow-tool primitives

## Security Auth
- [ ] [P127](p127-openai-oauth-login.md) — Provider OAuth login:
  subscription credentials for Claude Code / Codex in environments
  (Anthropic setup-token paste, OpenAI device flow), OpenAI API key via
  sign-in
- [ ] Send secrets to sandbox/VM/env
- [ ] Design capability based model for agents

## MCP
- [ ] [P150](p150-scalable-mcp-discovery-and-tool-search.md) — return complete
  MCP inventories to management views, expand transcript previews through CAS,
  and give model search one 8 KiB hit shape, 64 KiB pages, and full
  definitions by name
- [x] [P110](p110-universe-owned-mcp-auth.md) — make authentication part of
  each universe-scoped MCP server configuration, remove grant selection and
  grant references from sessions, and resolve the server's current credential
  immediately before provider I/O; edit the greenfield tables in place and
  reset infrastructure
- [x] [P99](p99-configurator-mcp.md) — multi-universe Configurator MCP over
  stateless Streamable HTTP, generated from a configurable subset of the
  universe-scoped TypeScript client contract with request-scoped gateway
  authentication and no operator methods
- [ ] [P143b](p143b-rmcp-oauth.md) — MCP OAuth on `rmcp` (proposed
  2026-08-31): make the official SDK the MCP OAuth protocol engine for
  protected-resource/authorization-server discovery, registration, PKCE,
  issuer/resource/scope semantics, challenges, exchange, and refresh while
  retaining Lightspeed's durable multi-universe flows, encrypted secrets,
  grant broker, audience enforcement, cross-process single-flight rotation,
  leases, and audit. Must precede P145; does not replace generic OAuth.
- [ ] [P144](p144-mcp-approvals.md) — MCP tool-call approvals (proposed
  2026-08-31): one pending-approval surface over two backends — OpenAI
  Responses approval request/response continuation and the native-execution
  dispatch gate — with parked runs, counter approval ids, run-control decide
  admissions, and the independent fix that opaque provider entries can never
  become a run's terminal output. Supersedes the `later/` approval sketch.
- [ ] [P145](p145-native-mcp-execution.md) — native MCP execution (proposed
  2026-08-31): record-owned `execution: provider | native` and
  `exposure: inject | search` where Lightspeed is the MCP client — small
  servers injected as namespaced function tools at request materialization
  (never into engine toolset state), large ones exposed through the
  session-global `mcp_find_tools`/`mcp_call` meta-tools with the authored
  record description as the index and schemas entering context on demand,
  calls dispatched as ordinary per-call tool activities over `tools/call`
  with broker-resolved credentials, deployment-scoped private network
  egress, and MCP on providers without MCP support (`openai:completions`).
  Configurator MCP in search exposure is the flagship acceptance. Replaces
  the "MCP tunnels to model providers" idea.
- [ ] [P146](p146-anthropic-tool-search.md) — Anthropic Tool Search for
  provider-mode MCP (proposed 2026-08-31): honor the record's
  `deferLoadingDefault` on Anthropic passthrough via the GA Tool Search Tool
  (one `tool_search_tool_bm25` entry plus `default_config.defer_loading` on
  the deferred `mcp_toolset`), fixing the adapter's silent drop and matching
  the OpenAI Responses behavior; breakpoints move to the last non-deferred
  tool, unsupported models get a typed error, and native mode stays
  provider-neutral.

## Framework/SDK
- [ ] External Temporal workflow SDK: authenticated access to P100's generic
  trusted managed-session creation capability (declare workflow tools +
  opaque receivers per binding; authorization at the control-plane boundary,
  no endpoint registry)
- [x] P100b promise-bearing (keyed promise set) and workflow-as-tool
  implementation (see [P100b](p100b-workflow-backed-tools.md)); complete
  2026-07-26 incl. live plugin-worker suite; discovery/catalog remains
  outside the session worker
- [x] [P90](p90-multi-tenancy.md) — multi-tenant worker: multiple universes
      per deployment, composed workflow ids, per-request universe resolution
      (`single` / `trusted-header` / `api-key` modes), principal pass-through,
      universe/api-key admin subcommands, per-binding bridge credentials
- [ ] Python SDK
     - [ ] API Client
     - [ ] Workflow helpers
- [ ] TypeScript SDK
     - [x] API Client
     - [ ] Workflow helpers
