# P82: Attractor Runtime Control Plane and Resume Hardening (Post-P81 Operations)

**Status**
- Deferred until CXDB-first migration series completion (`roadmap/p33-cxdb-first-architecture-pivot-and-spec-rebaseline.md` through `roadmap/p37-turnstore-sunset-and-cxdb-hardening.md`)
- Planned (2026-02-10)

**Goal**
Add a deterministic runtime control plane (`pause`, `cancel`, `continue`) and strengthen resume semantics for long-running, branch-heavy pipelines so operators can safely manage execution without state corruption.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Sections 3.1, 3.2, 5.3, 9.6, 11)
- Storage/correlation extension: `spec/04-cxdb-integration-spec.md` (Sections 4.4, 5.1, 5.3)
- Prerequisites:
  - `roadmap/p37-turnstore-sunset-and-cxdb-hardening.md`
  - `roadmap/completed/p30-attractor-observability-hitl-and-storage-abstractions.md`
  - `roadmap/later/p80-attractor-stage-outcome-contract-and-status-ingestion.md`
  - `roadmap/later/p81-attractor-true-parallel-and-fan-in-semantics.md`

**Context**
- Complex pipelines can run for long durations and require operator intervention.
- Current runtime favors uninterrupted execution; explicit control semantics are limited.
- We need robust interruption boundaries and deterministic continuation guarantees, especially around retries, human gates, and parallel branches.

## Scope
- Add in-process control APIs for run lifecycle management.
- Introduce safe interruption points and control-state persistence.
- Ensure checkpoint/resume correctness after pause/cancel.
- Surface control transitions through events and query APIs.
- Add deterministic tests for control behavior and restart parity.

## Out of Scope
- Distributed lease/claim worker coordination.
- Remote daemon protocol design.
- UI implementation beyond exposing host-consumable APIs/events.

## Priority 0 (Must-have)

### [ ] G1. Runtime control primitives (`pause`, `cancel`, `continue`)
- Work:
  - Add runtime control handle abstraction with thread-safe signaling.
  - Support:
    - `pause`: halt progress at next safe boundary
    - `continue`: resume paused run from in-memory state
    - `cancel`: terminate run deterministically with terminal status
  - Define control-state model (`running`, `pausing`, `paused`, `cancelling`, `cancelled`).
- Files:
  - `crates/forge-attractor/src/runtime.rs`
  - `crates/forge-attractor/src/runner.rs`
- DoD:
  - Control operations are deterministic and do not corrupt run state.

### [ ] G2. Safe interruption boundaries
- Work:
  - Define interruption checkpoints around:
    - stage boundaries
    - retry boundaries
    - human-gate waits
    - parallel branch joins
  - Ensure partial stage execution does not produce inconsistent checkpoint metadata.
- Files:
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/src/checkpoint.rs`
- DoD:
  - Pause/cancel always lands on well-defined, resume-safe boundaries.

### [ ] G3. Resume hardening after control actions
- Work:
  - Extend checkpoint metadata with control-state lineage markers.
  - Resume canceled/paused runs with explicit policy constraints (e.g., cancel => new run only; pause => continue/resume allowed).
  - Ensure retry counters and branch progress remain consistent post-resume.
- Files:
  - `crates/forge-attractor/src/checkpoint.rs`
  - `crates/forge-attractor/src/resume.rs`
  - `crates/forge-attractor/src/runner.rs`
- DoD:
  - Controlled interruption and continuation behavior is deterministic across repeated cycles.

## Priority 1 (Strongly recommended)

### [ ] G4. Control-plane observability and host query surfaces
- Work:
  - Emit typed control events (`RunPaused`, `RunResumed`, `RunCancelled`, `ControlRejected`) with correlation metadata.
  - Expose host query APIs for current control state and last control transition.
- Files:
  - `crates/forge-attractor/src/events.rs`
  - `crates/forge-attractor/src/queries.rs`
- DoD:
  - Hosts can observe and audit operator interventions in real time.

### [ ] G5. CLI host integration for runtime control
- Work:
  - Add CLI command surface for control operations against in-process runs.
  - Ensure user feedback includes control-state transitions and rejection reasons.
- Files:
  - `crates/forge-cli/src/main.rs`
- DoD:
  - Operators can safely manage long runs from CLI without ad hoc process kills.

### [ ] G6. Conformance and resilience tests
- Work:
  - Add deterministic tests for:
    - pause/continue cycles
    - cancel during retry windows
    - cancel during human-gate waits
    - pause/cancel around parallel/fan-in transitions
    - checkpoint/resume parity after control operations
- Files:
  - `crates/forge-attractor/tests/control_plane.rs`
  - `crates/forge-attractor/tests/control_resume_parity.rs`
  - `crates/forge-cli/tests/control_cli.rs`
- DoD:
  - Control semantics are test-backed and stable under repeated stress scenarios.

## Deliverables
- Deterministic runtime control primitives and state model.
- Resume-safe interruption boundaries and enriched checkpoints.
- Control observability/events/query surfaces for host integration.
- CLI control operations and resilience test coverage.

## Execution order
1. G1 control primitives
2. G2 interruption boundaries
3. G3 resume hardening
4. G4 observability/query integration
5. G5 CLI control surface
6. G6 conformance/resilience tests

## Exit criteria for this file
- Long-running pipelines can be paused/resumed/cancelled safely.
- Checkpoint and resume behavior remains deterministic after operator interventions.
- Control operations are observable, queryable, and covered by deterministic tests.
