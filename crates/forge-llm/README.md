# forge-llm

Unified LLM client library for Forge. This crate implements the multi-provider spec in `spec/01-unified-llm-spec.md`.

## Build

```
cargo build -p forge-llm
```

## Tests

Run the crate test suite:

```
cargo test -p forge-llm
```

Run live OpenAI integration tests (ignored by default):

```
RUN_LIVE_OPENAI_TESTS=1 cargo test -p forge-llm --test openai_live -- --ignored
```

Live tests require `OPENAI_API_KEY` (read from environment or from project-root `.env`).
Optional live-test settings:

- `OPENAI_LIVE_MODEL` (default: `gpt-5-mini`)
- `OPENAI_BASE_URL`
- `OPENAI_ORG_ID`
- `OPENAI_PROJECT_ID`
