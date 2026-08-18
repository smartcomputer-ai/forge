# PNNN: A2A Protocol Adapter (Agent-to-Agent)

**Status**
- Later / exploratory.
- Originally written 2026-07-08 as "ACP and A2A protocol adapters", covering
  IBM/BeeAI's Agent Communication Protocol (`agentcommunicationprotocol.dev`)
  and Google's A2A. Rewritten 2026-08-18: the IBM Agent Communication
  Protocol has since been folded into A2A under the Linux Foundation, so this
  document targets **A2A only**. There is no compatibility goal for the old
  Agent Communication Protocol REST surface (`POST /runs`, `awaiting`,
  distributed sessions); it is not implemented and will not be.
- Not to be confused with the **editor** ACP (Zed's Agent Client Protocol,
  `agentclientprotocol.com`), which is an editor↔agent protocol and has its
  own document: [pNNN-editor-acp-adapter.md](pNNN-editor-acp-adapter.md).
  The two adapters are orthogonal (see "Relationship to the editor ACP
  adapter" below) and share only the adapter rules.
- Builds on P92/P94 (unified suspension, engine-native awaits, log-backed
  mailbox, validated wake) and P93 fleet tools.
- References:
  - A2A specification: <https://a2a-protocol.org/latest/specification/>
  - A2A Life of a Task: <https://a2a-protocol.org/latest/topics/life-of-a-task/>

## Goal

Expose Lightspeed sessions and runs to other agents through an A2A adapter
without changing the internal execution model.

The internal model remains:

```text
Session          = durable conversation / collaboration context
Run              = immutable unit of agent work
RequestRun       = submit work that must create or return a run
SubmitMessage    = submit an inbound message; the receiver decides consume vs run
ResolvePromise   = record a promise fact
ResumeToolBatch  = trusted workflow-to-engine command (P94's "ResumeAwait")
                   that validates and applies an already-visible wake condition
CancelRun        = structured cancellation
```

The A2A adapter translates A2A objects into these commands and projections.
It must not become an alternate engine semantics.

## Fit

Lightspeed is not primarily an interop protocol. It is a deterministic,
event-sourced execution substrate with structured concurrency, promise
resolution, cancellation, mailbox delivery, idempotent submissions, and
Temporal-backed recovery.

A2A is an agent/agent protocol surface: an agent publishes an Agent Card,
another agent sends messages that create or continue tasks grouped by
`contextId`, and observes task state, artifacts, and streaming updates. It is
useful as an adapter layer for both directions:

- **Inbound (Lightspeed as A2A server):** other agents delegate work to a
  Lightspeed session/profile.
- **Outbound (Lightspeed as A2A client):** Lightspeed's fleet tools
  (`agent_spawn`/`agent_request`/`agent_send`) target a remote A2A agent
  instead of a local session.

Both directions enter Lightspeed through the same admission boundaries as
native clients and fleet tools.

## Concept mapping

| Concept | Lightspeed | A2A |
|---|---|---|
| Conversation scope | `Session` | `contextId` |
| Unit of work | `Run` | `Task` |
| New work | `RequestRun`, `agent_request`, `agent_spawn` | `message/send` returning a `Task` |
| Fire-and-forget input | `SubmitMessage` | `message/send` within an existing `contextId` |
| Awaiting input | parked `await { mailbox: true }` | `input-required` task state |
| Follow-up after completion | new run in same session | new task in same `contextId`, `referenceTaskIds` |
| Terminal result | run terminal output resolves promise | completed task artifacts/status |
| Progress | `session/events/read`, notifications | `message/stream` / `tasks/resubscribe` SSE, push notifications |
| Cancellation | `session/runs/cancel` → `CancelRun` | `tasks/cancel` |
| Discovery | agent profiles + tool manifests | Agent Card |

A2A is close to the shape of Lightspeed's session/agent communication system.
`contextId` groups related messages and tasks; terminal tasks are immutable;
follow-ups create new tasks in the same context, optionally referencing
previous task ids. That maps cleanly to sessions as contexts, runs as
immutable tasks, and follow-ups as additional runs in the same session.

The main difference is Lightspeed's receiver-side delivery rule:

```text
agent_send / SubmitMessage

if receiver is parked with mailbox:true:
  message wakes the current run (validated by ResumeToolBatch)
else:
  message becomes a new message-origin run
```

P94 makes that rule engine law by logging the message buffer and validating
mailbox wakes in the engine. A2A does not require this exact internal rule,
but it projects cleanly onto A2A task states.

## Inbound adapter shape (Lightspeed as A2A server)

```text
A2A contextId          -> Lightspeed session_id
A2A taskId             -> Lightspeed run_id
A2A message/send       -> SubmitMessage by default
A2A message/send       -> RequestRun when the adapter must force task creation
A2A message/stream     -> same admission + subscription to session events
A2A tasks/get          -> run projection from session/read or events
A2A tasks/cancel       -> CancelRun
A2A tasks/resubscribe  -> replay/subscribe session/events/read from a cursor
A2A Task state         -> projected run status (see state table)
A2A referenceTaskIds   -> context metadata pointing at prior run/artifact refs
A2A input-required     -> parked await with mailbox:true
A2A artifact           -> run output refs / produced context artifacts (CAS)
A2A Agent Card         -> generated from a published agent profile
```

Task-state projection (collapse conservatively):

```text
run accepted, not started         -> submitted
run executing                     -> working
run parked, await mailbox:true    -> input-required
run parked, other await           -> working
run completed                     -> completed
run cancelled                     -> canceled
run failed                        -> failed
```

Defaulting `message/send` to `SubmitMessage` preserves Lightspeed's receiver
semantics: an interactive session consumes the message as mailbox input,
while an idle session turns it into a message-origin run. When an A2A client
requires a stable task id at request acceptance time and the receiver would
otherwise consume the message, the adapter must choose `RequestRun` and
report the resulting run id as the task id.

Follow-up tasks never reopen old Lightspeed runs. They submit new work in the
same session, with references to the prior run's output or artifact refs.
This matches A2A task immutability and Lightspeed run immutability.

A2A `input-required` resolution enters as `SubmitMessage`, never as a direct
engine wake. The workflow observes engine state and admits `ResumeToolBatch`
internally:

```text
SubmitMessage    = external input fact
ResumeToolBatch  = internal validated completion of an already-parked await
```

Allowing external protocol clients to trigger the wake directly would bypass
the engine's wake predicate and recreate the "trust the resume" class P94
deleted.

Which sessions are reachable: an A2A server endpoint is bound to a universe
and either (a) creates one session per new `contextId` from a named profile
via `session/managed/start`, or (b) routes into an existing session whose id
is the `contextId`. Neither path exposes reducer internals or `session/config`
mutation to A2A callers.

## Outbound adapter shape (Lightspeed as A2A client)

Fleet tools already carry the right vocabulary. An A2A target is a
universe-scoped record (analogous to `model:<id>` and MCP server records):
Agent Card URL plus optional credentials, resolved immediately before I/O.

```text
agent_spawn(target=a2a:<id>)   -> message/send creating a task; promise resolves
                                  on terminal task state (via stream or push)
agent_request(target=a2a:<id>) -> same, awaited inline
agent_send(target=a2a:<id>)    -> message/send into an existing contextId
agent_read                     -> tasks/get
agent_cancel                   -> tasks/cancel
```

Outbound task ids and context ids are recorded as promise/handle metadata so
the promise can be resolved deterministically after worker restart. Remote
`input-required` surfaces to the local agent as a mailbox message on the
spawned handle, not as an automatic reply.

## Adapter rules

- Keep all external input at normal admission boundaries: `RequestRun`,
  `SubmitMessage`, `CancelRun`, and read/projection APIs.
- Do not expose the validated wake (`ResumeToolBatch`) as a public protocol
  method. It is a workflow-to-engine command.
- Preserve run/task immutability. Follow-ups create new runs, not restarted
  terminal runs.
- Preserve receiver-side delivery. The adapter may choose whether a call
  requires `RequestRun` or allows `SubmitMessage`, but the receiver engine
  decides mailbox-consume vs message-origin-run for submitted messages.
- Preserve idempotency. A2A message ids and task ids map to Lightspeed
  submission ids so retries do not duplicate work.
- Preserve structured cancellation. `tasks/cancel` maps to `CancelRun`;
  force-cancel remains an internal recovery/admin path.
- Project internal states conservatively. If A2A has fewer states than
  Lightspeed, collapse in the adapter rather than weakening the engine model.
- Credentials for outbound targets live in universe records, never in the
  session log or `SessionConfig`.

## Relationship to the editor ACP adapter

Editor ACP and A2A do not overlap functionally:

| | Editor ACP | A2A |
|---|---|---|
| Parties | user's editor ↔ one agent | agent ↔ agent |
| Purpose | interactive UX: prompts, streamed tool-call UI, permission prompts, client-provided fs/terminal | delegation: tasks, contexts, artifacts, discovery |
| Transport | JSON-RPC over stdio (remote in development) | HTTP JSON-RPC + SSE / push |
| Filesystem/terminal surface | yes (client-side capabilities) | none |
| Discovery | `initialize` capability negotiation | Agent Card |

Shared points are incidental: both are JSON-RPC, both borrow MCP content-part
JSON (so content mapping code can be shared), and both project onto the same
Lightspeed session/run model. One Lightspeed session may be reached through
both at once (a user drives it from an editor while another agent submits
into it over A2A) with no conflict, because both enter through
`RequestRun`/`SubmitMessage`.

## Open design questions

- Should A2A `message/send` default to `SubmitMessage`, or should the adapter
  expose an option that forces `RequestRun` when a stable task id is required
  immediately?
- How should `referenceTaskIds` be represented in Lightspeed context:
  structured metadata on submitted input, ordinary context entries, or a
  small adapter-owned index?
- How much A2A artifact identity should be backed by Lightspeed CAS refs
  versus adapter-owned artifact ids?
- Should Agent Cards be generated from agent profiles, tool manifests, or
  both? Which profiles are publishable?
- Push notifications: adapter-owned webhook registry, or reuse the emission
  spine (P100) with an A2A delivery target?

## Non-goals

- Do not make A2A the internal engine model.
- Do not expose engine reducer internals to A2A clients.
- Do not add a second public resume path that bypasses mailbox/promise wake
  validation.
- Do not let A2A task/message terminology reintroduce sender-side
  consume-vs-run decisions. Receiver-side delivery is a Lightspeed invariant.
- No compatibility with the retired IBM Agent Communication Protocol.
