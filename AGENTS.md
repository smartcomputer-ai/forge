# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Note: CLAUDE.md is a symlink to AGENTS.md — they are the same file.

## Build & Test Commands

```bash
cargo build                                    # Build entire workspace
cargo test                                     # Run all workspace tests
cargo test -p forge-llm                        # Test a single crate
cargo test -p forge-agent                      # Test agent crate
cargo test -p forge-attractor --tests          # Test attractor (integration tests only)
cargo test -p forge-cli --tests                # Test CLI (integration tests only)
cargo test -p forge-cxdb-runtime               # Test CXDB runtime
cargo test -p forge-llm test_name              # Run a single test by name
cargo test -p forge-llm -- --nocapture         # Run tests with stdout visible
```

Opt-in infrastructure tests (ignored by default, require local services or CLIs):
```bash
cargo test -p forge-cxdb-runtime --test live -- --ignored      # needs running CXDB server
cargo test -p cxdb --test integration -- --ignored             # needs running CXDB server
cargo test -p forge-llm --test cli_agent_e2e -- --ignored      # needs claude/codex/gemini CLIs (OAuth, no API keys)
cargo test -p forge-cli --test e2e_pipeline -- --ignored        # full-stack e2e: DOT → CLI agent → JSONL → artifacts → CXDB
```

Live provider tests (ignored by default, require API keys — costs real money):
```bash
cargo test -p forge-llm --test openai_live -- --ignored        # needs OPENAI_API_KEY
cargo test -p forge-llm --test anthropic_live -- --ignored      # needs ANTHROPIC_API_KEY
cargo test -p forge-agent --test openai_live -- --ignored       # needs OPENAI_API_KEY
cargo test -p forge-agent --test anthropic_live -- --ignored    # needs ANTHROPIC_API_KEY
cargo test -p forge-attractor --test live -- --ignored          # needs OPENAI_API_KEY
```

CLI host usage:
```bash
cargo run -p forge-cli -- run --dot-file examples/01-linear-foundation.dot --backend mock
cargo run -p forge-cli -- resume --dot-file <FILE> --checkpoint <PATH> --backend mock
cargo run -p forge-cli -- inspect-checkpoint --checkpoint <PATH> --json
```

## Architecture

Forge is a spec-first software factory stack centered on Attractor-style DOT pipeline orchestration. Crate dependency graph (bottom-up):

```
forge-cxdb            vendored CXDB client SDK (binary protocol, TLS, reconnect)
    ↓
forge-cxdb-runtime    CXDB runtime integration (typed store, client traits, testing fakes)
    ↓
forge-llm             unified multi-provider LLM client (OpenAI + Anthropic adapters)
    ↓
forge-agent           coding agent loop (session state machine, tools, provider profiles)
    ↓
forge-attractor       DOT pipeline parser → graph IR → execution engine + handlers
    ↓
forge-cli             CLI binary (clap) — run/resume/inspect Attractor pipelines
```

Key architectural patterns:
- **Trait-based adapters** — `ProviderAdapter` (stateless LLM call), `AgentProvider` (provider-owned agent loop), `NodeHandler` (pipeline nodes), `ExecutionEnvironment` (file/shell), `Interviewer` (HITL), `CxdbBinaryClient`/`CxdbHttpClient` (persistence). Shared via `Arc<dyn Trait>`.
- **Unified agent provider** — Every provider (HTTP API or CLI subprocess) implements `AgentProvider::run_to_completion()`. HTTP providers compose `ProviderAdapter` + `ToolRegistry` + `ExecutionEnvironment`. CLI providers (Claude Code, Codex, Gemini) spawn subprocess and parse JSONL. Session delegates to the provider. See `spec/06-unified-agent-provider-spec.md`.
- **Middleware chain** — LLM client composes middleware in onion model for `complete()`/`stream()`.
- **Explicit provider configuration** — Providers are explicitly configured, not auto-discovered from environment variables.
- **Session state machine** — `SessionState` enum (Idle/Processing/AwaitingInput/Closed) with explicit `can_transition_to()` validation.
- **Hierarchical errors** — Each crate defines its own `thiserror` error enums wrapping child crate errors.
- **Serialization** — JSON for external interfaces, msgpack (`rmp-serde`) for CXDB binary protocol and internal persistence.
- **Async runtime** — `tokio` with `current_thread` flavor everywhere (main and tests).
- **Rust edition 2024** for all Forge crates; vendored `cxdb` uses edition 2021.
- **Centralized status.json** — The runner writes `status.json` for ALL node types (not individual handlers). Fields: `outcome`, `preferred_next_label`, `suggested_next_ids`, `context_updates`, `notes`, `failure_reason`.
- **Model stylesheet specificity** — Selectors: `*` (universal, 0) < shape name (1) < `.class` (2) < `#node_id` (3). Nodes without an explicit `shape` attribute default to `"box"`.
- **Parallel error_policy** — Parallel handler supports `error_policy` attribute: `continue` (default), `fail_fast` (abort on first failure), `ignore` (downgrade failures to success).
- **Goal-gate enforcement** — Engine checks ALL graph nodes with `goal_gate=true` before allowing exit, including nodes never visited during execution.

## Specifications

Primary specs live in `spec/` — these are the source of truth:

- `spec/00-vision.md` — vision + principles + techniques
- `spec/01-unified-llm-spec.md` — unified LLM spec
- `spec/02-coding-agent-loop-spec.md` — coding agent loop spec
- `spec/03-attractor-spec.md` — attractor spec
- `spec/04-cxdb-integration-spec.md` — CXDB-first runtime persistence integration extension
- `spec/05-factory-control-plane-spec.md` — factory control-plane ideation (outer-loop goals and principles)
- `spec/06-unified-agent-provider-spec.md` — unified agent provider spec (provider-owned tool loops, CLI agent adapters)

When making changes, align behavior and terminology to these documents first.

## Code Structure

- Workspace root: `Cargo.toml` (workspace only)
- Crates:
  - `crates/forge-llm/` — unified LLM client library (primary target for spec/01)
  - `crates/forge-agent/` — coding agent loop library (primary target for spec/02)
  - `crates/forge-attractor/` — Attractor DOT front-end and runtime (primary target for spec/03)
  - `crates/forge-cli/` — in-process CLI host for Attractor runtime surfaces
  - `crates/forge-cxdb/` — vendored CXDB Rust client (package name: `cxdb`, not `forge-cxdb`)
  - `crates/forge-cxdb-runtime/` — CXDB runtime integration (binary/HTTP client traits, runtime store, deterministic fake)
- Persistence layering (see spec/04):
  - CXDB-first runtime contracts are the target architecture for `forge-agent` and `forge-attractor`.
  - Runtime persistence policy: `off` or `required` (no `best_effort` mode).
  - Schemas: Forge-native typed families (`forge.agent.runtime.v2`, `forge.attractor.runtime.v2`) with CXDB DAG-first lineage.
  - Legacy turnstore crates were removed; new persistence work targets `forge-cxdb-runtime` contracts.

## Environment Variables

| Variable | Purpose |
|---|---|
| `OPENAI_API_KEY` | OpenAI provider authentication |
| `ANTHROPIC_API_KEY` | Anthropic provider authentication |
| `OPENAI_BASE_URL` | Override OpenAI API endpoint |
| `ANTHROPIC_BASE_URL` | Override Anthropic API endpoint |
| `FORGE_CXDB_PERSISTENCE` | CXDB persistence mode (default: `required`; set to `off` to disable) |
| `FORGE_CXDB_BINARY_ADDR` | CXDB binary protocol address (default: `127.0.0.1:9009`) |
| `FORGE_CXDB_HTTP_BASE_URL` | CXDB HTTP base URL (default: `http://127.0.0.1:9010`) |
| `FORGE_CLAUDE_BIN` | Path to Claude Code CLI binary (default: `~/.local/bin/claude`) |
| `FORGE_CODEX_BIN` | Path to Codex CLI binary (default: `~/.local/bin/codex`) |
| `FORGE_GEMINI_BIN` | Path to Gemini CLI binary (default: `~/.local/bin/gemini`) |

## Test Strategy (Concise, Deterministic)

- Unit tests live next to code: place `mod tests` at the bottom of the same file with `#[cfg(test)]`. Keep them short, one behavior per test.
- Integration tests go under `tests/` when they cross crate boundaries, hit I/O, spawn the kernel stepper, or involve adapters.
- Naming: use `function_under_test_condition_expected()` style; structure as arrange/act/assert. Prefer explicit inputs over shared mutable fixtures.
- Errors: assert on error kinds/types (e.g., custom errors with `thiserror`) instead of string matching. Prefer `matches!`/`downcast_ref` over brittle text.
- Parallel-safe: tests run in parallel by default. Avoid global state and temp dirs without unique prefixes. Only serialize when necessary.
- Property tests (optional): add a small number of targeted property tests (e.g., canonical encoding invariants). Gate heavier fuzzing behind a feature.
- Doctests: keep crate-level examples compilable; simple examples belong in doc comments and are run with `cargo test --doc`.
- Async tests: if needed, use `#[tokio::test(flavor = "current_thread")]` to keep scheduling deterministic.

### Testing Rules (Non-Negotiable)

- **Tests must fail when the thing they test doesn't work.** A test that silently passes when its preconditions aren't met is worse than no test — it gives false confidence. If a test can't run, it must fail loudly with a clear error explaining what's missing.
- **Never use runtime env-var gating that hides failures.** Patterns like `if !enabled() { return; }` inside a test body are banned. If a test requires external resources (API keys, CLI binaries, running services), use `#[ignore]` so it doesn't run by default — but when it IS run, it must fail hard if those resources are missing.
- **`#[ignore]` is the ONLY acceptable gating mechanism.** Use `#[ignore = "reason"]` for tests that cost money (API calls) or require infrastructure that isn't always available. Never double-gate with `#[ignore]` + runtime early-return.
- **CXDB is required, not optional.** Pipeline tests that need CXDB must fail with a clear error if CXDB is unreachable. Do not silently skip persistence.
- **CLI agent binaries (claude, codex, gemini) are installed at `~/.local/bin/`.** Tests that spawn these binaries should fail with a clear error if the binary is not found, not silently skip.

## Important

When modifying specs or architecture:
1. Update the relevant spec files in `spec/`
2. Whenever a roadmap file is complete or partially complete mark what has been done. When the file is done mark the entire file as complete below the main title.
3. Update this file (AGENTS.md or CLAUDE.md) if the high-level architecture changes
4. When asked how many lines of code, use `cloc $(git ls-files)`

The specs in `spec/` are the source of truth. This file is just an index.
