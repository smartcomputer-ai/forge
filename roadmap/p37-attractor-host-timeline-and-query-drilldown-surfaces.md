# P37: Attractor Host Timeline and Query Drilldown Surfaces (Post-P36 Operability)

**Status**
- Planned (2026-02-10)

**Goal**
Provide robust host-facing timeline/query APIs for deep run inspection, including stage/branch/agent drilldown, so complex pipelines are operable and debuggable without reading raw turn logs.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Sections 9.6, 11.8, 11.11)
- Storage/correlation extension: `spec/04-cxdb-integration-spec.md` (Sections 3.4, 3.5, 4.4, 5.7)
- Prerequisites:
  - `roadmap/p30-attractor-observability-hitl-and-storage-abstractions.md`
  - `roadmap/p32-cxdb-adapter-and-dual-level-persistence.md`
  - `roadmap/p34-attractor-true-parallel-and-fan-in-semantics.md`
  - `roadmap/p36-attractor-attribute-policy-completion-and-contract-tightening.md`

**Context**
- Large graphs require fast, structured introspection for incident response and tuning.
- Typed events exist, but hosts still need stable query contracts over persisted state/timelines.
- CXDB enables rich projection browsing, but host APIs must remain backend-agnostic.

## Scope
- Define backend-agnostic host query contract for run/stage/branch/interview/checkpoint views.
- Implement timeline pagination/filtering and correlation-based drilldown.
- Provide stage->agent->tool linkage traversal.
- Add CLI query commands for timeline/inspection operations.
- Add deterministic query parity tests across memory/fs/cxdb backends.

## Out of Scope
- UI renderer/plugin implementation.
- HTTP transport protocol and server deployment mode.
- Distributed scheduling/coordination behavior.

## Priority 0 (Must-have)

### [ ] G1. Query contract v1 (backend-agnostic)
- Work:
  - Define canonical host query models for:
    - run summary
    - stage timeline
    - branch timeline
    - checkpoint snapshots
    - interview history
  - Define stable filter/sort/pagination semantics.
- Files:
  - `crates/forge-attractor/src/queries.rs`
  - `crates/forge-attractor/src/storage/types.rs`
- DoD:
  - Query contract is explicit and independent of concrete storage backend.

### [ ] G2. Timeline assembly and pagination
- Work:
  - Implement query adapters that assemble ordered timeline entries from persisted turns/events.
  - Support cursor pagination and filters by:
    - run id
    - node id
    - stage attempt id
    - branch id
    - event kind
- Files:
  - `crates/forge-attractor/src/queries.rs`
  - `crates/forge-attractor/src/storage/mod.rs`
- DoD:
  - Hosts can page and filter large timelines deterministically.

### [ ] G3. Stage-to-agent drilldown traversal
- Work:
  - Implement drilldown resolver from stage event -> stage-to-agent link -> agent turn timeline.
  - Include lightweight summary payloads (tool call count, tool error count, last head turn).
  - Preserve correlation provenance for auditability.
- Files:
  - `crates/forge-attractor/src/queries.rs`
  - `crates/forge-attractor/src/backends/forge_agent.rs`
- DoD:
  - Host can traverse run -> stage -> agent -> tool activity via stable APIs.

## Priority 1 (Strongly recommended)

### [ ] G4. CLI query/inspection commands
- Work:
  - Add CLI command family for timeline and drilldown inspection.
  - Provide both human-readable and JSON output modes.
  - Add filter flags aligned to query contract.
- Files:
  - `crates/forge-cli/src/main.rs`
- DoD:
  - Operators can inspect complex runs directly from CLI without ad hoc log parsing.

### [ ] G5. Query parity and performance guardrails
- Work:
  - Add deterministic parity tests for memory/fs/cxdb backends.
  - Add basic performance budgets for timeline queries on representative run sizes.
  - Add regression tests for cursor semantics and ordering stability.
- Files:
  - `crates/forge-attractor/tests/queries_parity.rs`
  - `crates/forge-attractor/tests/queries_pagination.rs`
  - `crates/forge-attractor/tests/queries_drilldown.rs`
- DoD:
  - Query results are stable across backends and resilient to large timelines.

### [ ] G6. Documentation and runbook updates
- Work:
  - Document query contract and CLI usage examples.
  - Add operator runbook for common debugging flows:
    - failed stage inspection
    - branch divergence analysis
    - stage-to-agent tool failure tracing
- Files:
  - `crates/forge-attractor/README.md`
  - `crates/forge-cli/README.md`
  - `README.md`
- DoD:
  - Query/drilldown workflows are documented and reproducible.

## Deliverables
- Stable host query contract with timeline/drilldown support.
- CLI inspection commands for complex run debugging.
- Backend parity coverage for query behavior.
- Documentation/runbook for day-2 operability.

## Execution order
1. G1 query contract v1
2. G2 timeline assembly/pagination
3. G3 stage-to-agent drilldown
4. G4 CLI query commands
5. G5 parity/performance tests
6. G6 docs/runbook

## Exit criteria for this file
- Complex runs are inspectable via structured timeline and drilldown queries.
- Query behavior is backend-agnostic, deterministic, and test-backed.
- Operators can debug run/stage/branch/agent issues without raw turn spelunking.
