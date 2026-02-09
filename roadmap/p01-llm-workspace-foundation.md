# P01: LLM Workspace + Crate Foundation

**Goal**
Establish the workspace layout and a standalone `forge-llm` crate as the primary implementation target for the unified LLM spec.

**Scope**
- Convert the repository to a Cargo workspace or add a workspace at the root.
- Create `forge-llm` crate with a minimal public API surface and module layout aligned to the spec layers.
- Decide crate boundaries if any supporting crates are needed (e.g., `forge-llm-core`, `forge-llm-providers`), defaulting to a single crate unless there is a clear separation of concerns.
- Add baseline dependencies for async HTTP, serde, error handling, and streaming.

**Out of Scope**
- Provider implementations.
- Actual request/response logic.

**Deliverables**
- Workspace root updated with members including `forge-llm`.
- `forge-llm` crate with modules for `types`, `errors`, `client`, `provider`, `stream`, `catalog`, and `utils`.
- Minimal `lib.rs` re-exports matching the unified spec terminology.
- CI or local build command documented in `forge-llm` README.

**Acceptance**
- `cargo build` succeeds for the workspace.
- `forge-llm` compiles with empty stubs and no warnings.
- Module layout matches the four-layer architecture in `spec/01-unified-llm-spec.md`.

**Status**
- Done (2026-02-09)
