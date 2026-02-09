# P19: Steering, Follow-up Queue, Loop Detection, Reasoning Controls
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Implement runtime steering and adaptive loop safety controls from spec Sections 2.6, 2.7, and 2.10.

**Scope**
- Implement `steer()` queue injection between tool rounds.
- Implement `follow_up()` queue processing after natural completion.
- Implement repeating tool-call-pattern loop detection with warning steering turn injection.
- Wire `reasoning_effort` pass-through and next-call update semantics.

**Out of Scope**
- Context compaction/summarization.

**Deliverables**
- Queue primitives and drain logic integrated into session loop.
- Loop signature tracking (`tool_name + args hash`) and windowed pattern checks.
- Tests for steering timing, follow-up chaining, and loop warning behavior.

**Acceptance**
- Steering turns are represented in history and converted to user-role messages for next LLM request.
- Follow-up inputs trigger a new processing cycle after current completion.
- Reasoning effort updates take effect on the next LLM call without restarting the session.

**Implemented**
- Integrated steering queue drain logic into the session loop in `crates/forge-agent/src/session.rs`:
  - drains queued `steer()` messages before the first LLM call for each input cycle
  - drains steering again between tool rounds
  - appends `SteeringTurn` entries to history and emits `STEERING_INJECTED`
- Added follow-up queue chaining behavior:
  - `follow_up()` messages are processed automatically after natural completion
  - each follow-up runs a new processing cycle via the same core loop
- Implemented loop detection based on tool-call signatures (`tool_name + arguments hash`) with configurable window:
  - detects repeating patterns of length 1/2/3 across the window
  - injects a warning `SteeringTurn` and emits `LOOP_DETECTION`
- Added runtime reasoning controls on session:
  - `set_reasoning_effort()` with validation (`low`/`medium`/`high`/`None`)
  - updates apply to the next built request without session restart
  - request builder continues passing `reasoning_effort` through to `forge-llm::Request`
- Added deterministic tests for:
  - steering timing + next-request visibility
  - follow-up chaining after completion
  - loop warning injection/event emission
  - reasoning effort next-call update semantics + invalid value rejection

**Validation**
- `cargo test -p forge-agent` passed.
