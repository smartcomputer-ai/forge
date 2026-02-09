# P18: Session Loop Core (LLM Call + Tool Rounds)
_Complete_

**Status**
- Done (2026-02-09)

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

**Implemented**
- Added core session processing methods in `crates/forge-agent/src/session.rs`:
  - `process_input()` (alias to `submit()`)
  - `submit()` implementing the LLM call + tool round loop
  - request-construction + history-to-message conversion helpers
- Implemented request building from:
  - provider profile metadata (`model`, `provider`, `provider_options`)
  - current session config (`reasoning_effort`)
  - converted turn history
  - active tool definitions and `tool_choice=auto`
- Implemented the loop flow:
  - append `UserTurn` and emit `USER_INPUT`
  - call `Client.complete()` directly
  - append `AssistantTurn` and emit `ASSISTANT_TEXT_START`/`ASSISTANT_TEXT_END`
  - dispatch tool calls via `ToolRegistry::dispatch()`
  - append `ToolResultsTurn`
  - repeat until natural completion or limit/abort conditions
- Enforced loop guards:
  - per-input round limit (`max_tool_rounds_per_input`) with `TURN_LIMIT` event
  - session-level turn limit (`max_turns`) with `TURN_LIMIT` event
  - abort-aware early termination (`request_abort()`)
- Added conversion layer from internal turns to low-level `forge-llm::Message` values, including assistant tool-call parts and tool-result messages.
- Added integration-style tests with mocked provider adapter responses for:
  - natural completion (no tool calls)
  - per-input round limit event + stop behavior
  - multiple sequential `submit()` calls preserving history consistency

**Validation**
- `cargo test -p forge-agent` passed.
- `cargo test` (workspace) passed.
