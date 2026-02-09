# P09: Anthropic Adapter (Messages API)
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Implement the Anthropic adapter with strict alternation, thinking blocks, and prompt caching support.

**Scope**
- Messages API request/response translation per spec section 7.3.
- Merge consecutive same-role messages to satisfy alternation constraints.
- Thinking and redacted thinking block round-tripping.
- `max_tokens` defaulting to 4096 when unset.
- `anthropic-beta` header handling via `provider_options`.
- Prompt caching via `cache_control` injection and opt-out flag.

**Out of Scope**
- OpenAI and Gemini adapters.

**Deliverables**
- `AnthropicAdapter` implementation with streaming translation.
- Unit tests with mocked responses for thinking blocks and tool use.

**Acceptance**
- Tool results are placed in user-role `tool_result` content blocks.
- Cache control injection and beta header behavior match section 2.10 and 8.6.
- Strict alternation enforced without losing message content.

**Completed**
1. Added `AnthropicAdapter` with native Messages API request/response translation at `crates/forge-llm/src/anthropic.rs`.
2. Implemented strict alternation by merging consecutive same-role translated messages without dropping content.
3. Implemented thinking + redacted thinking block round-trip translation for request history and assistant responses.
4. Implemented tool translation with tool results emitted as `tool_result` blocks in user-role Anthropic messages.
5. Implemented `max_tokens` defaulting to `4096` when unset.
6. Implemented `provider_options.anthropic.beta_headers`/`beta_features` support via the `anthropic-beta` request header.
7. Implemented automatic prompt caching breakpoint injection (`cache_control`) with opt-out via `provider_options.anthropic.auto_cache = false`.
8. Added streaming SSE translation for text/tool-use/thinking events and finish response assembly.
9. Added adapter unit tests with mocked responses for thinking blocks, tool use, beta headers, cache-control injection, and alternation behavior.
