# New Forge Agent Specification

This document specifies the redesigned `forge-agent` crate.

The new agent is Forge-native, AOS-inspired, built on top of `forge-llm`, and
designed to run on Temporal. Temporal is the target durable execution backend,
but the agent domain model is not Temporal-specific. The core agent is an
event/effect state machine that can run in-process for local development and
tests, or inside a Temporal workflow for production durability.

The thinking behind this rewrite lives in
[`spec/04-new-agent-idea.md`](./04-new-agent-idea.md). This specification turns
that direction into the design contract. Roadmap steps are intentionally out of
scope for this document.

## 1. Decisions

### 1.1 Build a Forge-native agent

Forge will continue building its own agent runtime instead of vendoring Codex or
another CLI agent as the core implementation.

Reasons:

- Forge needs a programmable library, not only a CLI subprocess.
- Forge needs durable run state, structured events, and effect receipts.
- Forge needs clean integration with Attractor, Temporal, and future control
  plane work.
- Codex, Claude Code, and Gemini CLI remain useful as external agent backends,
  but they are not the substrate for Forge's own runtime.

### 1.2 Use AOS as the conceptual reference

The most important AOS lesson is the shape of the runtime:

```text
input event -> reduce state -> emit effect intents -> execute effects -> append receipts -> reduce state
```

Forge adopts that shape without adopting the full AgentOS world/AIR/WASM stack.
The new agent should borrow from `refs/aos-agent/` for:

- session and run lifecycle modeling
- typed input events
- pending effect tracking
- tool registry and tool profiles
- turn planning and active context window construction
- transcript records/projections, source ranges, and provider-native context blobs
- context pressure, token counting, and compaction operation state
- per-call tool batch status, execution groups, and composite tool progress
- runner workspace and tool-runtime concepts
- chat/CLI projection from an event stream

### 1.3 Build on `forge-llm`

`forge-agent` uses `forge-llm` for provider communication. The native Forge
agent owns the agent loop and uses the low-level LLM client/provider adapter
surface directly. The `forge-llm` high-level tool loop is not the agent runtime.

The native agent should prefer `ProviderAdapter`/`Client` calls for HTTP API
providers. Existing `AgentProvider` implementations for CLI agents may still be
used as black-box provider backends where appropriate, especially for migration
or comparison, but they are not the core loop.

### 1.4 Design for Temporal, keep the core portable

The domain core must be deterministic and serializable. Temporal owns durable
workflow execution, timers, retries, cancellation, signals, and activity
scheduling. The Forge agent core owns the domain state, event types, effect
types, and transition semantics.

This split gives us:

- fast unit tests without a Temporal server
- a local runner for CLI development
- a clear Temporal mapping for production
- fewer dependencies on Temporal Rust SDK details
- a domain model that stays stable if the execution backend changes

### 1.5 Treat Codex as reference and optional backend

Codex is a useful reference for CLI UX, tool events, sandboxing, subagents,
hooks, and JSONL streaming. It should influence Forge design, especially the CLI,
event projection, and extensibility boundaries. It should not be temporalized
wholesale.

Forge should lift these Codex concepts into its own contracts:

- submission/event queue correlation ids and propagation
- resolved turn context snapshots, including model, tools, and runtime refs
- stable item lifecycle projections (`started`/`updated`/`completed`) for UI,
  CLI, JSONL, and future service clients
- explicit fork/rollback/history-rewrite operations
- parent/child subagent event and cancellation routing

Codex also has useful hook, permission, approval, and sandbox policy concepts.
Forge should treat those as deferred SDK extension surfaces, not first-cut core
runtime requirements. Future policy should be implemented mostly through
explicit lifecycle extension points around prompt submission, tool preflight,
tool completion, permission requests, and stop/interruption.

A Codex backend can exist as:

```text
Forge run -> Temporal activity/local runner effect -> codex exec --experimental-json -> Forge events
```

That backend is a bridge, not the primary Forge agent implementation.

## 2. Goals

- Provide a programmable coding-agent library, not just a command-line tool.
- Preserve every meaningful action as typed state, event, intent, or receipt.
- Support local in-process execution and Temporal-backed durable execution.
- Use `forge-llm` provider adapters for OpenAI, Anthropic, and future providers.
- Keep LLM calls, tool execution, MCP calls, and subagent operations outside
  deterministic state reduction.
- Make the CLI a projection/control surface over the same runtime.
- Make Attractor able to call the agent as a durable backend later.
- Support explicit context pressure handling, compaction, fork, and rollback
  operations without corrupting the source transcript.
- Publish stable item lifecycle projections suitable for CLI, JSONL, and web
  clients.
- Preserve clear SDK extension seams for future hooks and policy systems.
- Keep tests deterministic, focused, and able to run without live providers by
  injecting fake LLM/effect executors.

### 2.1 First-cut scope

The first implementation should focus on the core agent loop and rough feature
parity with the useful parts of `refs/aos-agent/`:

- agent definition/version records as reusable configuration
- session/run/turn lifecycle
- deterministic event/effect state transitions
- scoped agent journal events with large payloads stored by ref
- transcript records/projections and blob refs
- active context planning, token counting, and context pressure handling
- LLM generation through `forge-llm`
- SDK tool invocation contracts for runner-provided tools
- tool registry, tool profiles, tool batches, and per-call tool status
- compaction as a first-class context operation if needed to keep the loop
  viable
- local runner and CLI projection
- Temporal mapping that preserves deterministic workflow boundaries

## 3. Non-goals

- Do not re-create AgentOS AIR, worlds, WASM workflows, or governance in
  `forge-agent`.
- Do not make Temporal types the public agent domain model.
- Do not vendor Codex as the Forge runtime.
- Do not copy Codex app-server, realtime, product telemetry, or guardian
  implementation wholesale. Lift only the runtime boundary concepts Forge needs.
- Do not implement user/project-configurable hooks in the first cut.
- Do not build a full permission, approval, sandbox, or policy-review framework
  in the first cut. Keep any required local safety behavior adapter-local and
  design the future policy system as hooks/extensions around the core loop.
- Do not start by redesigning Attractor. Attractor integration comes after the
  agent runtime is coherent. Attractor is secondary and will likely NOT be the final runtime for the agent.
- Do not require CXDB, Temporal, or live LLM credentials for core unit tests.
- Do not hide skipped infrastructure behind runtime test gates. External
  resource tests must use `#[ignore]`.

## 4. Architecture

The new agent has four conceptual layers:

```text
Host surfaces
  forge CLI, tests, future Attractor backend, future service API

Runners
  local runner, Temporal runner

Agent core
  pure state transitions, event model, effect intent envelope, projections

Effect adapters
  forge-llm, MCP, confirmation, generic tool invocation

Runner/tool packages
  local host filesystem/shell tools, remote executors, subagents, provider-native tools

Future SDK extensions
  hooks, policy reviewers, approval surfaces, dynamic tools, third-party tool handlers
```

### 4.1 Agent core

The core is a deterministic state machine. It exposes operations equivalent to:

```text
apply(event, state) -> state
decide(state) -> [effect_intent]
```

Implementations may combine these into a single stepper, but the conceptual
boundary must remain clear:

- applying an event mutates only in-memory domain state
- deciding may create effect intents but does not perform I/O
- effect execution happens in runners/adapters
- completed effects return receipts, which are appended as events

Forge is journaled, ref-backed, and snapshot-driven:

```text
agent = journal + blob/CAS + bounded session state
```

The journal is scoped to agent/session events and is the product-level audit and
follow stream. It is not a fully generic event store and does not need to be the
only source from which every runtime optimization can be rebuilt. Large or
opaque payloads live in a blob/CAS store and are referenced by journal
events, transcript items, receipts, and state snapshots. `SessionState` is the
compact control snapshot actively managed by a local runner or Temporal
workflow.

The core owns:

- agent definition/version records
- `SessionState`
- `RunState`
- turn/context state
- pending effects
- tool registry/profile state
- resolved turn context snapshots
- active transcript boundary/context refs and context pressure state
- per-call tool batch state
- lifecycle and status transitions
- deterministic mapping from receipts to next work

The core does not own:

- HTTP calls to LLM providers
- filesystem operations
- process execution
- wall-clock sampling
- sleeps/timers
- network I/O
- Temporal APIs
- policy reviewer implementation details
- hook command execution

### 4.2 Runners

A runner is responsible for driving the core to quiescence.

It repeatedly:

1. accepts external inputs
2. appends input events
3. applies events to state
4. asks the core for effect intents
5. executes effects with adapters
6. appends receipt events
7. emits projection events for UI/log consumers

The runner boundary also provides a submission/event queue protocol. Every
external submission has a stable submission id and optional correlation id.
Projection events should carry enough ids to let clients
reconstruct which submission, run, turn, effect, or tool call they belong to.
Future hook and policy extensions should use the same correlation scheme.

There are two target runners.

**Local runner:** runs in-process on Tokio. It is the default for unit tests,
development, and the first CLI.

**Temporal runner:** runs one session as a Temporal workflow. It schedules LLM
and tool effects as activities, receives user/control-plane input as signals or
updates, and uses Temporal timers/retries/cancellation where appropriate.

### 4.3 Effect adapters

Effect adapters turn `AgentEffectIntent` into `AgentReceipt` events. They are
ordinary async Rust code and may perform I/O.

The `forge-agent` crate should define only core agent SDK contracts and
ecosystem-level effects:

- LLM generation via `forge-llm`
- token counting and compaction via `forge-llm` when provider-supported
- MCP tool calls
- confirmation where the runner supports it
- generic tool invocation

Blob storage is runner/adapter infrastructure, not a core agent effect.
Effects and receipts carry blob refs; adapters read and write bytes through
an injected blob store while materializing LLM context, tool results, raw
provider payloads, and transcript content.

Host execution is not an agent-core effect family. Shell commands, filesystem
operations, host sessions, sandboxes, containers, and remote executors are
runner/tool-package concerns. A local runner may provide standard host tools,
and a Temporal runner may map the same logical tools to activities or remote
workers, but `forge-agent` should not hardcode `HostExec`, `HostFs`, or
host-session effect variants.

Adapters must be idempotency-aware. The intent id is the idempotency key. A
Temporal activity retry must not turn one logical tool call into multiple
logical tool results.

Deferred extension adapters:

- hook execution
- policy review and permission resolution
- approval surfaces beyond basic confirmation
- dynamic tool discovery/loading
- runner-specific tool packages

### 4.4 Projections

The runtime event stream is the durable, structured source for UI and logs.
Human-facing views are projections, not authoritative state.

Important projections:

- CLI transcript
- run status
- stable turn/thread item lifecycle
- tool call tree
- context/compaction status
- token/cost usage
- file change summary
- Attractor stage result

Future extension projections should cover hook runs, approval requests,
sandbox/policy attempts, and dynamic tool loading without changing the core
event model.

The CLI should consume the same events a web UI or Attractor bridge would
consume.

## 5. Domain Model

Names below are conceptual. Rust names may evolve, but the concepts should stay
stable.

### 5.1 Identifiers

- `AgentId`: stable id for a reusable agent definition.
- `AgentVersionId`: immutable version id for prompts, tools, defaults, and
  other reusable agent configuration.
- `SessionId`: stable id for a conversation/thread.
- `RunId`: one user/task submission inside a session.
- `TurnId`: one LLM sampling turn inside a run.
- `EffectId`: deterministic id for an effect intent.
- `SubmissionId`: externally submitted command/input id for queue identity and
  idempotency.
- `ToolBatchId`: a group of tool calls returned by one LLM response.
- `ToolCallId`: a provider/model tool call id normalized by Forge.
- `ProjectionItemId`: stable id for a user, assistant, reasoning, tool,
  patch, or compaction item in UI/event projections.
- `JournalSeq`: monotonically increasing sequence within one session journal.
- `BlobRef`: content-addressed large payload reference serialized as a
  `sha256:<64hex>` string. Semantic role, preview text, media type, and display
  metadata belong to transcript, context, turn, projection, or compaction records
  rather than the storage reference itself.
- `TranscriptBoundary`: stable boundary marker for a transcript entry/event
  position used by forks, rewrites, rollbacks, and active context control.

IDs must be explicit in persisted events. Never depend on vector position as a
durable identity.

Deferred SDK extensions may add ids such as `HookRunId`, `PermissionGrantId`,
or policy-review ids without changing the core id scheme.

### 5.2 Agent definition and version

An agent is a reusable, versioned configuration bundle. It is not a session.

An agent definition includes:

- agent id
- stable string handle
- optional aliases/deployment labels outside the core crate

An agent version includes:

- agent version id
- agent id
- display name and description, because these may feed prompts and can change
  by version
- system and developer prompt refs
- default run config: provider, model, reasoning effort, output limits
- context and loop limits
- tool registry and default tool profile
- skill/plugin/app refs where applicable
- opaque extension config refs for future hooks, policy, and dynamic tools

Agent versions should be treated as immutable once used by a session/run. If a
prompt, model, tool profile, or skill changes mid-session, that change is
recorded explicitly and future runs/turns use a new effective config boundary.
The first cut does not need to model tenant or owner concepts; those can be
metadata/external linkage outside the core model.

### 5.3 Session

A session is a concrete timeline under an agent/version. Some products may use
one long-lived central session, while others may create many user/task sessions.
The SDK should support both by keeping identity and external linkage explicit
without baking in tenancy.

Session lifecycle:

```text
new -> active -> paused -> active -> closing -> closed
                  \------------------------/
```

Session state includes:

- session id
- agent id and effective agent version id
- lifecycle/status
- effective config revision
- current run, if any
- compact completed run summaries and refs
- turn/context state
- tool registry and selected profile
- tool runtime context
- pending follow-up inputs
- pending steering inputs
- pending confirmation requests
- active transcript boundary and active context/blob refs
- thread metadata, such as name, memory mode, and external linkage
- extension config refs for future hooks, policy, and dynamic tools
- latest journal sequence applied

Session state is a bounded control snapshot. It should not contain the full
transcript, full raw provider responses, full tool outputs, or unbounded run
history. Those belong in journal/projection records plus blob refs.

Only one foreground run is active in a session at a time. Subagents are separate
sessions with parent/child metadata.

### 5.4 Session lineage and forks

Sessions must be able to continue from an existing transcript or message
history that originated in another session. This is a first-class fork
capability, not an ad hoc copy/paste operation.

A new session may be created with an optional source:

- no source: start from an empty transcript plus configured initial context
- transcript prefix: continue from all messages up to a specific message/event
- transcript snapshot: continue from a compacted or exported message-history ref
- parent session/run: create a child/subagent session with inherited context

The forked session gets a new `SessionId` and independent future event log. It
retains lineage metadata pointing at the source session, source message/event
boundary, optional materialized history blob refs, and fork reason. New
events never mutate the source session.

Fork semantics are used for:

- manual "continue from here" workflows
- alternative branches from the same conversation state
- subagent startup from parent context
- retries or experiments that should not pollute the original session
- importing externally captured message history into Forge-native sessions

The active context planner should treat forked transcript messages like normal
durable inputs while preserving provenance so projections can show where the
history came from.

### 5.4.1 History rewrite and rollback

History rewrite is a first-class operation. The agent must support replacing
the active transcript with a compacted transcript, rolling back the last N user
turns, or pruning an unfinished live turn without mutating historical source
events.

A rewrite records:

- rewrite id and cause
- source transcript range
- replacement transcript/blob refs
- whether local filesystem changes were affected, if known
- resulting active transcript boundary

The full rewrite/rollback audit trail lives in the scoped journal and transcript
or projection records. Active `SessionState` only needs compact history control
state: the resulting active boundary, latest rewrite/rollback ids or refs, and
counts useful for deterministic next-step decisions.

Rollback changes model context only. It must not imply that external tool
effects, such as filesystem changes performed by a host tool package, have been
reverted.

### 5.5 Run

A run is the unit of user-visible work: "implement this", "review that",
"continue", or an Attractor stage prompt.

Run lifecycle:

```text
queued -> running -> waiting -> running -> completed
                     \-------> failed
                     \-------> cancelled
                     \-------> interrupted
```

Run state includes:

- run id
- cause and source
- effective agent version id and config revision
- effective `RunConfig`
- input refs
- current turn plan
- active LLM request, if any
- compact recent/completed tool-batch refs needed for next-step decisions
- active tool batch, if any
- pending effects
- latest output ref
- usage and cost records
- outcome

### 5.6 Resolved turn context

Before every LLM turn or tool batch, Forge resolves an immutable turn context
from session/run provenance plus effective run and turn configuration. This
mirrors Codex's `TurnContext` without copying its implementation. `RunConfig`
contains execution knobs; agent version id and config revision are provenance
fields on run state and resolved turn context.

The resolved turn context includes:

- agent version id and config revision
- provider, model, reasoning effort, reasoning summary, service tier, and
  response/structured-output settings
- current date/timezone and runtime/extension refs supplied by the runner
- base, developer, user, skill, plugin, app, domain, and runtime context refs
- selected tool profile and model-visible tool specs
- truncation and compaction policy
- correlation metadata
- extension context refs for future hooks, policy, and dynamic tool/MCP exposure

The resolved context is persisted or reproducible from persisted events. Tool
execution must use this snapshot, not mutable global process state.

### 5.7 Turn and context window

A turn is one LLM sampling request and the immediate response processing around
it. The agent may perform many turns in one run:

```text
LLM turn -> tool batch -> tool result turn -> LLM turn -> final answer
```

Forge uses an AOS-style context planner. Inputs are typed and prioritized
instead of treated as one growing string.

Turn input lanes:

- system
- developer
- conversation
- tool result
- steer
- summary
- memory
- skill
- domain
- runtime hint
- custom

The turn planner selects an active window from:

- pinned inputs
- durable inputs
- new run inputs
- tool result inputs
- steering inputs
- summaries/compactions
- tool definitions
- response format and provider options

The planner returns:

- selected message refs
- selected tool ids
- tool choice
- response format ref
- provider options ref
- prerequisites
- decision report

Required inputs may exceed normal budgets, but that should be visible in the
turn report. Non-required inputs may be dropped by priority and budget.

The context state should include:

- next transcript sequence and active transcript range
- active window items with provenance and provider compatibility metadata
- token count records with quality (`exact`, provider estimate, local estimate)
- context pressure records with reason and provider/model metadata
- pending context operation state
- latest compaction summary with source range, replacement blobs, and
  warnings

Full transcript ledgers and full compaction history live in journal/projection
surfaces, not unbounded workflow state. The planner should make context
decisions from active window items, transcript range/sequence counters,
`compacted_through`, pending context operation, and latest count/pressure
summaries.

Provider-native or opaque context blobs are allowed, but they must carry
provider compatibility metadata and cannot be silently reused with incompatible
providers.

### 5.8 Transcript and blobs

Large data should be stored by reference, not embedded everywhere.

Examples:

- user prompt body
- assistant message body
- reasoning summary
- raw LLM response
- tool arguments
- full tool output
- large file contents
- generated patches
- compaction output

Events may include short previews for UI convenience, but durable semantics
should point at refs.

Transcript items are product/UI-facing projection records, not unbounded active
workflow state. A transcript item includes:

- projection item id or transcript item id
- session id and journal sequence
- optional run id, turn id, effect id, tool batch id, and tool call id joins
- kind: user, assistant, reasoning, tool result, summary, patch, status, custom
- lifecycle/status where needed
- content ref and preview
- source event id
- timestamps

UIs follow the journal and transcript/projection records, then fetch blob
bodies as needed. They should not depend on Temporal workflow state.

### 5.9 Tool batch state

Tool calls returned by one model response form a tool batch. The batch records:

- observed calls exactly as the model/provider returned them
- planned calls after registry lookup and profile/capability filtering
- accepted, ignored, queued, pending, succeeded, failed, and cancelled status
  for each call
- execution groups based on parallel-safety and resource-key conflicts
- argument refs and full output/result refs
- model-visible truncated outputs
- source LLM output ref that produced the batch

Composite tools may emit multiple internal effects before producing one
model-visible result. Their intermediate state must be durable and tied to the
same `ToolCallId` and `ToolBatchId`.

### 5.10 Diagnostics and projections

Diagnostics are derived from the scoped journal, resolved turn reports,
context/compaction records, and projection items. The first-cut core does not
maintain a separate diagnostic timeline. Runners or backends may build
diagnostic views or debug timelines outside active session state.

## 6. Event and Journal Model

The agent has a scoped logical journal. In the local runner this may be an
explicit append-only SQLite/Postgres-backed log. In the Temporal runner,
Temporal history provides execution durability, but Forge should still persist
logical agent events as the product-level audit/follow stream.

A journal event includes:

- event id and monotonically increasing session-local sequence
- session id
- optional join ids grouped in a join block: run id, turn id, effect id, tool
  batch id, tool call id, submission id, correlation id, and causal
  parent event/effect ids
- observed timestamp supplied by the runner
- event kind with small inline data and refs for large payloads

The journal is replay-informative and forkable, but the system is not required
to rebuild every in-memory optimization from journal replay alone. Bounded
`SessionState` snapshots are the active control state; journal plus
transcript/projection records are the user-visible and auditable timeline.

### 6.1 Input events

Input events are accepted from users, CLIs, Attractor, tests, or service APIs.

Core input events:

- `SessionOpened`
- `SessionConfigUpdated`
- `RunRequested`
- `FollowUpInputAppended`
- `RunSteerRequested`
- `RunInterruptRequested`
- `SessionPaused`
- `SessionResumed`
- `SessionClosed`
- `TurnContextOverrideRequested`
- `SessionHistoryRewriteRequested`
- `SessionHistoryRollbackRequested`
- `ToolRegistrySet`
- `ToolProfileSelected`
- `ToolOverridesSet`
- `ConfirmationProvided`

Deferred hook/policy extensions may add events such as `HookRegistrySet`,
`ApprovalDecisionProvided`, `PermissionGrantProvided`, or
`ElicitationDecisionProvided`.

### 6.2 Lifecycle events

Lifecycle events are emitted by the core when state transitions happen:

- `SessionLifecycleChanged`
- `SessionStatusChanged`
- `RunLifecycleChanged`
- `TurnStarted`
- `TurnCompleted`
- `TurnFailed`
- `ToolBatchStarted`
- `ToolBatchCompleted`
- `ContextOperationStarted`
- `ContextOperationCompleted`
- `ContextPressureRecorded`
- `HistoryRewriteCompleted`

Deferred hook/policy extensions may add lifecycle events such as
`HookRunStarted`, `HookRunCompleted`, `ApprovalRequestStarted`, or
`ApprovalRequestCompleted`.

### 6.3 Effect events

Effects are represented in two phases:

- `EffectIntentRecorded`
- `EffectReceiptRecorded`
- `EffectStreamFrameObserved` for non-authoritative streaming progress

The intent must be recorded before external execution begins. The receipt
settles the intent. A failed effect produces a receipt with failure details,
not an invisible runner error.

### 6.4 Observation events

Observation events make the runtime visible to projections:

- `UserMessageObserved`
- `AssistantMessageObserved`
- `ReasoningObserved`
- `ToolCallObserved`
- `ToolOutputObserved`
- `FileChangeObserved`
- `ProjectionItemObserved`
- `TokenUsageObserved`
- `WarningObserved`
- `CostObserved`

Deferred hook/policy extensions may add observation events such as
`HookOutputObserved`, `ApprovalRequestObserved`, or `SandboxAttemptObserved`.

Observation events must not be the only source of state needed for replay. They
are projections over authoritative input/lifecycle/effect events.

### 6.5 Configuration boundary events

Agent/session configuration can change over time, but changes must create an
explicit boundary in the journal. Events such as `SessionConfigUpdated`,
`ToolRegistrySet`, `ToolProfileSelected`, or a future `AgentVersionChanged`
record the change. Every run or resolved turn context should reference the
effective agent version/config revision it used.

## 7. Effect Model

Effects are the only way the agent interacts with the outside world.

### 7.1 LLM effects

LLM effects are backed by `forge-llm`.

Initial LLM effect kinds:

- `LlmComplete`
- `LlmStream`
- `LlmCountTokens`
- `LlmCompact`

The request includes:

- provider id
- model id
- reasoning effort
- max tokens
- messages/active window refs
- tool definitions
- tool choice
- response format
- provider options
- metadata

The receipt includes:

- assistant text/ref
- reasoning summary/ref if available
- normalized tool calls
- raw provider response ref
- usage
- finish reason
- provider response id if available
- retry metadata

Streaming is represented as progress observations plus one terminal receipt.
Replay must not require re-streaming from the provider.

### 7.2 Tool effects

The tool registry maps model-visible tools to logical tool invocations. Tool
execution is an SDK extension point: the core records tool calls, plans batches,
emits generic invocation intents, and records receipts, while a runner or tool
package decides how those invocations are executed.

A tool spec includes:

- stable tool id
- provider-visible tool name
- description
- JSON schema for arguments
- mapper id or mapping strategy
- executor binding or handler id
- parallelism hint
- resource key
- required capabilities
- extension config refs, when needed

Tool execution may be parallel when:

- every selected tool is marked parallel safe
- resource keys do not conflict
- the provider/tool profile permits parallel results

The model receives truncated tool output when needed. The event stream and blob
store retain the full output.

Tool routing must be explicit enough to support static Forge tools, MCP tools,
provider-native tools, runner-local tools, remote tools, and dynamically
registered third-party tools. If a tool is visible to the model but unavailable
at execution time, the tool result should explain that failure to the model
instead of disappearing.

The tool execution API is split into three responsibilities:

- the core loop plans tool batches and emits `ToolInvoke` effect intents
- the tool dispatcher resolves an intent to a registered handler, validates and
  prepares arguments, and converts handler output into receipts
- the runner-specific execution driver decides how to run, wait for, cancel,
  and collect terminal outcomes for one or more dispatch requests

The dispatcher must not own runtime task primitives such as Tokio spawning,
Tokio joins, timers, channels, or Temporal workflow selectors. It may prepare
dispatch groups and invoke a single handler through an injected driver, but the
local runner, Temporal runner, or test harness owns the actual concurrency
mechanics. This preserves the same tool SDK for local execution and Temporal
execution, where workflow code must use Temporal activity futures, selectors,
cancellation scopes, and timers instead of Tokio scheduling primitives.

Tool handlers implement one logical invocation. A handler receives a normalized
`ToolInvocationRequest`, runtime context, and blob access. It returns
exactly one terminal `ToolInvocationReceipt` for that invocation. Handlers
should not read or mutate `SessionState`, call the reducer or decider, or decide
when the next LLM turn starts.

Tool progress streaming is deferred for the first dispatcher implementation.
The core event model already has `EffectStreamFrameObserved` for
non-authoritative streaming progress, and future tool runtimes may use it for
stdout/stderr chunks, progress updates, adapter metadata, UI logs, and
diagnostics. P42 should keep the handler API small and terminal-receipt driven,
then add a tool progress sink later once local and Temporal runner needs are
clear. Replay must not require re-running a stream.

Tool results may complete out of order at runtime, but model-visible tool result
ordering must remain deterministic. Runners should append terminal receipts as
they are observed, while the context planner should present tool results in
planned-call order or another explicit stable order.

Long-running tools that should let the model reason before underlying work
finishes must be modeled as resumable/background tools, not as partial reducer
state. The Codex-style pattern is:

```text
start tool -> return model-visible "running" receipt with handle and output snapshot
later poll/interaction tool call -> return another receipt with new output or completion
```

Future stream frames from a long-running tool remain projection-only unless the
tool chooses to return a model-visible receipt. A background host process,
remote job, or subagent wait may therefore continue adapter-locally after a
terminal receipt, as long as the receipt includes a durable handle and enough
metadata for future poll, write, interrupt, or close calls.

#### Codex reference: long-running CLI updates

Codex does not generally feed arbitrary intermediate tool stream frames back
into the same active LLM request. For normal tool calls, it starts tool futures
as completed model stream items arrive, allows parallel-safe tools to run
concurrently, records live UI events while they execute, then drains the
terminal tool outputs before the next follow-up LLM turn.

Codex's more advanced long-running CLI behavior comes from its unified exec
tool. A command runs under a process manager and streams stdout/stderr to the
UI. After a configured yield window, the tool can return a model-visible
terminal result even if the process is still alive. That result includes a
process/session handle, elapsed time, output snapshot, and "still running"
metadata. The next LLM turn can reason over that snapshot and choose to poll,
write stdin, interrupt, or close the process through later tool calls.

Forge should preserve this semantic pattern without copying Codex's Tokio
implementation. For the first Forge dispatcher, the resumable/background tool
chooses when to produce a model-visible receipt with a durable continuation
handle; live tool stream-frame sinks are a later observability extension.

### 7.3 Host tools

Host access is a tool package, not part of the `forge-agent` core crate.
Shell/process execution, filesystem read/write/edit/apply-patch/list/glob/grep,
host sessions, local sandboxes, containers, and remote executors may be provided
by a local runner, a Temporal activity worker, or a separate crate such as a
future `forge-agent-tools-host`.

Host tools should be registered like any other tool. Their implementation may
maintain adapter-local session state, but the core should only need stable tool
ids, schemas, capability requirements, parallelism/resource hints, invocation
intents, receipts, blob refs, and observations.

Host tool receipts should include enough execution metadata to explain what was
attempted: cwd/environment, command/process details, exit status, output refs,
and any adapter-local sandbox or permission information when the adapter
enforces one. A full policy/approval framework is deferred.

Host process tools should prefer the resumable/background pattern for commands
that can outlive one model turn. A command tool may return after a configured
yield window with a process/session handle, output snapshot, and "still running"
metadata, then expose separate poll/write/interrupt/close tools. This lets the
model continue reasoning while long-running work proceeds without requiring the
core dispatcher to inject partial tool outputs into an active LLM request.

### 7.4 Confirmation effects

Confirmation is an effect from the agent's perspective. The core may enter
`waiting` with a pending confirmation request. The runner decides how to surface
that request:

- local CLI prompt
- web/API request
- Temporal signal/update
- Attractor HITL bridge
- policy service
- another agent

The response is appended as an input event and the run resumes.

### 7.5 Deferred policy and safety extensions

The first cut should not include a full permission, approval, sandbox, or policy
review framework. Forge should keep the core loop small enough to finish, while
preserving an SDK extension seam where policy can be added later.

The future policy layer should be implemented mostly through lifecycle
extension points around:

- session start
- user prompt submission
- tool preflight
- permission or approval request
- tool completion
- stop/interruption

Likely future concepts:

- permission profiles with filesystem, network, and tool/MCP permissions
- approval policies and approval decisions
- sandbox attempts and escalation retries
- turn-scoped and session-scoped permission grants
- policy-review events and projections

Those concepts should become durable state/effects when implemented, but they
are not required for the initial core loop.

### 7.6 Subagent effects

Subagents are separate sessions. A parent can request:

- spawn
- send input
- wait
- interrupt
- close

Subagent execution should use the same runner family as the parent unless
explicitly overridden. A Temporal parent should normally spawn child workflows;
a local parent should spawn local sessions.

Depth limits are enforced by the parent runner/core. Parent and child
relationships should also define:

- inherited context and runner/tool capability configuration
- agent role/type metadata
- event forwarding and filtering
- cancellation propagation
- final status mapping (`running`, `interrupted`, `completed`, `errored`,
  `shutdown`, `not_found`)

Future policy extensions should define how child approval or permission
requests route through the parent when required.

### 7.7 Deferred hook effects

Hooks are future SDK extension points around the agent loop. They are not part
of the first-cut core. The core should be shaped so hooks can later be recorded
as effect intents/receipts without changing the session/run/turn model.

Potential hook points:

- `SessionStart`
- `UserPromptSubmit`
- `PreToolUse`
- `PermissionRequest`
- `PostToolUse`
- `Stop`

Hooks may eventually be synchronous or asynchronous, scoped to session or turn,
and sourced from user, project, system, managed config, plugin, or future
control-plane configuration. Hook outcomes may:

- add model-visible context within a budget
- emit warnings or diagnostics
- block/stop a tool or turn
- answer a permission request
- produce output stored by blob ref when large

When implemented, hook execution must be idempotency-aware and observable
through hook started, completed, failed, blocked, or stopped events.

## 8. Temporal Mapping

Temporal integration is a runner, not the domain model.

### 8.1 Workflow shape

The initial Temporal shape should be one workflow per agent session:

```text
AgentSessionWorkflow(session_id)
```

The workflow owns the current `SessionState` and drives runs to quiescence.
External control enters through signals or updates:

- open/configure session
- request run
- append follow-up
- steer active run
- interrupt active run
- provide confirmation
- request history rewrite/rollback
- pause/resume/close

Future hook/policy extensions may add signals or updates for approval
decisions, permission grants, and elicitation decisions.

### 8.2 Activities

All non-deterministic or effectful work is an activity:

- LLM complete/stream/count/compact
- generic tool invocation
- runner-provided host tools, when a host tool package is installed
- MCP calls
- blob/CAS store operations
- external notification
- black-box CLI agent invocation

Future hook/policy extensions add activities for hook execution, policy review,
permission resolution, and approval surfaces.

Workflow code must not call `forge-llm`, shell, filesystem, network, wall-clock,
or random APIs directly.

Workflow code must also not use Tokio task-spawning or Tokio join primitives as
the semantic implementation of agent tool parallelism. The workflow should
record the same logical `ToolInvoke` intents as the local runner, then schedule
activities with Temporal-native futures/selectors and cancellation scopes. A
Temporal activity may emit progress to an external projection/event store and
heartbeat for liveness, but the terminal activity result is the receipt that
advances the reducer.

### 8.3 History size and continue-as-new

Temporal history must not become the transcript store.

Large payloads go into a blob/CAS store and are referenced by events.
The workflow should continue-as-new at safe boundaries, such as:

- after a run completes
- after compaction
- when history size approaches a configured threshold

The continue-as-new payload should include compact `SessionState` plus refs, not
full transcript bodies.

### 8.4 Retries and idempotency

Temporal activity retries are allowed only when the effect adapter is
idempotency-aware. Every effect intent carries an `EffectId`. Adapters use that
id to dedupe externally visible results where possible.

Failures are classified as:

- retryable transport/provider failures
- non-retryable configuration/auth failures
- tool-level failures visible to the model
- runner/system failures visible to the host

Tool-level failures should usually become tool result receipts so the model can
recover. System failures may fail or cancel the run.

### 8.5 Cancellation and interruption

`RunInterruptRequested` is a domain input. The runner maps it to backend
cancellation mechanics:

- cancel active LLM stream/activity
- terminate or signal running host processes
- cancel child subagent waits where appropriate
- append cancellation/interruption receipts
- transition the run deterministically

Cancellation must settle or abandon pending effects explicitly. No pending
effect should disappear without a lifecycle event.

## 9. `forge-llm` Integration

The native agent constructs `forge_llm::Request` values from turn plans.

The mapping includes:

- active window message refs -> `Message`
- selected tools -> `ToolDefinition`
- tool choice -> provider-supported tool choice
- run config -> provider/model/reasoning/max tokens
- response format ref -> response format
- provider options ref -> provider options

The agent then calls `Client.complete()` or `Client.stream()` through an effect
adapter.

Provider-specific formatting belongs in `forge-llm` adapters or provider
profiles, not in arbitrary agent code. The agent may still own provider-aligned
tool profiles because model-visible tool shape is an agent/runtime decision.

CLI `AgentProvider` backends from `forge-llm` may be wrapped as a special effect
kind:

```text
ExternalAgentRun(provider = "codex-cli" | "claude-code" | "gemini-cli")
```

The receipt is normalized into Forge assistant/tool/usage events where the CLI
output makes that possible.

## 10. CLI Design

The CLI is a host surface over the agent runtime. It should feel closer to
Codex/AOS chat than to the current Attractor-only CLI.

The CLI should support:

- start a new session
- resume/list sessions
- submit user messages
- stream assistant/tool events
- steer an active run
- interrupt an active run
- select provider/model/reasoning
- select tool mode/profile
- inspect resolved turn context
- respond to confirmation requests
- show compaction and rollback status
- fork or roll back a session transcript
- inspect transcript and run state
- inspect tool calls and blob-backed outputs

Future hook/policy extensions may add CLI surfaces for permissions, sandbox
state, approval requests, hook status, and elicitation requests.

The CLI must consume projections from the runtime event stream. It should not
have a separate agent loop.

The local CLI can use the local runner. A future server-backed CLI can connect
to a Temporal-backed service using the same logical operations.

## 11. Attractor Integration

Attractor is not the first implementation target of this rewrite, but the
agent must be shaped so Attractor can use it later.

The future Attractor backend should be able to:

- create or resume an agent session for a pipeline/stage
- submit a stage prompt as a run
- pass provider/model/reasoning overrides
- stream tool and assistant events into Attractor logs
- receive a structured run outcome
- persist links from pipeline node attempts to agent session/run ids

Attractor should not know about individual LLM/tool rounds. It should treat the
agent as a durable codergen backend with structured observability.

## 12. Persistence and Storage

The core storage contract is logical:

- append/read scoped agent journal events
- put/get content-addressed blob bytes
- read/write bounded session state snapshots
- read agent definitions and immutable agent versions
- read transcript/projection items
- read session/run projections
- query by session id, run id, and external linkage
- create sessions from transcript boundaries or message-history snapshots
- query session lineage and fork relationships
- store and query diagnostic projections derived from journal/projection data

Future hook/policy extensions may add storage for permission grants, approval
decisions, hook outputs, and policy-review records.

The local runner may initially use filesystem storage. The Temporal runner may
use Temporal history for execution recovery and an external event/blob store
for product-facing logs and large payloads.

Payload rule:

- small metadata may be inline
- large or user/model/tool content should be stored by ref
- raw provider responses should be stored by ref
- full tool output should be stored by ref

The storage abstraction should not leak CXDB, filesystem, or Temporal details
into the agent core.

First-cut `forge-agent` storage contracts should live under `storage/`:

- `storage::JournalStore` for scoped event append/read
- `storage::SnapshotStore` for bounded `SessionState` snapshots
- `storage::BlobStore` for content-addressed blob bytes referenced by model
  records
- `storage::AgentDefinitionStore` for reusable agent definitions and immutable
  agent versions

Tool, LLM, planner, and runner code should depend on these logical contracts
instead of defining surface-specific blob readers. More specialized stores,
such as projection/query stores, idempotency indexes, and runtime handle stores,
can be added as those phases need them.

The first production backend may use explicit tables for agent definitions,
agent versions, sessions, session snapshots, journal events, transcript items,
runs/tool-call indexes, and blobs. These tables are not all equally
authoritative: agent versions define reusable configuration, the journal
records what happened in a session, transcript/projection rows support UI and
query, snapshots support execution resume, and blob/CAS storage owns large bytes.

### 12.1 Storage backend direction

Forge is expected to sunset CXDB and use Postgres as the primary production
runtime store. This does not change the core agent design.

The agent core should continue to target the logical storage contract above:
append/read events, store blobs, read projections, and query linkage. A
Postgres implementation should satisfy that contract with ordinary tables,
indexes, transactions, idempotency constraints, JSONB/bytea payloads, and
migrations.

Temporal storage remains separate. Temporal may use Postgres internally for its
own service state, but Forge must not treat Temporal history as the product
event/blob store.

## 13. Error Handling

Errors must be classified at the boundary where they occur.

### 13.1 Tool-visible errors

These become tool result receipts with `is_error = true`. The model sees them
and can recover.

Examples:

- runner-provided shell tool exits non-zero
- grep finds no matches when that is represented as an error by the tool
- patch failed to apply
- file not found for a requested read

### 13.2 Run-visible errors

These fail, interrupt, or cancel the current run.

Examples:

- context cannot be built
- selected tool profile is invalid
- LLM context length exceeded and no compaction path is available
- user interrupts the run

### 13.3 Session-visible errors

These close or poison the session until reconfigured.

Examples:

- invalid provider configuration
- missing authentication for required provider
- corrupt persisted session state
- incompatible state schema version

### 13.4 Runner/system errors

These are backend failures. They should be surfaced with enough metadata to
debug and retry from the host.

Examples:

- Temporal activity worker unavailable
- blob store unavailable
- local runner task panic
- filesystem persistence failure in required mode

## 14. Testing Strategy

The core should be tested with deterministic event sequences:

- input event -> state transition
- state -> expected effect intents
- receipt event -> state transition
- full run with fake LLM and fake tools

Tests should not depend on wall-clock timing, provider credentials, Temporal,
or external services unless marked `#[ignore]`.

Important test categories:

- lifecycle transitions
- turn planning and budget behavior
- tool profile selection
- LLM receipt normalization
- tool batch planning and parallelism grouping
- async dispatcher behavior with out-of-order completions and stable
  model-visible ordering
- resumable/background tool receipts and follow-up poll/interaction calls
- per-call tool status and composite tool progress
- history rewrite, rollback, fork lineage, and transcript source ranges
- interruption/cancellation settlement
- context compaction prerequisites
- diagnostic projection summaries when derived diagnostic records are dropped
- local runner end-to-end with fake adapters
- Temporal mapping tests with mocked activities or Temporal test utilities

Future hook/policy work should add focused tests for policy-review effects,
approval decisions, sandbox attempt metadata, hook context injection, blocking,
stopping, and large hook output refs.

Live provider and live Temporal tests are integration tests and must be ignored
by default.

## 15. Compatibility With Existing Specs

This spec supersedes the old `spec/02-coding-agent-loop-spec.md` as the primary
target for the `crates/forge-agent/` rewrite. Spec 02 remains useful historical
and design reference material, especially for provider-aligned tools,
truncation, subagents, and event vocabulary.

`spec/01-unified-llm-spec.md` remains the source of truth for `forge-llm`.

`spec/03-attractor-spec.md` remains the source of truth for DOT pipeline
orchestration. Attractor integration with the new agent should be designed
after the agent runtime is stable.

## 16. Open Questions

These are intentionally left open until implementation pressure clarifies them:

- Should the first Temporal workflow be per session or per run?
- Which blob/CAS backend should be the first production target?
- How much of the old `refs/forge-agent/` tool implementation should be copied
  versus reimplemented from AOS concepts?
- Which provider/tool profile should be implemented first?
- Should compaction be a first-class effect in the first implementation or a
  later extension?
- How much of the AOS CLI chat projection should be copied directly?

These questions should be resolved through roadmap documents and implementation
spikes, not by expanding this design spec into a task list.
