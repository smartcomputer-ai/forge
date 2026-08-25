# P135 — Bot Federation: Events Between Bots, Bots Configuring Bots

**Status**

- Proposed 2026-08-26, recommendation-first design for discussion, not a
  plan. Written after P134 landed, from the question "now that sub-agents
  are simple, what makes bots a powerful coordination system — how do bots
  talk to each other, and can a bot create and set up another bot?"
- Absorbs P131 workstream 6 (bot → bot events, bot → bot configuration)
  and the "Bot federation" note in `later/pNNN-fleet-vs-bots.md`.
- Builds on P130 (controller, store-then-wake admission, `#N` events,
  CEL filters, routing, coalescing, breakers, `selfConfig` / `selfEmit`),
  P134 (the two-tier rule: durable orchestration above, attached
  delegation below; provenance not ownership; root-scoped attenuating
  limits; catalog menus), and the shared `@lightspeed/bots/config` path.
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
another bot, hear from one, or configure one. Every real deployment wants a
*team* — a triage bot that hands incidents to an infra bot, a comms bot that
narrates what the others decide, an ops bot that tunes filters and
breakers when a bot misbehaves — and the only way to build one today is to
route everything through webhooks the operator wires by hand.

The design insight that keeps this small is the same one that made P134
small: **do not add a transport**. Sessions already speak one protocol to
durable work that finishes later (a promise); bots already speak one
protocol to the world (an event through admission). Bot ↔ bot is events
through admission, with three additions the existing machinery lacks:
subscriptions, a deterministic return path, and a loop bound. Bot → bot
configuration is the existing `selfConfig` tools pointed outward behind a
grant. Bot creation is the same grant with a profile allowlist — the
`features.subagents.agents[]` idea one tier up — and is the one piece this
doc recommends deferring.

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

### 4. Cross-bot configuration: the `manage` grant

The `selfConfig` pattern pointed outward, as P131 ws6 sketched. One nullable
jsonb column on the bot record:

```text
manage: {
  bots: string[],                          // names this bot may configure
  ops: ("trigger" | "brief" | "enable")[]  // what it may do to them
} | null
```

- The existing tools grow an optional `bot?: string` (target name; absent
  is self): `bot_status`, `bot_trigger_list`, `bot_trigger_put`,
  `bot_trigger_delete`, `bot_brief_put`. `bot_enable { bot, enabled }` is
  new and **target-only** — self-enable stays human (a bot must not
  un-pause itself), but disabling a flapping neighbour is the core ops-bot
  move. `bot_list` (name, enabled, profile, whether it accepts bot events,
  first line of its brief) is always-on and read-only; the universe is the
  trust boundary.
- Execution: target == self → `selfConfig`; else target ∈ `manage.bots`
  and op ∈ `manage.ops`, re-checked on the fresh row (the same defense in
  depth as `selfConfig`). The declaration fingerprint changes when
  `manage` flips, so the existing rotation path applies it.
- Both sides record it: the manager's feed gets `managed b: put trigger
  x`, the target's gets `configured_by a: put trigger x`, and the target's
  controller receives the config signal exactly as after a UI edit.
- Still human-only everywhere: budget, breaker, retention, profile,
  credentials, grants, delete. Scope widening stays a human act.

This is the entire "orchestrate each other" surface for tuning. It needs
no new machinery because `@lightspeed/bots/config` is already the one code
path the API and the tools share.

### 5. Bots creating bots — designed, recommended deferred

The grant shape is ready and mirrors `features.subagents`:

```text
manage.create?: { profiles: string[], maxBots: number }

bot_create { name, profileId, brief?, runsPerDay?, acceptsBotEvents?: boolean }
```

- `profileId` ∈ `create.profiles` — the profile is the capability container
  (tools, environment intent, sub-agent grant), so allowlisting it is
  exactly as safe as allowlisting an `agent`. Rendered to the model the way
  the sub-agent menu is: a catalog, never an enum in the schema.
- **Attenuation, fixed at one level**: a created bot gets `selfConfig` /
  `emit` only if its creator has them, `runsPerDay` ≤ the creator's, and
  `manage: null` always — no transitive creation (the `maxDepth: 1` of this
  tier, hard-coded because nothing has asked for more).
- **Count-bounded**: `maxBots` counts live bots whose origin is the creator.
  A bot is standing cost (schedules and pollers burn money unattended),
  which is why the bound is on *existing* bots, not lifetime creations.
- **Provenance, not ownership**: `bots.origin = { botId, botName, seq,
  sessionId, runId }`, set at creation, never changed; the UI shows
  "created by `ops` from event #12". Deleting the creator nulls the
  reference and leaves the bot. Management authority is
  `manage.bots ∪ { bots whose origin is me }`, so a creator can tune and
  disable what it created without listing names it did not know in
  advance; deletion stays human.
- `acceptsBotEvents` creates the `inbox` trigger so the creator can address
  the bot it just made.

Why defer: no current use case needs it, "wrap it in a bot" per entity is
wrong (perKey routing already gives one session per PR, customer, or
incident inside one bot), and the human-creates-the-shell workflow — one
click for name + profile, then the ops bot fills in triggers and brief
through §4 — keeps the P130 trust line ("anything that widens scope goes
through a human") intact with almost no UX cost. Build it when an ops-bot
is actually asked to set up bots on demand; everything it needs is
specified above.

### 6. Observability across bots

- Event chips: "from `a` #12", "addressed to `b`", "reply to #12".
- Sender activity: `emitted <kind> → b, c` / `→ nobody`; `loop_cut`;
  `replied`.
- Causation chain: from any event, walk `causationId` back to the external
  event and forward to every reply — the team's trace of one incident.
- Later: a wiring view on the Bots page derived from `bot` triggers (who
  listens to whom), and a bot directory as a refreshed context entry (the
  `subagents.catalog` pattern) once `bot_list` proves too pull-shaped.

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
3. **Budgets stay in their tiers.** Run budget per bot, tree limits per root
   (P134), hop bound and emit rate per exchange. No cross-bot budget; an
   operator who wants a team ceiling sets each bot's `runsPerDay`.
4. **Provenance, never ownership**, in all three places it now appears:
   `SessionOrigin`, `environment.origin_session`, `bots.origin`. No cascade
   deletes, no lifecycle trees, no parent controlling a child's runs.
5. **Deterministic identities everywhere**: per-receiver event id from
   (sender, invocation), reply id from (receiver, delivery, original event
   id), created bot name chosen by the model but unique per universe.
6. **A child is never a bot and a bot is never a child** (P134 §8) —
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
  Slack).
- `ops` — `manage: { bots: ["triage", "infra", "comms"], ops: ["trigger",
  "enable"] }`, subscribed to `bot.delivery.resolved`-style events with
  outcome `blocked` or to `loop_cut` (both cheap to publish from the
  controller). When `comms` floods, `ops` tightens its filter or disables
  it, and the activity feeds on both sides show who did what.

Nothing in this example touches the core, and every arrow is an event
through admission or a promise through P134.

## What is deliberately not built

- **`bot_ask` / any joined cross-bot call** — deadlock between singletons,
  lane starvation, unbounded latency. Replies are events (§2); synchronous
  answers are `agent_run` on a profile (§7.1).
- **Bot-per-entity spawning** — that is `perKey` routing inside one bot.
- **Ownership trees, cascades, transitive creation** — provenance only,
  depth fixed at one.
- **Cross-bot session or event access** — `bot_event_read`, `bot_event_list`,
  `bot_filter_test` stay self-only; a bot's outputs are its events. `bot_status`
  and `bot_trigger_list` on a target exist only under `manage`.
- **A universe-wide message bus product** (topics, retention, consumers).
  Subscriptions are triggers; the "bus" is admission fan-out over trigger
  rows, nothing more.
- **Cross-universe federation.**
- **The A2A adapter** — later, as another emit target / subscription source
  behind the same `bot_emit` and `bot` trigger, not a new tool family.
- **Reply timeouts and a bot directory context entry** — later, on
  evidence.

## Slices

1. **Bus** (1.5 d): `bot` trigger kind (spec, validation, UI form, create
   dialog checkbox), `bot_emit { to, reply }` with fan-out admission over
   trigger rows, envelope fields and the two columns, deterministic
   per-receiver ids, `emit` grant rename, `emitted` / `loop_cut` activity,
   `hops` cut, event chips. Migration replaces `self_emit` and adds the
   columns; dev databases reset.
2. **Replies** (1 d): controller-side reply admission on delivery finish,
   admission-side dropped replies, return-address routing to the asking
   session, per-session reply coalescing, `replied` activity, reply chip.
3. **Manage** (1 d): `manage` column and settings UI, `bot?` on the five
   tools, `bot_enable`, `bot_list`, two-sided activity, config signal to the
   target.
4. **Create** (1 d, **deferred**): `manage.create`, `bot_create`, `bots.origin`,
   attenuation and `maxBots`, provenance in the bot detail.
5. Later: reply timeouts, wiring view, directory catalog entry, A2A target.

1 → 2 → 3 in order; 4 only on demand; each is independently shippable and
dogfoodable (a two-bot exchange is visible after slice 1, useful after 2).

## Tests

- **Unit** (`platform/bots/test`): subscription matching (published vs
  addressed, `from` allowlist, disabled trigger, self-subscription);
  deterministic per-receiver ids across a retried invocation; `hops`
  propagation and the cut; reply document shape for each status; manage
  authorization (self vs target, op allowlist, origin-derived authority);
  attenuation on create.
- **Integration** (`BOTS_TEMPORAL_INTEGRATION=1`): `a` addresses `b` with
  `reply: true` from a keyed session → `b` runs and resolves → the reply
  lands in `a`'s same keyed session as one delivery; `b`'s filter rejects
  → `a` receives `dropped: filtered` without a run on `b`; `a`→`b`→`a`
  ping-pong stops at `MAX_BOT_HOPS` with `loop_cut` on the sender;
  `ops` puts a trigger on `b` → `b`'s controller reconciles and both feeds
  record it; history replay for the controller changes.
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
   the main session. If federation lands, the flips get more frequent
   (`emit`, `manage`); in-place add-only declaration admission in core is
   the one core change worth its price, still "decide after v1 contact".
5. **Email trigger, more presets, Channels bridge** — P131 ws2/4/5, parked
   or deferred by decision.
6. **Push transport for the UI** — long-poll everywhere; fine until it is
   not.
7. **Triage stage** — cheap model-side wake-vs-archive for ambient
   sources; still out until deterministic filters demonstrably fall short.
