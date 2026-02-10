# P39 DoD Matrix (CXDB-Native Turn Model and Runtime Semantics Rebase)

Status date: 2026-02-10

Legend:
- `[x]` complete and covered
- `[ ]` gap/deviation tracked

## Data Model
- [ ] Generic runtime envelope (`event_kind` + `payload_json`) is removed from active write paths. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/storage/mod.rs`
- [ ] Agent runtime events are represented as first-class typed records, not generic envelope events. Refs: `crates/forge-agent/src/session.rs`
- [ ] Attractor run/stage/checkpoint records are represented as first-class typed records with explicit schemas. Refs: `crates/forge-attractor/src/storage/types.rs`, `crates/forge-attractor/src/runner.rs`

## CXDB Primitive Alignment
- [ ] CXDB `parent_turn_id` is used as primary causal linkage in runtime append paths. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/storage/mod.rs`
- [ ] Redundant correlation fields that duplicate CXDB turn metadata are removed or minimized. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/storage/mod.rs`
- [ ] Only domain-specific correlation fields (cross-context/runtime) remain in typed payloads. Refs: `crates/forge-attractor/src/storage/types.rs`, `crates/forge-attractor/src/backends/forge_agent.rs`

## Registry and Projection
- [ ] New runtime registry bundles reflect concrete typed schemas (clean break). Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/runner.rs`
- [ ] Query paths consume typed projection fields directly without nested payload JSON reparse. Refs: `crates/forge-attractor/src/queries.rs`, `crates/forge-cxdb-runtime/src/adapter.rs`
- [ ] Semantic hints are applied where useful (`unix_ms`, durations, etc.) for projection clarity. Refs: `spec/04-cxdb-integration-spec.md`, `crates/forge-cxdb/docs/type-registry.md`

## Runtime Behavior
- [ ] Agent runtime preserves expected lifecycle/tool semantics with the new typed model. Refs: `crates/forge-agent/tests/*`
- [ ] Attractor runtime preserves run/stage/checkpoint/link semantics with the new typed model. Refs: `crates/forge-attractor/tests/*`
- [ ] FS lineage integration remains intact with new schemas. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/runner.rs`, `crates/forge-cxdb-runtime/src/runtime.rs`

## Spec and Docs
- [ ] `spec/04-cxdb-integration-spec.md` describes CXDB-native runtime modeling and no envelope-over-turn dependency. Refs: `spec/04-cxdb-integration-spec.md`
- [ ] Repository docs explain CXDB-native vs Forge-domain field ownership. Refs: `README.md`, `crates/forge-agent/README.md`, `crates/forge-attractor/README.md`
- [ ] AGENTS architecture index remains consistent with implemented model. Refs: `AGENTS.md`

## Verification Gates
- [ ] Deterministic suites green:
  - `cargo test -p forge-agent`
  - `cargo test -p forge-attractor`
  - `cargo test -p forge-cxdb-runtime`
- [ ] Live suite entrypoints remain healthy (env-gated):
  - `RUN_LIVE_OPENAI_TESTS=1 cargo test -p forge-llm --test openai_live -- --ignored`
  - `RUN_LIVE_ANTHROPIC_TESTS=1 cargo test -p forge-llm --test anthropic_live -- --ignored`
  - `RUN_LIVE_OPENAI_TESTS=1 cargo test -p forge-agent --test openai_live -- --ignored`
  - `RUN_LIVE_ANTHROPIC_TESTS=1 cargo test -p forge-agent --test anthropic_live -- --ignored`
