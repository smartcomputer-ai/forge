# Lightspeed Bots: a concrete product and architecture proposal

My strongest recommendation is to define a **Bot** as:

> **A revisioned, durable event router that owns a session topology and turns selected windows of world events into safe Lightspeed run admissions.**

That gives you a clean product equation:

**Sources observe. Bots decide. Sessions think. Tools and Flows act.**

A Bot should **not** be a more complicated session, a generic workflow builder, or a bundle of vendor integrations. It should sit above those systems and own the missing lifecycle:

```text
world events
    ↓
durable admission
    ↓
filtering, grouping, timing, activation, routing
    ↓
one or more Lightspeed sessions
    ↓
tools, workflow tools, Flows, Fleets, environments
```

The key strategic answer to your hardest question is:

> **Hardcode the platform contract, not the integrations.**
>
> Begin with several hardcoded integrations to prove the product, but make them implementations of a generic Source contract from the start. Let agents configure certified Sources early. Let agents generate new connector code only later, as reviewed, sandboxed, versioned artifacts—not as uncontrolled production code.

---

## 1. Why Bots fit the architecture you already have

The repository has already established nearly all the session-side primitives Bots need:

* A deterministic, event-sourced session core.
* Managed sessions with immutable lifecycle ownership.
* Workflow tools with fixed, trusted receivers.
* Push, pull, joined, and explicit-Promise completion semantics.
* Run admission, queuing, active-run steering, cancellation, keyed context, and terminal notifications.
* A clear rule that plugin-specific workflows must not be compiled into the stable session worker.

The P100/P100b/P106 sequence is especially relevant. It has already separated stable transport from application meaning: workflow plugins can declare typed tools, receive invocations in their own workflows, start workflows dynamically, and resolve durable results without teaching the session worker what “messaging,” “approval,” “deployment,” or any other domain means.

`session/managed/start` gives the controller an immutable relationship to its session, while `session/runs/start`, `session/runs/steer`, and `session/context/append` provide the important admission modes.

What is missing is the opposite direction:

```text
existing system:
session → workflow/controller/plugin

missing Bots system:
world → controller → session
```

That is why I would add Bots to the **platform/control-plane side**, not to `engine` and probably not initially to the main Rust session worker.

### Channels is already a specialized proto-Bot

The current Channels workflow is very close to a first implementation of the Bot pattern. It durably owns a managed session, validates and queues incoming signals, deduplicates inbound events, applies an activation policy, groups messages, uses debounce and maximum-wait windows, starts runs with deterministic identities, handles workflow-tool invocations, and reconciles run termination.

Its batching rule is particularly telling:

```text
flush at the earlier of:
- first event + maximum wait
- latest event + debounce interval
```

It also flushes at a maximum batch size, which prevents unbounded buffering.

I would **extract a reusable Bot control-loop library from Channels**, rather than immediately attempting to turn all Channels behavior into generic abstractions. The reusable portion is:

* durable inbox handling;
* event identity and deduplication;
* bounded queues;
* grouping and windows;
* activation decisions;
* managed-session ownership;
* deterministic run submissions;
* terminal reconciliation;
* self-receiver workflow-tool processing;
* continue-as-new state compaction.

Telegram presentation, room activation, typing indicators, media preparation, replies, and provider fallback remain Channels-specific.

### Work is complementary, not the Bot abstraction

The proposed `AgentWorkWorkflow` owns an objective and keeps running execution cycles until the agent explicitly reports complete or blocked. Its key distinction is:

```text
Run finished  ≠  objective achieved
```

That is useful, but it solves a different axis. It appears to remain a design rather than an implemented product workflow in the current checkout.

The eventual relationship should be:

* **Bot:** when something deserves work, where it goes, and how inputs are grouped.
* **Run:** one ordinary reasoning/execution cycle.
* **Work:** an optional execution policy that repeats cycles until a semantic objective disposition.
* **Flow:** deterministic preprocessing or orchestration.
* **Session:** context, reasoning, tools, promises, and execution state.

A Bot route could eventually say `execution: run` or `execution: work`, but Work should not be the foundation of Bots.

---

## 2. What the product frontier is converging toward

Across current agent products, four layers are becoming distinct:

1. **Persistent agent identity and context**
2. **Triggers or sources**
3. **Deterministic workflow/control logic**
4. **Agentic reasoning and action**

The public descriptions of Grok Bot emphasize a persistent cloud environment, multi-step work, approvals, coordination between Bots, and reusable routines that can later be rerun on demand or on a schedule. OpenAI’s current workspace-agent and Codex automation surfaces similarly combine long-running cloud agents, tools, memory, schedules, deployment surfaces, approvals, and centralized governance. ([The Verge][1])

Zapier’s 2026 move is an especially useful signal. It is folding standalone Agents into its main workflow editor so an agentic step can compose with ordinary triggers, filters, branches, approvals, typed inputs, run history, and deterministic steps. That suggests the market is learning that an agent should be a powerful unit **inside** a durable automation rather than the entire automation system. ([help.zapier.com][2])

Pipedream and Composio make another useful separation:

* A **trigger type** describes a category of event and its schemas.
* A **trigger/source instance** is one configured subscription or poller attached to a user connection.
* Actions and triggers are related through a connector, but they are distinct runtime contracts.
* Provider differences such as polling, webhook registration, retries, signing, authentication, and renewal are hidden below the event interface. ([Composio][3])

Lightspeed has a particularly strong opportunity here because its sessions already have much better continuity and durability than most workflow products’ stateless “AI steps.” The defensible product is not simply “we have 1,000 triggers.” It is:

> **An event can wake the right long-lived agent context, at the right semantic boundary, with durable control over what happens next.**

---

# 3. The Bot product object

A Bot should be a first-class, named, shared, revisioned object. Internally, I would model roughly these sections:

| Surface      | Responsibility                                                                   |
| ------------ | -------------------------------------------------------------------------------- |
| Identity     | Name, description, owner, published revision, status                             |
| Purpose      | Instructions, profile, optional objective or operating policy                    |
| Sessions     | Persistent primary session and optional keyed or ephemeral session factories     |
| Sources      | Schedules, webhooks, provider events, pollers, internal Lightspeed events        |
| Activation   | Filter, partition, debounce, maximum wait, batch limits, optional semantic judge |
| Routing      | Which session receives an activation and how it is admitted                      |
| Capabilities | Connections, workflow tools, MCPs, Flows, environments, approval policies        |
| Limits       | Concurrency, run rate, token/spend budgets, inbox limits, quiet hours            |
| Activity     | Complete event-to-decision-to-run trace, failures, replay, and explanations      |

The normal creation experience can remain extremely simple:

```text
Name
What should this Bot do?
When should it run?
Which profile and connections may it use?
```

The advanced product surface reveals the machinery.

## Product vocabulary

I would use these terms consistently:

* **Connector:** package describing one integration family, such as GitHub.
* **Connection:** authenticated access to one account or installation.
* **Trigger type:** `github.issue.changed`.
* **Source:** one configured trigger instance, such as changes in a particular repository.
* **Event:** one durable fact emitted by a Source.
* **Activation:** the decision that one or more events now deserve processing.
* **Route:** the policy mapping an activation to a session and admission mode.
* **Bot:** the durable owner of Sources, activation state, routes, and sessions.

This avoids overloading “trigger” to mean the provider capability, configured subscription, received event, and agent run.

---

# 4. Events should be facts, not chat messages

Do not convert every provider event immediately into a natural-language user message.

The Bot should first admit a structured event envelope, with the raw provider payload stored separately:

```text
BotEvent
  id
  sourceId
  triggerType
  triggerInstanceId
  triggerGeneration
  subject
  occurredAt
  receivedAt
  providerEventId
  dedupeKey
  partitionKey
  schemaRef
  payloadRef
  principalRef
  traceContext
```

A CloudEvents-like vocabulary is useful here because it standardizes minimal routing metadata such as event ID, source, type, subject, time, and schema while deliberately avoiding an opinion about the processing model. I would adopt the useful semantics rather than requiring every connector to expose literal CloudEvents documents. ([GitHub][4])

This mirrors Lightspeed’s provider philosophy:

* Parse and normalize only what the control plane needs.
* Keep provider-native detail opaque and CAS-backed.
* Do not invent a vast universal model of GitHub issues, emails, Slack messages, alerts, and Linear tickets.

The Bot can later materialize an activation input such as:

```text
The following 14 GitHub events were grouped for issue #314:
- label "customer-bug" added
- 3 comments added
- assignee changed from Alice to Bob
- latest CI run failed

The complete structured event batch is available through event tools.
```

That is a product presentation, not the source of truth.

## Event payloads must be treated as untrusted data

Email bodies, issue comments, webpages, logs, and chat messages may contain instructions intended to manipulate the agent. The source pipeline should therefore enforce a boundary:

* Event content enters as **quoted, untrusted data**.
* It does not modify system instructions.
* Input templates identify the event origin and authorization context.
* Sensitive actions remain capability- and approval-gated.
* Connectors receive only their declared credentials and egress.
* The Bot cannot silently grant itself broader permissions.

This becomes even more important once agents can configure integrations or generate adapters. Current agent-containment work repeatedly shows that local files, configuration, pasted prompts, network access, and credentials become one combined blast radius unless separated deliberately. ([Anthropic][5])

---

# 5. The real product center: activation

Most trigger products stop at:

```text
event arrived → run workflow
```

Bots need a more sophisticated middle:

```text
persist
  → authenticate
  → deduplicate
  → filter
  → partition
  → window
  → rate/budget check
  → optional semantic judge
  → route
  → admit to session
```

This activation layer is where Lightspeed can be substantially better than ordinary automation products.

## Deterministic activation first

The common path should not require an LLM. It should support:

* event-type filters;
* field predicates;
* source and subject filters;
* partitioning by repository, issue, email thread, incident, customer, or device;
* quiet hours;
* minimum severity;
* debounce;
* maximum wait;
* maximum events;
* rate limits;
* deduplication;
* cooldowns;
* run concurrency.

A non-Turing-complete expression language such as CEL is a good fit for filters and grouping keys. Inngest uses CEL for event expressions, while its debounce semantics include both a quiet period and a timeout so a continuous stream cannot postpone work forever. ([Inngest][6])

I would standardize the window contract around:

```text
groupBy
debounce
maxWait
maxEvents
maxBytes
eventExpiry
mergeStrategy
```

`mergeStrategy` can initially be:

* `all`
* `latest`
* `firstAndLatest`
* `providerDelta`
* `digest`

The Bot activity page should explain exactly why an event was:

* deduplicated;
* filtered;
* waiting in a window;
* merged into an existing activation;
* blocked by a budget;
* ignored by a semantic judge;
* queued behind an active run;
* delivered as context only;
* admitted as a new run.

That explainability is central to trust. A proactive system that silently decides not to act is much harder to debug than a passive chat system.

## Optional semantic activation

Some cases genuinely require model judgment:

> “Wake me only when this appears commercially important.”

> “Ignore routine monitoring alerts unless they form a novel pattern.”

> “Read incoming email, but activate the main agent only when I probably need to respond.”

For this, add an optional **Sensor** or **Judge** step:

```text
event batch
  → small stateless model
  → typed decision:
      ignore
      accumulate
      activate(route, priority, reason)
```

The sensor should:

* have no action tools;
* receive a bounded structured batch;
* return a validated schema;
* use a separate cheap profile/model;
* have a strict token and latency budget;
* never be the sole storage for its reasoning;
* produce a visible reason code.

The primary persistent session should not be awakened merely to determine whether it should have been awakened.

---

# 6. Session topology: one by default, more when it matters

A Bot may own one or more sessions, but the ordinary product should begin with one persistent primary session.

## Primary session

Best for:

* personal assistants;
* release managers;
* infrastructure supervisors;
* research monitors;
* team coordinators;
* low-volume operational Bots.

It accumulates stable context and receives successive activations.

## Keyed sessions

For event streams with independent entities, a single session will eventually become polluted. Allow a route to derive a session key:

```text
github.issue:${event.subject}
email.thread:${event.threadId}
incident:${event.incidentId}
customer:${event.customerId}
```

The Bot owns a session factory policy:

```text
profile
maximumActiveSessions
idleTTL
closePolicy
environmentPolicy
workflowToolBindings
```

This provides persistent context per issue, customer, thread, or incident without forcing all events through one transcript.

## Ephemeral sessions

Useful for:

* one-off enrichment;
* event classification;
* isolated security analysis;
* highly parallel work where no long-term memory is useful.

These should be an explicit choice, not the default.

## Supervisor and workers

At the advanced end, one primary session can receive periodic digests, create Fleet sub-agents, or route work to keyed sessions. But the Bot controller—not the supervisor model—must remain authoritative for subscriptions, identities, deduplication, budgets, and session ownership.

---

# 7. How an activation enters a session

A route should choose an explicit admission mode.

| Mode      | Meaning                                                               |
| --------- | --------------------------------------------------------------------- |
| `context` | Append keyed context but do not start or wake a run                   |
| `queue`   | Start a run; queue it if another run is active                        |
| `steer`   | Add information to the currently active run at its next turn boundary |
| `replace` | Cancel the active run and start a replacement; highly privileged      |
| `flow`    | Run deterministic preprocessing before deciding or admitting          |
| `work`    | Start or supply input to a durable objective loop                     |
| `ignore`  | Record the decision without session activity                          |

For a first release, I would implement only:

* `context`
* `queue`
* `steer`

These map directly onto existing session APIs. `replace` should be rare because incoming noise must not repeatedly cancel valuable work. `flow` and `work` can arrive as later execution adapters.

The active-run policy should be visible in the Bot configuration:

```text
While the session is busy:
  queue a new run
  steer the active run
  append context for later
  merge into the next activation
  drop low-priority activations
```

---

# 8. Runtime architecture

## The core runtime components

```text
Provider / external system
          │
          ▼
Connector ingress or poller
          │
          ▼
Event Gateway
  - signature/auth verification
  - raw payload persistence
  - event identity and dedupe
  - fast acknowledgement
          │
          ▼
Durable Bot Event Store / CAS
          │
       doorbell
          ▼
BotControllerWorkflow
  - reads bounded event ranges
  - filters, groups, windows
  - enforces limits
  - resolves route/session
  - admits context or run
          │
          ▼
Managed Lightspeed session(s)
```

### `BotControllerWorkflow`

One durable Temporal entity workflow per Bot should own:

* published Bot revision;
* source generations and cursors;
* open activation windows;
* session references;
* active and pending activation identities;
* rate and budget state;
* pause/drain state;
* connector reconciliation status;
* continue-as-new carry state.

It should not own:

* raw event bodies;
* complete session transcripts;
* arbitrary logs;
* large tool results;
* connector secrets;
* the full activity trace.

Those remain in Postgres/CAS and are read by activities.

Temporal Workflows are well suited to long-lived control state, timers, signals, retries, and recovering from event history. Temporal Schedules also already provide operations such as pause, trigger, update, list, describe, and backfill. ([Temporal Documentation][7])

### Do not signal every high-volume event into Temporal history

For email, GitHub, or ordinary messaging volume, one signal per event may be acceptable. For monitoring, telemetry, or bursty providers, it can create enormous workflow histories.

Use a **durable inbox plus doorbell** pattern:

1. Ingress verifies and persists events.
2. It advances a Bot/source cursor.
3. It sends or coalesces a small wake-up signal containing a cursor/range hint.
4. The Bot workflow activity reads a bounded page from the event store.
5. The workflow records only the decisions and compact window state it needs.
6. Continue-as-new periodically resets history.

The Postgres event record remains authoritative. Temporal history remains authoritative for the Bot’s durable control decisions.

### Schedules and timers are Sources

Treat recurring schedules, one-off reminders, and delayed wake-ups as visible Bot Sources rather than hidden session sleeps.

Examples:

```text
Every weekday at 08:00 Europe/Zurich
At 2026-09-01T09:00:00+02:00
Five days after event X unless event Y arrives
Every 15 minutes while incident status = open
```

An agent-created reminder should therefore create or update a Bot Source through a controller tool. It becomes inspectable, pausable, editable, auditable, and durable independently of the current session turn.

### Idempotency model

Exactly-once should not be promised. The system should provide at-least-once admission with deterministic deduplication.

Useful identities:

```text
Event:
  triggerInstanceId + generation + providerEventId

Activation:
  botRevision + route + partition + sorted event IDs + window generation

Run submission:
  botId + activationId + sessionId

Context key:
  bot:event:<eventId>
  bot:activation:<activationId>
```

Every provider retry, workflow retry, worker restart, and publish transition should converge on the same durable result.

### Source generations

Updating a webhook, filter, repository, connection, or trigger configuration should create a new immutable **Source generation**.

A safe publish sequence is:

```text
create/reconcile generation N+1
  → prove it is healthy
  → activate N+1
  → stop admitting new events from N
  → drain or expire N
  → unregister N
```

Every admitted event records the source generation and Bot revision under which it was accepted.

---

# 9. The integration strategy

There are four broad options.

| Option                                       | Advantage                        | Failure mode                                                    |
| -------------------------------------------- | -------------------------------- | --------------------------------------------------------------- |
| Hardcode every integration                   | Fastest initial UX, full control | Permanent integration treadmill                                 |
| Accept only generic webhooks                 | Tiny implementation              | Pushes complexity onto every customer                           |
| Depend entirely on Zapier/Pipedream/Composio | Immediate breadth                | Weak native control, external dependency, less self-hostability |
| Build a connector SDK and registry           | Sustainable and extensible       | Larger up-front platform investment                             |
| Let models write live connectors             | Potentially unlimited breadth    | Severe reliability, security, upgrade, and support burden       |

The right approach is a staged hybrid.

## Hardcode five source mechanics

The Bot runtime only needs to know a small number of ways that Sources operate:

1. **Schedule/timer**
2. **Signed webhook**
3. **Polling with checkpoint**
4. **Subscription or stream with renewal**
5. **Internal/manual event admission**

Every GitHub, Linear, email, PagerDuty, Datadog, Sentry, CRM, or custom integration is an adapter over one of those mechanics.

## Define a Source Adapter contract

Conceptually:

```text
ConnectorManifest
  id
  version
  triggerTypes
  connectionRequirements
  scopes
  workerCompatibility
  publisherSignature

TriggerType
  id
  title
  configSchema
  eventSchema
  deliveryKind
  sampleEvents

SourceAdapter
  validateConfig
  testConnection
  reconcileSubscription
  receiveWebhook
  poll(checkpoint)
  renew
  normalize(rawPayload)
  deriveProviderEventId
  health
  delete
```

The adapter should explicitly declare:

* whether delivery is push, poll, schedule, or stream;
* provider retry semantics;
* ordering guarantees;
* checkpoint representation;
* subscription expiry;
* required OAuth scopes;
* expected maximum frequency;
* schema revision;
* test fixtures;
* signature/challenge behavior.

Connectors execute in separate workers or services. Their types and provider SDKs do not enter the stable session worker.

## First-party integrations

For the initial product, I would build:

1. Schedule and one-off timer.
2. Generic signed webhook.
3. Manual/API event.
4. Internal Lightspeed event source.
5. Native GitHub source.

GitHub is the best first real integration because Lightspeed itself can dogfood it for:

* issue creation or labeling;
* pull request updates;
* review requests;
* CI failures;
* releases;
* repository pushes.

Then add one polling source to force the checkpoint contract to become real.

## Use an integration provider for breadth

A small team should not immediately build OAuth, webhook registration, schema discovery, renewal logic, and polling for hundreds of applications.

Pipedream’s model is attractive for an open-source-oriented project because its components are self-contained executable units and its registry includes source and action components. Composio is attractive because it provides an agent-oriented trigger catalog, trigger instances, managed connections, retries, structured payload delivery, and tools that let an agent configure certified triggers. ([Pipedream][8])

I would put either provider behind a Lightspeed-owned `IntegrationProvider` boundary:

```text
native
pipedream
composio
customer-hosted
```

The user sees a Lightspeed Source. The Bot receives a Lightspeed event. The provider is an implementation detail.

For enterprise deployments with data-residency or third-party-processing restrictions, native connectors and the generic signed webhook remain available.

### Why not make Zapier the core integration layer?

Zapier should be easy to connect through the generic webhook and API, especially for customers already using it. But it is better as an interoperability path than the architectural substrate. Its product is increasingly a general workflow environment in its own right; making it mandatory would place a second orchestration system between the event and the Lightspeed controller. ([help.zapier.com][2])

---

# 10. Self-configuring Bots versus self-writing integrations

These should be treated as two very different capabilities.

## Self-configuration should arrive early

A Bot’s primary session can safely receive controller-bound workflow tools such as:

```text
bot_inspect
bot_list_source_types
bot_list_connections
bot_create_source
bot_update_source
bot_pause_source
bot_delete_source
bot_test_source
bot_set_window
bot_set_route
bot_create_timer
bot_replay_event
bot_read_activity
```

These are an excellent fit for the existing workflow-tool architecture:

* The fixed receiver is the Bot controller.
* The model cannot choose an arbitrary Bot or workflow address.
* `Joined` completion can wait until a webhook subscription or schedule is actually active.
* `Accepted` can be used for asynchronous changes.
* Arguments are schema-validated.
* Every change is durable and auditable.
* Secrets remain connection handles rather than model-visible values.

The Bot controller changes a revisioned desired-state document. A connector reconciler performs provider-side effects. The model never directly executes “register this webhook using this secret.”

### Self-change policy

Each Bot or universe should have one of three policies:

| Policy           | Behavior                                                      |
| ---------------- | ------------------------------------------------------------- |
| Propose          | Agent prepares a diff; human publishes it                     |
| Limited autonomy | Low-risk timer, filter, or batching changes may auto-publish  |
| Managed autonomy | Certified source changes auto-publish within scope and budget |

Changes involving new OAuth scopes, new outbound action authority, public ingress, environment credentials, or expanded data access should normally require approval.

## Natural-language Bot building

The configuration UI should support:

> “When a GitHub issue in `lightspeed` gets the `customer-bug` label, create one persistent session per issue. Group further updates for two minutes, but never wait longer than ten minutes. Queue the session if it is already working.”

The system produces a visible declarative spec and a test explanation:

```text
Source: GitHub issue events
Filter: repository = lightspeed AND label_added = customer-bug
Partition: issue node ID
Window: debounce 2m, max wait 10m
Route: keyed issue session
Busy policy: queue
```

The declarative representation must remain visible and editable. Natural language is a builder, not the only source of truth.

## Self-writing connectors should come later

There is real value in allowing an agent to read provider documentation or an OpenAPI specification and draft a connector. Current workflow products already demonstrate models generating and modifying workflow definitions through controlled APIs. ([n8n Blog][9])

But production connector generation should operate like software development:

```text
model produces:
  manifest
  schemas
  adapter code
  fixtures
  contract tests
  permission declaration

system performs:
  compilation
  linting
  unit tests
  replay tests
  simulated webhook verification
  restricted live test

human/admin performs:
  scope review
  data-access review
  publication approval

registry stores:
  immutable signed version
```

Generated adapters should run with:

* brokered credential handles;
* explicit provider domains;
* restricted egress;
* no Lightspeed database access;
* memory and CPU limits;
* event-size and rate caps;
* no arbitrary subprocess authority unless specifically granted;
* immutable published revisions.

A running Bot should never silently rewrite its production connector when an API changes. It may diagnose the failure and propose a new connector revision or pull request.

### Learned routines

The xAI-style “show it once, save a routine” idea is useful, but the routine should compile into visible Lightspeed artifacts:

* a **Source** for when it runs;
* a **Flow** for deterministic operations;
* a **Skill** for reasoning instructions;
* a set of **Tools and permissions**;
* optional approval checkpoints;
* a Bot route and session policy.

Do not store an opaque browser macro plus a vague prompt as the only representation.

Browser/computer use remains a valuable fallback when an API or connector does not exist, but it should not be the default integration mechanism. It is harder to secure, test, observe, and keep compatible.

---

# 11. A possible `BotSpec`

A preliminary declarative model could look like this:

```yaml
kind: Bot
metadata:
  id: customer-bug-triage
  displayName: Customer Bug Triage

spec:
  profile: github-maintainer

  sessions:
    primary:
      mode: persistent

    issue:
      mode: keyed
      key: "github.issue:${event.subject}"
      profile: github-maintainer
      idleTtl: 30d
      maximumActive: 200

  sources:
    - id: customer-bugs
      connector: github
      connection: github-production
      trigger: issue.changed
      config:
        repository: smartcomputer-ai/lightspeed
      filter: >
        event.change.type == "label_added" &&
        event.change.label == "customer-bug"

    - id: morning-review
      connector: schedule
      trigger: cron
      config:
        expression: "0 8 * * 1-5"
        timezone: Europe/Zurich

  activation:
    groupBy:
      - source.id
      - event.subject
    debounce: 2m
    maxWait: 10m
    maxEvents: 50
    mergeStrategy: digest

  routes:
    - when: source.id == "customer-bugs"
      session: issue
      admission: queue

    - when: source.id == "morning-review"
      session: primary
      admission: queue

  limits:
    maximumConcurrentRuns: 10
    runsPerHour: 50
    tokensPerDay: 2000000
    maximumPendingEvents: 10000

  selfManagement:
    mode: propose
```

The actual v1 should probably be smaller. The important design property is that every advanced field can be omitted.

---

# 12. Product UI

## Bots list

Each row should show:

* running, paused, degraded, or draft;
* last activation;
* next scheduled wake;
* number of pending events;
* active sessions and runs;
* source health;
* recent failure;
* current spend or budget utilization.

## Bot detail

I would organize it around user questions rather than implementation nouns:

### Overview

* What does this Bot do?
* Which profile does it use?
* Is it active?
* What happened most recently?

### Sources — “When it wakes”

* Connected applications.
* Trigger type and configuration.
* Schedule or timer.
* Current subscription status.
* Sample and most recent events.
* Test Source.

### Activation — “What deserves attention”

* Filters.
* Grouping key.
* Debounce and maximum wait.
* Optional semantic judge.
* Quiet hours.
* Limits.

### Routing — “Where work goes”

* Primary versus keyed session.
* Queue versus steer versus context.
* Session lifecycle.
* Optional Flow or Work execution.

### Tools and permissions — “What it may do”

* MCPs.
* workflow tools;
* connections and scopes;
* environments;
* approvals;
* self-management policy.

### Sessions

* Primary session.
* Active keyed sessions.
* Session state, last activation, and idle expiry.
* Link into the full session transcript.

### Activity

This is the most important screen:

```text
GitHub webhook received
→ signature valid
→ event persisted
→ duplicate: no
→ matched source customer-bugs
→ filter passed
→ added to issue #314 window
→ window flushed after 2m debounce
→ activation act_...
→ routed to session bot/.../issue/314
→ run queued
→ run started
→ 3 workflow tools called
→ run completed
```

It should also display skipped decisions and allow:

* replay with the original revision;
* explicitly reprocess under the current revision;
* inspect raw structured payload;
* inspect generated session input;
* retry connector reconciliation;
* send a test event.

Without this trace, proactive Bots will feel nondeterministic even when their runtime is correct.

---

# 13. API surface

A plausible first public surface:

```text
bots/create
bots/read
bots/list
bots/spec/put
bots/publish
bots/pause
bots/resume
bots/drain
bots/delete

bots/sources/test
bots/sources/reconcile

bots/events/read
bots/events/replay

bots/activity/read
bots/sessions/list

connections/create
connections/read
connections/list
connections/delete
```

Connector workers need a separate capability-protected admission boundary, conceptually:

```text
bot/events/admit
```

It should not be an unauthenticated general-purpose API. The gateway must identify the Source instance and generation from its credential or endpoint, rather than letting the caller choose arbitrary Bot routing fields.

Bot workflows should continue using existing session APIs rather than gaining an internal privileged session transport:

```text
session/managed/start
session/context/append
session/runs/start
session/runs/steer
session/runs/cancel
session/read
session/events/read
```

This is both architecturally cleaner and a continuous proof that the public session API is sufficient for first-party products.

---

# 14. Relationship to existing Lightspeed systems

| Existing concept | Relationship to Bots                                             |
| ---------------- | ---------------------------------------------------------------- |
| Session          | Execution and context owned or referenced by a Bot               |
| Managed session  | Correct lifecycle boundary for Bot-owned sessions                |
| Workflow tool    | Session-to-Bot or session-to-plugin command/reply mechanism      |
| Emission         | Durable session/workflow fact crossing controller boundaries     |
| Channel          | Specialized Source and delivery product; can reuse Bot kernel    |
| Flow             | Deterministic preprocessing, transformation, or orchestration    |
| Fleet            | Session-level delegation initiated during work                   |
| Work             | Optional repeated-objective execution policy                     |
| Profile          | Reusable configuration for Bot-created sessions                  |
| Environment      | Compute made available to sessions, not owned by the event layer |
| Configurator MCP | Natural route for controlled Bot-management tools                |

The important boundary is:

> **Bots own admission and lifecycle. Sessions own reasoning.**

That prevents event subscriptions, polling checkpoints, webhook signatures, rate limits, and provider SDKs from leaking into the session core.

---

# 15. The first implementation slice

I would build the first coherent product in this order.

## 1. Define the contracts

Create:

* `BotSpecV1`
* `BotEventEnvelopeV1`
* `BotActivationV1`
* `BotActivityEventV1`
* `SourceTypeV1`
* `SourceInstanceV1`

Keep v1 intentionally narrow.

## 2. Build `BotControllerWorkflow`

Support:

* one persistent managed session;
* durable event doorbells;
* bounded inbox reads;
* deduplication;
* `groupBy`;
* debounce;
* maximum wait;
* maximum event count;
* `context`, `queue`, and `steer`;
* pause and resume;
* continue-as-new.

## 3. Extract generic mechanics from Channels

Extract modules for:

* deterministic identities;
* bounded inbox;
* window scheduling;
* terminal-token tracking;
* managed-session reconciliation;
* activity recording.

Do not initially force Channels to use the new Bot product. First share tested machinery; migrate product behavior only after the generic contracts stabilize.

## 4. Implement four Sources

* Manual/API.
* Schedule/timer.
* Generic signed webhook.
* Internal Lightspeed event.

Then implement native GitHub as the first real connector.

## 5. Build activity tracing before broad integration work

The first users must be able to see:

* received;
* deduplicated;
* filtered;
* windowed;
* activated;
* routed;
* queued;
* failed.

Otherwise connector and activation bugs will be extremely expensive to diagnose.

## 6. Add controlled self-management

Give the primary session controller-bound tools for:

* inspecting the Bot;
* creating a timer;
* updating filters and windows;
* pausing or resuming a Source;
* testing a Source;
* proposing a Bot revision.

This immediately makes Bots feel more autonomous without requiring model-generated runtime code.

## 7. Add a long-tail integration provider

Only after the Source contract has been proven by schedule, webhook, GitHub, and one polling integration should you bind Pipedream or Composio behind it.

## 8. Add keyed sessions and semantic sensors

These are the next high-value capabilities because they address context pollution and high-volume event streams.

---

# 16. Things I would explicitly avoid

### Do not extend `AgentSessionWorkflow` with trigger state

It would make every session carry webhook, schedule, polling, and subscription semantics and undo the separation already established by workflow tools.

### Do not send every event to an LLM

A hundred monitoring alerts should normally create one activation, not a hundred runs.

### Do not build a full Zapier-like DAG editor first

That would expand the product into an enormous adjacent category. Keep Bots opinionated. Use Flows when deterministic multi-step orchestration is actually needed.

### Do not let event payloads choose routing or capabilities

A provider event may propose data. The admitted Source binding decides the Bot, route, connection, and allowed interpretation.

### Do not make browser automation the universal connector

Use APIs, webhooks, polling, MCP, and workflow tools first. Browser control fills gaps.

### Do not claim exactly-once execution

Use durable facts, deterministic identities, at-least-once transport, and idempotent effects.

### Do not let a Bot silently mutate its production code

Configuration changes can be policy-governed. Code changes are reviewed artifacts.

### Do not keep every entity in one transcript

Keyed sessions should be part of the product model before high-volume customer, incident, email, or issue Bots become common.

### Do not measure success by connector count alone

Connector breadth can be obtained through a provider. Lightspeed’s differentiation should be reliable activation, persistent context, durable autonomy, safe self-configuration, and inspectable decision history.

---

# Bottom line

The correct abstraction is not “a scheduled session” and not “a managed workflow with an agent somewhere inside it.”

It is:

> **A Bot is a durable controller for observation, activation, and session lifecycle.**

Its durable loop is:

```text
observe facts
→ decide whether and when they matter
→ select the right agent context
→ admit work safely
→ observe the result
→ remain alive
```

The system should have three levels of extensibility:

1. **Core source mechanics**, maintained by Lightspeed.
2. **Certified connector adapters**, installed from first-party, community, or integration-provider registries.
3. **Agent-generated connector drafts**, tested and reviewed before publication.

That lets a small team ship a strong native product without committing to hand-writing the world’s integrations, while preserving the open, self-hostable, workflow-native architecture that makes Lightspeed unusual.

The concise product principle I would use internally is:

> **Hardcode the control plane. Make the edges installable. Let agents configure the graph. Let them propose code—but never make invisible code the product.**

[1]: https://www.theverge.com/ai-artificial-intelligence/978666/spacexai-grok-bot-ai-agent-beta-launch "https://www.theverge.com/ai-artificial-intelligence/978666/spacexai-grok-bot-ai-agent-beta-launch"
[2]: https://help.zapier.com/hc/en-us/articles/47402591569805-Migrating-from-Agents-to-AI-by-Zapier "Migrating from Agents to AI by Zapier – Zapier"
[3]: https://docs.composio.dev/docs/triggers "https://docs.composio.dev/docs/triggers"
[4]: https://github.com/cloudevents/spec/blob/main/cloudevents/spec.md "https://github.com/cloudevents/spec/blob/main/cloudevents/spec.md"
[5]: https://www.anthropic.com/engineering/how-we-contain-claude "https://www.anthropic.com/engineering/how-we-contain-claude"
[6]: https://www.inngest.com/docs/guides/debounce "https://www.inngest.com/docs/guides/debounce"
[7]: https://docs.temporal.io/workflows "https://docs.temporal.io/workflows"
[8]: https://pipedream.com/docs/components "https://pipedream.com/docs/components"
[9]: https://blog.n8n.io/n8n-mcp-server/ "https://blog.n8n.io/n8n-mcp-server/"
