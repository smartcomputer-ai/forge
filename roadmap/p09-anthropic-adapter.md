# P09: Anthropic Adapter (Messages API)

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
