# Forge

Forge is a Rust implementation effort of the external Attractor specification ecosystem from strongDM:

- Attractor spec source: https://github.com/strongdm/attractor
- Software factory vision: https://factory.strongdm.ai/

The goal is to be a faithful implementation of the Attractor-oriented specs, with deterministic behavior and strong conformance coverage.

## What Forge currently does

This repository currently implements two core layers:

- `forge-llm` (`crates/forge-llm`): unified multi-provider LLM client aligned to `spec/01-unified-llm-spec.md`
- `forge-agent` (`crates/forge-agent`): coding-agent loop aligned to `spec/02-coding-agent-loop-spec.md`

Today, Forge is a library workspace, not a runnable product. There is no top-level CLI or daemon entrypoint yet.

## How it works

Forge is organized around spec-first layers:

1. Specs in `spec/` define behavior and terminology.
2. `forge-llm` provides normalized request/response/tool abstractions across providers.
3. `forge-agent` runs a provider-aligned agent loop (`LLM call -> tool execution -> repeat`) with events, truncation, steering, and subagents.
4. Conformance tests validate cross-provider runtime behavior using deterministic mocked adapters.


## Current status

- `spec/01` implementation: largely complete in `forge-llm`
- `spec/02` implementation: largely complete in `forge-agent`
- `spec/03` Attractor pipeline runner (DOT DSL engine): not implemented yet in this repository

**What still needs to be done**

- Implement `spec/03-attractor-spec.md` runtime components (DOT parser/executor, node handlers, checkpoint/resume, HITL flows)
- Add a public-facing executable surface (CLI/service) once the runtime layer is in place

## Getting started

### Prerequisites

- Rust stable toolchain (`rustup` + `cargo`)

### Build

```bash
cargo build
```

### Run tests

```bash
# Workspace tests
cargo test

# Crate-specific runs
cargo test -p forge-llm
cargo test -p forge-agent
```

`forge-llm` also includes optional live-provider tests (ignored by default) that require API keys.

```bash
RUN_LIVE_OPENAI_TESTS=1 cargo test -p forge-llm --test openai_live -- --ignored
RUN_LIVE_ANTHROPIC_TESTS=1 cargo test -p forge-llm --test anthropic_live -- --ignored
```

## Project layout

- `spec/`: source-of-truth specs
- `roadmap/`: implementation milestones and DoD tracking
- `crates/forge-llm/`: unified LLM client
- `crates/forge-agent/`: coding-agent loop

## Contributing

See `CONTRIBUTING.md` and `AGENTS.md` for contribution rules and spec alignment requirements.
