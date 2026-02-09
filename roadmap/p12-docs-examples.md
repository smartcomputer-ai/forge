# P12: Docs and Examples

**Goal**
Provide concise, accurate documentation and usage examples that reflect the unified spec.

**Scope**
- `forge-llm` crate README with quickstart and environment setup.
- Examples for `generate`, `stream`, tool calling, and structured output.
- Documentation for provider options and escape hatches.

**Out of Scope**
- Marketing or product docs beyond the crate.

**Deliverables**
- README and example files under `forge-llm/examples`.
- Inline docs for key types and functions.

**Acceptance**
- Examples compile and run against mocked providers or are gated by env vars.
- Docs match the terminology and semantics in `spec/01-unified-llm-spec.md`.
