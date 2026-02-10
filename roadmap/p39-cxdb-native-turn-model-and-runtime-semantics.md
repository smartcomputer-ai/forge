# P39: CXDB-Native Turn Model and Forge Runtime Semantics Rebase

**Status**
- Planned (2026-02-10)

**Goal**
Move Forge persistence from envelope-over-turn records to Forge-native typed turns that match actual Agent and Attractor runtime semantics, while relying on CXDB turn graph primitives for lineage.

**Source**
- Spec of record: `spec/04-cxdb-integration-spec.md`
- Semantic source of truth for Attractor behavior: `spec/03-attractor-spec.md`
- Runtime code shape baseline:
  - `crates/forge-attractor/src/events.rs`
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-agent/src/session.rs`
- CXDB references:
  - `crates/forge-cxdb/docs/architecture.md`
  - `crates/forge-cxdb/docs/protocol.md`
  - `crates/forge-cxdb/docs/type-registry.md`
- Prerequisites:
  - `roadmap/completed/p33-cxdb-first-architecture-pivot-and-spec-rebaseline.md`
  - `roadmap/completed/p38-cxdb-fstree-and-workspace-snapshot-integration.md`

**Context**
- Current persistence still serializes a generic `StoredTurnEnvelope` with `event_kind` + `payload_json`.
- Attractor runtime already has explicit semantic categories in code (`PipelineEvent`, `StageEvent`, `ParallelEvent`, `InterviewEvent`, `CheckpointEvent`) but persistence flattens these into envelope events.
- Agent runtime already has distinct message turn types, but operational lifecycle/tool telemetry is collapsed into generic `forge.agent.event`.
- CXDB already stores turn graph structure (`context_id`, `turn_id`, `parent_turn_id`, `depth`, `type_id`, `type_version`, append timestamp, content hash); re-encoding these as Forge payload fields creates duplication.

## Core Modeling Decision
Do not adopt external schemas verbatim. Model persistence around Forge runtime semantics as implemented today:
- Attractor stages are stage-attempt lifecycle facts, not agent message turns.
- Agent messages remain transcript turns; agent operational telemetry is separate typed facts.
- Cross-runtime linkage is explicit (`stage -> agent`) and minimal.
- Contexts represent execution threads, not graph nodes.

## Context Topology (v2)
- One Attractor run-attempt context is the orchestration spine for that attempt.
- Agent work runs in agent-session contexts that may span multiple stage attempts/nodes when thread/session continuity is intended.
- `forge.link.stage_to_agent` is the authoritative join between stage-attempt facts and agent-session turns.
- Do not create one context per attractor node by default.
- Parallel branch contexts are introduced only when true branch execution is implemented (see `roadmap/later/p81-attractor-true-parallel-and-fan-in-semantics.md`).

## Field Ownership Contract
CXDB-native fields (do not duplicate in payload unless strictly required for cross-context navigation):
- `context_id`, `turn_id`, `parent_turn_id`, `depth`
- `type_id`, `type_version`
- append timestamp and content hash

Forge-domain fields (payload/typed fields):
- `run_id`, `graph_id`, `node_id`, `stage_attempt_id`, `attempt`
- stage status/outcome fields, routing intent, human-gate result
- agent session/tool lifecycle details
- cross-context link keys (`agent_context_id`, `agent_head_turn_id`) where needed

## Scope
- Replace generic envelope encoding with typed record schemas per Forge semantic family.
- Align Attractor persistence with runtime event categories already defined in code.
- Split agent operational events from agent transcript turns.
- Lock context topology to run-spine + agent-session contexts (not per-node contexts).
- Use CXDB `parent_turn_id` as canonical in-context causality.
- Keep only cross-context linkage metadata as explicit typed fields.
- Update query surfaces to projection-first typed reads.

## Out of Scope
- Backward compatibility for old envelope payloads.
- Dual-write and migration adapters.
- UI renderer changes.

## Proposed Forge Semantic Type Families (v2)

### Attractor
- `forge.attractor.run_lifecycle`  
  kinds: `initialized`, `resumed`, `finalized`
- `forge.attractor.stage_lifecycle`  
  kinds: `started`, `completed`, `failed`, `retrying`
- `forge.attractor.parallel_lifecycle`  
  kinds: `started`, `branch_started`, `branch_completed`, `completed`
- `forge.attractor.interview_lifecycle`  
  kinds: `started`, `completed`, `timeout`
- `forge.attractor.checkpoint_saved`
- `forge.attractor.route_decision` (new; record selected next step explicitly instead of only inferring from checkpoint payload)
- `forge.attractor.dot_source`
- `forge.attractor.graph_snapshot`
- `forge.link.stage_to_agent`

### Agent
- Keep transcript turn families:
  - `forge.agent.user_turn`
  - `forge.agent.assistant_turn`
  - `forge.agent.tool_results_turn`
  - `forge.agent.system_turn`
  - `forge.agent.steering_turn`
- Replace generic `forge.agent.event` with:
  - `forge.agent.session_lifecycle` (`started`, `ended`)
  - `forge.agent.tool_call_lifecycle` (`started`, `ended`)

## CAS/Artifact Note (Non-blocking in P39)
CXDB already uses content-hash-addressed blobs. Forge may keep hash references in typed payloads for domain linkage (for example `dot_source_ref`, `graph_snapshot_ref`, artifact refs), but should not add a parallel hash identity system.

## Priority 0 (Must-have)

### [ ] G1. Semantic inventory freeze from Forge runtime code
- Work:
  - Document canonical persisted semantic facts directly from current runtime behavior.
  - Freeze v2 type families and per-type required fields before implementation.
  - Freeze context-topology contract (run-spine context, agent-session contexts, optional branch contexts).
- Files:
  - `spec/04-cxdb-integration-spec.md`
  - `roadmap/p39-dod-matrix.md`
- DoD:
  - No event/type naming in the implementation is derived from external schema examples; all names map to Forge runtime semantics.
  - No implicit one-context-per-node model is introduced.

### [ ] G2. Envelope removal and typed schema contract
- Work:
  - Remove `payload_json`-centric schema contract from runtime write paths.
  - Replace generic `event_kind` envelope with typed fields per family.
  - Publish new clean-break registry bundle IDs for agent and attractor runtime schemas.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-attractor/src/storage/mod.rs`
  - `crates/forge-attractor/src/storage/types.rs`
  - `spec/04-cxdb-integration-spec.md`
- DoD:
  - Runtime writes use typed payloads only; no envelope reconstruction required.

### [ ] G3. Attractor persistence rebased to runtime event categories
- Work:
  - Map persisted attractor turns to `Pipeline/Stage/Parallel/Interview/Checkpoint` lifecycle categories.
  - Add explicit route decision records.
  - Keep stage-attempt semantics first-class (`node_id`, `stage_attempt_id`, `attempt`).
- Files:
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/src/storage/types.rs`
  - `crates/forge-attractor/src/storage/mod.rs`
  - `crates/forge-attractor/src/queries.rs`
- DoD:
  - Attractor trace can be read as stage/run lifecycle without interpreting generic `event_kind` strings.

### [ ] G4. Agent persistence split: transcript vs operational lifecycle
- Work:
  - Keep message turns as transcript records.
  - Replace `forge.agent.event` with typed session/tool lifecycle records.
  - Ensure tool call start/end joins via `call_id`.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-agent/tests/*`
- DoD:
  - Agent trace no longer encodes operational lifecycle in a generic event envelope.

### [ ] G5. CXDB DAG-first causality cleanup
- Work:
  - Set `parent_turn_id` deterministically for in-context causal chain.
  - Drop payload fields that duplicate CXDB turn lineage primitives.
  - Preserve only cross-context joins (for example stage->agent link context/head refs).
  - Ensure run-stages remain on run context spine while agent turns remain in agent-session contexts.
- Files:
  - `crates/forge-agent/src/session.rs`
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/src/backends/forge_agent.rs`
- DoD:
  - Causality is represented by CXDB DAG, not mirrored in payload correlation fields.

### [ ] G6. Typed projection-first queries
- Work:
  - Rewrite query helpers to consume typed fields directly.
  - Remove envelope decoding and nested payload reparsing.
  - Keep run/stage/checkpoint/link queries stable over new schemas.
- Files:
  - `crates/forge-attractor/src/queries.rs`
  - `crates/forge-cxdb-runtime/src/adapter.rs`
  - `crates/forge-cxdb-runtime/src/runtime.rs`
- DoD:
  - Query paths are projection-native and schema-driven.

## Priority 1 (Strongly recommended)

### [ ] G7. Deterministic and live conformance refresh
- Work:
  - Replace envelope-oriented tests with typed-schema conformance tests.
  - Validate stage/agent linkage and route-decision queries under new schemas.
- Files:
  - `crates/forge-agent/tests/*`
  - `crates/forge-attractor/tests/*`
  - `crates/forge-cxdb-runtime/tests/live.rs`
- DoD:
  - Deterministic and live test suites cover new semantic families.

### [ ] G8. Docs and operator model update
- Work:
  - Document CXDB-native vs Forge-domain field ownership.
  - Document how to read agent transcript vs attractor stage lifecycle traces.
- Files:
  - `README.md`
  - `crates/forge-agent/README.md`
  - `crates/forge-attractor/README.md`
  - `crates/forge-cli/README.md`
  - `AGENTS.md` (if architecture index wording changes)
- DoD:
  - Docs reflect Forge-native schema families and CXDB DAG-first modeling.

## Deliverables
- Clean-break typed runtime schema families aligned to Forge semantics.
- Clear separation between attractor stage lifecycle and agent transcript lifecycle.
- CXDB DAG-first causality with minimal cross-context join metadata.
- Projection-native query and test coverage for the new model.

## Execution order
1. G1 semantic inventory freeze
2. G2 envelope removal + registry contract
3. G3 attractor semantic family implementation
4. G4 agent semantic family implementation
5. G5 DAG-first causality cleanup
6. G6 query rewrite
7. G7 conformance/live validation
8. G8 docs refresh

## Exit criteria for this file
- Attractor nodes are represented as stage/run lifecycle facts, not as generic agent-like messages.
- Agent transcript turns and agent operational lifecycle are distinct typed records.
- Context model is run-spine + agent-session (+ optional branch contexts), not per-node contexts.
- No core runtime write/query path depends on `payload_json` envelope parsing.
