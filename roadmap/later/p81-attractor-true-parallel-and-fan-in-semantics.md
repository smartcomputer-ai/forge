# P81: Attractor True Parallel and Fan-In Semantics (Post-P80 Runtime Semantics)

**Status**
- Planned (2026-02-10)
- Rebaselined on post-migration CXDB-first architecture (2026-02-10)
- Carries P39-deferred runtime enforcement for branch/attempt fork semantics (2026-02-10)

**Goal**
Replace synthetic parallel summaries with true branch execution semantics, deterministic join/fan-in behavior, and stable branch-level state/event/query contracts for complex DAG orchestration.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Sections 3.8, 4.8, 4.9, 11.6)
- Storage/correlation extension: `spec/04-cxdb-integration-spec.md` (Sections 3.4, 3.7, 4.4, 5.3, 5.4)
- Baseline:
  - `roadmap/p37-turnstore-sunset-and-cxdb-hardening.md`
  - `roadmap/p37-dod-matrix.md`
- Prerequisites:
  - `roadmap/later/p80-attractor-stage-outcome-contract-and-status-ingestion.md`
  - `roadmap/completed/p29-attractor-state-checkpoint-fidelity-and-advanced-handlers.md`
  - `roadmap/completed/p30-attractor-observability-hitl-and-storage-abstractions.md`

**Context**
- Existing `parallel` and `parallel.fan_in` behavior is useful but does not yet represent full branch pipeline execution.
- Large factory graphs need real fan-out/fan-in with deterministic branch lineage, retries, checkpoints, and observability.
- We need semantics that stay deterministic in single-process mode and remain portable to future distributed coordination.
- P39 freezes the context topology/data-model contract; this milestone implements the deferred runtime behavior for:
  - one forked context per fan-out branch,
  - retry attempts forked from stable node-entry base turns.

## Scope
- Implement true branch execution for `parallel` nodes.
- Implement deterministic fan-in merge semantics over executed branch results.
- Persist branch lineage and correlation metadata.
- Add checkpoint/resume behavior that preserves branch progress.
- Add branch-aware event/query surfaces and conformance tests.

## Out of Scope
- Distributed lease/claim worker coordination.
- Remote host protocols and HTTP server mode.
- UI renderer implementation.

## Priority 0 (Must-have)

### [ ] G1. Parallel execution model v2
- Work:
  - Execute real branch subflows from `parallel` outgoing edges.
  - Preserve parent context isolation with deterministic per-branch clones.
  - Define max concurrency policy and deterministic scheduling order.
  - Emit branch run identity metadata (`branch_id`, `branch_run_id`, lineage refs).
  - Enforce P39 context-fork policy for branch execution on runtime paths.
- Files:
  - `crates/forge-attractor/src/handlers/parallel.rs`
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/src/runtime.rs`
- DoD:
  - `parallel` node runs real branch execution paths with deterministic outcomes.

### [ ] G2. Fan-in merge contract v2
- Work:
  - Consume actual branch outcomes (not synthetic summaries).
  - Define deterministic merge policies:
    - winner selection
    - aggregate context shaping
    - conflict-resolution rules for context keys
  - Preserve branch provenance in merged output.
- Files:
  - `crates/forge-attractor/src/handlers/parallel_fan_in.rs`
  - `crates/forge-attractor/src/outcome.rs`
- DoD:
  - `parallel.fan_in` behavior is deterministic, policy-driven, and provenance-preserving.

### [ ] G3. Checkpoint/resume for branch topology
- Work:
  - Extend checkpoint model to record branch execution state and fan-in readiness.
  - Resume in-progress parallel phases without re-running completed branches.
  - Ensure one-hop fidelity and retry semantics remain correct in branch resumes.
  - Enforce retry attempt fork-parent policy (stable node-entry base turn) across resume boundaries.
- Files:
  - `crates/forge-attractor/src/checkpoint.rs`
  - `crates/forge-attractor/src/resume.rs`
  - `crates/forge-attractor/src/runner.rs`
- DoD:
  - Interrupted parallel runs resume deterministically with branch parity.

## Priority 1 (Strongly recommended)

### [ ] G4. Branch-aware storage/query/event contracts
- Work:
  - Persist branch-level timeline and linkage records for host drill-down.
  - Add query helpers for branch status, branch lineage, and fan-in decisions.
  - Add event payload fields for branch start/completion/failure/retry with stable IDs.
- Files:
  - `crates/forge-attractor/src/events.rs`
  - `crates/forge-attractor/src/queries.rs`
  - `crates/forge-attractor/src/storage/types.rs`
- DoD:
  - Hosts can inspect branch-level execution and merge decisions end-to-end.

### [ ] G5. Conformance + parity suite
- Work:
  - Add deterministic integration tests for:
    - fan-out/fan-in traversal correctness
    - join policy outcomes
    - branch checkpoint/resume parity
    - deterministic fake-CXDB contract behavior plus optional live CXDB smoke coverage
- Files:
  - `crates/forge-attractor/tests/parallel_runtime.rs`
  - `crates/forge-attractor/tests/parallel_resume.rs`
  - `crates/forge-attractor/tests/parallel_storage_contract.rs`
- DoD:
  - True parallel semantics are stable and regression-resistant.

## Deliverables
- True branch execution semantics for `parallel`.
- Deterministic fan-in merge contract over real branch outcomes.
- Branch-safe checkpoint/resume behavior.
- Branch-level observability/queryability with parity tests.

## Execution order
1. G1 true parallel branch execution
2. G2 fan-in merge contract
3. G3 branch checkpoint/resume
4. G4 branch-aware event/query/storage surfaces
5. G5 conformance and parity tests

## Exit criteria for this file
- Parallel/fan-in nodes execute as real orchestration primitives, not synthetic summaries.
- Branch behavior is deterministic and resume-safe.
- Hosts can audit branch decisions and merged outcomes with full provenance.
