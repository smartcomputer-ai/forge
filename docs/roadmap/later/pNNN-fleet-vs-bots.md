# Fleet vs Bots: One Delegation Story

**Status**

- Design review, 2026-08-24. Written from a full-codebase survey (core fleet
  control plane, engine primitives, API contract, platform usage, docs
  history P82–P92) triggered by the question: now that the product is
  coalescing around Bots (P130), should Fleet move out of core — or go?
- Recommendation at the end: remove the fleet control plane from core, keep
  the generic substrate it was built on, and re-grow delegation at the bot
  tier when a use case demands it. This is a decision doc, not a plan;
  slices follow once the direction is agreed.
- 2026-08-24, after review discussion: Lukas leans option C, with one
  correction that stands — the template objection (profiles are stamped
  many times; bots are named singletons; workflow tools require managed
  sessions, so bot-tier delegation can never serve ordinary or
  profile-templated sessions). Adopted direction is therefore **C′**: C's
  removals plus a slim, governed, profile-grantable delegation kernel in
  core. See "Refinement: the template objection". Bot federation
  (bot→bot events and cross-bot configuration) goes to the roadmap at the
  platform tier.
- 2026-08-24: Foundry retirement implemented as the first C′ slice: package,
  workers, routes, gateway exception, UI/CLI surface, release staging, and
  packs/releases storage removed. Fleet removal and the delegation kernel
  remain follow-up work.

## The two concepts

**Fleet** (P82–P85, P92): model-initiated, mid-run delegation. A session's
model calls `agent_spawn` (clone/fork/profile), `agent_send`/`agent_request`,
and parks on `await` until child runs resolve promises. Authority lives in
the model: it decides what to spawn, when to wait, what to do with results.

**Bots** (P130): a deterministic platform controller above sessions. Events
in, routing/coalescing/budgets/breakers applied by code, managed sessions
driven as workers, everything observable in an activity feed. Authority
lives in the controller; models reason inside the sessions it runs.

These are the same shape at different tiers — a bot controller *is* a
"manager agent" whose management layer is deterministic code instead of a
prompted model.

## What the survey found

**1. The engine is already clean.** Core's fleet-specific footprint is three
policy items: the `FleetFeature` config block, `FeaturesConfig.fleet`, and a
`fleet_policy` field on tool-batch requests. Everything fleets run on —
promises, `await`/`cancel`/`detach`/`sleep`, `RunTerminalNotifyIntent`, the
P100 emission spine, session clone/fork, the `session_links` table — is
generic substrate shared with workflow tools, environment jobs, timers, and
managed sessions. Environment jobs pull in the concurrency tools with no
fleet involvement. "Fleet primitives in the engine" turn out to be one
relationship string (`"fleet_child"`) written into a generic link table.

**2. The control plane is real but tenantless.** ~5,400 lines
(`temporal-server/src/fleet.rs` 4.3k, `tools/fleet` 1.1k) of well-built,
digest-deterministic spawn/send/request machinery — with exactly one
consumer: the Foundry manager profile, whose delegation is entirely
model-driven (no Foundry code ever observes or bounds the children it
prompts the model to create). Bots, Channels, CLI, eval, test-support, and
every fixture in the repo grant fleet zero times. Foundry is the frozen
proto-bot whose product future is Bots; when it goes, fleet's consumer count
is zero.

**3. The grant is exposed; the graph is not.** Clients can *enable* fleet
through config/UI/configurator-MCP, but `SessionView` has no parent/child/
lineage fields, the TS client has no link types, `session_links` is
unenforced ("nothing enforces them yet") and unexposed, `api-reference.md`
never mentions fleets, and the sessions UI TODOs the sub-agent tree. A user
can create a fleet and then cannot see it. The model sees more of the graph
than the human does.

**4. The safety layer was scoped and never built.** P92's follow-up
`p93-fleet-safety.md` (spawn budgets, tree/active-work index, tree
observability) does not exist — while the incident that motivated P92 (a
WhatsApp-bound assistant recursively spawning subagents, one child wedged
unclosable in `cancelling`) is exactly what it would have prevented. Foundry
grants `spawn.bases: ["self","session","profile"]` — everything, unbounded.
Meanwhile the bot tier *already has* the equivalents: run budgets, flood
breakers, self-emission caps, activity trail, capability grants
(`selfConfig`/`selfEmit`), generation rotation.

**5. Drift is shipping.** The `FleetFeature` description in seven generated
artifacts (OpenRPC, TS client, configurator, web reference) advertises
`agent_cancel`, which no longer exists, and omits `agent_request`, which
does. The README markets "a fleet of agents that build, test, and critique
your next feature"; the roadmap still describes the removed `agent_wait`.
P129 carries fleet-only special cases (`MessageBuffered` mailbox delivery,
steering-vs-await interactions, a cancel-grace open question).

**6. The one forward-looking use is outbound.** The A2A adapter idea
(`pNNN-a2a-protocol-adapter.md`) wants fleet's *vocabulary* —
spawn/send/read/cancel mapping onto `message/send`/`tasks/get`/`tasks/cancel`
— for calling remote agents. That is a platform adapter concern; it needs
the verbs, not the in-core control plane.

## The real tension

Not mechanism — the substrate composes fine. The tension is that Lightspeed
currently tells two stories for "more than one session's worth of work":

- *Fleet story*: trust the model to be the manager. Evidence: one incident,
  no observability, no budgets, no adoption.
- *Bot story*: deterministic controller manages; models reason. Evidence:
  the entire P130 arc, dogfooded, observable, budgeted, self-configuring
  within grants.

The industry converged the same way (the P130 research): every shipping
"ambient agent" product is a deterministic trigger/route/budget layer around
model sessions, not a model-run org chart. And the repo's own prior writing
already picked the winner: **"Bots own admission and lifecycle. Sessions own
reasoning"** (bots-alternative, adopted into P130's boundary).

What fleet *uniquely* offers and bots do not (yet):

1. **Mid-run structured concurrency** — spawn N, `await all/any`, join
   results inside one reasoning loop. Routed bot sessions are fire-and-
   forget; their terminals report to the controller, not into another
   session's turn.
2. **Clone/fork lineage** — children that share the parent's context.
   (Note: managed sessions are banned as clone/fork sources, so *bot*
   sessions could never use this half anyway; profile spawning is the only
   base that composes with bots today.)
3. A ready vocabulary for A2A.

(1) is the load-bearing one, and critically it is *not* fleet code — it is
the promise/await substrate plus P100 joined workflow tools, all of which
stay regardless.

## Options

**A. Status quo** — keep both, ship P93 someday. Cost: two delegation
stories forever, a headline capability with no consumer, contract surface
and UI cards to maintain, safety debt in the tier with the least
observability. Rejected: this is the option the last two months already
voted against.

**B. Bots adopt fleet as-is** — keep the core control plane, but expose
`features.fleet` only through bot profiles (a `selfDelegate` grant beside
`selfConfig`/`selfEmit`), bases restricted to `profile`, and implement P93's
budgets as bot policy. Cheapest path to "governed fleets"; but core still
carries a 5.4k-line control plane, the contract still exposes a grant whose
graph no client can see, and every fleet fix happens in the slow tier.
Viable fallback if mid-run delegation is needed *soon*.

**C. Remove the control plane; keep the substrate; re-grow at the bot tier
(recommended).**

Remove from core:
- `temporal-server/src/fleet.rs`, `tools/src/fleet/`, the `*_for_fleet`
  gateway entry points, fleet toolset derivation, and the fleet-only
  dispatch/dedup paths in `session_tools.rs`;
- `FleetFeature`/`FleetProfilesConfig`/`FleetSpawnConfig` from engine + api
  (+ regenerate the contract; breaking, and we are greenfield);
- the web/config-editor Fleet card, configurator exposure, profile
  reference entry;
- the `session_links` table and link store traits (fleet is its only
  writer; recreate under whatever concept next needs edges);
- the P129 fleet-only `MessageBuffered` special case if nothing else uses
  it after removal (audit at implementation time).

Keep in core (all generic, all load-bearing elsewhere):
- promises, `await`/`cancel`/`detach`/`sleep` (environment jobs and P100b
  joined tools depend on them);
- `RunTerminalNotifyIntent` + emission spine (bots' terminal tracking is
  built on it);
- session clone/fork + `source_session_id` lineage (real debugging value;
  cheap; also the P130 friction list wants branching someday).

Retire Foundry outright (decided 2026-08-24): bots supersede all of it, so
nothing folds — packs/releases tables, routes, workers, the
`foundryDirectTerminalToken` gateway special case, and the web surface all
delete. The manager-profile delegation story is covered by this doc's C′
kernel; `foundry_release_record` goes without replacement (a future
"deployments" platform primitive is the anticipated successor if
structured deployment outcomes are wanted again). Verified: core Rust,
including the workflow-tool plugin live suite, has zero Foundry
dependencies — the removal is platform-only.

When a real use case wants mid-run delegation again, build it where the
governance already lives: **bot-tier workflow tools** — e.g. `bot_delegate`
(joined, promise-backed via the existing P100 machinery): the model asks,
the controller spawns a managed profile session it fully owns — budgeted,
breaker-guarded, in the activity feed, retention-swept, visible in the bot
UI — and the reply resolves the parent's parked call. That is P93's safety
layer for free, at the tier that iterates weekly. The A2A adapter later
lands in the same place as another delegate target.

**D. Radical unification** — make "bot" the only nucleus: every session
belongs to a bot, fleets/Foundry/Channels all become bot policies. Directionally
where the product may end, but it forces a core rewrite (sessions without
controllers are the API's whole surface) for no near-term gain. Revisit
after bots have earned that gravity in production.

## Refinement: the template objection (C′, adopted direction)

Pure C has a hole: **profiles are Lightspeed's template mechanism — one
profile, many stamped instances — while bots are named singletons.** And
workflow tools exist only on *managed* sessions, so a bot-tier
`bot_delegate` can never serve an ordinary interactive session or a
profile meant to be instantiated many times. Within a bot the objection
does not bite (one bot's grant covers every session it routes); outside
bots it does: "wrap it in a bot" per spawning use case is the wrong
economics for a capability that wants to be a profile line.

Resolution: delegation stays a **profile-expressible core capability**,
rebuilt at roughly a quarter of the old control plane's size and governed
from day one:

- **One tool**: `delegate { profile, input }` — spawn an ordinary profile
  child (digest-deterministic ids as before), start its run, return a
  promise; the generic `await` joins it. No clone/fork bases, no
  send/read/list graph surface, no link table — children are plain
  sessions with `source_session_id` lineage.
- **The grant**: `features.delegation { profiles: allowlist, maxChildren,
  maxDepth, runBudget }`. Depth rides on the child session record so the
  engine enforces `maxDepth`; budget and child-count are grant-local.
  This is P93's safety layer folded into the capability instead of
  deferred behind it.
- **Observability is part of the kernel, not a follow-up**: lineage
  (`sourceSessionId`, depth) exposed on session views and the sessions UI,
  closing the "model sees more of the graph than the human" gap.
- **Nesting**: a child's profile may itself grant `delegation` — trees of
  templated agents, bounded by depth, cancelled by the existing generic
  cascade (run-scoped promises → child `CancelRun`).
- **Bots compose, not replace**: a bot's worker profiles may carry the
  grant, giving the bot fan-out with two budget layers (controller budget
  above, delegation budget below). `bot_delegate` as a separate workflow
  tool is superseded by this kernel.

What C′ changes about C's ledger: `agent_spawn`'s profile base and the
promise plumbing survive in miniature; everything else in the removal list
still goes. The A2A adapter later targets the same `delegate` verb with a
remote target instead of needing the old seven-tool vocabulary.

## Review notes (2026-08-24, agreed)

Two adjustments to C′, both accepted:

1. **Shrink `fleet.rs` in place; do not delete and regrow.** Simply put:
   the kernel keeps the hard half of fleet — deterministic child ids,
   starting the child workflow, run-terminal → parent-promise resolution —
   so removing it and rebuilding it later means rewriting tested code. What
   actually goes is the `self`/`session` (clone/fork) bases,
   `agent_send`/`agent_read`/`agent_list`, the link table, and the
   fleet-only dispatch paths. Do it as one refactor: rename `agent_spawn` →
   `delegate`, restrict bases to `profile`, delete the graph tools, add
   `maxDepth`/`maxChildren` to the grant, expose lineage. "A quarter of the
   size" holds for a subtraction, not a rewrite.
2. **Delegated children of bot sessions need an owner.** Simply put: a bot
   controller cannot see below its own sessions. Children delegated from a
   bot's worker session are ordinary sessions — outside the activity feed,
   budget, retention sweep, and breaker — and nothing closes them (the
   cancel cascade stops runs, not sessions). Reuse the P125 environment
   pattern: the child records `sourceSessionId` as provenance plus a
   default close trigger (close with parent), never ownership; the bot's
   retention sweep, budget, and UI walk lineage to include descendants.
   "Two budget layers" is only true once the upper layer can count the
   lower.

## Bot federation (platform tier, roadmap)

Wanted, cheap, and independent of the core kernel — neither item ever
needed fleet:

- **Bot → bot events**: `bot_emit` grows a `targetBot`; the event keeps
  `source: bot:<sender>` so provenance, loop caps, and the receiver's
  filters/breaker all apply unchanged.
- **Bot → bot configuration**: the `bot_trigger_put` / `bot_brief_put`
  family grows a target-bot form behind a new operator grant
  (`manageBots` — the `selfConfig` pattern pointed outward), enabling an
  ops-bot that tunes other bots without new machinery.

Tracked as a P131 workstream.

## Consequences of C

- **Wins**: one delegation story; ~5.4k lines and a whole contract feature
  gone; the README claim becomes honest ("bots run fleets of sessions" —
  which is what the code will actually do); the P93 debt dissolves into
  bot-tier guardrails that already exist; P129's fleet special cases can
  simplify; the engine loses its last product-flavored grant.
- **Losses**: mid-run `await`-join of *model-chosen* children until
  `bot_delegate` exists (no current consumer loses anything real); clone/
  fork loses its only caller (kept as a store capability); the A2A doc's
  "tools already exist" shortcut becomes "tools to be recreated at the
  adapter tier".
- **Risks**: none operational (nothing in production grants fleet).
  Foundry's fallback profile hard-requires the fleet grant, but Foundry
  retirement (decided, above) removes that consumer; sequencing is Foundry
  removal first, then fleet removal + kernel.

## Non-goals

- Deleting the promise/await/emission substrate — it is the best part of
  the fleet work and everything else stands on it.
- Rebuilding the seven-tool fleet surface. C′'s kernel is one verb plus
  the existing `await`; send/read/list graph tools return only if a real
  consumer demands them.
- An agent-type/manifest/graph system (docs/spec/03 stays deferred).
