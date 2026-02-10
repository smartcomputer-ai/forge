# forge-agent

`forge-agent` is the coding-agent-loop library for Forge, built on top of `forge-llm`.

This crate is the implementation target for `spec/02-coding-agent-loop-spec.md`. The initial foundation includes a public module layout for:

- session orchestration (`session`)
- lifecycle/events (`events`, `turn`, `config`)
- provider profiles (`profiles`)
- tool registry and dispatch primitives (`tools`)
- execution environment abstraction (`execution`)
- truncation/context safeguards (`truncation`)

## Build

```bash
cargo build -p forge-agent
```

## Tests

Run deterministic unit/integration tests:

```bash
cargo test -p forge-agent
```

Run the CXDB persistence integration suite only:

```bash
cargo test -p forge-agent --test cxdb_persistence_integration
```

Run default-ignored live smoke tests (OpenAI):

```bash
RUN_LIVE_OPENAI_TESTS=1 cargo test -p forge-agent --test openai_live -- --ignored
```

Live OpenAI tests require `OPENAI_API_KEY` (read from env or project-root `.env`).
Optional overrides: `OPENAI_LIVE_MODEL`, `OPENAI_BASE_URL`, `OPENAI_ORG_ID`, `OPENAI_PROJECT_ID`.

Run default-ignored live smoke tests (Anthropic):

```bash
RUN_LIVE_ANTHROPIC_TESTS=1 cargo test -p forge-agent --test anthropic_live -- --ignored
```

Live Anthropic tests require `ANTHROPIC_API_KEY` (read from env or project-root `.env`).
Optional overrides: `ANTHROPIC_LIVE_MODEL`, `ANTHROPIC_BASE_URL`.

## Persistence In Tests

`SessionConfig.cxdb_persistence` controls persistence behavior:

- `off`: no CXDB calls; useful for baseline behavior tests.
- `required`: write failures are terminal; use for strict persistence assertions.

Integration tests can wire a CXDB backend with `Session::new_with_cxdb_persistence(...)`:
- deterministic in-process fake backend (`forge_cxdb_runtime::MockCxdb`)
- live CXDB endpoints (binary + HTTP) when environment is configured

The dedicated suite `tests/cxdb_persistence_integration.rs` demonstrates:

- enabling persistence with `required` mode for CXDB-backed runs
- querying persisted turns from the configured CXDB backend
- disabling persistence with `off` mode


## `forge-agent` orchestration APIs

`forge-agent` now exposes a few APIs intended for higher-level runtimes (like an Attractor codergen backend):

- Per-submit request overrides (`submit_with_options`) for provider/model/reasoning/system prompt changes.
- Structured submit result (`submit_with_result`) to avoid replaying full history for outcome mapping.
- Session checkpoint/restore (`checkpoint` / `from_checkpoint`) plus thread-key continuity metadata.

Example (simplified):

```rust
use forge_agent::{Session, SubmitOptions};

// 1) Per-node submit overrides
session.submit_with_options(
    "Plan next change",
    SubmitOptions {
        provider: Some("openai".to_string()),
        model: Some("gpt-5.2-codex".to_string()),
        reasoning_effort: Some("high".to_string()),
        system_prompt_override: Some("Stage: plan".to_string()),
        ..Default::default()
    },
).await?;

// 2) Structured result for backend mapping
let result = session.submit_with_result(
    "Implement the plan",
    SubmitOptions::default(),
).await?;
// result.assistant_text
// result.tool_call_ids
// result.tool_error_count
// result.usage
// result.thread_key

// 3) Checkpoint and restore
let snapshot = session.checkpoint()?;
let restored = Session::from_checkpoint(
    snapshot,
    provider_profile,
    execution_env,
    llm_client,
    event_emitter,
)?;
```
