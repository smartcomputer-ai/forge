# P20: Tool Output Truncation, Context Awareness, and Timeout Messaging
_Complete_

**Status**
- Done (2026-02-09)

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

**Implemented**
- Completed Section 5 truncation behavior in `crates/forge-agent/src/truncation.rs`:
  - character-first truncation remains the primary safeguard
  - head/tail and tail modes now include explicit guidance text in warning markers
  - line-based truncation remains a secondary pass after character truncation
- Added/expanded truncation tests in `crates/forge-agent/src/truncation.rs`:
  - marker contains removed-count and actionable guidance
  - pathological huge single-line output is still reduced by character truncation even when line limit is configured
  - tail mode keeps suffix and reports removed prefix count
- Ensured full-output event vs truncated LLM content behavior is covered with deterministic tests in `crates/forge-agent/src/tools.rs`:
  - `TOOL_CALL_END` carries full untruncated output
  - `ToolResult.content` contains the truncated output with warning marker
- Implemented context-window awareness warning emission in `crates/forge-agent/src/session.rs` and `crates/forge-agent/src/events.rs`:
  - approximate usage heuristic: `total_chars_in_history / 4`
  - warning threshold: `> 80%` of provider profile `context_window_size`
  - emits an informational warning event payload via `SessionEvent::context_usage_warning(...)`
- Added session tests in `crates/forge-agent/src/session.rs`:
  - emits context-usage warning when threshold is exceeded
  - does not emit warning below threshold
- Timeout messaging behavior from Section 5.4 was already implemented and retained in `crates/forge-agent/src/execution.rs` with coverage.

**Validation**
- `cargo test -p forge-agent` passed.
