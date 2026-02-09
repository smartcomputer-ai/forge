# P14: Agent Workspace + Crate Foundation
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Create a new `forge-agent` crate in the workspace as the dedicated implementation target for `spec/02-coding-agent-loop-spec.md`.

**Scope**
- Add `crates/forge-agent` and register it in workspace `Cargo.toml`.
- Establish module layout for session loop, profiles, tools, execution environments, events, and truncation.
- Wire dependency on `forge-llm` low-level client APIs (`Client.complete()` / `Client.stream()` support).
- Add baseline dependencies (async runtime, serde, schema validation, process management, time handling).

**Out of Scope**
- Full session loop behavior.
- Provider-specific profile implementations.
- Subagents and full integration tests.

**Deliverables**
- New crate: `crates/forge-agent`.
- Public API skeleton aligned with spec terminology (`Session`, `SessionConfig`, `ProviderProfile`, `ExecutionEnvironment`, `ToolRegistry`, events).
- Buildable crate with stubs and compile-safe placeholders.
- README for crate intent and module map.

**Acceptance**
- `cargo build` succeeds for workspace with `forge-agent` included.
- `forge-agent` exports compile with no dead-end type holes.
- Module naming aligns to Sections 2-7 in `spec/02-coding-agent-loop-spec.md`.

**Implemented**
- Added new workspace member: `crates/forge-agent`.
- Added crate manifest and baseline dependencies including `forge-llm`.
- Added compile-safe module skeleton:
  - `config`, `errors`, `events`, `execution`, `profiles`, `session`, `tools`, `truncation`, `turn`
- Added public re-export surface in `crates/forge-agent/src/lib.rs`.
- Added crate README with module intent and build command.
- Updated `AGENTS.md` architecture index to include `forge-agent`.

**Validation**
- `cargo build` (workspace) passed.
- `cargo test -p forge-agent` passed.
