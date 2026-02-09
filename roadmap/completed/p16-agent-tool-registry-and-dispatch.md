# P16: Tool Registry, Validation, and Dispatch Pipeline
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Implement the tool registry and execution pipeline defined in spec Section 3.8.

**Scope**
- Implement `ToolDefinition`, `RegisteredTool`, `ToolRegistry`.
- Enforce JSON argument parsing + schema validation before execution.
- Implement unknown-tool fallback as `ToolResult(is_error=true)` instead of session failure.
- Implement sequential and parallel tool execution modes based on provider profile capability.

**Out of Scope**
- Concrete filesystem/shell tool implementations.
- Session loop integration details beyond dispatcher API.

**Deliverables**
- Tool registry APIs: register/unregister/get/definitions/names.
- Dispatch layer: lookup -> validate -> execute -> truncate hook -> emit -> return.
- Unit tests for validation errors, name collisions, and parallel dispatch behavior.

**Acceptance**
- Invalid args fail fast with structured tool error results.
- Custom tool registration overrides profile defaults (latest-wins).
- Parallel dispatch path produces one result per tool call with stable call-id mapping.

**Implemented**
- Extended `ToolRegistry` with async dispatch entrypoint:
  - `dispatch(tool_calls, execution_env, config, event_emitter, options)`
  - dispatch options include `session_id` and `supports_parallel_tool_calls`.
- Implemented dispatch pipeline in `crates/forge-agent/src/tools.rs`:
  - lookup tool by name
  - parse JSON arguments (`raw_arguments` supported)
  - validate arguments against tool JSON schema (`required`, `type`, `additionalProperties`)
  - execute registered tool
  - truncate output via truncation layer hook
  - emit `TOOL_CALL_START` / `TOOL_CALL_END` events (full untruncated output on success)
  - return `ToolResult` with truncated content
- Added unknown-tool fallback that returns `ToolResult { is_error: true }` instead of failing the session.
- Added sequential and parallel execution modes, with parallel preserving input ordering and call-id stability.
- Kept registry behavior where latest registration wins for name collisions.

**Validation**
- `cargo test -p forge-agent` passed with dispatcher coverage:
  - unknown-tool fallback behavior
  - validation failure short-circuit without executor invocation
  - raw JSON argument parsing + schema validation
  - parallel dispatch ordering and stable call-id mapping
  - tool-call start/end event emission contract
