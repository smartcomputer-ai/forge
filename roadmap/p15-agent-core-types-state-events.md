# P15: Agent Core Types, State, and Events
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Implement the foundational data model for session orchestration, turn history, lifecycle state, and event emission.

**Scope**
- Define core records/enums: `Session`, `SessionState`, `SessionConfig`, turn types, `SessionEvent`, `EventKind`.
- Implement event emitter abstraction with async-consumable stream/iterator.
- Define error taxonomy for tool-level vs session-level errors.
- Add deterministic unit tests for type/state transitions and event payload contracts.

**Out of Scope**
- LLM request loop execution.
- Tool execution behavior.

**Deliverables**
- Core model module(s) in `crates/forge-agent/src/`.
- Event emission primitives with typed payload helpers.
- Tests for lifecycle transitions and event ordering invariants.

**Acceptance**
- Session lifecycle supports: `IDLE`, `PROCESSING`, `AWAITING_INPUT`, `CLOSED`.
- Event kinds in spec Section 2.9 are represented and serializable.
- Unit tests cover creation, transition, and event payload shape guarantees.

**Implemented**
- Added explicit session lifecycle transition rules in `SessionState` (`IDLE`, `PROCESSING`, `AWAITING_INPUT`, `CLOSED`) with invalid transition errors.
- Added structured error taxonomy:
  - `SessionError` for lifecycle/config/event-serialization failures.
  - `ToolError` for unknown-tool/validation/execution failures.
  - `AgentError` now wraps session/tool/LLM/error-environment concerns.
- Refined event system primitives:
  - `EventKind` now aligns to spec Section 2.9 and serializes in `SCREAMING_SNAKE_CASE`.
  - `EventData` moved to a typed wrapper with helper insert/get utilities and JSON-object payload conversion.
  - `SessionEvent` gained typed payload helper constructors (`assistant_text_end`, `tool_call_start`, `tool_call_end_*`, `turn_limit_round`, etc.).
  - `EventEmitter` now exposes `subscribe()` for async-consumable event streams.
  - `BufferedEventEmitter` now supports snapshot + subscription with ordered replay for deterministic consumers.
- Updated turn model for structured tool result payloads (`serde_json::Value`) and added constructor helpers for turn records.
- Added deterministic unit tests for:
  - lifecycle transition validity/invalidity
  - session start/end event ordering and final-state payload
  - async event stream consumption
  - event kind serialization contract
  - typed event payload shape guarantees

**Validation**
- `cargo test -p forge-agent` passed.
