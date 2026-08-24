# P131 — Bot Trigger Long Tail: Poll, Pollers, Email, Presets, Channels

**Status**

- Proposed / not started. Extracted 2026-08-24 from P130's slice 5 so each
  piece can be scoped and shipped on its own; P130 carries the running
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

### 1. The `poll` primitive (L0) — recommended first

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
  workstream 3's approval flow or P110-style grants — do not build a
  parallel secret store.
- Fits the existing surface: `bot_trigger_put` grows `kind: "poll"` fields;
  the flood breaker and CEL validation already generalize.

### 2. Email trigger (L0)

An inbound address per trigger (`<trigger-token>@bots.<domain>`), because
email is the one push transport everything supports. Needs an inbound
provider decision (SES / Postmark / self-hosted LMTP) — the only piece with
a real infrastructure dependency. Parsing: envelope → event (`kind:
"email"`, summary from subject, body + attachments to CAS, sender/thread as
route-key candidates). Defer attachments-as-VFS until wanted.

### 3. Agent-authored pollers (L2) — the headline, most design-heavy

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

### 5. Channels bridge

Chat platforms as an event *source*: a Channels binding that forwards
messages into a bot (`kind: "chat"`, thread-based route key, coalescing for
bursty rooms) so "watch this Slack channel and act" works. This is a
Channels-side emitter into the existing bot ingest — no new bot-side
machinery expected. (Distinct from operator chat, which P130 resolved via
the sessions page's Direct-input override.)

### 6. Bot federation (from the fleet-vs-bots review)

Two small platform-tier items, independent of the rest of this doc (see
`later/pNNN-fleet-vs-bots.md`):

- **Bot → bot events**: `bot_emit` grows a `targetBot`; the event keeps
  `source: bot:<sender>`, so provenance tagging, the self-emission cap,
  and the receiver's filters and breaker apply unchanged.
- **Bot → bot configuration**: target-bot forms of `bot_trigger_put` /
  `bot_brief_put` behind a new `manageBots` operator grant (the
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
