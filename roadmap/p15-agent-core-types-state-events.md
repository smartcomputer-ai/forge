# P15: Agent Core Types, State, and Events

**Status**
- Planned (2026-02-09)

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
