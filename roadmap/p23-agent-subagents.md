# P23: Subagents (Spawn, Coordinate, Wait, Close)

**Status**
- Planned (2026-02-09)

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

