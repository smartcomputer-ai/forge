# P32: CXDB Adapter and Dual-level Persistence Activation (Spec 04 post-P31)

**Status**
- Planned (2026-02-10)

**Goal**
Implement CXDB-backed turnstore adapter and activate dual-level persistence for Attractor and Agent timelines, including stage-to-agent drill-down.

**Source**
- Spec of record: `spec/04-cxdb-integration-spec.md` (Phases C and D)
- Runtime prerequisites:
  - `roadmap/p27.1-turnstore-foundation-and-agent-persistence.md`
  - `roadmap/p31-attractor-conformance-tests-docs-and-dod-matrix.md`

**Context**
- P31 closes deterministic Attractor runtime conformance on local storage backends.
- This phase introduces CXDB transport/projection integration without destabilizing runtime cores.

## Scope
- Implement `forge-turnstore-cxdb` adapter for append/fork/read operations.
- Integrate adapter with `forge-agent` and Attractor runtime in `best_effort` mode.
- Persist DOT source and normalized graph snapshots to CXDB-backed store.
- Expose stage-to-agent correlation records and drill-down query paths.
- Add parity tests between local backends and CXDB-backed runs.

## Out of Scope
- Mandatory CXDB mode for all runs.
- Full distributed worker claim/lease coordination rollout.
- Renderer/plugin execution in host UIs.

## Priority 0 (Must-have)

### [ ] G1. Add `forge-turnstore-cxdb` crate
- Work:
  - Implement `TurnStore` interface over CXDB append/fork/read APIs.
  - Support deterministic idempotency keys on append paths.
  - Support typed registry bundle publishing/retrieval.
- Files:
  - `Cargo.toml` (workspace membership)
  - `crates/forge-turnstore-cxdb/Cargo.toml`
  - `crates/forge-turnstore-cxdb/src/lib.rs`
  - `crates/forge-turnstore-cxdb/src/adapter.rs`
- DoD:
  - Adapter passes contract tests shared with memory/fs turnstore backends.

### [ ] G2. Wire `forge-agent` + Attractor runtime to CXDB adapter
- Work:
  - Add runtime config and bootstrap wiring for CXDB-backed turnstore selection.
  - Persist all required agent and attractor records through adapter in `best_effort` mode.
  - Persist stage-to-agent link records and verify traversal from run->stage->agent->tool.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/src/backends/forge_agent.rs`
- DoD:
  - Dual-level timeline is queryable and causally linked in CXDB.

### [ ] G3. DOT and graph snapshot persistence
- Work:
  - Persist DOT source (or artifact ref + hash) for each run.
  - Persist normalized graph snapshot used at runtime initialization.
  - Ensure large payload handling remains deterministic.
- Files:
  - `crates/forge-attractor/src/runtime.rs`
  - `crates/forge-attractor/src/storage/types.rs`
- DoD:
  - Observability layer can reconstruct run intent and executed graph from store data alone.

## Priority 1 (Strongly recommended)

### [ ] G4. Cross-backend parity and resilience suite
- Work:
  - Add integration tests comparing outcomes/order/idempotency across memory/fs/cxdb backends.
  - Add failure-mode tests for `off`, `best_effort`, `required` store modes.
- Files:
  - `crates/forge-turnstore/tests/parity.rs`
  - `crates/forge-attractor/tests/cxdb_parity.rs`
  - `crates/forge-agent/tests/cxdb_parity.rs`
- DoD:
  - CXDB-backed runs preserve deterministic semantics versus pre-existing backends.

### [ ] G5. Operational hardening and runbook docs
- Work:
  - Document endpoint topology (binary write path, HTTP projection path).
  - Document security controls, redaction, and retention policy hooks.
- Files:
  - `README.md`
  - `crates/forge-turnstore-cxdb/README.md`
  - `spec/04-cxdb-integration-spec.md` (if rollout details change)
- DoD:
  - Deployment and troubleshooting path is documented and reproducible.

## Deliverables
- `forge-turnstore-cxdb` adapter crate.
- Dual-level attractor+agent persistence to CXDB.
- DOT/graph visibility in store-backed observability.
- Parity and resilience coverage across backends.

## Execution order
1. G1 adapter crate
2. G2 runtime wiring
3. G3 DOT/graph persistence
4. G4 parity/resilience tests
5. G5 docs/runbooks

## Exit criteria for this file
- CXDB adapter is production-ready behind configuration flags.
- Run -> stage -> agent -> tool drill-down is supported via persisted links.
- Local deterministic test path remains available and green.
