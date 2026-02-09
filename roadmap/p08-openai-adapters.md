# P08: OpenAI Adapters (Responses + Compatible)

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
