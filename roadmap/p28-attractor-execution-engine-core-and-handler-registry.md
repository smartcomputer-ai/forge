# P28: Attractor Execution Engine Core and Handler Registry (Spec 03 §§3-4,10)

**Status**
- Planned (2026-02-09)

**Goal**
Implement the runtime traversal engine, deterministic edge routing, retry/failure logic, and core handler registry needed to execute normalized Attractor graphs.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Sections 3, 4, 10, 11.3, 11.4, 11.5, 11.6, 11.9)
- Agent integration prerequisite: `roadmap/completed/p25-agent-attractor-backend-readiness.md`

**Context**
- P27 provides parse/transform/lint and normalized graph IR.
- Runtime should remain headless and event-driven.
- Codergen execution should use backend injection so `forge-agent` can be plugged in without coupling execution core to provider details.

## Scope
- Implement `run()` lifecycle (`parse/validate` already completed in P27).
- Implement traversal loop and edge selection algorithm.
- Implement handler registry and core handlers:
  - `start`, `exit`, `codergen`, `conditional`, `wait.human`, `tool`
- Implement retry/backoff and failure routing semantics.
- Implement condition expression evaluator.

## Out of Scope
- Full checkpoint/resume fidelity semantics.
- Artifact store/file threshold behavior.
- Parallel/fan-in/manager-loop advanced handlers.
- HTTP server mode.
- CXDB store adapter.

## Priority 0 (Must-have)

### [ ] G1. Runtime engine skeleton and run lifecycle
- Work:
  - Implement `PipelineRunner::run(graph, config)` with deterministic control flow.
  - Implement lifecycle phases: initialize -> execute -> finalize.
  - Wire terminal-node behavior and goal-gate checks at exit.
- Files:
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/src/runtime.rs`
- DoD:
  - Engine can execute a simple linear graph from start to exit.

### [ ] G2. Edge selection + condition language evaluator
- Work:
  - Implement 5-step edge selection priority:
    1) condition match
    2) preferred label
    3) suggested next IDs
    4) weight
    5) lexical tiebreak
  - Implement condition expression evaluator (`=`, `!=`, `&&`, keys `outcome`, `preferred_label`, `context.*`).
- Files:
  - `crates/forge-attractor/src/routing.rs`
  - `crates/forge-attractor/src/condition.rs`
- DoD:
  - Routing is deterministic and matches spec examples.

### [ ] G3. Retry policy, backoff, and failure routing
- Work:
  - Implement `max_retries` semantics (`max_attempts = max_retries + 1`).
  - Implement retry backoff config + jitter behavior.
  - Implement failure-routing precedence:
    - fail edge (`outcome=fail`)
    - node retry target
    - node fallback retry target
    - terminate with failure
- Files:
  - `crates/forge-attractor/src/retry.rs`
  - `crates/forge-attractor/src/runner.rs`
- DoD:
  - Retries, exhaustion, and fallback routing are test-covered and spec-aligned.

### [ ] G4. Handler interface + registry + shape/type resolution
- Work:
  - Define handler trait and registry with override precedence:
    1) explicit node `type`
    2) shape mapping
    3) default handler
  - Define outcome model with status, notes, context updates, preferred label, suggested IDs.
- Files:
  - `crates/forge-attractor/src/handlers/mod.rs`
  - `crates/forge-attractor/src/handlers/registry.rs`
  - `crates/forge-attractor/src/outcome.rs`
- DoD:
  - Registry resolves handlers exactly per spec mapping and precedence.

### [ ] G5. Core handlers (start/exit/codergen/conditional/wait.human/tool)
- Work:
  - `start` and `exit` no-op handlers.
  - `codergen` handler:
    - prompt resolution with fallback to label
    - `$goal` expansion integration
    - writes prompt/response/status to stage directory
    - backend contract (`String | Outcome`)
  - `conditional` pass-through handler.
  - `wait.human` handler with interviewer interface integration and choice derivation from outgoing edges.
  - `tool` handler for command/tool execution with deterministic outputs.
- Files:
  - `crates/forge-attractor/src/handlers/start.rs`
  - `crates/forge-attractor/src/handlers/exit.rs`
  - `crates/forge-attractor/src/handlers/codergen.rs`
  - `crates/forge-attractor/src/handlers/conditional.rs`
  - `crates/forge-attractor/src/handlers/wait_human.rs`
  - `crates/forge-attractor/src/handlers/tool.rs`
- DoD:
  - Section 11.6 baseline handler matrix is executable and tested.

## Priority 1 (Strongly recommended)

### [ ] G6. `forge-agent` codergen backend adapter
- Work:
  - Implement an adapter that maps node model/provider/reasoning overrides into `forge-agent::SubmitOptions`.
  - Map `SubmitResult` and tool error summaries into Attractor `Outcome`.
  - Thread-key continuity support for upcoming fidelity work.
- Files:
  - `crates/forge-attractor/src/backends/forge_agent.rs`
- DoD:
  - Attractor codergen nodes can be executed through `forge-agent` without replaying raw history.

### [ ] G7. Execution tests for Sections 11.3/11.4/11.5/11.6/11.9
- Work:
  - Add runtime tests for linear/branching/retry/goal-gate/human-gate behavior.
  - Add deterministic tests for condition parser/evaluator.
- Files:
  - `crates/forge-attractor/tests/execution_core.rs`
  - `crates/forge-attractor/tests/handlers_core.rs`
  - `crates/forge-attractor/tests/conditions.rs`
- DoD:
  - Core execution semantics are covered by deterministic test cases.

## Deliverables
- Runtime traversal engine with deterministic routing/retry behavior.
- Handler registry + core handlers.
- Pluggable codergen backend interface with `forge-agent` adapter.
- Execution conformance test coverage for core sections.

## Execution order
1. G1 lifecycle skeleton
2. G2 routing + conditions
3. G3 retry/failure routing
4. G4 handler registry
5. G5 core handlers
6. G6 forge-agent backend adapter
7. G7 tests

## Exit criteria for this file
- End-to-end execution works for linear and branching pipelines.
- Core handlers and routing behavior match spec semantics.
- Codergen backend contract is implemented and integration-ready.

