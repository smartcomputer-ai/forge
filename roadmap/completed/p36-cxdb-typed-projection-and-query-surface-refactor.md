# P36: CXDB Typed Projection and Query-Surface Refactor
**Complete** (2026-02-10)

**Status**
- Complete (2026-02-10)
- G1 completed (2026-02-10)
- G2 completed (2026-02-10)
- G3 completed (2026-02-10)
- G4 completed (2026-02-10)
- G5 completed (2026-02-10)
- G6 completed (2026-02-10)

**Goal**
Move host/runtime read paths to CXDB typed projection APIs and enforce schema/registry discipline (msgpack numeric tags + registry bundles) for stable drilldown and long-term compatibility.

**Source**
- Spec of record: `spec/04-cxdb-integration-spec.md`
- Related references:
  - `crates/forge-cxdb/docs/protocol.md`
  - `crates/forge-cxdb/README.md`
  - `spec/04-cxdb-integration-spec.md` (Sections 3.7, 4.2, 4.7)
- Prerequisites:
  - `roadmap/p34-cxdb-direct-runtime-write-path-migration.md`
  - `roadmap/p38-cxdb-fstree-and-workspace-snapshot-integration.md`

**Context**
- Current query logic decodes JSON envelopes from raw turn payload bytes and bypasses CXDB typed projection strengths.
- Production compatibility requires stable schema discipline and registry-backed projection.

## Scope
- Enforce typed payload encoding contracts.
- Publish/use registry bundles as part of write lifecycle.
- Refactor query/drilldown surfaces to prefer HTTP typed projections.
- Preserve deterministic pagination/order semantics and correlation linkage.

## Out of Scope
- UI renderer/plugin implementation.
- Non-CXDB read backends.

## Priority 0 (Must-have)

### [x] G1. Schema and type registry discipline
- Work:
  - Define/implement Forge-owned schema bundles for agent/attractor/link records.
  - Add bundle publication flow tied to runtime startup/versioning policy.
  - Add compatibility policy for schema evolution and unknown fields.
- Files:
  - `spec/04-cxdb-integration-spec.md`
  - `crates/forge-cxdb/docs/` (add/update registry docs if vendored artifacts are expanded)
  - runtime bootstrap paths in `crates/forge-agent` / `crates/forge-attractor`
- DoD:
  - Registry bundle lifecycle is deterministic and documented.

### [x] G2. Writer encoding migration to msgpack numeric tags
- Work:
  - Migrate persisted runtime payloads from JSON envelope bytes to deterministic msgpack payloads aligned with published types.
  - Preserve critical correlation fields and event semantics.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-attractor/src/storage/types.rs`
  - `crates/forge-attractor/src/runner.rs`
- DoD:
  - Runtime writes are projection-ready without JSON transcode dependency.

### [x] G3. Query path migration to HTTP typed projections
- Work:
  - Refactor run/stage/checkpoint/link queries to consume typed HTTP projection responses.
  - Keep explicit cursor paging (`before_turn_id`) and deterministic ordering guarantees.
  - Preserve stage->agent->tool drilldown semantics.
- Files:
  - `crates/forge-attractor/src/queries.rs`
  - `crates/forge-cxdb-runtime/src/runtime.rs`
  - `crates/forge-cli/src/main.rs`
- DoD:
  - Read/query surfaces no longer rely on local JSON payload decoding for CXDB mode.

### [x] G4. Query contract and host API stabilization
- Work:
  - Define stable query response models (run timeline, stage timeline, linkage drilldown) sourced from typed projection data.
  - Document backward/forward compatibility constraints.
- Files:
  - `crates/forge-attractor/src/queries.rs`
  - `README.md`
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - Host query contract is explicit, stable, and projection-native.

## Priority 1 (Strongly recommended)

### [x] G5. Deterministic and live projection parity tests
- Work:
  - Add tests verifying typed projection parity with expected runtime semantics.
  - Add live tests covering registry publish + typed retrieval + cursor pagination.
- Files:
  - `crates/forge-attractor/tests/*`
  - `crates/forge-cxdb-runtime/tests/live.rs`
  - `crates/forge-cxdb/tests/integration.rs`
- DoD:
  - Projection read path is regression-safe and operationally validated.

### [x] G6. Performance and operability guardrails
- Work:
  - Add baseline budgets for timeline/drilldown query latency and payload size behavior.
  - Document failure handling for registry mismatch and partial projection decode.
- Files:
  - `crates/forge-attractor/README.md`
  - `crates/forge-cli/README.md`
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - Projection-based host surfaces are predictable in production operations.

## Deliverables
- Projection-native runtime/query contract using CXDB HTTP typed views.
- Registry-aware, msgpack-based write/read discipline.
- Deterministic and live test coverage for typed projection flows.

## Execution order
1. G1 schema/registry discipline
2. G2 writer encoding migration
3. G3 query path migration
4. G4 host query contract stabilization
5. G5 projection parity/live tests
6. G6 performance/operability guardrails

## Exit criteria for this file
- CXDB typed projections are the default read contract for Forge drilldown/query surfaces.
- Runtime payload encoding and schema evolution are controlled and test-backed.
