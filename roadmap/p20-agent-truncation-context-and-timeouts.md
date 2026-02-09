# P20: Tool Output Truncation, Context Awareness, and Timeout Messaging

**Status**
- Planned (2026-02-09)

**Goal**
Implement the context-safety mechanisms in spec Section 5 and integrate them into tool result handling.

**Scope**
- Implement character-first truncation with per-tool defaults and mode support (`head_tail`, `tail`).
- Implement optional line-based truncation after character truncation.
- Ensure `TOOL_CALL_END` emits full untruncated output while LLM receives truncated content.
- Implement approximate context usage checks and warning event emission.
- Implement standardized timeout error message payload for shell tool outputs.

**Out of Scope**
- Automatic context compaction.

**Deliverables**
- Truncation utility module with default limits/modes/line limits.
- Config override support (`tool_output_limits`, line limits).
- Tests for pathological outputs (huge single line) and marker correctness.

**Acceptance**
- Character truncation always runs first.
- Default limits match spec table (read_file 50k, shell 30k, grep/glob 20k, etc.).
- Truncation marker clearly states removed character count and guidance.

