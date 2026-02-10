# P39 DoD Matrix (CXDB-Native Turn Model and Forge Runtime Semantics Rebase)

Status date: 2026-02-10

Legend:
- `[x]` complete and covered
- `[ ]` gap/deviation tracked

## G1 Freeze Artifacts
- [x] Canonical persisted semantic facts are documented from runtime behavior sources (`events.rs`, `runner.rs`, `session.rs`) in `spec/04-cxdb-integration-spec.md` section `3.3.1`.
- [x] V2 type family list and per-family required field minimums are frozen in `spec/04-cxdb-integration-spec.md` section `3.3.2`.
- [x] Context topology contract (run context, thread context, attempt/branch contexts) is frozen in `spec/04-cxdb-integration-spec.md` section `3.4`.
- [x] Fork-trigger policy and `fidelity=full` thread-reuse policy are frozen in `spec/04-cxdb-integration-spec.md` section `3.4`.

## Semantic Alignment (Forge-first)
- [x] Persisted event/type names map to Forge runtime semantics (Attractor `Pipeline/Stage/Parallel/Interview/Checkpoint`, Agent transcript + lifecycle), not external schema imports. Refs: `crates/forge-attractor/src/events.rs`, `crates/forge-attractor/src/runner.rs`, `crates/forge-agent/src/session.rs`
- [x] Attractor stage-attempt lifecycle is first-class (`node_id`, `stage_attempt_id`, `attempt`, status/retry fields). Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/src/storage/types.rs`
- [x] Agent transcript turns remain distinct from agent operational lifecycle telemetry. Refs: `crates/forge-agent/src/session.rs`
- [x] Context topology follows run-spine + thread/agent-context model; no default one-context-per-node writes. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/src/backends/forge_agent.rs`, `crates/forge-agent/src/session.rs`
- [x] Context classes are explicit and enforced: run context, thread context (`fidelity=full`), attempt/branch contexts (divergence only). Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/src/backends/forge_agent.rs`

## Data Model
- [x] Generic runtime envelope (`event_kind` + `payload_json`) is removed from active write paths. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/storage/mod.rs`
- [x] `forge.agent.event` is replaced by typed lifecycle families (`session_lifecycle`, `tool_call_lifecycle`). Refs: `crates/forge-agent/src/session.rs`
- [x] Attractor persistence uses typed lifecycle families (`run/stage/parallel/interview/checkpoint`) plus explicit `route_decision` and `stage_to_agent` link records. Refs: `crates/forge-attractor/src/storage/types.rs`, `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/src/backends/forge_agent.rs`

## CXDB Primitive Alignment
- [x] CXDB `parent_turn_id` is the primary in-context causal linkage in append paths. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/storage/mod.rs`
- [x] Payload fields do not duplicate CXDB lineage primitives (`turn_id`, `parent_turn_id`, `depth`) except for intentional cross-context joins. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/storage/types.rs`
- [x] Cross-context linkage remains explicit and minimal (`pipeline_context_id`, `agent_context_id`, `agent_head_turn_id`). Refs: `crates/forge-attractor/src/backends/forge_agent.rs`, `crates/forge-attractor/src/storage/types.rs`
- [x] Run-stage events stay on attractor run context spine; agent turns stay on agent-session contexts; joins occur via typed link records. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/src/backends/forge_agent.rs`, `crates/forge-agent/src/session.rs`
- [x] Thread-context reuse is keyed by resolved thread key only under `fidelity=full`; non-full fidelity does not reuse thread context. Refs: `crates/forge-attractor/src/backends/forge_agent.rs`, `crates/forge-attractor/src/runner.rs`
- [x] Parallel/retry fork policy is frozen in schemas/lineage contract; full runtime enforcement is tracked in `roadmap/later/p81-attractor-true-parallel-and-fan-in-semantics.md`. Refs: `spec/04-cxdb-integration-spec.md`, `roadmap/later/p81-attractor-true-parallel-and-fan-in-semantics.md`

## Registry and Projection
- [x] New runtime registry bundles represent typed schemas for all v2 families (clean break). Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/runner.rs`
- [x] Query paths consume typed projection fields directly without nested payload reparsing. Refs: `crates/forge-attractor/src/queries.rs`, `crates/forge-cxdb-runtime/src/adapter.rs`, `crates/forge-cxdb-runtime/src/runtime.rs`
- [x] Semantic hints are used for time/duration fields where useful (`unix_ms`, `duration_ms`, etc.). Refs: `spec/04-cxdb-integration-spec.md`, `crates/forge-cxdb/docs/type-registry.md`, `crates/forge-attractor/src/runner.rs`

## Runtime Behavior
- [x] Agent runtime preserves session/tool behavior under new typed records. Refs: `crates/forge-agent/tests/*`
- [x] Attractor runtime preserves run traversal, stage retry, checkpoint, and stage->agent link behavior under new typed records. Refs: `crates/forge-attractor/tests/*`
- [x] FS lineage integration remains intact with new schemas. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/runner.rs`, `crates/forge-cxdb-runtime/src/runtime.rs`
- [x] Artifact hash references remain payload-level linkage only; no parallel Forge hash identity system is introduced. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-cxdb/docs/protocol.md`

## Spec and Docs
- [x] `spec/04-cxdb-integration-spec.md` reflects Forge-native semantic type families and no envelope-over-turn dependency. Refs: `spec/04-cxdb-integration-spec.md`
- [x] Docs explain CXDB-native vs Forge-domain field ownership and how to read attractor vs agent traces. Refs: `README.md`, `crates/forge-agent/README.md`, `crates/forge-attractor/README.md`
- [x] AGENTS architecture index remains consistent with implemented model. Refs: `AGENTS.md`

## Verification Gates
- [x] Deterministic suites green:
  - `cargo test -p forge-agent`
  - `cargo test -p forge-attractor`
  - `cargo test -p forge-cxdb-runtime`
- [x] Live suite entrypoints remain healthy (env-gated):
  - `RUN_LIVE_OPENAI_TESTS=1 cargo test -p forge-llm --test openai_live -- --ignored`
  - `RUN_LIVE_ANTHROPIC_TESTS=1 cargo test -p forge-llm --test anthropic_live -- --ignored`
  - `RUN_LIVE_OPENAI_TESTS=1 cargo test -p forge-agent --test openai_live -- --ignored`
  - `RUN_LIVE_ANTHROPIC_TESTS=1 cargo test -p forge-agent --test anthropic_live -- --ignored`
