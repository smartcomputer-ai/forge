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
- `forge-cxdb-runtime` (`crates/forge-cxdb-runtime`): CXDB runtime and host integration contracts (binary/HTTP clients, runtime store, deterministic fake).

## Current status

- `spec/01` and `spec/02` core layers are implemented with deterministic test coverage.
- `spec/03` Attractor runtime core, host surfaces, and conformance suites are implemented for headless and CLI-first operation.
- `spec/04` adopts CXDB-first persistence architecture

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
cargo test -p forge-cxdb-runtime
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
- `crates/forge-llm/`, `crates/forge-agent/`, `crates/forge-attractor/`, `crates/forge-cli/`, `crates/forge-cxdb-runtime/`, `crates/forge-cxdb/`

## CXDB operations

- Binary (`:9009`) is the default write-heavy path for runtime appends and artifacts.
- HTTP (`:9010`) is the default typed read/projection path for turn listing/paging and registry APIs.
- Forge runtime typed records are persisted as msgpack with stable numeric tags and published registry bundles (`forge.agent.runtime.v2`, `forge.attractor.runtime.v2`).
- Production deployments should keep binary endpoints on trusted private networks with TLS/network controls and place HTTP behind authenticated gateways.

## CXDB field ownership and trace model

- CXDB-native lineage fields (`context_id`, `turn_id`, `parent_turn_id`, `depth`, `type_id`, `type_version`, append/content-hash metadata) are the source of truth for causality.
- Forge payload fields carry domain semantics (`run_id`, `node_id`, `stage_attempt_id`, `attempt`, routing/human-gate outcomes, tool/session lifecycle details).
- Cross-context joins are explicit typed link facts (for example `forge.link.stage_to_agent`), not mirrored CXDB lineage fields.
- Attractor traces should be read as run/stage/parallel/interview/checkpoint lifecycle records on the run context spine.
- Agent traces should be read as transcript turn families plus separate operational lifecycle records (`forge.agent.session_lifecycle`, `forge.agent.tool_call_lifecycle`).

## CXDB trust boundaries

- Binary (`FORGE_CXDB_BINARY_ADDR`) is a trusted runtime plane; expose only on private service networks.
- HTTP (`FORGE_CXDB_HTTP_BASE_URL`) is a host/query plane; expose through authenticated gateways and policy controls.
- Use TLS for both planes in production, or isolate them on encrypted private overlays with strict ACLs.
- Treat registry write permissions as privileged; bundle publication controls schema compatibility for typed readers.

## CXDB host config

- `FORGE_CXDB_PERSISTENCE`: `off` or `required` (default: `off`)
- `FORGE_CXDB_BINARY_ADDR`: CXDB binary endpoint (default: `127.0.0.1:9009`)
- `FORGE_CXDB_HTTP_BASE_URL`: CXDB HTTP endpoint (default: `http://127.0.0.1:9010`)

## CXDB incident workflows

- Append path failures (`create_context`/`append_turn`/`get_head`):
  1. Validate binary endpoint reachability/TLS between runtime host and CXDB.
  2. Confirm `FORGE_CXDB_PERSISTENCE=required` is intentional for the run mode.
  3. Check idempotency key determinism and parent turn resolution on retries.
  4. Verify context lifecycle ordering (`create`/`fork` before append).
- Projection path failures (`list_turns`/paging):
  1. Validate HTTP endpoint reachability and auth policy.
  2. Confirm cursor inputs (`before_turn_id`, `limit`) and context id conversion.
  3. Compare projection lag against binary write success to separate ingest vs query issues.
- Registry mismatch (`publish/get bundle`, decode drift):
  1. Confirm target `bundle_id` exists and is accessible via HTTP registry APIs.
  2. Verify writer `type_id`/`type_version` pairs match published bundle schemas.
  3. Re-publish bundle before first writes of new schema versions.
- FS snapshot or attachment failures (`PUT_BLOB`/`ATTACH_FS`):
  1. Validate snapshot policy limits (`max_files`, `max_file_size`, exclude patterns).
  2. Confirm blob upload success before attachment.
  3. Verify referenced `fs_root_hash` exists and matches appended lineage metadata.

## Contributing

See `CONTRIBUTING.md` and `AGENTS.md` for coding standards, test expectations, and spec-alignment requirements.
