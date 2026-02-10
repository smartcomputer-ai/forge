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
