# P11: Cross-Provider Tests and DoD Matrix

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
