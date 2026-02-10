# P37 DoD Matrix (P33-P37 CXDB-First Migration Closure)

Status date: 2026-02-10

Legend:
- `[x]` complete and covered
- `[ ]` gap/deviation tracked

## Architecture
- [x] Runtime cores (`forge-agent`, `forge-attractor`) use CXDB-first persistence contracts without `forge-turnstore` runtime dependencies. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/storage/mod.rs`, `crates/forge-cxdb-runtime/src/runtime.rs`
- [x] Turnstore crates are removed from workspace membership and source tree. Refs: `Cargo.toml`, `AGENTS.md`
- [x] `forge-llm` remains CXDB-independent. Refs: `crates/forge-llm/Cargo.toml`, `spec/04-cxdb-integration-spec.md`

## Runtime Write Path
- [x] Agent write path persists turns through CXDB runtime store with `off`/`required` policy behavior. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-agent/tests/cxdb_parity.rs`, `crates/forge-agent/tests/cxdb_persistence_integration.rs`
- [x] Attractor write path persists run/stage/checkpoint/link records through CXDB runtime contracts. Refs: `crates/forge-attractor/src/storage/mod.rs`, `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/tests/cxdb_parity.rs`
- [x] Deterministic idempotency and parent-resolution behavior is covered in CXDB runtime tests. Refs: `crates/forge-cxdb-runtime/src/testing.rs`, `crates/forge-cxdb-runtime/tests/live.rs`

## FS Lineage
- [x] CXDB runtime boundary exposes fs lineage primitives (`put_blob`, `attach_fs`) and related validation behavior. Refs: `crates/forge-cxdb-runtime/src/adapter.rs`, `crates/forge-cxdb-runtime/src/runtime.rs`
- [x] Vendored CXDB client includes `fstree` capture/upload and fs attachment flow for lineage operations. Refs: `crates/forge-cxdb/src/fstree/upload.rs`, `crates/forge-cxdb/src/fs.rs`, `crates/forge-cxdb/tests/fstree_integration.rs`
- [x] Operational runbooks include fs snapshot/attachment failure handling. Refs: `README.md`, `crates/forge-cxdb/README.md`, `spec/04-cxdb-integration-spec.md`

## Typed Projection and Query Surface
- [x] Runtime envelopes are deterministic msgpack payloads with stable tag mappings and typed IDs/versions. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/storage/types.rs`, `spec/04-cxdb-integration-spec.md`
- [x] Registry bundle publish/read paths are integrated in runtime/query flows. Refs: `crates/forge-agent/src/session.rs`, `crates/forge-attractor/src/runner.rs`, `crates/forge-cxdb-runtime/src/runtime.rs`
- [x] Host query paths use typed projection APIs with deterministic paging semantics. Refs: `crates/forge-attractor/src/queries.rs`, `crates/forge-cli/src/main.rs`, `roadmap/p36-cxdb-typed-projection-and-query-surface-refactor.md`

## Operations and Hardening
- [x] Endpoint topology and trust-boundary guidance is documented for binary and HTTP CXDB planes. Refs: `README.md`, `crates/forge-cxdb/README.md`, `spec/04-cxdb-integration-spec.md`
- [x] Append/projection/registry/fs incident workflows are documented with deterministic troubleshooting steps. Refs: `README.md`, `spec/04-cxdb-integration-spec.md`
- [x] Deferred roadmap wave (`p80`-`p84`) is rebaselined on post-migration CXDB-first architecture. Refs: `roadmap/later/p80-attractor-stage-outcome-contract-and-status-ingestion.md`, `roadmap/later/p81-attractor-true-parallel-and-fan-in-semantics.md`, `roadmap/later/p82-attractor-runtime-control-plane-and-resume-hardening.md`, `roadmap/later/p83-attractor-attribute-policy-completion-and-contract-tightening.md`, `roadmap/later/p84-attractor-host-timeline-and-query-drilldown-surfaces.md`

## Verification Gates
- [x] Deterministic suites green:
  - `cargo test -p forge-agent`
  - `cargo test -p forge-attractor`
  - `cargo test -p forge-cxdb-runtime`
- [x] Live smoke entrypoints green (ignored-by-default command paths executed; env-gated tests no-op cleanly when credentials/services are absent):
  - `cargo test -p forge-agent --test openai_live -- --ignored`
  - `cargo test -p forge-agent --test anthropic_live -- --ignored`
  - `cargo test -p forge-attractor --test live -- --ignored`
