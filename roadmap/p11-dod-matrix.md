# P11 DoD Matrix (Spec Section 8)

**Status**
- In progress (2026-02-09)
- Gemini rows are deferred because `roadmap/p10-gemini-adapter.md` is currently skipped.

## 8.1 Provider Adapter Coverage
- [x] OpenAI native Responses API adapter exists: `crates/forge-llm/src/openai.rs`
- [x] Anthropic native Messages API adapter exists: `crates/forge-llm/src/anthropic.rs`
- [ ] Gemini native adapter exists: deferred (p10 skipped)

## 8.2 Translation Correctness
- [x] OpenAI request/response translation (complete + stream): `crates/forge-llm/tests/openai_integration_mocked.rs`
- [x] Anthropic request/response translation (complete + stream): `crates/forge-llm/tests/anthropic_integration_mocked.rs`
- [x] Anthropic alternation + tool_result-in-user translation: `crates/forge-llm/tests/anthropic_integration_mocked.rs`
- [ ] Gemini translation tests: deferred (p10 skipped)

## 8.3 Streaming Contract
- [x] OpenAI stream event coverage (delta + finish): `crates/forge-llm/tests/openai_integration_mocked.rs`
- [x] Anthropic stream event coverage (reasoning/tool + finish): `crates/forge-llm/tests/anthropic_integration_mocked.rs`
- [x] Cross-provider event ordering invariant: `crates/forge-llm/tests/cross_provider_conformance.rs`
- [ ] Gemini stream contract: deferred (p10 skipped)

## 8.4 Tool Calling / Tool Loop
- [x] High-level tool loop round trip (OpenAI mocked integration): `crates/forge-llm/tests/openai_integration_mocked.rs`
- [x] Cross-provider tool loop conformance (OpenAI + Anthropic): `crates/forge-llm/tests/cross_provider_conformance.rs`
- [x] Anthropic tool_result request placement: `crates/forge-llm/tests/anthropic_integration_mocked.rs`

## 8.5 Reasoning / Usage / Finish Reasons
- [x] OpenAI live reasoning token assertions: `crates/forge-llm/tests/openai_live.rs`
- [x] Anthropic live reasoning token assertions: `crates/forge-llm/tests/anthropic_live.rs`
- [x] Length/tool_calls finish-reason mapping (OpenAI + Anthropic live): `crates/forge-llm/tests/openai_live.rs`, `crates/forge-llm/tests/anthropic_live.rs`

## 8.6 Prompt Caching / Provider Options
- [x] Anthropic prompt caching injection + beta header behavior: `crates/forge-llm/src/anthropic.rs`, `crates/forge-llm/src/anthropic.rs` tests
- [x] Anthropic provider option passthrough in integration: `crates/forge-llm/tests/anthropic_integration_mocked.rs`

## Structured Output Conformance
- [x] Cross-provider `generate_object()` schema conformance (OpenAI + Anthropic): `crates/forge-llm/tests/cross_provider_conformance.rs`
