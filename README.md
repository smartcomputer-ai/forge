# Forge

Forge is a Rust workspace implementing a spec-first software factory stack centered on Attractor-style orchestration.

Upstream spec references:

- Attractor ecosystem source: https://github.com/strongdm/attractor
- Factory vision: https://factory.strongdm.ai/

## Workspace crates

- `forge-llm` (`crates/forge-llm`): unified multi-provider LLM client (`spec/01-unified-llm-spec.md`).
- `forge-agent` (`crates/forge-agent`): coding-agent loop (`spec/02-coding-agent-loop-spec.md`).
- `forge-attractor` (`crates/forge-attractor`): DOT pipeline parser/runtime (`spec/03-attractor-spec.md`).
- `forge-cli` (`crates/forge-cli`): in-process CLI host for running/resuming/inspecting Attractor pipelines.
- `forge-turnstore` (`crates/forge-turnstore`): transitional compatibility/test shim while runtime cores migrate to CXDB-first contracts.
- `forge-turnstore-cxdb` (`crates/forge-turnstore-cxdb`): transitional adapter crate for CXDB write/projection integration during migration.

## Current status

- `spec/01` and `spec/02` core layers are implemented with deterministic test coverage.
- `spec/03` Attractor runtime core, host surfaces, and conformance suites are implemented for headless and CLI-first operation.
- `spec/04` adopts CXDB-first persistence architecture; direct runtime migration and turnstore sunset are tracked in `roadmap/p34` through `roadmap/p37`.

## Build

```bash
cargo build
```

## Test

```bash
# Full workspace
cargo test

# Targeted crates
cargo test -p forge-llm
cargo test -p forge-agent
cargo test -p forge-attractor --tests
cargo test -p forge-cli --tests
cargo test -p forge-turnstore
cargo test -p forge-turnstore-cxdb
```

Optional live-provider tests remain ignored by default and require credentials.

```bash
RUN_LIVE_OPENAI_TESTS=1 cargo test -p forge-llm --test openai_live -- --ignored
RUN_LIVE_ANTHROPIC_TESTS=1 cargo test -p forge-llm --test anthropic_live -- --ignored
```

## CLI host usage (in-process)

```bash
# Run from DOT file
cargo run -p forge-cli -- run --dot-file examples/01-linear-foundation.dot --backend mock

# Resume from checkpoint
cargo run -p forge-cli -- resume --dot-file examples/01-linear-foundation.dot --checkpoint /path/to/checkpoint.json --backend mock

# Inspect checkpoint
cargo run -p forge-cli -- inspect-checkpoint --checkpoint /path/to/checkpoint.json --json
```

## Project layout

- `spec/`: source-of-truth specifications
- `roadmap/`: milestone plans and completion tracking
- `examples/`: sample DOT graphs
- `crates/forge-llm/`, `crates/forge-agent/`, `crates/forge-attractor/`, `crates/forge-cli/`, `crates/forge-turnstore/`, `crates/forge-turnstore-cxdb/`

## CXDB operations

- Binary (`:9009`) is the default write-heavy path for runtime appends and artifacts.
- HTTP (`:9010`) is the default read/projection path for paging and registry APIs.
- Production deployments should keep binary endpoints on trusted private networks with TLS/network controls and place HTTP behind authenticated gateways.

## CXDB host config

- `FORGE_CXDB_PERSISTENCE`: `off` or `required` (default: `off`)
- `FORGE_CXDB_BINARY_ADDR`: CXDB binary endpoint (default: `127.0.0.1:9009`)
- `FORGE_CXDB_HTTP_BASE_URL`: CXDB HTTP endpoint (default: `http://127.0.0.1:9010`)

## Contributing

See `CONTRIBUTING.md` and `AGENTS.md` for coding standards, test expectations, and spec-alignment requirements.
