# P23: Subagents (Spawn, Coordinate, Wait, Close)
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Add child agent orchestration from spec Section 7 with depth-limited recursive safety.

**Scope**
- Implement subagent session creation sharing parent `ExecutionEnvironment`.
- Implement `spawn_agent`, `send_input`, `wait`, `close_agent` tools.
- Enforce `max_subagent_depth` and per-subagent turn limits.
- Route subagent completion summaries/results back to parent as tool results.

**Out of Scope**
- Advanced multi-agent scheduling policies.

**Deliverables**
- Subagent manager and handle lifecycle model.
- Parent/child event propagation strategy.
- Tests for spawn/wait flow, cancellation, depth-limit rejection, and cleanup on parent close.

**Acceptance**
- Subagents run independent histories while operating on shared filesystem context.
- Parent can wait for completion and receive deterministic result structure.
- Recursive spawning beyond max depth is blocked with clear error output.

**Implemented**
- Integrated subagent orchestration into session dispatch in `crates/forge-agent/src/session.rs`:
  - intercepts and handles `spawn_agent`, `send_input`, `wait`, and `close_agent` tool calls with session-aware logic
  - keeps standard tools on existing `ToolRegistry::dispatch(...)` path
  - applies standard tool event emission (`TOOL_CALL_START`/`TOOL_CALL_END`) and truncation pipeline for subagent tool outputs
- Added subagent lifecycle model:
  - `SubAgentRecord` stores child `Session` + latest `SubAgentResult`
  - `SubAgentResult` includes `{ output, success, turns_used }`
  - parent-visible handles remain in `subagents: HashMap<String, SubAgentHandle>`
- Implemented depth-limited spawning:
  - tracked via `subagent_depth` on `Session`
  - enforced against `SessionConfig.max_subagent_depth` with explicit error output
- Implemented child session creation behavior:
  - shares parent `ExecutionEnvironment` and `ProviderProfile`/`Client`
  - creates independent history in child session
  - supports per-child `max_turns` override via `spawn_agent.max_turns` (default `50`)
- Implemented parent close cleanup:
  - parent transition to `CLOSED` now closes all child sessions and marks child handles failed
- Registered subagent tool definitions in all provider registries in `crates/forge-agent/src/tools.rs`:
  - `spawn_agent`, `send_input`, `wait`, `close_agent`
  - all provider default registries now include subagent tools
- Extended `ExecutionEnvironment` with `delete_file`/`move_file` support (required by existing patch operations and subagent-compatible filesystem operations) in `crates/forge-agent/src/execution.rs`.

**Tests**
- Added subagent tool flow test (`spawn_agent` -> `wait`) with deterministic JSON result payload assertions.
- Added depth-limit rejection test for recursive spawn blocking.
- Added cleanup-on-parent-close test ensuring child status is updated during close.
- Added registry coverage asserting subagent tools are present in provider registries.

**Validation**
- `cargo test -p forge-agent` passed.
