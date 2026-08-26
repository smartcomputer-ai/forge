# P135 — Bot Federation: Events Between Bots

**Status**

- Proposed 2026-08-26, recommendation-first design for discussion, not a
  plan. Written after P134 landed, from the question "now that sub-agents
  are simple, what makes bots a powerful coordination system — how do bots
  talk to each other, and can a bot create and set up another bot?"
- Revised the same day after review. The first draft allowed bot → bot
  *configuration* behind a `manage` grant while deferring bot → bot
  *creation*; Lukas asked why one and not the other. The asymmetry does not
  hold: a poll trigger put on a neighbour is as durable and as costly as a
  new bot, and `selfConfig` already lets a bot mint such triggers on itself.
  The real question is whether any bot has authority over another bot, and
  configuration and creation are the same answer to it. Adopted position:
  **neither** — bot ↔ bot is events only; authority over a bot belongs to
  the bot itself (`selfConfig`) and to humans. The `manage` grant
  (configure *and* create, one grant) is kept below as the alternative to
  reach for if an ops-bot use case ever demands authority.
- Same day, second review question: how does bot A know about B, and when?
  Answered in §1 — a derived `bot:directory` in context (the P134 catalog
  idea) replaces the pull-shaped `bot_list`. A keyed rewrite mid-context
  would invalidate the provider prefix cache from the old position on a
  session that lives for months, so the directory is published through
  [P136](p136-context-catalogs.md)'s external catalog (supersede at the
  tail, never a rewrite); the interim snapshot-plus-deltas design is
  dropped. The brief says *when*, the directory says *who*, subscriptions
  say *whether*. The caching work itself is
  [P137](p137-prompt-caching.md). P135 slice 1 needs P136 slice 4 first.
- Absorbs P131 workstream 6 (bot → bot events; its bot → bot configuration
  item is withdrawn by the position above) and the "Bot federation" note in
  `later/pNNN-fleet-vs-bots.md`.
- Builds on P130 (controller, store-then-wake admission, `#N` events,
  CEL filters, routing, coalescing, breakers, `selfConfig` / `selfEmit`)
  and P134 (the two-tier rule: durable orchestration above, attached
  delegation below; provenance not ownership; deterministic identities;
  no new transports).
- Greenfield: no wire, schema, or event back-compat; `bot:self` and
  `self-<uuid>` event ids go away rather than being kept.

## Why

After P134 the product has exactly two ways to get more than one session's
worth of work done, and they no longer overlap:

- **Attached delegation** (P134): a run fans out to one-shot children over
  promises and joins their results inside its own reasoning loop. Vertical,
  synchronous from the model's point of view, bounded by root-scoped tree
  limits.
- **Durable orchestration** (P130): a deterministic controller admits
  events, filters, routes, coalesces, budgets, and drives managed sessions
  for months. Horizontal, asynchronous, bounded by budgets and breakers.

The second story stops at the edge of one bot. A bot today is an island: it
can receive from the world and emit to itself, but it cannot address
another bot or hear from one. Every real deployment wants a *team* — a
triage bot that hands incidents to an infra bot, a comms bot that narrates
what the others decide, an ops bot that notices when a bot misbehaves — and
the only way to build one today is to route everything through webhooks the
operator wires by hand.

The design insight that keeps this small is the same one that made P134
small: **do not add a transport**. Sessions already speak one protocol to
durable work that finishes later (a promise); bots already speak one
protocol to the world (an event through admission). Bot ↔ bot is events
through admission, with three additions the existing machinery lacks:
subscriptions, a deterministic return path, and a loop bound. Nothing else.

What this deliberately does *not* add is authority: no bot configures,
enables, or creates another bot. Coordination needs communication and
per-bot policy, not a manager; the deterministic layer (breakers, budgets,
auto-disable) already plays the manager, and P130 chose it over a prompted
one for exactly this reason. A bot that wants a neighbour to change asks;
the neighbour's human-written brief decides whether to listen.

## The shape

```text
                       universe event bus
   ┌──────────────┐  bot_emit {to?, reply?}   ┌──────────────┐
   │ bot a        │ ────────────────────────▶ │ bot b        │
   │ controller   │   subscription trigger    │ controller   │
   │              │ ◀──────────────────────── │              │
   └──┬───────────┘   reply (deterministic:   └──┬───────────┘
      │               b's delivery outcome)      │
   sessions (roots)                           sessions (roots)
      ├─ agent_run  ▶ child (one-shot, joined)   ├─ agent_run ▶ child
      └─ agent_spawn ▶ child (promise)           └─ …
```

Horizontal edges are events: durable, asynchronous, never park a session,
bounded by hops and rate. Vertical edges are promises: attached, joined,
one-shot, bounded by tree limits. The two never cross — a session never
parks on another bot, and a child never emits a bot event.

## Design

### 1. The bus: a `bot` trigger kind and `bot_emit { to, reply }`

A bot controls its inputs through triggers; nothing reaches an inbox
without one (webhooks need an ingest URL, the endpoint is operator-authed,
self-emission is a grant). Bot → bot keeps that rule: **the receiver
subscribes.**

```text
trigger kind: "bot"
spec: {
  from?: string[],          // sender bot names; absent = any bot in the universe
  addressedOnly?: boolean   // default true: only events with `to` == this bot
}
filter / route / coalesce / deliver: unchanged, and this is the point —
every knob a receiver has for webhooks it has for bots.
```

The common case is one trigger: `bot_trigger_put { name: "inbox", kind:
"bot" }` — accept events other bots address to me. "Watch what
`release-bot` publishes" is `{ kind: "bot", from: ["release-bot"],
addressedOnly: false }`. The create-bot dialog offers "accept events from
other bots" as a checkbox that creates the `inbox` trigger; there is no
implicit subscription.

`bot_emit` grows two fields and drops its self-only framing:

```text
bot_emit { kind, summary, data?, to?: botName, reply?: boolean, sessionKey? }
```

- `to` absent: **publish**. Admission finds every enabled `bot` trigger in
  the universe whose `from` admits the sender and whose `addressedOnly` is
  false, and admits one event per matching trigger through the normal path
  (filter, route, coalesce, breaker, store-then-wake). Zero receivers is
  fine; the sender's activity records `emitted <kind> → nobody`.
- `to` present: **addressed**. Same fan-out, restricted to that bot's
  triggers that accept the sender (addressed-only triggers match here;
  published-only ones do too). Zero receivers is a typed tool error the
  model reads: "`b` has no bot-kind trigger accepting events from `a`".
- Self-addressing (`to` == self, or `sessionKey`) is the existing
  self-emit behaviour through the same code: the sender is also a receiver
  when its own `inbox` trigger admits it. `source: "bot:self"` becomes
  `source: "bot:<sender>"` for every bot-originated event; "self" is a
  comparison, not a string.
- `reply: true` asks for the return path in §2.

The stored document (`BotEventDocumentV1`) gains `sender: { bot, seq?,
sessionId }`, `to?`, `causationId?`, `hops`, `reply?`; `bot_events` gains
the two columns queries need, `sender_bot_id` and `causation_id`. The
per-receiver event id is `bot:<senderBotId>:<digest(invocationId)>` — the
tool invocation id is stable across activity retries, so a retried emit
converges on one event per receiver (today's `self-<uuid>` does not).

Grant: `selfEmit` is renamed `emit` — it gates whether this bot may emit at
all, self or otherwise; receivers protect themselves with subscriptions.
The emission rate cap stays (breaker rate, else 60/hour) and counts every
emit by the sender, not only self-addressed ones.

**How a sender knows whom it can address** follows P134's answer for the
agent menu — a catalog in context, not a tool the model must remember to
call and not an enum in a schema — published through
[P136](p136-context-catalogs.md)'s external catalog so it never rewrites
the prefix of a session that lives for months:

- **One key, `bot:directory`.** Before a delivery, the controller derives
  the directory and puts it as `InputItem::Catalog` on
  `session/context/append`; a same-content put is a no-op. One line per
  bot in the universe — name, a one-line `description` (a new column — the
  brief is the job description *for* the bot, the description is what
  *other* bots see, the `whenToUse` line), enabled state, and its relation
  to the reader, **derived from the receivers' `bot` triggers**: accepts
  events addressed by me / subscribes to what I publish / not listening to
  me.
- **A change lands at the tail.** The engine keeps the previous version
  rendered byte-for-byte and appends the new one with a "supersedes"
  header; superseded copies are the first thing compaction drops and the
  current one survives it. A bot therefore learns of a neighbour at the
  tail of its context, at its next run, exactly as it learns of events
  and replies — and nothing in the prefix moves.
- **No run, no budget.** A directory change never wakes the bot; it is
  read when the next delivery does.

One declaration — the subscription — drives both routing and discovery, so
there is no sender-side allowlist to keep in sync (an optional `emit: { to:
[...] }` narrowing can be added if a deployment wants it). The universe is
the trust boundary and the directory carries no authority; the brief says
*when* to use a neighbour, the directory says *who* is there. Beyond ~50
bots the directory lists the listening ones and points at the UI.

### 2. Replies are deterministic: the delivery outcome is the return path

The question "can bot A ask bot B and get an answer" is where the P134
simplifications matter most. A joined `bot_ask` that parks A's session on
B is the wrong primitive: bots are singletons, so A→B→A parks both
forever; a parked lane blocks every other delivery to that session; and B's
latency is unbounded by design (coalescing windows, budget parking,
breakers, retention). Attached delegation earned its one-shot, depth-bounded
shape exactly to avoid this. So: **no parking, ever**. Replies are events.

But the reply must not depend on B's model remembering to emit one. The
controller already knows when a delivery finishes and what the model
decided (`bot_event_resolve { outcome, summary }`), so the reply is
deterministic:

- When B's controller finishes a delivery containing events with `reply:
  true`, it admits into each such event's sender a `bot.reply` event:
  `{ kind: "bot.reply", source: "bot:b", correlationId: <the original
  event id>, causationId: <the delivery id>, summary: resolution.summary ??
  status, data: { status: resolved | run_failed | unresolved, outcome?,
  summary?, runId, events: [#N…] } }`.
- When admission **drops** the event before any run — filter did not
  match, receiver disabled, breaker tripped, budget-parked bot disabled
  later — the drop itself replies: `data: { status: "dropped", reason }`.
  Cheap, immediate, and the single most useful feedback a model can get
  ("`b` filtered your event").
- **The event carries its own return address.** The emitting session is
  known at the tool invocation (`ExecuteBotToolInput.sessionId`), so the
  reply is routed to *that* session (main or keyed; generation rotation
  applies as for any routed target). The sender's controller tracks
  nothing; replies need no subscription trigger and bypass the sender's
  filters (you always hear back on your own asks). They coalesce per
  asking session under a fixed short window (debounce 5 s, max wait 30 s,
  max 20) so a fan-out ask comes back as one batch, and deliver with
  `whenBusy: queue`.

This gives `deferred` — a label with no mechanism today — its meaning: "I
asked and am waiting". Run 1 emits and resolves `deferred`; the reply
arrives as a new delivery to the same session; run 2 resolves `handled`.
Both bots' activity feeds show the whole exchange.

Reply timeouts (a `status: "timeout"` reply if nothing comes back within a
window) need sender-side outstanding-ask state and timers; deliberately
left for later, once hanging asks prove confusing in practice.

### 3. Loop bounds: causation, hops, rate

Provenance tagging plus per-receiver breakers rate-limit A→B→A cycles but
do not stop them (P131's own note). Two additions close that:

- Every bot-originated event carries `causationId` (the delivery the
  emitting session was handling, taken from the controller summary the
  tool activity already receives) and `hops` = the causing delivery's
  highest `hops` + 1 (0 for events from the world). The `bot_events` chain
  is a trace of the whole team's reaction to one external event.
- Admission cuts events with `hops > MAX_BOT_HOPS` (8, a universe
  constant to start) and records `loop_cut` on the sender. Together with
  the sender's rate cap and the receiver's breaker, a runaway exchange
  ends within one hop budget instead of one daily budget.

### 4. Influence without authority

How an "ops bot" works when no bot may configure another:

- It **subscribes** to what it cares about — replies with `status:
  dropped`, `loop_cut`, `blocked` outcomes, budget exhaustion — cheap for
  the controller to publish as events on the bus (a small, fixed
  vocabulary of `bot.*` system kinds, rendered like any event).
- It **asks**: `bot_emit { to: "comms", kind: "tuning.request", summary:
  "your incident filter admits every comment; suggest …", reply: true }`.
- The receiver **decides**. The event is untrusted data like every event;
  `comms`'s brief, written by a human, says "honour tuning requests from
  bot `ops`" — or does not. With `selfConfig`, `comms` applies the change
  itself and its activity feed records `self_configured` with the causing
  event, so the trail reads "ops asked, comms agreed". The reply tells
  `ops` what happened.
- The **hard stops stay deterministic**: the flood breaker, `runsPerDay`,
  poll auto-disable, and the hop cut act without any model in the loop,
  and re-enabling stays human. An ops bot that wants a neighbour stopped
  tells a human — through a reply, a Channels receiver later, or the UI —
  it does not pull the plug.

Each bot stays sovereign over its own configuration and the human stays
sovereign over every bot's scope. Coordination is by messages and
per-bot policy, which is also how teams of people work.

### 5. Alternative, not adopted: a `manage` grant

Recorded so the decision can be revisited with evidence rather than
re-derived. If a real use case demands cross-bot *authority* — an ops bot
that must act on neighbours without waiting for their briefs to agree, or
must stand up bots on demand — the grant is one document and covers both
halves at once, because they are the same question:

```text
manage: {
  bots: string[],                             // names this bot may configure
  ops: ("trigger" | "brief" | "enable")[],
  create?: { profiles: string[], maxBots: number }
} | null
```

- Configure: the `selfConfig` tools grow an optional `bot?` target;
  `bot_enable { bot, enabled }` is target-only (a bot never un-pauses
  itself); both feeds record `managed` / `configured_by`; the target's
  controller gets the config signal as after a UI edit.
- Create: `bot_create { name, profileId, brief?, runsPerDay?,
  acceptsBotEvents? }` with `profileId` ∈ `create.profiles` (the profile
  is the capability container, so the allowlist is the authority — the
  `features.subagents.agents[]` idea one tier up); attenuation fixed at
  one level (created bots get `selfConfig` / `emit` only if the creator
  has them, `runsPerDay` ≤ the creator's, `manage: null` always);
  `maxBots` counts *live* bots whose origin is the creator, because a bot
  is standing cost; `bots.origin = { botId, botName, seq, sessionId,
  runId }` as provenance never ownership (deleting the creator nulls it);
  management authority = `manage.bots ∪ { bots whose origin is me }`;
  deletion stays human.

Build it whole or not at all. Half of it — configuration without creation,
or the reverse — has no principled boundary.

### 6. Observability across bots

- Event chips: "from `a` #12", "addressed to `b`", "reply to #12".
- Sender activity: `emitted <kind> → b, c` / `→ nobody`; `loop_cut`;
  `replied`.
- Causation chain: from any event, walk `causationId` back to the external
  event and forward to every reply — the team's trace of one incident.
- The Bots page draws the wiring graph from the same `bot` trigger rows
  the directory is derived from (who listens to whom); later polish.

### 7. How it composes with sub-agents — the rules

1. **Horizontal is events, vertical is promises.** A session never parks on
   another bot. To get an answer with a bot's *context* (its inbox, brief,
   memory), emit with `reply: true` and resolve `deferred`. To get an
   answer *synchronously*, `agent_run` the bot's **profile** — the caller's
   `features.subagents.agents[]` may list it — and accept that the child
   has the profile only: no inbox, no brief, no bot tools.
2. **Children never emit.** `bot_*` tools are bindings on the bot's managed
   sessions; a child is created from the pinned profile without them. The
   bot session emits after `agent_run` returns. Every bot event's sender is
   a bot session, so provenance is never ambiguous.
3. **No cross-bot authority.** A bot reshapes itself within `selfConfig`
   and nothing else; humans set every bot's scope. Neighbours ask.
4. **Budgets stay in their tiers.** Run budget per bot, tree limits per root
   (P134), hop bound and emit rate per exchange. No cross-bot budget; an
   operator who wants a team ceiling sets each bot's `runsPerDay`.
5. **Provenance, never ownership**: `SessionOrigin`,
   `environment.origin_session`, and — if the alternative is ever built —
   `bots.origin`. No cascades, no lifecycle trees.
6. **Deterministic identities everywhere**: per-receiver event id from
   (sender, invocation), reply id from (receiver, delivery, original event
   id).
7. **A child is never a bot and a bot is never a child** (P134 §8) —
   unchanged; federation adds bot ↔ bot only.

## Worked example: an incident team

- `triage` — webhook triggers from Sentry and PagerDuty (`perKey` on
  incident id), `emit` granted. On an incident it publishes
  `incident.opened` and, if it wants a root cause, addresses
  `investigate` to `infra` with `reply: true`, resolving `deferred`.
- `infra` — `inbox` trigger (`from: ["triage"]`), profile with a standing
  `existing` environment and `features.subagents` (`log-reader`,
  `deploy-diff`). It `agent_run`s two children in one turn, joins their
  results, writes a finding, resolves `handled` with the finding as
  summary. The controller replies to `triage`'s incident session
  deterministically; `triage`'s next run posts the finding.
- `comms` — `{ kind: "bot", from: ["triage", "infra"], addressedOnly:
  false }` with filter `event.kind.startsWith("incident.")` and a 30 s
  coalescing window, so a burst of incident events becomes one Slack
  digest (once the Channels receiver exists; until then a webhook to
  Slack). `selfConfig` on; its brief says tuning requests from `ops` are
  to be applied when they narrow, never when they widen.
- `ops` — subscribed to `bot.*` system events (`dropped` replies,
  `loop_cut`, `blocked` outcomes). When `comms` floods, `ops` asks it to
  tighten its filter; `comms` does and replies; if the flood continues the
  breaker trips deterministically and a human re-enables. Both feeds show
  who asked, who agreed, and what stopped it.

Nothing in this example touches the core, no bot holds authority over
another, and every arrow is an event through admission or a promise
through P134.

## What is deliberately not built

- **Cross-bot authority** — no configuring, enabling, or creating another
  bot. The `manage` alternative in §5 is the whole of it, if ever.
- **`bot_ask` / any joined cross-bot call** — deadlock between singletons,
  lane starvation, unbounded latency. Replies are events (§2); synchronous
  answers are `agent_run` on a profile (§7.1).
- **Bot-per-entity spawning** — that is `perKey` routing inside one bot.
- **Cross-bot session or event access** — `bot_status`, `bot_event_read`,
  `bot_event_list`, `bot_trigger_list`, `bot_filter_test` stay self-only;
  a bot's outputs are its events.
- **A universe-wide message bus product** (topics, retention, consumers).
  Subscriptions are triggers; the "bus" is admission fan-out over trigger
  rows, nothing more.
- **Cross-universe federation.**
- **The A2A adapter** — later, as another emit target / subscription source
  behind the same `bot_emit` and `bot` trigger, not a new tool family.
- **Reply timeouts** — later, on evidence.

## Slices

1. **Bus** (1.5 d): `bot` trigger kind (spec, validation, UI form, create
   dialog checkbox), `bot_emit { to, reply }` with fan-out admission over
   trigger rows, envelope fields and the two columns, deterministic
   per-receiver ids, `emit` grant rename, the `description` column and the
   `bot:directory` catalog (P136 external catalog), `emitted` /
   `loop_cut` activity, `hops` cut, event chips. Migration replaces
   `self_emit` and adds the columns; dev databases reset.
2. **Replies** (1 d): controller-side reply admission on delivery finish,
   admission-side dropped replies, return-address routing to the asking
   session, per-session reply coalescing, `replied` activity, reply chip,
   the `bot.*` system event vocabulary published on the bus.
3. Later: reply timeouts, wiring view, A2A target;
   the `manage` alternative only on demonstrated demand.

1 → 2 in order, after P136 slice 4; each is independently shippable and dogfoodable (a two-bot
exchange is visible after slice 1, useful after 2).

## Tests

- **Unit** (`platform/bots/test`): subscription matching (published vs
  addressed, `from` allowlist, disabled trigger, self-subscription);
  deterministic per-receiver ids across a retried invocation; `hops`
  propagation and the cut; reply document shape for each status;
  directory rendering (relations derived from subscription rows) and the
  no-op put when nothing changed.
- **Integration** (`BOTS_TEMPORAL_INTEGRATION=1`): `a` addresses `b` with
  `reply: true` from a keyed session → `b` runs and resolves → the reply
  lands in `a`'s same keyed session as one delivery; `b`'s filter rejects
  → `a` receives `dropped: filtered` without a run on `b`; `a`→`b`→`a`
  ping-pong stops at `MAX_BOT_HOPS` with `loop_cut` on the sender;
  history replay for the controller changes.
- **Platform**: `test:migrations` asserts the new columns; `npm run check`
  and `check:identity` green; the bots integration suite keeps its nine
  scenarios.

## The rest of the bot list (not federation)

For completeness, what else "make bots really good" means, ordered by how
often it has already bitten:

1. **`bot_trigger_put` raw secrets** — P133 removes the field; until then
   webhook/poll secrets transit CAS and history.
2. **Descendant-aware budgets** — the bot budget is an activation budget;
   counting P134 descendants through `session/list { rootSessionId }` makes
   it a real one. Small.
3. **Tier-2 per-trigger CEL projections** — the generic renderer covers
   current needs; revisit when a preset is not enough.
4. **Declaration rotation cost** — every grant flip or tool revision rotates
   the main session. In-place add-only declaration admission in core is
   the one core change worth its price, still "decide after v1 contact".
5. **Email trigger, more presets, Channels bridge** — P131 ws2/4/5, parked
   or deferred by decision.
6. **Push transport for the UI** — long-poll everywhere; fine until it is
   not.
7. **Triage stage** — cheap model-side wake-vs-archive for ambient
   sources; still out until deterministic filters demonstrably fall short.
