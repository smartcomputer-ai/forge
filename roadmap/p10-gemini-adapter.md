# P10: Gemini Adapter (GenerateContent API)

**Goal**
Implement the Gemini adapter using the native GenerateContent API with streaming support.

**Scope**
- Request/response translation per spec section 7.3.
- System instruction mapping and developer role merging.
- Tool call ID synthesis and functionResponse mapping.
- Streaming translation for SSE and JSON chunk formats.
- Prompt caching usage mapping and thinking token mapping.

**Out of Scope**
- OpenAI and Anthropic adapters.

**Deliverables**
- `GeminiAdapter` implementation with streaming translation.
- Unit tests covering tool call ID synthesis and streaming finish behavior.

**Acceptance**
- Function calls use synthetic IDs and map back correctly for tool results.
- `usageMetadata.thoughtsTokenCount` maps to `reasoning_tokens`.
- Streaming emits start/delta/end events per spec.
