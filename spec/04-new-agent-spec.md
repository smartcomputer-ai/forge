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
- transcript ledger, source ranges, and provider-native context artifacts
- context pressure, token counting, and compaction operation state
- per-call tool batch status, execution groups, and composite tool progress
- host session and workspace concepts
- bounded run traces for diagnostics and replay explanation
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

- submission/event queue correlation ids and trace propagation
- resolved turn context snapshots, including cwd, environment, model, and tools
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
- Keep LLM calls, shell commands, filesystem writes, MCP calls, and subagent
  operations outside deterministic state reduction.
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

- session/run/turn lifecycle
- deterministic event/effect state transitions
- transcript ledger and artifact refs
- active context planning, token counting, and context pressure handling
- LLM generation through `forge-llm`
- host/session/filesystem/shell effects
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
  pure state transitions, event model, effect intent model, projections

Effect adapters
  forge-llm, host filesystem/shell, MCP, blob/artifact store, subagents

Future SDK extensions
  hooks, policy reviewers, approval surfaces, dynamic tools
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

The core owns:

- `SessionState`
- `RunState`
- turn/context state
- pending effects
- tool registry/profile state
- resolved turn context snapshots
- transcript references
- transcript ledger and context pressure state
- per-call tool batch state
- bounded run trace summaries
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
external submission has a stable submission id, correlation id, and optional
trace context. Projection events should carry enough ids to let clients
reconstruct which submission, run, turn, effect, or tool call they belong to.
Future hook and policy extensions should use the same correlation scheme.

There are two target runners.

**Local runner:** runs in-process on Tokio. It is the default for unit tests,
development, and the first CLI.

**Temporal runner:** runs one session as a Temporal workflow. It schedules LLM
and tool effects as activities, receives human/user control as signals or
updates, and uses Temporal timers/retries/cancellation where appropriate.

### 4.3 Effect adapters

Effect adapters turn `AgentEffectIntent` into `AgentReceipt` events. They are
ordinary async Rust code and may perform I/O.

Initial adapters:

- LLM generation via `forge-llm`
- token counting and compaction via `forge-llm` when provider-supported
- blob/artifact put/get
- host session open/close
- shell command execution
- filesystem read/write/edit/apply-patch/list/glob/grep/stat
- MCP tool calls
- subagent spawn/send/wait/close
- human input where the runner supports it

Adapters must be idempotency-aware. The intent id is the idempotency key. A
Temporal activity retry must not turn one logical tool call into multiple
logical tool results.

Deferred extension adapters:

- hook execution
- policy review and permission resolution
- approval surfaces beyond basic human input
- dynamic tool discovery/loading

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
- bounded run trace
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
- `ArtifactRef`: content-addressed or store-addressed large payload reference.
- `TranscriptRef`: a stable reference to a transcript prefix or message-history
  snapshot that can seed another session.

IDs must be explicit in persisted events. Never depend on vector position as a
durable identity.

Deferred SDK extensions may add ids such as `HookRunId`, `PermissionGrantId`,
or policy-review ids without changing the core id scheme.

### 5.2 Session

A session is the long-lived container for configuration, transcript, tool
registry, host runtime context, and run history.

Session lifecycle:

```text
new -> active -> paused -> active -> closing -> closed
                  \------------------------/
```

Session state includes:

- session id
- lifecycle/status
- base `SessionConfig`
- current run, if any
- completed run summaries
- turn/context state
- tool registry and selected profile
- host runtime context
- pending follow-up inputs
- pending steering inputs
- pending human requests
- transcript/artifact refs
- thread metadata, such as name, memory mode, and external linkage
- extension config refs for future hooks, policy, and dynamic tools

Only one foreground run is active in a session at a time. Subagents are separate
sessions with parent/child metadata.

### 5.3 Session lineage and forks

Sessions must be able to continue from an existing transcript or message
history that originated in another session. This is a first-class fork
capability, not an ad hoc copy/paste operation.

A new session may be created with an optional source:

- no source: start from an empty transcript plus configured initial context
- transcript prefix: continue from all messages up to a specific message/event
- transcript snapshot: continue from a compacted or exported message-history ref
- parent session/run: create a child/subagent session with inherited context

The forked session gets a new `SessionId` and independent future event log. It
retains lineage metadata pointing at the source session, source transcript ref,
source message/event boundary, and fork reason. New events never mutate the
source session.

Fork semantics are used for:

- manual "continue from here" workflows
- alternative branches from the same conversation state
- subagent startup from parent context
- retries or experiments that should not pollute the original session
- importing externally captured message history into Forge-native sessions

The active context planner should treat forked transcript messages like normal
durable inputs while preserving provenance so projections can show where the
history came from.

### 5.3.1 History rewrite and rollback

History rewrite is a first-class operation. The agent must support replacing
the active transcript with a compacted transcript, rolling back the last N user
turns, or pruning an unfinished live turn without mutating historical source
events.

A rewrite records:

- rewrite id and cause
- source transcript range
- replacement transcript/artifact refs
- whether local filesystem changes were affected, if known
- resulting active transcript boundary

Rollback changes model context only. It must not imply that host filesystem
effects have been reverted.

### 5.4 Run

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
- effective `RunConfig`
- input refs
- current turn plan
- active LLM request, if any
- completed tool batches
- active tool batch, if any
- pending effects
- latest output ref
- usage and cost records
- run trace
- outcome

### 5.5 Resolved turn context

Before every LLM turn or tool batch, Forge resolves an immutable turn context
from session defaults plus run/turn overrides. This mirrors Codex's `TurnContext`
without copying its implementation.

The resolved turn context includes:

- provider, model, reasoning effort, reasoning summary, service tier, and
  response/structured-output settings
- working directory, selected environment, shell/runtime hints, current date,
  and timezone
- base, developer, user, skill, plugin, app, domain, and runtime context refs
- selected tool profile and model-visible tool specs
- truncation and compaction policy
- trace/correlation metadata
- extension context refs for future hooks, policy, and dynamic tool/MCP exposure

The resolved context is persisted or reproducible from persisted events. Tool
execution must use this snapshot, not mutable global process state.

### 5.6 Turn and context window

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

- transcript ledger with sequence numbers and source ranges
- active window items with provenance and provider compatibility metadata
- token count records with quality (`exact`, provider estimate, local estimate)
- context pressure records with reason and provider/model metadata
- compaction records with source range, preserved items, replacement artifacts,
  and warnings

Provider-native or opaque context artifacts are allowed, but they must carry
provider compatibility metadata and cannot be silently reused with incompatible
providers.

### 5.7 Transcript and artifacts

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

### 5.8 Tool batch state

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

### 5.9 Run trace

Each run maintains a bounded diagnostic trace separate from the authoritative
event log and from UI projections. The trace explains why the state machine made
decisions without forcing clients to replay every low-level event.

Trace entries should cover:

- run start/finish
- turn planning decisions
- LLM request/receipt
- tool call observation and planning
- effect emission and receipt settlement
- context pressure and compaction
- intervention, interruption, cancellation, and future extension decisions

Trace retention is bounded. When entries are dropped, the summary records how
many were dropped and the first/last retained sequence.

## 6. Event Model

The agent has a logical event log. In the local runner this may be an explicit
append-only log. In the Temporal runner, Temporal history provides execution
durability, but Forge should still persist or expose logical agent events as
the product-level transcript.

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
- `HumanInputProvided`

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

The tool registry maps model-visible tools to executor effects.

A tool spec includes:

- stable tool id
- provider-visible tool name
- description
- JSON schema for arguments
- mapper
- executor
- parallelism hint
- resource key

Tool execution may be parallel when:

- every selected tool is marked parallel safe
- resource keys do not conflict
- the provider/tool profile permits parallel results

The model receives truncated tool output when needed. The event stream and
artifact store retain the full output.

Tool routing must be explicit enough to support static Forge tools, MCP tools,
and provider-native tools. Future SDK extensions may add deferred/discoverable
tools and dynamically registered tools. If a tool is visible to the model but
unavailable at execution time, the tool result should explain that failure to
the model instead of disappearing.

### 7.3 Host session effects

Some tools require a host session: local process context, sandbox, container,
or future remote executor.

Host effects:

- `HostSessionOpen`
- `HostSessionClose`
- `HostExec`
- `HostSessionSignal`
- `HostFsRead`
- `HostFsWrite`
- `HostFsEdit`
- `HostFsApplyPatch`
- `HostFsGrep`
- `HostFsGlob`
- `HostFsStat`
- `HostFsExists`
- `HostFsListDir`

The host runtime context records the current host session id and status. If a
turn selects host tools before the host is ready, the turn planner emits an
`OpenHostSession` prerequisite instead of letting generation proceed with
unusable tools.

Host tool receipts should include enough execution metadata to explain what was
attempted: cwd/environment, command/process details, exit status, output refs,
and any adapter-local sandbox or permission information when the adapter
enforces one. A full policy/approval framework is deferred.

### 7.4 Human input effects

Human input is an effect from the agent's perspective. The core may enter
`waiting` with a pending human request. The runner decides how to surface that
request:

- local CLI prompt
- web/API request
- Temporal signal/update
- Attractor HITL bridge

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

- inherited context, environment, and shell/session configuration
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
- produce output stored by artifact ref when large

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
- provide human input
- request history rewrite/rollback
- pause/resume/close

Future hook/policy extensions may add signals or updates for approval
decisions, permission grants, and elicitation decisions.

### 8.2 Activities

All non-deterministic or effectful work is an activity:

- LLM complete/stream/count/compact
- shell/process execution
- filesystem operations
- MCP calls
- blob/artifact store operations
- external notification
- black-box CLI agent invocation

Future hook/policy extensions add activities for hook execution, policy review,
permission resolution, and approval surfaces.

Workflow code must not call `forge-llm`, shell, filesystem, network, wall-clock,
or random APIs directly.

### 8.3 History size and continue-as-new

Temporal history must not become the transcript store.

Large payloads go into an artifact/blob store and are referenced by events.
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
- respond to human-input requests
- show compaction and rollback status
- fork or roll back a session transcript
- inspect transcript and run state
- inspect tool calls and artifacts

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

- append/read agent events
- put/get artifact bytes
- read session/run projections
- query by session id, run id, and external linkage
- create sessions from transcript refs or message-history snapshots
- query session lineage and fork relationships
- store and query run trace summaries

Future hook/policy extensions may add storage for permission grants, approval
decisions, hook outputs, and policy-review records.

The local runner may initially use filesystem storage. The Temporal runner may
use Temporal history for execution recovery and an external artifact/event store
for product-facing logs and large payloads.

Payload rule:

- small metadata may be inline
- large or user/model/tool content should be stored by ref
- raw provider responses should be stored by ref
- full tool output should be stored by ref

The storage abstraction should not leak CXDB, filesystem, or Temporal details
into the agent core.

### 12.1 Storage backend direction

Forge is expected to sunset CXDB and use Postgres as the primary production
runtime store. This does not change the core agent design.

The agent core should continue to target the logical storage contract above:
append/read events, store artifacts, read projections, and query linkage. A
Postgres implementation should satisfy that contract with ordinary tables,
indexes, transactions, idempotency constraints, JSONB/bytea payloads, and
migrations.

Temporal storage remains separate. Temporal may use Postgres internally for its
own service state, but Forge must not treat Temporal history as the product
event/artifact store.

## 13. Error Handling

Errors must be classified at the boundary where they occur.

### 13.1 Tool-visible errors

These become tool result receipts with `is_error = true`. The model sees them
and can recover.

Examples:

- shell command exits non-zero
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
- artifact store unavailable
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
- per-call tool status and composite tool progress
- history rewrite, rollback, fork lineage, and transcript source ranges
- interruption/cancellation settlement
- context compaction prerequisites
- bounded run trace summaries when trace entries are dropped
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
- Which artifact store should be the first production target?
- How much of the old `refs/forge-agent/` tool implementation should be copied
  versus reimplemented from AOS concepts?
- Which provider/tool profile should be implemented first?
- Should compaction be a first-class effect in the first implementation or a
  later extension?
- How much of the AOS CLI chat projection should be copied directly?

These questions should be resolved through roadmap documents and implementation
spikes, not by expanding this design spec into a task list.
