# P41: Agent Reducer, Decider, and Local Stepper

**Status**
- Planned (2026-05-05)

**Goal**
Implement the first executable Forge-native agent loop on top of the p40 core
model. This phase turns scoped journal events plus bounded `SessionState` into
next effect intents, and settles receipts back into state and journal events.

P41 is still not the production runtime. It should prove the deterministic
domain loop with fake adapters and an in-memory/local test harness. Real host
tools, production artifact stores, Temporal workflows, CXDB/Postgres
persistence, and CLI UI are follow-on phases.

**Source**
- Spec of record: `spec/04-new-agent-spec.md`
- Model foundation: `roadmap/p40-new-agent-core.md`
- Tool execution follow-on: `roadmap/p42-agent-tools.md`

## Design Position

Forge uses a journaled, ref-backed, snapshot-driven loop:

```text
input -> journal event -> reduce bounded state -> decide effect intents
effect receipt -> journal event -> reduce bounded state -> decide next work
```

The reducer/decider must be deterministic and side-effect free. Runners execute
effects outside the core and append receipts. Large payload bytes stay in the
artifact/CAS layer and are referenced by events, transcript items, context
items, and receipts.

## Prerequisites From P40

- Agent definition/version primitives exist.
- Session state is explicitly bounded and contains only control data needed for
  next-step decisions.
- Journal events have session-local sequence and causality joins.
- Transcript/projection items are separate from active state.
- Artifact put/get is not an agent effect; adapters use artifact storage
  infrastructure directly.
- Fake artifact/effect helpers are available for deterministic tests.

## Scope

### In scope
- Pure reducer APIs for applying journal events to `SessionState`.
- Pure decider APIs for producing effect intents from `SessionState`.
- Local in-process stepper that:
  - appends input events
  - reduces state
  - records emitted effect intents
  - calls fake effect executors
  - appends receipts
  - reduces state again until quiescent
- First turn planner behavior sufficient for deterministic tests.
- First LLM loop behavior using fake LLM receipts.
- Tool-call observation/planning and generic `ToolInvoke` intent emission.
- Tool-result turn continuation with fake tool receipts.
- Context pressure/count/compaction control flow with fake receipts if needed.
- Transcript/projection event emission from authoritative journal events.
- Deterministic tests for complete fake runs.

### Out of scope
- Real LLM provider calls.
- Real tool execution, including host shell/filesystem tools.
- Real MCP calls.
- Temporal workflow/activity implementation.
- Postgres/SQLite/CXDB/S3/filesystem production persistence.
- CLI/TUI rendering.
- Hooks, approval, permissions, sandbox policy, and dynamic tool loading.

## Target Module Shape

The exact file split can change, but p41 should add or clarify:

- `reducer.rs`
  - `apply_event(state, event) -> state/events or result`
  - journal event validation and state transition helpers
- `decider.rs`
  - `decide_next(state) -> Vec<AgentEffectIntent>`
  - run/turn/tool/context decision rules
- `stepper.rs`
  - local deterministic stepper over fake stores/executors
- `planner.rs`
  - first turn/context planning implementation
- `journal.rs`
  - append-only in-memory journal for tests
- `testing.rs`
  - fake LLM/tool/artifact helpers

## Priority 0: Reducer Foundation

### [ ] G1. Journal append contract and sequencing
- Work:
  - Define an append result that assigns or validates session-local journal
    sequence.
  - Ensure every appended event carries session id and causal ids where
    applicable.
  - Reject duplicate or out-of-order event ids/sequences in deterministic tests.
- DoD:
  - Journal sequence is monotonic per session.
  - Reducer receives already-ordered events and never samples time.

### [ ] G2. Input and lifecycle reduction
- Work:
  - Apply session open/pause/resume/close events.
  - Apply run requested/follow-up/steer/interrupt/human input events.
  - Apply lifecycle events for run/turn transitions.
- DoD:
  - Invalid lifecycle transitions fail with typed model errors.
  - Pending input queues and current run state update deterministically.

### [ ] G3. Effect reduction
- Work:
  - Apply `EffectIntentRecorded` by inserting pending effect records.
  - Apply stream frames as non-authoritative observations only.
  - Apply `EffectReceiptRecorded` by settling pending effects.
  - Classify receipt failures into model-visible tool results vs runner/system
    failures where the data is already explicit.
- DoD:
  - No pending effect disappears without settled/abandoned state.
  - Receipt application is idempotent for the same `EffectId`.

### [ ] G4. Tool batch reduction
- Work:
  - Create active tool batches from LLM receipts that contain observed tool
    calls.
  - Plan accepted/unavailable calls from the selected tool registry/profile.
  - Update per-call status on generic tool receipts.
  - Complete the batch when all calls are terminal.
- DoD:
  - Parallel grouping is data-only and deterministic.
  - Unavailable tools become model-visible failed tool results.

## Priority 1: Decider and Planner

### [ ] G5. Run/turn decider
- Work:
  - Start a queued run when the session can accept foreground work.
  - Allocate turn ids deterministically.
  - Decide whether the next action is planning, LLM generation, tool execution,
    compaction/counting, waiting, completion, or failure.
- DoD:
  - `decide_next` emits no duplicate effect intents for already-pending work.
  - Loop limits are enforced from state/config.

### [ ] G6. First context planner
- Work:
  - Select required prompt refs, run input refs, recent context refs,
    tool-result refs, summaries, and selected tool definitions.
  - Produce `ResolvedTurnContext`.
  - Emit count/compaction prerequisites only as fake-supported control paths in
    this phase.
- DoD:
  - Planner output is deterministic for the same state.
  - Large content remains referenced by `ArtifactRef`.

### [ ] G7. LLM and tool continuation loop
- Work:
  - Emit `LlmComplete`/`LlmStream` intent for a ready turn.
  - On LLM receipt with no tool calls, append final assistant transcript item
    and complete the run.
  - On LLM receipt with tool calls, create/execute a tool batch.
  - On completed tool batch, emit the next LLM turn with tool result refs.
- DoD:
  - Fake run can complete: user input -> LLM -> final answer.
  - Fake run can complete: user input -> LLM tool calls -> tool receipts ->
    LLM final answer.

## Priority 2: Local Stepper and Projections

### [ ] G8. Local stepper with fake executors
- Work:
  - Implement an in-process stepper that drives state to quiescence using fake
    LLM/tool/human/subagent executors.
  - Keep artifact reads/writes in fake adapter infrastructure, not core
    effects.
- DoD:
  - Stepper tests require no live services or CLI binaries.
  - The stepper emits journal events and bounded state snapshots.

### [ ] G9. Transcript/projection emission
- Work:
  - Derive transcript/projection items from journal/effect/lifecycle events.
  - Include joins for session/run/turn/effect/tool ids.
  - Store only previews plus artifact refs.
- DoD:
  - A fake run yields user, assistant, reasoning, tool-call, tool-output, and
    status projection items as applicable.

### [ ] G10. Quiescence and interruption semantics
- Work:
  - Define quiescent states: waiting for input, waiting for human response,
    waiting on pending effects, completed, failed, cancelled, interrupted.
  - Apply interrupt/cancel events to active runs and pending effects.
- DoD:
  - No stepper loop spins without new input/effect receipts.
  - Cancellation settles or abandons pending effects explicitly.

## Testing

- Unit tests live beside reducer/decider modules.
- Integration-style local stepper tests use fake adapters and in-memory
  journal/artifact stores.
- Tests must fail loudly; no runtime env-var gating.

Required test flows:

- open session -> request run -> fake final LLM answer -> completed run
- fake tool call round trip -> final answer
- unavailable tool -> model-visible tool error -> recovery answer
- follow-up queued while run active
- steering input applied before next turn
- interrupt active run with pending effect
- context pressure triggers fake compaction path
- journal sequence and id allocation remain deterministic

## Acceptance

- `cargo test -p forge-agent` passes with deterministic tests only.
- The loop is executable with fake LLM/tool executors.
- No real provider, host tool, MCP server, Temporal worker, CXDB, Postgres, S3,
  or CLI UI is required.
- Active `SessionState` remains bounded; transcript history is exposed through
  journal/projection records plus artifact refs.
- p42 can add real generic tool dispatch without changing p41 core loop
  concepts.
