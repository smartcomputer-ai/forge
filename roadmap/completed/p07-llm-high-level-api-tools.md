# P07: High-Level API + Tool Loop
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Deliver the `generate`, `stream`, `generate_object`, and `stream_object` APIs with tool execution and retries.

**Scope**
- `generate()` and `stream()` with prompt standardization and validation.
- Tool definition types and `ToolChoice` mapping.
- Active tool execution loop with parallel execution and ordered results.
- Stop conditions and `max_tool_rounds` semantics.
- `GenerateResult`, `StepResult`, `StreamResult`, and incremental JSON parsing for `stream_object`.
- Retry integration using `RetryPolicy` for per-step retries.

**Out of Scope**
- Provider adapter implementations.

**Deliverables**
- High-level API module in `forge-llm` with the full set of functions.
- Unit tests for tool loop behavior, parallel execution ordering, and prompt validation.

**Acceptance**
- `generate()` rejects when both `prompt` and `messages` are provided.
- `max_tool_rounds = 0` disables tool execution.
- Parallel tool calls execute concurrently and return results in call order.
- `generate_object()` raises `NoObjectGeneratedError` when validation fails.

**Completed**
1. Added `high_level` API module with `generate`, `stream`, `generate_object`, and `stream_object`.
2. Added high-level types: `GenerateOptions`, `GenerateResult`, `StepResult`, `Tool`, `ToolResult`, `StreamResult`, `StreamObjectResult`.
3. Implemented prompt/message standardization and validation (including mutual exclusivity of `prompt` and `messages`).
4. Implemented per-step retry integration using `RetryPolicy` + `compute_backoff_delay`.
5. Implemented active tool loop with parallel execution (`join_all`) and call-order-preserving result collection.
6. Implemented `max_tool_rounds` semantics, including `0` disabling automatic tool execution.
7. Implemented object-generation parsing/validation with `NoObjectGeneratedError` on failures.
8. Refactored `stream()` to run on live provider `Client::stream()` events (including multi-step tool continuation), replacing synthetic replay output.
9. Added unit tests for acceptance criteria and core loop behavior.
