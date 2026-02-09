# P19: Steering, Follow-up Queue, Loop Detection, Reasoning Controls

**Status**
- Planned (2026-02-09)

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

