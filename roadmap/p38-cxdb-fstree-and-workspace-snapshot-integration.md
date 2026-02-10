# P38: CXDB FSTree and Workspace Snapshot Integration
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
Adopt CXDB filesystem snapshot primitives (`fstree` capture/upload + `append_turn_with_fs`/`attach_fs`) as first-class persistence for workspace and stage artifact lineage.

## Why this exists
- Preserve auditable workspace lineage for agent/attractor runs, not only selected blob artifacts.
- Make replay/debug/root-cause workflows able to answer: "what exact workspace state produced this turn/stage outcome?"
- Close the remaining FS-lineage gap after P37 by wiring existing vendored `cxdb::fstree` helpers into runtime write paths.

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

## Architecture alignment (current code structure)
- Runtime persistence boundary is `crates/forge-cxdb-runtime` (not direct `cxdb` usage in `forge-agent`/`forge-attractor`).
- `forge-agent` persists via `SessionPersistenceWriter` in `crates/forge-agent/src/session.rs`.
- `forge-attractor` persists via `AttractorStorageWriter` in `crates/forge-attractor/src/storage/mod.rs` and `RunStorage` in `crates/forge-attractor/src/runner.rs`.
- Vendored `fstree` capture/upload implementation is synchronous and client-coupled in `crates/forge-cxdb/src/fstree/*`.
- Existing runtime artifact contract only exposes blob writes (`AttractorArtifactWriter::put_blob`), so FS attachment needs a first-class runtime contract extension.

## Out of Scope
- Full file restore tooling in hosts (read-only drilldown first).
- Distributed file synchronization protocols.

## Priority 0 (Must-have)

### [x] G1. Runtime fs snapshot contract and policy
- Work:
  - Define explicit snapshot trigger policy for both runtimes:
    - agent: `session_start`, `before_tool_call`, `after_tool_call`, `assistant_completion`, `session_end`
    - attractor: `run_start`, `stage_start`, `stage_end`, `checkpoint`, `run_end`
  - Define include/exclude behavior and limits (`max_files`, `max_file_size`, `follow_symlinks`).
  - Define deterministic defaults for CI/local parity.
- Files:
  - `spec/04-cxdb-integration-spec.md`
  - `crates/forge-attractor/src/runtime.rs` (extend `RunConfig`)
  - `crates/forge-agent/src/config.rs` (extend `SessionConfig`)
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-attractor/src/runner.rs`
- DoD:
  - Snapshot capture policy is explicit and configurable.

### [x] G2. Integrate `fstree` capture/upload + turn attachment
- Work:
  - Add runtime-level FS lineage contract in `forge-cxdb-runtime` that can:
    - capture + upload fstree from a workspace root with policy options
    - append with fs root (preferred path)
    - fallback to post-hoc `attach_fs`
  - Implement this in the SDK-backed adapter using vendored `cxdb` client.
  - Keep runtime callsites (`forge-agent`/`forge-attractor`) dependent only on `forge-cxdb-runtime` traits/types.
- Files:
  - `crates/forge-cxdb-runtime/src/adapter.rs`
  - `crates/forge-cxdb-runtime/src/runtime.rs`
  - `crates/forge-cxdb-runtime/src/testing.rs`
  - `crates/forge-attractor/src/storage/mod.rs`
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-agent/src/session.rs`
- DoD:
  - Relevant turns in CXDB can carry attached fs root hashes.

### [x] G3. Envelope/schema updates for fs lineage
- Work:
  - Add stable envelope/correlation payload fields for fs lineage references:
    - `fs_root_hash`
    - `snapshot_policy_id`
    - optional summary stats (`file_count`, `dir_count`, `symlink_count`, `total_bytes`, `bytes_uploaded`)
  - Ensure correlation fields keep stage/session linkage intact.
- Files:
  - `crates/forge-agent/src/session.rs` (tag map + registry descriptor update)
  - `crates/forge-attractor/src/storage/mod.rs` (tag map + registry descriptor update)
  - `crates/forge-attractor/src/storage/types.rs`
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - FS lineage metadata is queryable and causally linked.

### [x] G4. Artifact model consolidation
- Work:
  - Reconcile current blob-only artifact references with fs-root lineage references.
  - Clarify when to use:
    - blob hash only
    - fs_root attachment
    - both
- Files:
  - `crates/forge-attractor/src/runner.rs` (`persist_run_graph_metadata` + stage/checkpoint event payloads)
  - `crates/forge-agent/src/session.rs` (event payload enrichment)
  - `crates/forge-attractor/src/storage/types.rs`
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - Artifact persistence model is coherent and documented.

## Priority 1 (Strongly recommended)

### [x] G5. Optional blob read-path parity support
- Work:
  - If needed for host drilldown, expose/implement missing GET_BLOB support in vendored CXDB client.
  - Add retrieval helpers for fs tree objects and file entries used in debugging workflows.
- Files:
  - `crates/forge-cxdb/src/fs.rs`
  - `crates/forge-cxdb-runtime/src/adapter.rs`
  - `crates/forge-cxdb/src/lib.rs`
  - `crates/forge-cxdb/tests/*`
- DoD:
  - Read-path capabilities required by host drilldown are available and tested.

### [x] G6. Deterministic tests and live coverage
- Work:
  - Add deterministic unit/integration tests for snapshot capture policy and attachment behavior.
  - Add live tests for end-to-end fs upload + attachment against running CXDB.
- Files:
  - `crates/forge-cxdb-runtime/tests/live.rs`
  - `crates/forge-cxdb-runtime/src/testing.rs`
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
2. Extend `forge-cxdb-runtime` contracts for FS capture/upload + append/attach flows
3. G2 capture/upload + attachment integration in runtimes
4. G3 envelope updates
5. G4 artifact model consolidation
6. G5 read-path parity support
7. G6 tests and live validation

## Concrete implementation approach (recommended)
1. Extend runtime contracts first:
   - Add `FsSnapshotPolicy`, `FsSnapshotStats`, and a `capture_upload_workspace(...) -> { fs_root_hash, stats }` API in `forge-cxdb-runtime`.
   - Add `append_turn_with_fs(...)` to avoid append+attach race windows when possible.
2. Thread policy through runtime config:
   - `SessionConfig.fs_snapshot` and `RunConfig.fs_snapshot` with deterministic defaults (`follow_symlinks=false`, conservative excludes, fixed limits).
3. Hook runtime write points:
   - Agent: call snapshot capture around selected persistence events in `Session::persist_envelope` path.
   - Attractor: call snapshot capture in `RunStorage` around stage/checkpoint boundaries.
4. Persist lineage metadata:
   - Add envelope payload fields (`fs_root_hash`, `snapshot_policy_id`, `snapshot_stats`) and include in registry bundle field descriptors.
5. Keep blob artifacts for high-value payloads:
   - Continue blob refs for large logical artifacts (dot/graph/checkpoint payloads), and add FS root lineage alongside, not instead.
6. Validate:
   - deterministic tests for policy decisions and trigger boundaries
   - live tests for upload + append-with-fs/attach fallback.

## Exit criteria for this file
- Forge run/stage history includes auditable workspace snapshot lineage in CXDB.
- Snapshot behavior is deterministic, configurable, and test-backed.
