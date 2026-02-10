# P80: Attractor Stage Outcome Contract and Status Ingestion (Post-P37 Runtime Semantics)

**Status**
- Deferred until CXDB-first migration series completion (`roadmap/p33-cxdb-first-architecture-pivot-and-spec-rebaseline.md` through `roadmap/p37-turnstore-sunset-and-cxdb-hardening.md`)
- Planned (2026-02-10)

**Goal**
Make stage outputs first-class runtime control inputs by defining and enforcing a deterministic stage outcome contract (including `status.json` ingestion) that directly drives routing/gates/retries.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Sections 3.2, 3.3, 4.5, 5.2, 10, 11)
- Storage/correlation extension: `spec/04-cxdb-integration-spec.md` (Sections 3.3, 3.5, 4.4)
- Prerequisites:
  - `roadmap/p37-turnstore-sunset-and-cxdb-hardening.md`
  - `roadmap/p30-attractor-observability-hitl-and-storage-abstractions.md`
  - `roadmap/p31-attractor-conformance-tests-docs-and-dod-matrix.md`
  - `roadmap/p32-cxdb-adapter-and-dual-level-persistence.md`

**Context**
- Current runtime can execute pipelines, but complex factory graphs rely on robust stage-to-routing contracts.
- Large graphs frequently emit status/artifact files and expect those to drive branch decisions.
- We need deterministic, testable semantics for converting stage outputs into `NodeOutcome` and context updates.

## Scope
- Define a canonical stage outcome schema and mapping contract.
- Add deterministic ingestion of stage status artifacts (for codergen/tool-oriented stages).
- Add strict validation/normalization for outcome fields.
- Expose parsed outcome details via events/logs/query surfaces.
- Add conformance tests for contract behavior across mock and agent-backed paths.

## Out of Scope
- New provider implementations or provider multiplexing expansions.
- Distributed worker leasing/coordination.
- UI renderer/plugin systems.

## Priority 0 (Must-have)

### [ ] G1. Canonical stage outcome schema and mapping rules
- Work:
  - Define canonical fields and precedence rules for outcome materialization:
    - `status`
    - `notes`
    - `preferred_label`
    - `suggested_next_ids`
    - `context_updates`
  - Define normalized status enum compatibility (`success`, `partial_success`, `retry`, `fail`).
  - Define strict/lenient parsing behavior for unknown/malformed fields.
- Files:
  - `crates/forge-attractor/src/outcome.rs`
  - `spec/03-attractor-spec.md` (if contract clarifications are needed)
- DoD:
  - Contract is explicit, deterministic, and enforced in runtime code.

### [ ] G2. `status.json` ingestion pipeline
- Work:
  - Add stage artifact ingestion path for `status.json`.
  - Convert parsed status payload into `NodeOutcome` and runtime context updates.
  - Enforce precedence ordering between backend result and artifact-derived overrides.
  - Emit structured diagnostics/events when ingest fails or is partial.
- Files:
  - `crates/forge-attractor/src/handlers/codergen.rs`
  - `crates/forge-attractor/src/handlers/tool.rs`
  - `crates/forge-attractor/src/runner.rs`
- DoD:
  - Pipelines can route on deterministic status outcomes produced by stage artifacts.

### [ ] G3. Routing/gate integration hardening
- Work:
  - Ensure edge conditions and goal gates consume final normalized outcome values.
  - Add deterministic fallback behavior when output contract is missing.
  - Add lint warnings for graphs that rely on contract fields but do not produce them.
- Files:
  - `crates/forge-attractor/src/routing.rs`
  - `crates/forge-attractor/src/lint.rs`
  - `crates/forge-attractor/src/condition.rs`
- DoD:
  - Contract-driven routing is stable and predictable for large review/checkpoint loops.

## Priority 1 (Strongly recommended)

### [ ] G4. Observability + query surfacing
- Work:
  - Include normalized outcome contract payloads in typed runtime events.
  - Include outcome materialization provenance (backend vs artifact) in host query surfaces.
- Files:
  - `crates/forge-attractor/src/events.rs`
  - `crates/forge-attractor/src/queries.rs`
- DoD:
  - Hosts can inspect exactly why a stage produced a given routing outcome.

### [ ] G5. Conformance and regression suite
- Work:
  - Add deterministic tests for:
    - valid/invalid `status.json` ingestion
    - precedence rules
    - outcome-to-routing behavior
    - parity across `mock` and `agent` backend modes
- Files:
  - `crates/forge-attractor/tests/outcome_contract.rs`
  - `crates/forge-attractor/tests/routing_contract.rs`
  - `crates/forge-cli/tests/backend_modes.rs`
- DoD:
  - Contract behavior is exhaustively test-backed and deterministic.

## Deliverables
- Canonical stage outcome contract with deterministic materialization semantics.
- `status.json` ingestion integrated with routing/gate logic.
- Host-visible provenance for outcome derivation.
- Conformance coverage preventing regressions.

## Execution order
1. G1 contract definition and normalization
2. G2 status ingestion integration
3. G3 routing/gate hardening
4. G4 observability/query surfacing
5. G5 conformance tests

## Exit criteria for this file
- Stage artifact outputs can deterministically drive routing behavior.
- Complex review/check pipelines can rely on stable contract semantics.
- Outcome provenance is observable and test-covered.
