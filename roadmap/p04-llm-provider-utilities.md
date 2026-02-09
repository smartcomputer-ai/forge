# P04: Provider Utilities

**Status**
- Done (2026-02-09)

**Goal**
Build shared utilities required by provider adapters and streaming.

**Scope**
- HTTP client wrapper with consistent timeouts and headers.
- SSE parser utility that handles `event:`, `data:`, `retry:`, comments, and multi-line data.
- Stream accumulator that builds a `Response` from `StreamEvent` sequences.
- JSON schema helpers for tool parameters and response schema translation.
- File/URL helpers for image inputs (path detection, MIME inference, base64 encoding).

**Out of Scope**
- Provider adapters themselves.
- High-level generate/stream APIs.

**Deliverables**
- `utils::http`, `utils::sse`, `utils::stream_accumulator`, `utils::schema`, `utils::file_data` modules.
- Unit tests for SSE parsing edge cases and file path detection.

**Acceptance**
- SSE parser passes fixtures for OpenAI, Anthropic, and Gemini streaming formats.
- Stream accumulator produces identical `Response` as a synthetic non-streaming result.
- File path support matches the spec image handling requirements.
