# P141 — Bots as the product: console, wizard, setup

**Status**

- Proposed and implemented 2026-08-27 (uncommitted on `bot-envs`). A
  product/UX redesign of the bot surface in `platform/web`, plus two small
  server changes: `POST /bots` takes `triggers` (created with the bot, all
  rolled back on a refusal) and the bot list carries `pendingCount` and
  `lastEvent` from `bot_events`. No schema change. Profile ownership was
  deliberately not tracked (§6): the wizard creates a profile named after
  the bot and references it, and that is all. Nothing here touches the engine.
- Builds on P130 (controller, triggers, `#N` events, `selfConfig`/`emit`),
  P134 (sub-agent lineage), P135 (bot ids, inbox trigger, directory), P139
  (chat triggers), P140 (bot environment, close/delete).
- Keeps P130's decision (2026-08-24) that operator messages are ordinary
  client runs on the managed session; the bot page makes that the default
  instead of an override (§5).

## 1. The read

Today the bot surface is organised around the runtime's concepts. The list
shows a profile id and a trigger count. The detail page shows "controller
health", "inbox now", raw session ids with a Reset button, and a triggers
section. Everything editable sits in a dialog behind a gear. The profile —
the bot's brain — lives on another page and must exist before the bot.
Talking to the bot means going to Sessions, finding the managed session,
and flipping a switch that warns you it bypasses the manager.

Every piece is correct. The seams are in the wrong places for a person who
wants to hire a bot, give it a job, and check on it.

The person's mental model is a **colleague with a job**:

| The person thinks | The system has |
| --- | --- |
| who it is | `botId`, `displayName`, `description` |
| its job | `brief` (+ profile instructions) |
| what it can use | profile `config` (model, features) |
| where it works | profile `environment` |
| when it wakes | `bot_triggers` |
| its conversations | main / keyed / per-event sessions, sub-agents |
| what it did | `bot_events` with outcomes, controller state |
| its limits | `runsPerDay`, breaker, TTL, `selfConfig`, `emit`, inbox |

And three things you do with a colleague: **talk** to them, **watch** what
they are doing, **set up** their job. That is the whole information
architecture.

## 2. Principles

1. **Talk · Watch · Set up.** The bot page has exactly three tabs: Chat,
   Activity, Setup. Nothing about a bot lives anywhere else.
2. **The bot is self-contained.** Its setup (instructions, capabilities,
   environment) lives on the bot by default. Profiles remain the reuse
   mechanism for power users and for sub-agents, not a prerequisite.
3. **Forms and chat edit the same record, and each sees the other's
   changes.** The Setup tab reflects what the bot did through
   `bot_trigger_put`; a form save reaches the bot at its next boundary.
4. **Complexity behind named seams**, not hidden: wizard vs Setup; Basics
   vs Advanced per trigger; own setup vs shared profile; console vs
   channels; timeline row vs payload.
5. **Speak the person's language in labels; keep the API vocabulary.**
   `runsPerDay` is "Daily run limit"; `session_busy` is "Working".
6. **Every screen answers "is it working?" first.** Status, then what it
   is doing now, then what it did, then how it is configured.

## 3. The bot page

Header: a deterministic face colour from the id, display name, `botId` in
mono, a status word (Starting / Idle / Working on #48 / Paused / Out of
budget until 00:00 UTC / Needs attention / Closed), and Pause/Resume. Close
and Delete live in Setup › Danger zone.

Routes:

```
/u/:slug/bots                          list
/u/:slug/bots/new                      wizard (full page)
/u/:slug/bots/:botId                   Chat, main conversation
/u/:slug/bots/:botId/chat/:sessionId   Chat, a thread or sub-agent
/u/:slug/bots/:botId/activity
/u/:slug/bots/:botId/setup[#section]
```

### 3.1 Conversations as tabs

Revised after the first live look (2026-08-27): a conversation column
under the bot's tabs was a third navigation level and most bots have one
or two sessions, so conversations *are* the tabs. The tab row reads
`Main · PR-912 · PR-907 · +3 ▾ │ Activity · Setup`: Main and the three
most recently active threads inline, the rest — older threads and
sub-agents under their parent — behind `+N`; the selected conversation is
always inline, so a deep link never lands in the overflow. With one
session the row is `Main · Activity · Setup`.

The transcript takes the full width: `SessionDetail` embedded renders no
header of its own (no "Managed by" badge, no second title). What that
header carried sits in a chevron menu on the active tab — session id,
copy id, open on the Sessions page, reset, then overflow-safe metadata as
the final read-only section. The
composer sends a plain message to that session (§5); sub-agents of the
open conversation also appear in the lineage strip above the transcript.

The standalone Sessions detail uses the same identity-and-actions menu.
Its bar keeps only the display name, live state, lifecycle state, and
management badge; copy id, settings, close/delete, and metadata move into
the menu. A bot-controlled session also links directly back to that exact
conversation on the bot page, using its lifecycle-controller ownership
rather than guessing from a session-id prefix.

### 3.2 Activity

One live timeline, newest first. A row is one line —
`#48 · GitHub PR opened · acme/api#912 → PR-912 → handled — "Reviewed and
left 3 comments"` — with outcome chip, trigger chip, and time; expanded,
it shows the payload, the run, usage (cached %), the session link (which
opens Chat at that thread), and Replay. Coalesced batches render as one
row with N events inside.

A strip above the timeline answers "now" and "today": buffers with their
flush countdown, active deliveries, pending count; runs used / limit,
sub-agents, last error. Filters: trigger, outcome, time. "Send a test
event" moves here — it is a developer tool, not a header action.

### 3.3 Setup

A stack of collapsible sections, each reading as one line while closed —
"50 runs a day · flood 20/10 min · can change own triggers · inbox: any" —
so the page at a glance is a summary and you open only what you are
editing (Brief and Triggers start open; `#triggers` deep links open their
section). The first version had a left anchor nav; it was another level
of chrome and went. Every section saves on its own.

- **Identity** — display name, id (immutable, mono), description (what
  other bots read), face colour.
- **Brief** — the job description, markdown, large. An "Ask ⟨bot⟩ to
  rewrite this" link opens Chat with a drafted message.
- **Triggers** — one card per trigger: kind icon, plain-language summary
  ("Weekdays 09:00 Europe/Berlin", "GitHub webhook · pull_request",
  "Telegram · @acme_bot · direct messages"), state with reason when paused
  (breaker, poll failed, one-shot fired, operator, bot closed), last fired,
  and actions: Fire now / Send sample, Pause, Edit. The editor is today's per-kind form split into
  **Basics** and an **Advanced** disclosure (filter, routing, coalescing,
  when busy, thread retention) whose closed state shows a sentence: "One
  thread per pull request · waits up to 2 min to batch · queues when
  busy".
- **Session profile** — the profile selector, base instructions, model and
  reasoning effort, model run controls, capability toggles, environment
  selection/provisioning, metadata, and automatic deletion. Environment
  intent lives inside the Environment capability, with the selected
  environment's power and idle-policy controls beside it. One save updates
  the profile document; shared-profile warnings and the Profiles-page link
  remain at the top.
- **Other bots** — both directions of bot-to-bot messaging in one place,
  because that is one topic to a person even though it is a grant plus a
  trigger to the system: "Can message other bots" (`emit`) and "Accepts
  messages from: nobody / any bot / only …" (the `bot`-kind inbox trigger,
  created and edited through the trigger API; "Routing & batching…" opens
  the ordinary trigger editor). The Triggers section hides the inbox and
  its picker omits "Other bots" so the record has one surface. Two earlier
  placements — inbox under Guardrails, and inbox only under Triggers with
  `emit` under Guardrails — split the topic and were replaced.
- **Guardrails** — Daily run limit, Flood protection (breaker), Thread
  retention (routed TTL), "Can change its own brief and triggers"
  (`selfConfig`).
- **Danger zone** — Close, Delete, with today's copy.

## 4. Creating a bot: the wizard

A full page (`/bots/new`), five steps, a summary card on the right that
fills in as you go, every step skippable with defaults. It creates the bot
**and** its own setup in one go; "Use a shared profile" is an option on
step 4.

1. **Job** — Start from a template (Blank · Pull-request reviewer · Daily
   digest · Chat assistant · On-call responder · Repository watcher; each
   prefills brief, triggers, capabilities), name (id derived, editable
   until created), brief.
2. **Wake-ups** — multi-pick: Schedule (cron builder), Webhook (GitHub
   preset; the URL appears after creation), Chat account, Poll, Other
   bots, "None yet" (you can always message it). Each pick adds a mini card with
   the kind's essentials; filter/route/coalesce take per-kind defaults.
3. **Session profile** — choose an own or shared profile. An own profile
   exposes base instructions, model and run controls, toggles, environment
   choice, metadata, and automatic deletion in the same editor used later.
   Readiness remains inline: no model credential → link to Integrations; no
   environment → link to Environments; no messaging account → who to ask.
4. **Other bots** — choose whether it may send to other bots and which bots
   may address it.
5. **Guardrails** — daily run limit (default on), "can change its own
   brief and triggers" (default on: chat-to-configure is the point),
   "can message other bots" (off). Create.

Landing: the bot page, Chat, Main — and the wizard sends the first
message: "You were just created. Introduce yourself in two
sentences and confirm your setup — triggers, tools, environment. Ask about
anything unclear." The reply is the smoke test (credential, environment,
tools) and the first moment the bot feels like a colleague.

## 5. Chat: the composer is a client run, not an event

Talking to a bot is a conversation, not an event. The bot page's composer
sends `POST /sessions/:id/messages` on Main (or on the thread being
viewed) — the path today's "Direct input" override uses — without the
switch and without the warning. Nothing new is routed, numbered, or
resolved.

What the engine already gives us: runs on one session serialise, so an
operator message queues behind an active delivery and a delivery queues
behind an open chat run; they never interleave inside a turn. The
controller's lane accounting does not see chat runs, and the only effect
is a delivery queuing behind the operator's message in the engine — which
is what a person expects.

What we forgo by not admitting chat as an event, and why that is fine:

- no `#N` and no Activity row — the conversation lives in Chat;
- not counted against the daily run limit — a person typing is not the
  flood the limit exists for;
- no per-message outcome — the bot should not have to `bot_event_resolve`
  a reply to its operator;
- controller status reads `idle` while a chat run is open — the header
  derives "Working" from the session's run state as well.

Two small changes, neither architectural: one sentence in the bot's
standing instructions — plain messages that are not headed "event #N"
come from the people who manage the bot and are to be followed — and the
Sessions page's "Direct input" switch becomes plain input on bot-managed
sessions (or keeps a milder note; the bot page never shows it). Who may
message a bot is a check on the messages route (managers by default).

The self-configuration loop needs nothing new either: `bot_trigger_put`
and `bot_brief_put` already exist under `selfConfig`. The Setup tab must
refetch triggers and the bot record when the 3 s state poll sees a change
(today the triggers query is separate and would show the bot's edit only
on reload). Provenance on trigger cards ("added by Triage") would need an
`origin` column; it is a later nicety, not part of this proposal.

## 6. The bot's profile

The wizard writes a profile named after the bot (`profileId = botId`,
description "Setup of bot ⟨id⟩") and the bot references it — nothing on the
bot row records that, and nothing cleans it up: the point is that a bot can
be created with its setup in one flow, not that the platform tracks
ownership. Setup › Session profile edits whatever profile the bot references
through the ordinary `PUT /profiles/:id` route, overlaying only changed fields
onto the latest revision; when other bots use the same profile the section
says so. "Use a shared profile" is the wizard's other path, and Setup can
switch profiles later. A `bots.setup` document on the row (P130's original
`inline` sketch) and an ownership column were both considered and dropped:
they buy a tidier Profiles list at the cost of a second document model.

## 7. The bot list

A roster, not a table: face, name, status dot, one line of "what it is
doing" (active delivery summary → last outcome → "Waiting for its first
event"), a pending badge, last active; grouped Active / Paused / Closed,
sorted by activity. The list endpoint gains `lastOutcome` and
`inFlightCount` from `bot_events` alone (null outcome + delivery id =
in flight), so no Temporal query per row.

## 8. Language

| API | Label |
| --- | --- |
| `idle` / `session_busy`, `delivering_event` / `budget_exhausted` / `degraded` / `initializing` | Idle / Working / Out of budget / Needs attention / Starting |
| main session / keyed & per-event sessions | Main / Threads |
| `runsPerDay` / `breaker` / `routedSessionCloseAfterMs` | Daily run limit / Flood protection / Close inactive threads |
| `selfConfig` / `emit` / inbox trigger | Can change its own brief and triggers / Can message other bots / Accepts messages from other bots |
| Settings › Setups | Templates (frees "Setup" for the bot tab) |

## 9. Backend changes

| Change | Why | Size |
| --- | --- | --- |
| `POST /bots` accepts `triggers`, created in order and rolled back together | §4 | small — done |
| Bot list carries `pendingCount` and `lastEvent` from `bot_events` | §7 | small — done |
| Sample payloads for token-verified webhooks ("Send sample" on the card) | §3.3 | UI only — done |
| Templates | §4 | static in web — done |
| `POST /bots/reconcile { profileId }` signals every open bot on a profile; Setup and the Profiles page call it after a save, so a profile edit reaches its bots at their next idle moment instead of at the next unrelated config change | §6 | small — done |
| Controller rotates Main when a new profile revision cannot be applied in place: `ensureBotSession` compares the session's pinned provider api kind with the profile's before applying (the engine's `ProviderCompatibility` rejection is the backstop) and reports `BOT_SESSION_PROFILE_UNAPPLICABLE`, which joins the declaration-mismatch rotation path. The previous Main is left open, as it is for a declaration rotation | §6 | small — done |
| Fire-now for schedules | §3.3 | not built; needs a route over Temporal's trigger-immediately |

Everything else is `platform/web`.

## 10. Phasing

Built 2026-08-27, in one pass:

- `platform/web`: routes `/bots/new`, `/bots/:id`, `/bots/:id/chat/:sid`,
  `/bots/:id/activity`, `/bots/:id/setup`; `BotsPage` roster
  (`rosterLine`), `components/bot/detail.tsx` shell, `chat.tsx` (conversations
  + embedded `SessionDetail` with `embedded`/`sessionHref` props, the
  introduction message), `activity.tsx` (strip, filters, timeline, test
  event, replay), `setup.tsx` (seven sections, per-section saves, unified
  session-profile switcher, inbox as a guardrail), `triggers.tsx` (cards with
  `trigger-summary.ts` sentences, `DeliveryFields` behind an Advanced
  disclosure, `ScheduleFields`, `TriggerKindPicker`, `TriggerKindFields`,
  `triggerCreateBody` for the wizard, "Send sample"), `BotCreatePage.tsx`
  (five steps, summary rail, `templates.ts`), `face.tsx` (deterministic
  colour), `status.tsx` (person-facing status words). The create-bot and
  settings dialogs are gone; Settings › Setups is labelled Templates.
- Not built: fire-now for schedules; an "Ask the bot" affordance; a
  mobile pass beyond the responsive layout; hiding bot threads on the
  Sessions page.

## 11. Open questions

- May members who cannot manage the bot message it? Today the bot page
  is manager-only, like Sessions.
- Profiles in the main nav, or under a Library group with Workspaces now
  that a bot's profile is made with the bot?

## 12. Deliberately not proposed

No second lifecycle authority; no admission of operator chat as events
and no `console` trigger kind (§5); no audit table (transcripts suffice);
no bot-to-bot graph view; no bot-scoped provisioning
(P140); no bot-editable capabilities; no per-bot profile rows cluttering
the Profiles list once §6 lands.
