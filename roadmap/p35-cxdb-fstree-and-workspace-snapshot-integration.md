# P35: CXDB FSTree and Workspace Snapshot Integration

**Status**
- Planned (2026-02-10)

**Goal**
Adopt CXDB filesystem snapshot primitives (`fstree` capture/upload + `append_turn_with_fs`/`attach_fs`) as first-class persistence for workspace and stage artifact lineage.

**Source**
- Spec of record: `spec/04-cxdb-integration-spec.md`
- CXDB references:
  - `crates/forge-cxdb/README.md`
  - `crates/forge-cxdb/docs/protocol.md`
  - `crates/forge-cxdb/src/fstree/*`
- Prerequisites:
  - `roadmap/p34-cxdb-direct-runtime-write-path-migration.md`

**Context**
- Forge currently stores selected large artifacts via blob refs but does not capture full workspace snapshots in CXDB lineage.
- Vendored CXDB already provides deterministic Merkle snapshot helpers and attachment operations.

## Scope
- Integrate CXDB `fstree` snapshot capture/upload in runtime flows.
- Attach workspace snapshot roots to relevant turns.
- Define snapshot policies and limits for determinism/cost control.
- Expose FS-root references in persisted envelopes and query surfaces.

## Out of Scope
- Full file restore tooling in hosts (read-only drilldown first).
- Distributed file synchronization protocols.

## Priority 0 (Must-have)

### [ ] G1. Runtime fs snapshot contract and policy
- Work:
  - Define when snapshots are captured (e.g., stage start/end, checkpoint boundaries, configurable policy).
  - Define include/exclude behavior and limits (`max_files`, `max_file_size`, symlink policy).
  - Define deterministic defaults for CI/local parity.
- Files:
  - `spec/04-cxdb-integration-spec.md`
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-agent/src/session.rs`
- DoD:
  - Snapshot capture policy is explicit and configurable.

### [ ] G2. Integrate `fstree` capture/upload + turn attachment
- Work:
  - Wire `cxdb::fstree::capture` and upload APIs into runtime persistence path.
  - Prefer atomic `append_turn_with_fs` where applicable.
  - Use `attach_fs` post-hoc only when needed.
- Files:
  - `crates/forge-cxdb/src/fs.rs` (if API extension needed)
  - `crates/forge-cxdb/src/fstree/upload.rs`
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-agent/src/session.rs`
- DoD:
  - Relevant turns in CXDB can carry attached fs root hashes.

### [ ] G3. Envelope/schema updates for fs lineage
- Work:
  - Add stable payload fields for fs lineage references:
    - `fs_root_hash`
    - `snapshot_policy_id`
    - optional summary stats (`file_count`, `bytes_uploaded`)
  - Ensure correlation fields keep stage/session linkage intact.
- Files:
  - `crates/forge-attractor/src/storage/types.rs`
  - `crates/forge-agent/src/session.rs`
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - FS lineage metadata is queryable and causally linked.

### [ ] G4. Artifact model consolidation
- Work:
  - Reconcile current blob-only artifact references with fs-root lineage references.
  - Clarify when to use:
    - blob hash only
    - fs_root attachment
    - both
- Files:
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/src/storage/types.rs`
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - Artifact persistence model is coherent and documented.

## Priority 1 (Strongly recommended)

### [ ] G5. Optional blob read-path parity support
- Work:
  - If needed for host drilldown, expose/implement missing GET_BLOB support in vendored CXDB client.
  - Add retrieval helpers for fs tree objects and file entries used in debugging workflows.
- Files:
  - `crates/forge-cxdb/src/fs.rs`
  - `crates/forge-cxdb/src/lib.rs`
  - `crates/forge-cxdb/tests/*`
- DoD:
  - Read-path capabilities required by host drilldown are available and tested.

### [ ] G6. Deterministic tests and live coverage
- Work:
  - Add deterministic unit/integration tests for snapshot capture policy and attachment behavior.
  - Add live tests for end-to-end fs upload + attachment against running CXDB.
- Files:
  - `crates/forge-attractor/tests/*`
  - `crates/forge-agent/tests/*`
  - `crates/forge-cxdb/tests/fstree_integration.rs`
- DoD:
  - FS snapshot lineage is regression-tested and operationally validated.

## Deliverables
- Runtime-integrated CXDB fs snapshots with turn attachments.
- Stable envelope fields for fs lineage metadata.
- Deterministic and live test coverage for fs snapshot flows.

## Execution order
1. G1 snapshot policy contract
2. G2 capture/upload + attachment integration
3. G3 envelope updates
4. G4 artifact model consolidation
5. G5 read-path parity support
6. G6 tests and live validation

## Exit criteria for this file
- Forge run/stage history includes auditable workspace snapshot lineage in CXDB.
- Snapshot behavior is deterministic, configurable, and test-backed.
