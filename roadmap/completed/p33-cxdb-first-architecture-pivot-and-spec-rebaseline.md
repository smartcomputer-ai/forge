# P33: CXDB-First Architecture Pivot and Spec Rebaseline (Post-P32)

**Complete** (2026-02-10)

**Status**
- Completed (2026-02-10)
- G1 completed (2026-02-10)
- G2 completed (2026-02-10)
- G3 completed (2026-02-10)
- G4 completed (2026-02-10)
- G5 completed (2026-02-10)
- G6 completed (2026-02-10)

**Goal**
Rebaseline Forge persistence around a CXDB-first architecture, replacing the current multi-backend `turnstore` abstraction strategy with direct CXDB contracts in runtime cores while preserving deterministic local testability.

**Source**
- Spec of record: `spec/04-cxdb-integration-spec.md` (this milestone updates the spec)
- Related specs:
  - `spec/02-coding-agent-loop-spec.md`
  - `spec/03-attractor-spec.md`
- Prerequisites:
  - `roadmap/completed/p27.1-turnstore-foundation-and-agent-persistence.md`
  - `roadmap/p32-cxdb-adapter-and-dual-level-persistence.md`

**Context**
- Forge now vendors `crates/forge-cxdb` and already routes most meaningful persistence behavior through CXDB semantics.
- The current `forge-turnstore` layer leaks backend details and currently includes capability mismatch risk (for example, required trait methods that are unsupported in the CXDB client path).
- Agent and Attractor already pull in CXDB-specific types for constructors/wiring, indicating the abstraction boundary is no longer real.

## Scope
- Adopt CXDB as the primary runtime persistence contract for Forge.
- Define and document the architectural pivot in spec/roadmap/docs.
- Establish migration policy for existing `forge-turnstore` and `forge-turnstore-cxdb` crates.
- Preserve deterministic unit/integration testing strategy without requiring a running CXDB service.

## Out of Scope
- Stage outcome contract changes (`status.json` ingestion) beyond what is needed for persistence contract migration.
- Parallel/fan-in semantic changes.
- Runtime control-plane semantics.

## Priority 0 (Must-have)

### [x] G1. Architecture and terminology rebaseline in specs/docs
- Work:
  - Update `spec/04-cxdb-integration-spec.md` from storage-abstraction-first to CXDB-first runtime architecture.
  - Define clear terms:
    - `CXDB runtime write path`
    - `CXDB projection read path`
    - `optional local artifact mirror`
    - `test doubles/fakes` (instead of generic persistent backends)
  - Clarify that `forge-llm` remains CXDB-independent.
- Files:
  - `spec/04-cxdb-integration-spec.md`
  - `AGENTS.md`
  - `README.md`
- DoD:
  - Spec language and repository architecture docs consistently describe CXDB-first behavior.

### [x] G2. Crate-boundary policy for CXDB-first runtime cores
- Work:
  - Define approved dependency direction:
    - `forge-agent` and `forge-attractor` may depend on CXDB-facing contracts/types.
    - `forge-llm` must remain decoupled.
  - Define where conversion layers belong (only at host/runtime boundaries, not hidden behind generic turnstore adapters).
  - Document deprecation posture for `forge-turnstore` crates.
- Files:
  - `spec/04-cxdb-integration-spec.md`
  - `AGENTS.md`
  - `roadmap/p34-cxdb-direct-runtime-write-path-migration.md`
- DoD:
  - The target crate graph and dependency rules are explicit and enforceable.

### [x] G3. CXDB-first runtime persistence contract definition
- Work:
  - Specify runtime-facing CXDB capability contracts directly (append/read/registry/blob/fs-sync).
  - Replace generic `TurnStore` framing in spec with CXDB-native operation framing.
  - Keep `off`/`required` behavior as runtime policy independent of transport, modeled as CXDB disabled/enabled.
- Files:
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - Runtime contract maps 1:1 to CXDB protocol/HTTP capabilities with no leaky mandatory methods.

### [x] G4. Migration policy and compatibility strategy
- Work:
  - Define phased migration:
    - compatibility phase (coexistence)
    - deprecation warnings
    - removal/sunset phase
  - Define what parity means after the pivot (behavioral parity via fakes/mocks, not persistent backend portability).
- Files:
  - `roadmap/p37-turnstore-sunset-and-cxdb-hardening.md`
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - There is a concrete, low-risk migration path from current code to CXDB-first runtime.

## Priority 1 (Strongly recommended)

### [x] G5. Test strategy rebaseline for CXDB-first architecture
- Work:
  - Replace backend-portability parity framing with:
    - deterministic mock/fake CXDB contract tests
    - optional live CXDB integration tests
    - end-to-end runtime conformance tests in `off` and `required` modes.
  - Define expected minimum coverage for write/read/fs-sync/registry paths.
- Files:
  - `spec/04-cxdb-integration-spec.md`
  - `roadmap/p34-cxdb-direct-runtime-write-path-migration.md`
  - `roadmap/p38-cxdb-fstree-and-workspace-snapshot-integration.md`
- DoD:
  - Test guidance matches architecture and is actionable for implementation milestones.

### [x] G6. Backlog and sequencing alignment
- Work:
  - Create active refactor milestones (`p34+`) for code migration and cleanup.
  - Mark deferred Attractor feature milestones (`roadmap/later/p80..p84`) as explicitly sequenced after the CXDB-first migration series.
- Files:
  - `roadmap/later/p80-attractor-stage-outcome-contract-and-status-ingestion.md`
  - `roadmap/later/p81-attractor-true-parallel-and-fan-in-semantics.md`
  - `roadmap/later/p82-attractor-runtime-control-plane-and-resume-hardening.md`
  - `roadmap/later/p83-attractor-attribute-policy-completion-and-contract-tightening.md`
  - `roadmap/later/p84-attractor-host-timeline-and-query-drilldown-surfaces.md`
- DoD:
  - Active queue is unambiguous and the refactor sequence is clearly front-loaded.

## Deliverables
- CXDB-first architecture definition in spec/docs.
- Migration policy and crate-boundary rules.
- Follow-on implementation milestones for direct runtime migration and turnstore sunset.

## Execution order
1. G1 spec/docs rebaseline
2. G2 crate-boundary policy
3. G3 runtime contract definition
4. G4 migration policy
5. G5 test strategy rebaseline
6. G6 backlog sequencing alignment

## Exit criteria for this file
- CXDB-first architecture is explicitly adopted in the spec and repository-level docs.
- The migration plan from turnstore abstraction to direct CXDB contracts is approved and actionable.
