# P12: Docs and Examples
_Complete_

**Status**
- Done (2026-02-09)
- Re-scoped: prioritize README/API explanations; defer standalone `examples/` files unless later needed.

**Goal**
Provide concise, accurate documentation and usage examples that reflect the unified spec.

**Scope**
- `forge-llm` crate README with quickstart and environment setup.
- Clear API behavior notes for `generate`, `stream`, tool calling, structured output, and provider options.
- Documentation for provider options and escape hatches.

**Out of Scope**
- Marketing or product docs beyond the crate.
- Creating and maintaining standalone `forge-llm/examples/*` files for now.

**Deliverables**
- Expanded README that documents usage patterns and provider semantics.
- Inline docs for key types and functions.

**Acceptance**
- README accurately describes behavior and points to test files as executable references.
- Docs match the terminology and semantics in `spec/01-unified-llm-spec.md`.
