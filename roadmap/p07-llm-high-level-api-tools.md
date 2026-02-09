# P07: High-Level API + Tool Loop

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
