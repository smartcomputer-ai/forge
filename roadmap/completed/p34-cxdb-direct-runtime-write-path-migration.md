# P34: CXDB-Direct Runtime Write-Path Migration (Agent + Attractor)
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
Migrate Agent and Attractor runtime write paths from `forge-turnstore`-based interfaces to direct CXDB-facing contracts, preserving deterministic behavior and a simple CXDB enabled/disabled persistence policy.

**Source**
- Spec of record: `spec/04-cxdb-integration-spec.md` (CXDB-first architecture)
- Prerequisites:
  - `roadmap/p33-cxdb-first-architecture-pivot-and-spec-rebaseline.md`
  - `roadmap/p32-cxdb-adapter-and-dual-level-persistence.md`

**Context**
- Runtime behavior is already CXDB-oriented but still wrapped in `TurnStore`/adapter abstractions.
- Current layering adds conversion overhead and weakens correctness guarantees in edge cases.

## Scope
- Introduce direct runtime persistence contracts around CXDB operations.
- Remove `TurnStore` as the runtime dependency in Agent and Attractor write paths.
- Preserve `off`/`required` handling as a CXDB toggle.
- Maintain stage-to-agent linkage semantics and causal ordering.

## Out of Scope
- Full turnstore crate removal (handled in P37).
- Fstree workspace snapshot integration (handled in P35).
- Query/projection surface migration (handled in P36).

## Priority 0 (Must-have)

### [x] G1. Agent persistence migration to CXDB-direct contracts
- Work:
  - Replace `Arc<dyn TurnStore>` session persistence dependency with CXDB writer contract(s).
  - Migrate context creation, append, and head lookup to direct CXDB contract calls.
  - Keep deterministic idempotency key policy and sequence numbering.
  - Replace `TurnStoreWriteMode` semantics with a CXDB enablement toggle (`off`/`required`), removing turnstore terminology.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/src/config.rs`
  - `crates/forge-agent/tests/turnstore_integration.rs` (migrate/rename)
  - `crates/forge-agent/tests/cxdb_parity.rs`
- DoD:
  - Agent runtime persistence no longer requires `forge-turnstore` traits.

### [x] G2. Attractor runtime write path migration to CXDB-direct contracts
- Work:
  - Replace `AttractorStorageWriter` blanket implementation over `TurnStore` with explicit CXDB-targeted writer contract.
  - Migrate run/stage/checkpoint/link append paths to direct CXDB calls.
  - Preserve sequence number and idempotency construction semantics.
  - Replace `StorageWriteMode` behavior with CXDB enablement semantics (`off`/`required`) and remove error downgrade paths.
- Files:
  - `crates/forge-attractor/src/storage/mod.rs`
  - `crates/forge-attractor/src/storage/types.rs`
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/src/backends/forge_agent.rs`
- DoD:
  - Attractor runtime writes are CXDB-direct and do not depend on `forge-turnstore` contracts.

### [x] G3. Host/bootstrap wiring migration
- Work:
  - Move CXDB wiring responsibility to host/bootstrap layers with explicit binary+HTTP endpoint config.
  - Remove duplicate constructor paths that only differ by adapter wrappers.
  - Standardize env/config names for binary/HTTP endpoints and CXDB enablement settings.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-cli/src/main.rs`
  - `README.md`
- DoD:
  - Runtime construction path is direct and unambiguous.

### [x] G4. Correctness fixes uncovered by abstraction removal
- Work:
  - Fix deterministic idempotency fallback key parent-resolution behavior.
  - Ensure returned parent turn metadata reflects committed parent semantics.
  - Remove capability mismatches in mandatory runtime contracts.
- Files:
  - `crates/forge-turnstore-cxdb/src/adapter.rs` (or replacement CXDB runtime module)
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-attractor/src/runner.rs`
- DoD:
  - No known idempotency/parent metadata correctness regressions remain.

## Priority 1 (Strongly recommended)

### [x] G5. Deterministic fake-CXDB contract test harness
- Work:
  - Add shared fake CXDB client harness for agent/attractor tests.
  - Remove backend portability assumptions from tests and assert direct CXDB behavior contracts instead.
- Files:
  - `crates/forge-agent/tests/*`
  - `crates/forge-attractor/tests/*`
  - `crates/forge-turnstore-cxdb/tests/*` (as long as crate exists)
- DoD:
  - Unit/integration tests stay deterministic without requiring live CXDB.

### [x] G6. Live CXDB smoke and resilience coverage update
- Work:
  - Expand live tests to validate binary write path specifically (not HTTP-only fallbacks).
  - Cover toggle behavior: `off` skips writes, `required` fails deterministically under endpoint failures.
- Files:
  - `crates/forge-turnstore-cxdb/tests/live.rs` (or successor runtime CXDB integration tests)
  - `crates/forge-agent/tests/cxdb_parity.rs`
  - `crates/forge-attractor/tests/cxdb_parity.rs`
- DoD:
  - Live checks confirm direct CXDB write-path correctness and resilience modes.

## Deliverables
- Agent/Attractor write paths migrated to CXDB-direct contracts.
- Preserved CXDB toggle semantics (`off`/`required`) and stage-agent linkage behavior.
- Correctness fixes applied to idempotency/parent metadata handling.

## Execution order
1. G1 agent migration
2. G2 attractor migration
3. G3 host/bootstrap wiring
4. G4 correctness fixes
5. G5 deterministic fake-CXDB tests
6. G6 live smoke/resilience coverage

## Exit criteria for this file
- Runtime write paths no longer depend on `forge-turnstore` abstraction.
- Direct CXDB persistence behavior is deterministic and test-backed.
