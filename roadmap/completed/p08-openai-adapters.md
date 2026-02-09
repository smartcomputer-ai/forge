# P08: OpenAI Adapters (Responses + Compatible)
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Implement the OpenAI provider adapter using the Responses API and a separate OpenAI-compatible adapter for third-party endpoints.

**Scope**
- OpenAI Responses API request/response translation per spec section 7.3.
- Streaming translation for Responses API SSE events.
- Reasoning effort mapping and reasoning token accounting.
- Tool calling translation and tool result formatting.
- Image input handling via data URI or URL.
- OpenAI-compatible adapter using Chat Completions for third-party providers.

**Out of Scope**
- Anthropic and Gemini adapters.

**Deliverables**
- `OpenAIAdapter` and `OpenAICompatibleAdapter` implementations.
- Unit tests with mocked HTTP responses for non-streaming and streaming flows.

**Acceptance**
- Uses `/v1/responses` for OpenAI and `/v1/chat/completions` for compatible endpoints.
- Reasoning tokens are populated from `usage.output_tokens_details.reasoning_tokens` when present.
- Tool calls and tool results round-trip correctly.

**Completed**
1. Added `OpenAIAdapter` with native Responses API request/response translation at `crates/forge-llm/src/openai.rs`.
2. Added `OpenAICompatibleAdapter` for Chat Completions compatible endpoints at `crates/forge-llm/src/openai.rs`.
3. Implemented Responses API SSE streaming translation (text deltas, tool-call deltas, finish event with final response/usage).
4. Implemented reasoning-effort request mapping and reasoning-token usage mapping.
5. Implemented tool-call and tool-result translation across request/response paths.
6. Added image input translation (URL/data URI/local path handling) for Responses API requests.
7. Added env-based OpenAI provider factory registration into `Client::from_env()`.
8. Added mocked HTTP tests for complete and stream paths asserting endpoint usage and key translation behavior.
