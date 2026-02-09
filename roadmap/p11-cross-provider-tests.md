# P11: Cross-Provider Tests and DoD Matrix

**Status**
- In progress (2026-02-09)

**Goal**
Validate parity across providers and ensure the Definition of Done checklist is executable.

**Scope**
- Unit tests for request/response translation and error mapping.
- Integration tests using mocked HTTP servers for OpenAI, Anthropic, and Gemini.
- Streaming tests for event ordering and accumulation.
- Conformance tests for tool loops and structured output.

**Out of Scope**
- Live API integration tests requiring real keys (these can be optional or gated by env vars).

**Deliverables**
- `tests/` integration suite with deterministic fixtures.
- A structured DoD checklist file or test runner that mirrors section 8 of the spec.

**Acceptance**
- All tests run in parallel safely and deterministically.
- DoD items are represented as tests or checklist entries with clear mapping.

**Completed (Partial)**
1. Added crate-level integration test suite path at `crates/forge-llm/tests/`.
2. Added mocked OpenAI integration tests at `crates/forge-llm/tests/openai_integration_mocked.rs` covering:
   - OpenAI Responses complete path through `Client`.
   - OpenAI Responses stream path through `Client`.
   - OpenAI-compatible Chat Completions complete path through `Client`.
3. Added optional live OpenAI integration tests at `crates/forge-llm/tests/openai_live.rs` with `#[ignore]` + env gating (`RUN_LIVE_OPENAI_TESTS=1`, `OPENAI_API_KEY`).
4. Expanded live OpenAI Responses coverage with:
   - reasoning token usage field assertions,
   - low `max_output_tokens` truncation -> finish reason mapping,
   - stream text-delta + terminal finish assertions (with transient retry handling),
   - invalid-model error mapping assertions (`400` + `model_not_found` + `InvalidRequest`).
5. Added additional live Responses checks for:
   - stream truncation path (`length` finish reason under low token caps),
   - required tool-choice path producing tool calls (`finish_reason = tool_calls`).
6. Added tool-usage depth tests:
   - live streaming tool-call event coverage (`ToolCallStart`/`ToolCallEnd`) with argument extraction checks,
   - mocked high-level `generate()` tool-loop integration asserting `function_call_output` round-trip to Responses API.
