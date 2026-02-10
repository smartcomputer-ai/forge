# AGENTS

## Specifications
Primary specs live in `spec/`:

- `spec/00-vision.md` — vision + principles + techniques
- `spec/01-unified-llm-spec.md` — unified LLM spec
- `spec/02-coding-agent-loop-spec.md` — coding agent loop spec
- `spec/03-attractor-spec.md` — attractor spec
- `spec/04-cxdb-integration-spec.md` — CXDB-first runtime persistence integration extension

When making changes, align behavior and terminology to these documents first.

## Code Structure
- Workspace root: `Cargo.toml` (workspace only)
- Crates:
  - `crates/forge-llm/` — unified LLM client library (primary target for spec/01-unified-llm-spec.md)
  - `crates/forge-agent/` — coding agent loop library (primary target for spec/02-coding-agent-loop-spec.md)
  - `crates/forge-attractor/` — Attractor DOT front-end and runtime target (primary target for spec/03-attractor-spec.md)
  - `crates/forge-cli/` — in-process CLI host for Attractor runtime surfaces (primary target for roadmap P30 host milestones)
  - `crates/forge-cxdb/` — vendored CXDB Rust client (binary protocol, fs helpers, reconnecting client)
  - `crates/forge-cxdb-runtime/` — CXDB runtime integration crate (binary/HTTP client traits, runtime store, deterministic fake)
- Transitioning persistence layering (see roadmap/spec 04, p33-p37):
  - CXDB-first runtime contracts are the target architecture for `forge-agent` and `forge-attractor`.
  - Runtime persistence policy is a CXDB enablement toggle: `off` or `required` (no `best_effort` mode).
  - `crates/forge-turnstore/` is transitional and may be removed or retained only as a compatibility/test shim after the migration completes.
  - `crates/forge-turnstore-cxdb/` is now a compatibility shim that re-exports `forge-cxdb-runtime`.

## Test Strategy (Concise, Deterministic)

- Unit tests live next to code: place `mod tests` at the bottom of the same file with `#[cfg(test)]`. Keep them short, one behavior per test.
- Integration tests go under `tests/` when they cross crate boundaries, hit I/O, spawn the kernel stepper, or involve adapters.
- Naming: use `function_under_test_condition_expected()` style; structure as arrange/act/assert. Prefer explicit inputs over shared mutable fixtures.
- Errors: assert on error kinds/types (e.g., custom errors with `thiserror`) instead of string matching. Prefer `matches!`/`downcast_ref` over brittle text.
- Parallel-safe: tests run in parallel by default. Avoid global state and temp dirs without unique prefixes. Only serialize when necessary.
- Property tests (optional): add a small number of targeted property tests (e.g., canonical encoding invariants). Gate heavier fuzzing behind a feature.
- Doctests: keep crate-level examples compilable; simple examples belong in doc comments and are run with `cargo test --doc`.
- Async tests: if needed, use `#[tokio::test(flavor = "current_thread")]` to keep scheduling deterministic.

## Important
When modifying specs or architecture:
1. Update the relevant spec files in `spec/`
2. Whenever a roadmap file is complete or partially complete mark what has been done. When the file is done mark the entire file as compelte below the main title.
3. Update this file (AGENTS.md or CLAUDE.md) if the high-level architecture changes
4. Note: CLAUDE.md is a symlink to AGENTS.md - they are the same file
5. When asked how many lines of code, use `cloc $(git ls-files)`

The specs in `spec/` are the source of truth. This file is just an index.
