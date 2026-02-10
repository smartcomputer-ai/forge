# P39: CXDB-Native Turn Model and Runtime Semantics Rebase

**Status**
- Planned (2026-02-10)

**Goal**
Rebase Forge runtime persistence on CXDB-native primitives and typed records, removing redundant envelope indirection (`event_kind` + `payload_json`) and duplicated lineage fields that CXDB already provides.

**Source**
- Spec of record: `spec/04-cxdb-integration-spec.md`
- CXDB references:
  - `crates/forge-cxdb/docs/architecture.md`
  - `crates/forge-cxdb/docs/protocol.md`
  - `crates/forge-cxdb/docs/http-api.md`
  - `crates/forge-cxdb/docs/type-registry.md`
- Prerequisites:
  - `roadmap/completed/p33-cxdb-first-architecture-pivot-and-spec-rebaseline.md`
  - `roadmap/completed/p38-cxdb-fstree-and-workspace-snapshot-integration.md`

**Context**
- Forge currently writes distinct type IDs for agent and attractor records, but both are wrapped in a generic envelope shape with `event_kind` and stringified `payload_json`.
- Query/read paths re-decode nested payload JSON instead of consuming strongly typed projection fields directly.
- CXDB already provides core turn graph semantics (`turn_id`, `parent_turn_id`, `depth`, declared type/version, content hash, append timestamp), so duplicating these semantics in payload correlation creates drift risk.
- Project is early-stage and can adopt a clean break without compatibility shims.

## Scope
- Replace envelope-style event encoding with concrete typed record schemas.
- Align Forge lineage semantics with CXDB turn graph semantics first.
- Remove or minimize duplicated metadata that CXDB already owns.
- Rework query surfaces to rely on typed projections directly.
- Keep agent and attractor type families distinct while making causality explicit.

## Out of Scope
- Backward compatibility for existing persisted Forge envelope records.
- Transitional dual-read or migration adapters.
- UI renderer design work.

## Design Constraints (Hard Requirements)
- No `payload_json` field in runtime record schemas.
- No synthetic parent linkage fields when CXDB `parent_turn_id` already represents causality.
- `event_kind` only exists when it is domain data, not as a generic envelope requirement.
- Registry descriptors must map directly to domain fields with stable numeric tags.

## Priority 0 (Must-have)

### [ ] G1. Runtime data model contract rewrite (clean break)
- Work:
  - Define a new runtime schema contract for agent and attractor records with explicit type sets.
  - Remove envelope contract as primary runtime record shape.
  - Update `spec/04-cxdb-integration-spec.md` sections that currently describe generic envelope framing.
- Files:
  - `spec/04-cxdb-integration-spec.md`
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-attractor/src/storage/mod.rs`
  - `crates/forge-attractor/src/storage/types.rs`
- DoD:
  - Runtime write contract is defined in CXDB-native terms with concrete typed records and no generic payload wrapper.

### [ ] G2. Agent records as first-class typed turns
- Work:
  - Replace generic `forge.agent.event` envelope usage with concrete typed event records (for example session lifecycle, tool call lifecycle, model response summaries).
  - Keep message turns (`forge.agent.user_turn`, `forge.agent.assistant_turn`, etc.) but remove unnecessary envelope-only fields.
  - Use CXDB parent linkage (`parent_turn_id`) as the canonical turn ordering/causality mechanism.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/tests/*`
- DoD:
  - Agent turns project as explicit typed records without nested JSON decoding.

### [ ] G3. Attractor stage/run/checkpoint records as first-class typed turns
- Work:
  - Replace event envelope dependence for attractor run/stage/checkpoint records with concrete typed schemas.
  - Keep stage-to-agent linkage as explicit typed records, but remove duplicated IDs where representable via context/turn lineage.
  - Encode stage causality primarily through real parent edges and explicit typed fields only when semantically necessary.
- Files:
  - `crates/forge-attractor/src/storage/types.rs`
  - `crates/forge-attractor/src/storage/mod.rs`
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/tests/*`
- DoD:
  - Attractor runtime records are projection-native and do not require `event_kind` dispatch + payload reparse.

### [ ] G4. CXDB linkage-first write semantics
- Work:
  - Set explicit `parent_turn_id` where required for deterministic causal chains.
  - Remove duplicated correlation fields that re-state CXDB graph primitives.
  - Keep only correlation fields that are genuinely cross-context/cross-runtime.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/src/backends/forge_agent.rs`
- DoD:
  - Causality is represented primarily by CXDB turn DAG primitives, with minimal supplemental metadata.

### [ ] G5. Query surface rewrite to typed projection-first
- Work:
  - Remove envelope reconstruction and nested JSON decode assumptions from query helpers.
  - Read typed fields directly from projected turn data.
  - Ensure run/stage/checkpoint/link query APIs are stable over typed records.
- Files:
  - `crates/forge-attractor/src/queries.rs`
  - `crates/forge-cxdb-runtime/src/adapter.rs`
  - `crates/forge-cxdb-runtime/src/runtime.rs`
- DoD:
  - Query paths are projection-native and no longer depend on envelope-specific parsing logic.

## Priority 1 (Strongly recommended)

### [ ] G6. Registry bundle simplification and semantic field discipline
- Work:
  - Publish new bundle IDs for clean-break schemas.
  - Add semantic hints (`unix_ms`, `duration_ms`, etc.) where appropriate.
  - Eliminate fields whose values duplicate CXDB metadata (for example turn parent/depth surrogates).
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-attractor/src/runner.rs`
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - Registry descriptors are lean, semantically clear, and projection-friendly.

### [ ] G7. Deterministic conformance and live validation refresh
- Work:
  - Replace envelope-specific tests with typed-schema conformance tests.
  - Validate CXDB query/read behavior against new record schemas.
  - Keep live suites for provider + CXDB integration intact.
- Files:
  - `crates/forge-agent/tests/*`
  - `crates/forge-attractor/tests/*`
  - `crates/forge-cxdb-runtime/tests/live.rs`
- DoD:
  - New schema model is test-backed across deterministic and live paths.

### [ ] G8. Docs and operator mental-model update
- Work:
  - Update repository docs to describe CXDB-native runtime modeling.
  - Document what metadata is CXDB-native vs Forge-domain.
  - Provide guidance for reading traces directly from typed projection fields.
- Files:
  - `README.md`
  - `crates/forge-agent/README.md`
  - `crates/forge-attractor/README.md`
  - `crates/forge-cli/README.md`
  - `AGENTS.md` (if architecture summaries change)
- DoD:
  - Documentation matches the new runtime data model and removes envelope-era assumptions.

## Deliverables
- Clean-break runtime record model aligned to CXDB primitives.
- Distinct agent and attractor typed turn families without envelope indirection.
- Query surfaces that read typed projection directly.
- Updated spec/docs/test coverage for the new model.

## Execution order
1. G1 contract rewrite in spec + crate boundaries
2. G2 agent write model
3. G3 attractor write model
4. G4 linkage-first causality cleanup
5. G5 query surface rewrite
6. G6 registry discipline
7. G7 conformance/live validation
8. G8 docs update

## Exit criteria for this file
- Forge runtime data model uses CXDB turns as primary semantic structure, not an envelope-over-turn pattern.
- Agent and attractor traces are directly understandable from typed projected fields.
- No core query path requires nested payload JSON reconstruction.
