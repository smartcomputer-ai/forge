# P16: Tool Registry, Validation, and Dispatch Pipeline

**Status**
- Planned (2026-02-09)

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

