# P37: Turnstore Sunset and CXDB Hardening Completion

**Status**
- In progress (2026-02-10)
- G1 completed (2026-02-10)
- G2 in progress (2026-02-10)
- Progress update (2026-02-10):
  - Removed `crates/forge-turnstore` and `crates/forge-turnstore-cxdb` from workspace membership and source tree.
  - Migrated runtime crates to `crates/forge-cxdb-runtime` contracts only.
  - Rebased `forge-attractor` storage contracts to local CXDB-first types/errors with no turnstore crate dependency.

**Goal**
Complete the CXDB-first migration by retiring `forge-turnstore` abstraction dependencies from runtime cores, finalizing crate/workspace cleanup, and hardening CXDB operational guarantees.

**Source**
- Spec of record: `spec/04-cxdb-integration-spec.md`
- Prerequisites:
  - `roadmap/p34-cxdb-direct-runtime-write-path-migration.md`
  - `roadmap/p35-cxdb-fstree-and-workspace-snapshot-integration.md`
  - `roadmap/p36-cxdb-typed-projection-and-query-surface-refactor.md`

**Context**
- After direct runtime migration, turnstore crates become either compatibility shims or dead abstractions.
- Final cleanup is required to prevent architecture drift and reduce maintenance surface.

## Scope
- Remove or archive legacy turnstore runtime abstractions.
- Complete workspace/dependency cleanup.
- Tighten operational controls and runbooks for CXDB-first deployments.
- Finalize conformance and DoD matrix for the migration series.

## Out of Scope
- New orchestration features unrelated to persistence architecture.
- Multi-backend persistence support revival.

## Priority 0 (Must-have)

### [x] G1. Runtime dependency cleanup and turnstore sunset
- Work:
  - Remove `forge-turnstore` runtime dependencies from `forge-agent` and `forge-attractor`.
  - Decide final disposition:
    - remove `forge-turnstore` + `forge-turnstore-cxdb` crates, or
    - keep only as non-runtime compatibility/test utilities with explicit deprecation markers.
  - Update workspace membership accordingly.
- Files:
  - `Cargo.toml`
  - `crates/forge-agent/Cargo.toml`
  - `crates/forge-attractor/Cargo.toml`
  - `crates/forge-turnstore/*` (if retained/deprecated)
  - `crates/forge-turnstore-cxdb/*` (if retained/deprecated)
- DoD:
  - Runtime cores are CXDB-first with no legacy abstraction coupling.

### [~] G2. Contract and naming cleanup
- Work:
  - Remove stale terminology (`turnstore`) from runtime-facing APIs/config where no longer accurate.
  - Keep migration aliases only where necessary and explicitly deprecated.
- Files:
  - `crates/forge-agent/src/*`
  - `crates/forge-attractor/src/*`
  - `crates/forge-cli/src/main.rs`
  - `README.md`
  - `AGENTS.md`
- DoD:
  - Public API/docs terminology reflects CXDB-first architecture.

### [ ] G3. Operational hardening completion
- Work:
  - Finalize endpoint topology, TLS/network guidance, and trust-boundary documentation.
  - Document incident/debug workflows for:
    - append path failures
    - projection path failures
    - registry mismatch
    - fs snapshot/attachment failures
- Files:
  - `README.md`
  - `crates/forge-cxdb/README.md`
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - Deployment and troubleshooting guidance is complete for CXDB-first operation.

### [ ] G4. Final migration conformance matrix
- Work:
  - Publish migration DoD matrix across p33-p37 covering:
    - architecture
    - write path
    - fs lineage
    - projection read path
    - operations
  - Require green deterministic and live smoke suites before closure.
- Files:
  - `roadmap/p37-dod-matrix.md` (new)
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - Migration series has explicit, testable closure criteria.

## Priority 1 (Strongly recommended)

### [ ] G5. Repository cleanup and dead-code elimination
- Work:
  - Remove obsolete tests, adapters, and compatibility glue no longer needed after migration.
  - Ensure no stale crate references in docs/examples/CI.
- Files:
  - workspace-wide targeted cleanup
- DoD:
  - Repository reflects a coherent CXDB-first architecture with minimal cruft.

### [ ] G6. Post-migration follow-on roadmap reactivation
- Work:
  - Re-baseline deferred feature milestones (stage outcome contract, true parallel, control plane, etc.) on top of CXDB-first foundation.
  - Ensure deferred roadmap files point to the new persistence baseline.
- Files:
  - `roadmap/later/p80-attractor-stage-outcome-contract-and-status-ingestion.md`
  - `roadmap/later/p81-attractor-true-parallel-and-fan-in-semantics.md`
  - `roadmap/later/p82-attractor-runtime-control-plane-and-resume-hardening.md`
  - `roadmap/later/p83-attractor-attribute-policy-completion-and-contract-tightening.md`
  - `roadmap/later/p84-attractor-host-timeline-and-query-drilldown-surfaces.md`
- DoD:
  - Next roadmap wave starts from the new architecture without ambiguity.

## Deliverables
- Turnstore abstraction sunset (or explicit non-runtime deprecation state).
- Fully CXDB-first runtime and docs terminology.
- Operational runbook hardening and migration DoD closure.

## Execution order
1. G1 dependency cleanup and sunset decision
2. G2 contract/naming cleanup
3. G3 operational hardening completion
4. G4 final migration DoD matrix
5. G5 repository cleanup
6. G6 follow-on roadmap reactivation

## Exit criteria for this file
- CXDB-first migration is complete, documented, and conformance-validated.
- Legacy turnstore abstraction no longer influences runtime architecture.
