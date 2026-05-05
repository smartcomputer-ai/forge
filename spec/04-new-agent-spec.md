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
- host session and workspace concepts
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
hooks, and JSONL streaming. It should influence Forge design, especially the CLI
and event projection. It should not be temporalized wholesale.

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
- Keep tests deterministic, focused, and able to run without live providers by
  injecting fake LLM/effect executors.

## 3. Non-goals

- Do not re-create AgentOS AIR, worlds, WASM workflows, or governance in
  `forge-agent`.
- Do not make Temporal types the public agent domain model.
- Do not vendor Codex as the Forge runtime.
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
- transcript references
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
- human input/approval where the runner supports it

Adapters must be idempotency-aware. The intent id is the idempotency key. A
Temporal activity retry must not turn one logical tool call into multiple
logical tool results.

### 4.4 Projections

The runtime event stream is the durable, structured source for UI and logs.
Human-facing views are projections, not authoritative state.

Important projections:

- CLI transcript
- run status
- tool call tree
- context/compaction status
- token/cost usage
- file change summary
- Attractor stage result

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
- `ToolBatchId`: a group of tool calls returned by one LLM response.
- `ToolCallId`: a provider/model tool call id normalized by Forge.
- `ArtifactRef`: content-addressed or store-addressed large payload reference.

IDs must be explicit in persisted events. Never depend on vector position as a
durable identity.

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
- transcript/artifact refs

Only one foreground run is active in a session at a time. Subagents are separate
sessions with parent/child metadata.

### 5.3 Run

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
- outcome

### 5.4 Turn and context window

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

### 5.5 Transcript and artifacts

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
- `ToolRegistrySet`
- `ToolProfileSelected`
- `ToolOverridesSet`
- `HumanInputProvided`
- `ApprovalDecisionProvided`

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

### 6.3 Effect events

Effects are represented in two phases:

- `EffectIntentRecorded`
- `EffectReceiptRecorded`

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
- `TokenUsageObserved`
- `WarningObserved`
- `CostObserved`

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

### 7.4 Human effects

Human input and approval are effects from the agent's perspective. The core may
enter `waiting` with a pending human request. The runner decides how to surface
that request:

- local CLI prompt
- web/API request
- Temporal signal/update
- Attractor HITL bridge

The response is appended as an input event and the run resumes.

### 7.5 Subagent effects

Subagents are separate sessions. A parent can request:

- spawn
- send input
- wait
- interrupt
- close

Subagent execution should use the same runner family as the parent unless
explicitly overridden. A Temporal parent should normally spawn child workflows;
a local parent should spawn local sessions.

Depth limits and permissions are enforced by the parent runner/core.

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
- provide approval decision
- pause/resume/close

### 8.2 Activities

All non-deterministic or effectful work is an activity:

- LLM complete/stream/count/compact
- shell/process execution
- filesystem operations
- MCP calls
- blob/artifact store operations
- external notification
- black-box CLI agent invocation

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
- inspect transcript and run state
- inspect tool calls and artifacts

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
- interruption/cancellation settlement
- context compaction prerequisites
- local runner end-to-end with fake adapters
- Temporal mapping tests with mocked activities or Temporal test utilities

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
