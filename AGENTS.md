# AGENTS.md

Guidance for agents working in this repository.

Note: `CLAUDE.md` is a symlink to `AGENTS.md`.

## Project Shape

Lightspeed is moving toward a single hosted agent product built around a
deterministic, event-sourced engine and a Temporal-backed runtime. The current
direction is product-first, not a general agent SDK or an Attractor/factory
pipeline runner.

Use these files as the index:

- `README.md` — current overview, runtime model, capabilities, and commands.
- `docs/design.md` — public design walk-through (deterministic core, context
  management, CAS offloading, Temporal hosting), moved out of the README.
- `docs/spec/01-agent-idea.md` — working design notes for the new agent direction.
- `Cargo.toml` — workspace membership.
- `clients/typescript/` — generated public TypeScript API client.
- `platform/` — first-party TypeScript management server, web UI, operator CLI,
  shared inputs, database schema, Channels workers, Bots workers, and
  Configurator MCP. `platform/web/src/demo/` is the in-browser demo backend
  (the only mock of the platform API; see `platform/README.md`).
- `crates/api/contract/` — committed generated API schema, method manifest,
  OpenRPC, and human reference.
- `dev.sh` and `scripts/dev/` — first-run bootstrap, unified profile-aware
  development supervisor, local Docker stack, environment exports, and reset
  helpers.
- `docs/roadmap/` — implementation plans and historical milestones.

## Build & Test

```bash
cargo build
cargo test
cargo test -p engine
cargo test -p api
cargo test -p api-projection
cargo test -p temporal-workflow
cargo test -p temporal-server
cargo test -p test-support
cargo test -p tools
cargo test -p store-fs
cargo test -p store-pg
cargo test -p mcp
cargo test -p auth
cargo test -p environments
cargo test -p llm-runtime
cargo test -p llm-clients
cargo test -p eval
cargo test -p cli --tests
cargo test -p llm-clients test_name
cargo test -p llm-clients -- --nocapture
npm install
npm run check
npm run test:integration:channels
npm run test:integration:bots
LIGHTSPEED_PLATFORM_MIGRATION_TEST_URL=postgres://... npm run test:migrations
```

Live provider tests are ignored by default and require API keys:

```bash
cargo test -p llm-clients --test openai_responses_live -- --ignored
cargo test -p llm-clients --test openai_completions_live -- --ignored
cargo test -p llm-clients --test anthropic_messages_live -- --ignored
cargo test -p llm-runtime --test openai_responses_live -- --ignored
cargo test -p llm-runtime --test openai_completions_live -- --ignored
cargo test -p llm-runtime --test anthropic_messages_live -- --ignored
```

Additional per-capability live suites exist for the supported provider API kinds under
`crates/llm-runtime/tests/` (`*_compaction_live`, `*_mcp_live`,
`*_prompts_live`, `*_skills_live`).

Temporal live tests share local Temporal/PostgreSQL state and must not run in
parallel. Always pass `--test-threads=1` after the Cargo test-harness separator,
including when running a filtered test. Source `scripts/dev/env.sh` first so the live
tests use the local stack configuration:

```bash
source scripts/dev/env.sh
cargo test -p temporal-server --test temporal_live -- --ignored --test-threads=1
cargo test -p temporal-server --test environment_provider_live -- --ignored --test-threads=1
cargo test -p temporal-server --test preprocess_live -- --ignored --test-threads=1
cargo test -p temporal-server --test environment_provider_live temporal_live_environment_daemon_jobs_round_trip -- --ignored --test-threads=1 --nocapture
```

`temporal_live_slow` holds live tests that wait out production activity
budgets (the LLM schedule-to-close test takes ~30 minutes); run it on its own,
never as part of a routine live pass:

```bash
cargo test -p temporal-server --test temporal_live_slow -- --ignored --test-threads=1
```

After changing `api` wire types, regenerate the committed contract artifacts
under `crates/api/contract/` (`cargo test -p api` fails while they are stale):

```bash
cargo run -p api --bin export-schema
cargo run -p temporal-workflow --bin export-workflow-contract
```

The export includes JSON Schema, the method manifest, OpenRPC, and the generated
human reference at `crates/api/contract/api-reference.md`. Method-level summaries
and descriptions belong in the Rust method manifest; parameter/field docs
belong on the Rust wire DTOs so every generated consumer stays aligned.
The workflow export includes the receiver-side emission schema, constants and
derivation vectors, and its generated integrator reference under
`crates/temporal-workflow/contract/`; `cargo test -p temporal-workflow` fails
while those committed artifacts are stale.

After changing the API contract, regenerate and verify every TypeScript
consumer from the repository root:

```bash
npm install
npm run check
```

CLI usage:

```bash
cargo run -p cli -- chat --api-url http://127.0.0.1:18080/rpc --new
cargo run -p cli -- chat --api-url http://127.0.0.1:18080/rpc --new "summarize this repository"
cargo run -p cli -- chat --api-url http://127.0.0.1:18080/rpc --new --json "summarize this repository"
# Run the server before using --api-url.
cargo run -p temporal-server
cargo run -p cli -- chat --api-url http://127.0.0.1:18080/rpc --session session_1 "hello"
```

Unified development profiles run through the root `dev.sh` launcher. `full`
is the default; `npm run dev` delegates to the same launcher, and connector
processes remain opt-in through
`LIGHTSPEED_CHANNELS_CONNECTORS`:

```bash
./dev.sh
./dev.sh platform
./dev.sh runtime
./dev.sh demo
./dev.sh infra
./dev.sh --plan full
./dev.sh status
./dev.sh stop
./dev.sh down
./dev.sh reset
```

The web UI has two build paths: `npm run build:web` (the live SPA the
Platform server hosts) and `npm run build:demo` (`platform/web/dist-demo/`, the
same SPA over the in-browser demo backend, publishable as a static site).
`./dev.sh demo` (alias `npm run demo`) serves the demo build in development
with no Docker or runtime; it replaces the former stub gateway as the
frontend-only loop. A new platform API route needs a stub
under `platform/web/src/demo/routes/`, and demo content lives in
`platform/web/src/demo/fixtures/` (one module per showcased universe).

`stop` terminates the tracked host supervisor but keeps infrastructure;
`down` stops the host supervisor before tearing down Compose. Internal
infrastructure primitives live under `scripts/dev/infra/` and are not the primary
developer command surface.

The server never migrates PostgreSQL implicitly. Before starting it against a
new or upgraded database, run `cargo run -p temporal-server -- migrate`; use
`cargo run -p temporal-server -- schema-version` for a non-mutating diagnostic.
Release construction, snapshots, and tagged publication are documented in
`docs/releasing.md`.

## Crates

- `crates/engine/` — deterministic session kernel plus built-in CoreAgent:
  dynamic session log storage, CoreAgent command/event/state models, planning,
  codecs, storage traits, and the substrate-neutral drive machine.
- `crates/api/` — client-facing session/run/item/profile API types, views,
  notifications, and JSON-RPC method DTOs.
- `crates/api-projection/` — shared CoreAgent-to-`api` projection
  helpers for local and workflow-backed gateways.
- `crates/temporal-workflow/` — Temporal workflow, signals, queries, and
  activity DTOs for sessions, workflow-backed tools, and environment jobs.
- `crates/temporal-server/` — hosted runtime binary and modules for the Temporal
  worker, HTTP/JSON-RPC gateway, managed-session and workflow-tool admission,
  profile applier, and combined local/small-deployment mode.
- `crates/test-support/` — fast in-process runner harness for tests/evals. It
  is not a production runtime and must not expose an `AgentApiService`.
- `crates/tools/` — optional tool packages for explicit VFS and environment
  filesystem domains, environment actions, web, prompts, and skills.
- `crates/vfs/` — virtual filesystem models, validation, snapshots, mutable
  workspaces, transient workspace-link resolution, and store traits.
- `crates/environment-protocol/`, `crates/environment-client/`, and
  `crates/environment-daemon/` — environment provider/data-plane wire types,
  the typed transport client, and the passive `lightspeed-envd` execution
  daemon. Lightspeed reaches external daemons directly and provider-managed
  daemons through the provider, opening both routes on demand.
- `crates/environment-provider-incus/` — standalone stateless Incus controller
  and passive on-demand data endpoint. It depends only on the environment
  protocol boundary and reconstructs target state from Incus inventory and
  deployment configuration; it must not depend on Lightspeed stores, API
  internals, engine, or Temporal runtime.
- `crates/store-fs/` — filesystem-backed session log and content-addressed blob
  store adapters.
- `crates/store-pg/` — PostgreSQL-backed session store, CAS catalog, MCP server
  catalog, agent profile catalog, environment registry, and AEAD-encrypted auth
  grant/secret storage.
- `crates/mcp/` — provider-independent remote MCP server catalog DTOs,
  validation, and store traits.
- `crates/profiles/` — agent profile registry validation helpers,
  errors, and the substrate-neutral `ProfileStore` trait over `api` profile
  DTOs.
- `crates/auth/` — generic auth grant/secret/provider records,
  OAuth client and authorization-flow records, PKCE helpers, the MCP OAuth
  and GitHub App drivers, store traits, typed broker errors, the runtime
  token broker with single-flight refresh and on-demand minting (P69), and
  deployment-scoped inbound API keys for gateway authentication (P90).
- `crates/environments/` — operator provider records, universe-scoped routing
  bindings, environment lifecycle intents, minimal incarnation identities,
  universe-scoped credential bindings, validation, errors, and store traits.
  Sessions retain only an active environment id in event-sourced core state.
  Provider job DTOs live in `environment-protocol`; no Lightspeed job registry is
  persisted.
- `crates/eval/` — eval harness for agent/tool workflows.
- `crates/llm-runtime/` — CoreAgent LLM runtime from planned requests to
  provider-native client calls.
- `crates/llm-clients/` — provider-native OpenAI and Anthropic API clients.
- `crates/cli/` — command-line chat client for the API gateway.

## Architecture Rules

- Keep `engine` deterministic. It should not execute provider calls, shell
  commands, filesystem operations, network I/O, or workflow activities.
- Execute side effects outside the core through runtime adapters, workflow
  activities, or tool packages. CoreAgent uses separate LLM and tool traits
  rather than a generic effect event lifecycle.
- Keep provider message/request/response structures native to each API kind.
  Do not rebuild a fake universal LLM message model.
- Parse only reducer facts needed for deterministic branching; keep other
  provider-native data opaque/blob-backed.
- Keep provider request vocabulary out of `engine`. The core plans a
  provider-neutral `LlmRequest` intent with opaque `ProviderParams`
  (`api_kind` + versioned JSON body) transiently for runtime execution; durable
  planned-turn events store only fingerprints and revisions. Typed param schemas
  and wire-request materialization live in `llm-runtime` adapters, and admission
  boundaries validate params before they enter the session log. Transport config
  (base URLs, credentials, headers) stays in runtime deployment config, not in
  `ModelSelection` or the session log.
- Universe model-provider rows are named `model:<providerId>` and may carry an
  OpenAI-compatible endpoint plus API-key/OAuth authentication, or a
  credentialless endpoint for local servers. Resolve the row immediately before
  provider I/O. Only built-in `openai` and `anthropic` ids may fall back to
  deployment clients; a custom provider without a complete endpoint must fail
  before network I/O. Endpoint headers are non-secret and must not override
  transport-owned authentication/content headers.
- Keep clients on `api`. CLIs, TUIs, editors, hosted gateways, and future
  Temporal frontends should not consume reducer internals directly.
- Use Lightspeed-owned names for every supported product configuration key,
  persisted identifier, Temporal identity, browser storage key, and deployment
  input. Do not reintroduce imported pre-release aliases;
  `npm run check:identity` enforces this boundary across the repository.
- Treat hosted `session/runs/start` as an acceptance boundary, not a
  final-output boundary: it returns once the run is `queued` (behind an active
  run) or `running`. Clients should follow `session/events/read` or refresh
  `session/read` for progress and completion.
- Active-run control is admitted live. The session workflow drains client
  admissions at every drive action boundary and races in-flight model/tool
  activities against them; `session/runs/cancel` cancels the open turn or
  pending tool calls in the engine at once (no grace turn) and abandons the
  activity (`TryCancel` + worker heartbeat), `session/runs/steer` appends
  steering that materializes at the run's next turn boundary — never while a
  turn is in flight, whose request is frozen at its planned revisions — and
  a run with unconsumed steering takes one more turn instead of completing
  on a final-output turn (accepted while running or parked, never waking an
  await); a second `session/runs/start` queues. While a turn is in flight
  only run-control admissions land; context/config/tool mutations wait for
  the turn boundary. A cancelling run never asks the runtime for work: the
  engine resolves its own open turn/batch. Do not reintroduce "process
  admissions only between runs" or a farewell LLM turn on cancel.
- Treat `session/managed/start` as a trusted creation boundary. Lifecycle
  ownership and caller-declared workflow tools are immutable session metadata;
  do not expose them through ordinary `session/start` or mutable session config.
  Runtime-owned system bindings are separate add-only admissions and do not
  assign lifecycle ownership.
- Reuse the generic workflow-tool protocol for workflow integrations:
  immutable bindings, emissions, keyed Promises, workflow starts, replies,
  deadlines, and cancellation. Do not add feature-specific transports or
  compile external plugin workflow types into the stable session worker.
- Sub-agents (P134) are attached delegation on that protocol, distinct from
  bots' durable orchestration. `features.subagents` is the authority: an
  allowlisted agent menu plus root-scoped, attenuating limits. `agent_run`
  (joined) and `agent_spawn` (promise) are system start-on-call bindings
  whose `SubagentExecutionWorkflow` creates the child from the pinned
  profile revision, records typed lineage (`SessionOrigin` — provenance,
  never ownership), resolves the parent's `reply` promise, and closes the
  child. Children are one-shot; cancellation from any direction closes the
  child. Do not add a parent-side delegation transport, child↔parent
  messaging, an agent graph surface, or an enum menu in the tool schema —
  the menu is a refreshed catalog context entry.
- Bot federation (P135) is events through admission, never authority: a
  bot addresses another with `bot_emit { to }` through the receiver's
  single `bot`-kind inbox trigger (at most one per bot), which owns
  filter, route, coalesce, and delivery policy. `bot_emit` is joined and
  returns `{ to, seq }` or a typed refusal; the sender rate cap and
  `MAX_BOT_HOPS` bound every exchange. Every event path — webhook, poll,
  schedule, self and addressed emits, receipts — goes through
  `platform/bots/src/admission.ts` (`storeBotEvent` / `admitTriggerEvent`).
  Replies are deterministic receipts (`bot.reply`) sent by the receiver's
  controller when the delivery finishes, routed by a logical session
  (base id, never a generation); no `bot_ask`, no joined cross-bot call,
  no bot configures or creates another. Discovery is the `bot:directory`
  catalog (only bots whose inbox accepts the reader). Bots are addressed
  by an authored, immutable `botId` (`bots.name`) plus a mutable
  `displayName`; the uuid row key never leaves the database, and
  model-facing `bot_*` results carry `#N` and labels, never digests
  (`activities/tool-views.ts`).
- Channels are bot triggers (P139). A chat connection is a `bot_triggers`
  row of kind `chat` (account, scope, activation, access, pairing); there
  is no binding record and no channel-owned session. Every activated
  message is one event through `admitTriggerEvent` (id
  `chat:<trigger>:…`), routed `perKey` per conversation into
  `bot:v1:<bot>:k-…`; the conversation workflow
  (`channelConversationWorkflowV1`) is the *receiver* of that session's
  `message_*` tools, whose declarations travel on the event (`tools` CAS
  ref) and are merged verbatim at `ensureRoutedSession`. The controller
  signals `bot_delivery_v1` `started` / `finished` receipts to the event's
  `notify` endpoint (typing, and the text-reply fallback when no
  `message_*` tool was used); a run that used a carried tool counts as
  `handled`. Messages are named to the model by the bot's `#N` in both
  directions — inbound is the event, a send is an archived `chat.sent`
  row — and `message_send { text, replyTo: 17 }` resolves numbers to
  provider ids inside the conversation workflow. Per-trigger
  `sessionTtlMs` (0 = never, the chat default) overrides
  `routedSessionTtlMs`. Do not reintroduce bindings, a channel-owned
  session, provider message ids in tool arguments, or a second lifecycle
  controller.
- Bot decisions live in the controller's Temporal history; Postgres is the
  read model. `bot_events` is the bot's numbered log of what arrived and
  what it sent, each row with a write-once `outcome` (the model's
  `handled`… or the system's `steered` / `run_failed` / `archived`; null =
  pending) written when the delivery finishes; trigger incidents are
  trigger state (`disabled_reason`, `last_filter_error`); controller state
  is the live snapshot. A filter miss is never stored. Do not reintroduce
  an activity/audit table or archived filter misses.
- Catalogs (VFS, skill, sub-agent, and client `Catalog` entries) are
  append-with-supersede: a keyed catalog write appends the new version with
  `supersedes` set and leaves the earlier one active and rendered
  byte-for-byte, so the provider prefix cache holds on long-lived sessions;
  superseded versions are compactable, capped per key, and cleared by
  `RemoveContext`. Look up "the current entry for a key" with
  `current_context_entry` (newest), never the first match. Clients publish
  their own catalogs (a bot directory, a roster) as `InputItem::Catalog` on
  `session/context/append`; run input rejects them. Do not reintroduce
  in-place catalog rewrites or catalogs in the system prompt.
- Model-facing ids are counters and names, never hashes. `PromiseId` is a
  session counter (`promise_<n>`) that executors number from the tool
  batch's `promise_id_base` (a per-call dispatch owns slot `base + index`)
  and the reducer accepts only at or above that base and never twice;
  producer correlation lives on `PromiseSource`, and the `sourceResolution`
  emission id includes the holder workflow id. Workflow-tool
  acknowledgements show the model only its promise handle(s); invocation
  and execution digests stay in `output_json`. `job_submit` promises are
  keyed by the model's own `job_id` (`ArrayItemField`), and job handles
  default to the active environment. Do not reintroduce digest-shaped ids
  or index-derived keys in anything the model must copy back.
- Prompt caching is the adapters' job, and the rendered prefix must stay
  stable to keep it. The Anthropic adapter places `cache_control`
  breakpoints on every request (system prompt, last tool, last block of the
  last message; TTL from `prompt_cache_ttl`); the OpenAI adapters send the
  session id as `prompt_cache_key`. Markers are placement, not content —
  tests about lowering strip them. Anything that rewrites context before
  the tail (an instructions rewrite, compaction, an in-place catalog
  replace) invalidates the cache from that point; append instead where you
  can. Usage, including cache reads and writes, is on `RunView.usage` and
  `turnGenerationCompleted`; the LLM activity warns when a large prompt
  misses right after a hit.
- Anthropic thinking must stay visible: the adapter derives adaptive thinking
  with `display: summarized` from `reasoningEffort` (current models omit the
  summary by default, which leaves every reasoning entry blank), maps
  `reasoningEffort: none` to `thinking: disabled` (Claude Opus 5 thinks when
  a request carries no thinking config), accepts `xhigh`, and surfaces
  `output_tokens_details.thinking_tokens` as `reasoning_tokens`. Thinking
  counts toward `max_tokens`, so the adapter's default output cap and the
  compaction cap leave room for it. A content-filter stop on any provider
  (an Anthropic `refusal`, an OpenAI `content_filter`) fails the turn — the
  engine maps `LlmFinish::ContentFilter` to a failed turn and the adapters
  attach the provider's category/explanation — never an empty "successful"
  answer, and never a server-side fallback to another model. A cut-off at
  the output cap (`LlmFinish::Length`) fails the turn the same way but keeps
  the assistant's partial text in the log; tool calls without results and
  unfinished thinking are dropped because they are not replay-safe. Live
  coverage lives in
  `crates/llm-runtime/tests/anthropic_messages_live.rs` (thinking replay and
  the thinking-through-tools round trip, default model `claude-opus-5`);
  keep those asserting on summary text, not just block presence.
- Session config is a sparse, capability-oriented document (core sections plus
  default-off feature grants) replaced whole via `session/config/put` with an
  expected revision. Do not reintroduce field-level patch vocabulary; registry
  documents (profiles, MCP servers) follow the same put-with-expected-revision
  pattern. The session toolset — including remote MCP tools declared under
  `features.mcp` — is derived from config and never written directly by
  clients. See `docs/roadmap/p95-config-redesign.md`.
- MCP authentication belongs to the universe MCP server record. Sessions and
  profiles select only `serverId`; they never select or retain an auth grant.
  Resolve the server's current grant immediately before provider I/O. See
  `docs/roadmap/p110-universe-owned-mcp-auth.md`.
- VFS session topology is declared only by
  `features.vfs.workspaceLinks`. Snapshots and mutable workspace heads remain
  catalog resources; resolved links are transient, and no session-link or
  mount table is authoritative. See `docs/roadmap/p107-session-workspace-links.md`.
- Enabling `features.environments` permits externally selected active
  environments and grants ordinary file/process tools against that active
  environment. `features.vfs.tools` separately grants dedicated `vfs_*` tools
  against linked VFS snapshots/workspaces. Never fuse, overlay, implicitly
  synchronize, or target-route these filesystems. Only
  `features.environments.selectionTools` exposes model discovery/selection;
  jobs remain an independent sub-grant. Prompts and `skills.catalog.vfs` are
  VFS-only. See `docs/roadmap/p113-explicit-vfs-and-environment-tool-domains.md`.
- A profile's `environment` is an intent, not an id: `existing` activates a
  universe environment and never closes it; `provision` creates one
  environment per session (request id derived from the session id) from the
  universe's enabled binding for `providerId`, activates it while it is still
  provisioning, and by default closes it with the session. Environments record
  `originSession` as provenance and an optional close trigger, never
  ownership. Environment-dependent tool calls against a not-ready environment
  do not wait inside the tool activity: the worker reports
  `EnvironmentNotReady`, the workflow runs `await_environment_ready`, then
  re-dispatches the call. Do not put provisioning in `SessionConfig`, on
  `session/start`, or behind a model tool. See
  `docs/roadmap/p125-profile-provisioned-environments.md`.
- Environment power is intent plus observation (P126). `desiredPower`
  (`running | paused | suspended | stopped`) is a Lightspeed-owned column that
  the lifecycle reconciler converges through one provider verb,
  `controller/setTargetPower`; observed state stays in `status`
  (`paused`/`suspended`/`offline`). Providers advertise the states they
  support per target (`powerStates`); Lightspeed validates against that and
  never stores activity. Idle detection is the daemon's monotonic
  `env/idle` report read on demand by the power reaper, which applies the
  environment's staged `idlePolicy` (pause → suspend → stop → close, skipping
  stages the provider lacks). A powered-down provisioned environment wakes on
  use: the resolver sets desired `running` and reports `NotReady`, reusing the
  P125 `await_environment_ready` path. The power decision never consults
  sessions — the daemon's idle report is the only activity signal, so N
  sessions (of any bot or client) sharing an environment need no
  coordination — and use cancels a pending power-down (a `ready`
  environment whose `desiredPower` is lower is written back to `running`
  at resolution). Do not add per-call `lastUsedAt` writes, provider-side
  policy, a session→environment index, or feature-specific pause/resume
  verbs. See `docs/roadmap/p126-environment-power-and-idle-policy.md`.
- A bot's environment is the profile's `existing` environment (P140): a
  universe resource nothing bot-related creates, closes, or deletes; its
  idle policy lives on the environment and is edited there. Per-session
  `provision` remains the sandbox-per-event choice. Do not add bot- or
  controller-scoped provisioning, generations, shared scopes, or leases —
  a first version of P140 tried and was reverted.
- Bots have three lifecycle states (P140): `disabled` is the reversible
  pause (sessions and chat context stay, the environment sleeps by idle
  policy); `close` is terminal — the server writes `closedAt` first, disables
  every trigger (`bot_closed`), and the controller archives what is pending,
  force-closes every session, records `closedSessions`, and completes
  instead of continuing as new; `delete` closes first, `session/delete`s
  the recorded sessions, and removes the row so the name is free. Admission
  refuses on the row (`closedAt`) — waking is a signal-with-start, so
  nothing else stops a late event from resurrecting a closed controller.
  Do not add a reopen, a drain period, or a second lifecycle authority.
- Preserve Rust 2024 and the existing crate-local `thiserror` error style.
- Use `tokio` current-thread tests where async tests are needed.

## Environment

Local commands load a root `.env` file when present. The `.env` file usually
exists in development environments; check with the developer before running
live commands.

See `docs/variables.md` for the authoritative reference, including the strict
separation between core runtime, Platform, Channels, Configurator, environment
services, development, test, and release variables.

## Test Rules

- Unit tests live next to code in `mod tests`; integration tests go under
  `tests/` when they cross crate boundaries or hit I/O.
- Tests must fail when the thing they test does not work.
- Do not silently skip tests with runtime env-var gates. Use `#[ignore]` for
  tests that require API keys, external services, or other opt-in resources.
- When an ignored test is explicitly run, it must fail clearly if its
  prerequisites are missing.
- Prefer asserting error kinds/types over brittle string matching.
- Keep tests parallel-safe: avoid shared global state and non-unique temp paths.

## Maintenance

- If high-level architecture changes, update `README.md`, this file, and the
  relevant docs under `docs/spec/` or `docs/roadmap/`.
- When a roadmap item is completed or partially completed, mark what changed in
  that roadmap file.
- When asked how many lines of code, use `cloc $(git ls-files)`.
