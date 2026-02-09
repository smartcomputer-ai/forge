# P18: Session Loop Core (LLM Call + Tool Rounds)

**Status**
- Planned (2026-02-09)

**Goal**
Implement `process_input()` and the core turn loop from spec Section 2.5 using low-level `forge-llm` APIs.

**Scope**
- Build requests from session history, provider profile, tool definitions, and config.
- Call `Client.complete()` directly (no SDK high-level `generate()` loop).
- Record assistant turns, execute tool rounds, append tool results turns, and continue until completion.
- Enforce per-input round limits and global turn limits.
- Add abort-aware behavior for graceful loop termination.

**Out of Scope**
- Steering/follow-up queue behavior (P19).
- Subagents (P23).

**Deliverables**
- Session processing engine with deterministic state transitions.
- Conversion layer from internal turn history to `forge-llm` request messages.
- Integration-style tests with mocked provider adapters.

**Acceptance**
- Natural completion exits when response has no tool calls.
- Loop limit conditions emit `TURN_LIMIT` and stop correctly.
- Multiple sequential `submit()` calls on the same session work without corrupting history.

