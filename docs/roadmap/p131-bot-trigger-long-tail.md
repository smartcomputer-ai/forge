# P131 — Bot Trigger Long Tail: Poll, Pollers, Email, Presets, Channels

**Status**

- Extracted 2026-08-24 from P130's slice 5 so each piece can be scoped and
  shipped on its own. Workstream 1 (`poll`) shipped 2026-08-24 and
  collapsed most of 3; 2, 4, 6 are parked until after P132, P133, and the
  fleet simplification; 5 is deferred (2026-08-25, not needed for now). P130 carries the running
  implementation log for slices 1–4 plus the event-input redesign,
  capability grants, and hardening round this builds on.
- Builds on P130 (controller, store-then-wake admission, `#N` events with
  rendered prompts, CEL filters/routing, coalescing, breakers, `selfConfig`
  / `selfEmit` grants), P125/P126 (provisioned environments, power/idle),
  and the Channels workers.

## Why

Slices 1–4 cover sources that push: schedules fire, webhooks arrive, events
get POSTed. The long tail of real-world sources does not push — internal
dashboards, vendor APIs without webhooks, mailboxes, chat rooms. The P130
research settled the strategy: **hardcode transports, not services** — a few
more L0 primitives, thin per-provider presets over them, and guardrailed
agent-authored pollers to absorb everything else. This doc turns that into
five independently shippable workstreams.

## Workstreams

### 1. The `poll` primitive (L0) — IMPLEMENTED 2026-08-24

Shipped end to end on the `bots` branch: trigger `kind: "poll"` with
`source: http | exec`, `intervalMs` (min 60s), optional `items` dot-path,
and `cursor: idSet | watermark`; realized as a Temporal Schedule (interval
spec, overlap SKIP) firing `botPollFireWorkflowV1` → `pollBotTrigger`
activity (fetch or environment job with typed not-ready retries → diff
against the row-owned cursor → per-item admission with filters, routing,
coalescing, `#N` + rendered prompts). Baseline-on-enable delivers nothing;
per-fire cap 100 with `poll_truncated`; consecutive-failure auto-disable
(10) plus the shared flood breaker; spec edits reset the cursor; filtered
poll items advance the cursor without being archived (deliberate volume
choice, counted per fire). Self-config via `bot_trigger_put` (http source),
UI form (http), API/exec via trigger CRUD. Migration
`0002_bot_poll_triggers` (post-squash journal), platform schema revision 3.
Original sketch follows:

A third trigger kind beside `schedule` and `webhook`: call an HTTP source on
an interval and admit what changed.

- Spec: `{ url, method?, headers?, intervalMs (min enforced), extract?
  (CEL or dot-path to the item list), cursor: idSet | watermark(field) }`.
  Zapier's dedupe model: an id-set cursor for unordered feeds, an
  `updatedAt` watermark for ordered ones. Cursor state is a column on the
  trigger row — Lightspeed-owned, replayable, no hidden state.
- Runtime: reuse the schedule machinery — a Temporal Schedule per poll
  trigger fires a `botPollFireWorkflowV1` whose activity fetches, diffs
  against the cursor, and admits one event per new item through the normal
  admission path (filters, routing, coalescing, breaker all apply for
  free).
- Auth: static headers in the spec first (secret-sealing caveat applies as
  it does to webhook secrets); broker-backed credentials arrive with
  [P133](p133-retrievable-grant-leases.md) retrievable-grant leases — do not
  build a parallel secret store.
- Fits the existing surface: `bot_trigger_put` grows `kind: "poll"` fields;
  the flood breaker and CEL validation already generalize.
- **Two sources, one primitive.** The spec carries a `source` discriminator
  from day one:
  - `source: http` — the bots activity worker fetches directly (v1).
  - `source: exec` — the fire activity runs a one-shot command in an
    environment via the existing `environments/jobs/create`/`read` API
    (jobs are instance-owned, credential-injected, session-independent —
    no core change needed) and treats the job output as the item feed.
    Same cursor/diff/admission downstream. Covers internal databases,
    CLIs, and filesystems with no public API.
    **Verified 2026-08-24: the jobs API does not wake sleeping
    environments today.** `environments/jobs/create` loads the record and
    dials the data plane directly (`session_jobs.rs`), bypassing
    `environment_resolver::resolve_for_connection` — the P126 wake-on-use
    path (paused/suspended/offline + provider power support → set desired
    power `running` → typed `NotReady`) that session tool dispatch uses.
    Against a suspended environment, jobs/create just fails with a generic
    connect rejection and wakes nothing. **Prerequisite core patch: DONE 2026-08-24.** The jobs
    create/read/cancel paths now resolve through
    `environment_resolver::resolve_for_connection`, so a powered-down
    environment gets desired power `running` set and the call fails with
    the new typed `environment_not_ready` API error kind (JSON-RPC
    -32012, mapped in the TS client) instead of a generic connect
    rejection. Poll-fire activities lean on Temporal retries: fire → wake
    → retry until ready → run job → idle reaper re-sleeps the
    environment. Live-validated in
    `temporal_live_environment_power_intent_converges_and_wakes_on_use`
    (jobs/create against a suspended environment: typed error + desired
    power flip observed).
  Resident in-environment poller daemons remain workstream 3: they fight
  the idle policy (a daemon either pins the environment awake or gets
  suspended mid-watch), so they must earn residency — websockets, log
  tailing, warm scraping sessions — and need the power-interplay design
  first.

### 2. Email trigger (L0)

An inbound address per trigger (`<trigger-token>@bots.<domain>`), because
email is the one push transport everything supports. Needs an inbound
provider decision (SES / Postmark / self-hosted LMTP) — the only piece with
a real infrastructure dependency. Parsing: envelope → event (`kind:
"email"`, summary from subject, body + attachments to CAS, sender/thread as
route-key candidates). Defer attachments-as-VFS until wanted.

### 3. Agent-authored pollers (L2) — largely collapsed by workstream 1

Update 2026-08-24: with exec polls shipped and `bot_trigger_put` accepting
`environmentId`+`argv` (revision 8), a bot with the `selfConfig` grant and
environment tools can already author a poller end to end — write and test
the script in its environment, then register the exec trigger itself.
Lukas explicitly accepted this without an extra approval gate; the standing
guardrails apply (grant-gated trigger_put, 60s minimum interval, breaker,
10-failure auto-disable, budgets, visible cursor, `self_configured`
activity trail). Best with a stable `existing`-environment profile —
per-session provisioned environments close with their session and would
strand the trigger. What remains of this workstream is only: an approval
gate if operator policy ever wants one, and resident daemons (below).
Original sketch:

For sources with no API shape at all: the bot writes its own poller — a
small program checking a source on an interval, keeping a cursor, POSTing
findings to its own L0 endpoint. Gumloop proved the model; copy their
guardrail set verbatim: *read-only credentials, sandbox-tested against live
data before activation, dedupe cursor, minimum interval, circuit breaker,
metered*.

Lightspeed's structural advantage: **the sandbox already exists.** A poller
is a daemon job in the bot's provisioned environment (P125/P126) — same
lifecycle, idle policy, credential brokering, and provider isolation the
environment system already provides. What P131 adds is the control loop:

- a `bot_poller_propose` tool (gated like `selfConfig`; activation and any
  scope increase require human approval in the UI),
- job templating + the ingest-URL/token handoff so the poller can POST to
  its own endpoint trigger,
- metering + auto-disable wired to the existing breaker/activity plumbing.

Revisit in-place tool-declaration evolution with the usage data this
generates (the rotation cost is paid per declaration change today).

### 4. More webhook presets (L1)

Slack, Linear, Stripe, Sentry/PagerDuty. Post-redesign a preset is one
small record: verification scheme, where the event name and dedupe id live,
default route keys (thread ts, issue id, object id), and the prompt
projection (subject object for the renderer). Each is an afternoon because
the transport is shared. The GitHub App installation story
(`later/pNNN-platform-github-app-installations.md`) slots here as the one
preset needing real credential plumbing.

### 5. Channels bridge — SUPERSEDED by [P139](p139-channels-as-bot-triggers.md) 2026-08-26

P139 goes further than the shape below: a chat connection *is* a `chat`
trigger on a bot, `channel_bindings` is deleted, and Channels never owns a
session. The sketch stays as history.

Chat platforms as an event *source*: a Channels binding that forwards
messages into a bot (`kind: "chat"`, thread-based route key, coalescing for
bursty rooms) so "watch this Slack channel and act" works. This is a
Channels-side emitter into the existing bot ingest — no new bot-side
machinery expected. (Distinct from operator chat, which P130 resolved via
the sessions page's Direct-input override.)

Shape, settled 2026-08-24: a session has one lifecycle controller but any
number of tool receivers, so the bot stays the controller and Channels is
both a *source* (inbound messages become bot events) and a *receiver*
(outbound `channel_*` send tools bound to the channel workflow). No core
change, and not co-management — two controllers is a P100 non-goal.

### 6. Bot federation (from the fleet-vs-bots review)

Specified 2026-08-26 in [P135](p135-bot-federation.md): the events item
below grows subscription triggers, deterministic replies, and loop bounds;
the configuration item is withdrawn there (no cross-bot authority — a bot
reshapes only itself, neighbours ask). The sketch stays as history.

Two small platform-tier items, independent of the rest of this doc (see
`later/pNNN-fleet-vs-bots.md`):

- **Bot → bot events**: `bot_emit` grows a `targetBot`; the event keeps
  `source: bot:<sender>`, so provenance tagging, the self-emission cap,
  and the receiver's filters and breaker apply unchanged. Add a causation
  id and a hop bound to the event: provenance plus per-receiver breakers
  rate-limit but do not stop A→B→A cycles.
- **Bot → bot configuration**: target-bot forms of `bot_trigger_put` /
  `bot_brief_put` behind a new `manageBots` operator grant, allowlisted per
  target bot and operation (the
  `selfConfig` pattern pointed outward) — an ops-bot that tunes other
  bots.

## Order and shape

1 (`poll`) is the smallest and load-bearing for 3; do it first. 2, 4, 5 are
independent and parallelizable; each is a thin adapter onto shipped
machinery. 3 is its own design session (approval UX, guardrail enforcement
points, metering) and should come after 1 has real usage.

## Non-goals

- Buying a trigger catalog (Composio et al.): stays a deferred accelerator
  that would plug into the L0 endpoint cleanly if ever wanted.
- MCP as a trigger surface: no out-of-band push in the spec; the WG is in
  ideation. MCP remains the tool surface (P110).
- A universal payload schema per service: presets project prompts; the
  archived envelope stays raw.
