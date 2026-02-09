# P02: Core Data Model

**Goal**
Implement the unified data model types that form the stable contract across providers.

**Scope**
- `Message`, `Role`, `ContentPart`, `ContentKind`.
- Content payload structs: `ImageData`, `AudioData`, `DocumentData`, `ToolCallData`, `ToolResultData`, `ThinkingData`.
- `Request`, `Response`, `FinishReason`, `Usage`, `Warning`, `RateLimitInfo`.
- `StreamEvent` and `StreamEventType`.
- Convenience constructors and accessors (`Message.system`, `Message.user`, `message.text`, `response.text`, etc.).
- Serde support for public types where appropriate.

**Out of Scope**
- Provider translation logic.
- Streaming transport parsing.

**Deliverables**
- Rust types aligned to `spec/01-unified-llm-spec.md` sections 3.1–3.14.
- Unit tests for constructors, text accessors, and `Usage` addition semantics.

**Acceptance**
- All types compile and are re-exported from `forge-llm`.
- Tests verify `Usage` addition rules and text concatenation rules.
- Types are documented with spec language and role/content constraints.
