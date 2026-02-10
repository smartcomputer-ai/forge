# forge-attractor

`forge-attractor` is the DOT-based pipeline runtime for Forge, implementing the Attractor orchestration layer from `spec/03-attractor-spec.md`.

## Architecture

`forge-attractor` is split into these layers:

- Front-end: DOT parsing, graph IR normalization, lint/validation, stylesheet parsing and transforms.
- Runtime engine: deterministic traversal, handler dispatch, routing, retries, goal-gates, checkpointing, and resume.
- Host surfaces: runtime event stream, interviewer abstractions, storage-backed query APIs.
- Backend adapters: codergen backend integration (including `forge-agent` adapter and tool-hook bridge support).

Related crates:

- `forge-agent`: coding-agent loop used by codergen backend integration.
- `forge-turnstore`: storage abstraction used for run/event persistence and query parity.
- `forge-cli`: in-process host surface for run/resume/inspect workflows.

## Key capabilities

- Parse/validate Attractor DOT pipelines and execute deterministically.
- Support handlers: `start`, `exit`, `codergen`, `wait.human`, `conditional`, `parallel`, `parallel.fan_in`, `tool`, `stack.manager_loop`.
- Emit typed runtime events for pipeline/stage/parallel/interview/checkpoint lifecycles.
- Persist run/stage/checkpoint/linkage events via turnstore-backed storage.
- Query run metadata, stage timelines, checkpoint snapshots, and stage-to-agent linkage records.

## Run tests

```bash
# All attractor tests
cargo test -p forge-attractor --tests

# Conformance suites added in P31
cargo test -p forge-attractor --test conformance_parsing
cargo test -p forge-attractor --test conformance_runtime
cargo test -p forge-attractor --test conformance_state
cargo test -p forge-attractor --test conformance_stylesheet
cargo test -p forge-attractor --test integration_smoke
```

## Known gaps

- HTTP server mode (`spec/03` section 9.5 and 11.11 optional HTTP endpoints) is intentionally deferred.
- Live provider smoke tests are tracked separately as optional/env-gated coverage under P31 G5.

## Roadmap references

- P30 host surfaces and observability: `roadmap/p30-attractor-observability-hitl-and-storage-abstractions.md`
- P31 conformance/docs closure: `roadmap/p31-attractor-conformance-tests-docs-and-dod-matrix.md`
- P31 DoD matrix tracker: `roadmap/p31-dod-matrix.md`
