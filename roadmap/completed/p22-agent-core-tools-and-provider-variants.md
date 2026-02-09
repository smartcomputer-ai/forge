# P22: Core Tools + Provider-Specific Editing Variants
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Implement the built-in toolset and provider-aligned editing semantics.

**Scope**
- Implement shared tools: `read_file`, `write_file`, `shell`, `grep`, `glob`.
- Implement Anthropic/Gemini-aligned `edit_file` (`old_string`/`new_string`, uniqueness checks).
- Implement OpenAI-aligned `apply_patch` v4a parser/executor.
- Register profile-specific tool lists with correct substitutions (`apply_patch` replacing `edit_file` for OpenAI profile).

**Out of Scope**
- Subagent tools (`spawn_agent`, `send_input`, `wait`, `close_agent`) until P23.

**Deliverables**
- Tool executor implementations backed by `ExecutionEnvironment`.
- `apply_patch` grammar validation and robust hunk application behavior.
- Tests for edit ambiguity errors, patch parse failures, and successful multi-file updates.

**Acceptance**
- OpenAI profile exposes `apply_patch` and not `edit_file` for modifications.
- Anthropic profile uses `edit_file` with exact-match behavior and clear conflict errors.
- Tool outputs feed truncation pipeline before `ToolResult` return.

**Implemented**
- Added built-in provider-aware tool registry builders in `crates/forge-agent/src/tools.rs`:
  - `build_openai_tool_registry()`
  - `build_anthropic_tool_registry()`
  - `build_gemini_tool_registry()`
  - shared registration helper for `read_file`, `write_file`, `shell`, `grep`, `glob`
- Implemented shared tool executors in `crates/forge-agent/src/tools.rs` backed by `ExecutionEnvironment`:
  - `read_file` (line-numbered output with `offset`/`limit`)
  - `write_file`
  - `shell` (structured exit/stdout/stderr output)
  - `grep`
  - `glob`
- Implemented Anthropic/Gemini-style `edit_file` semantics:
  - exact `old_string` matching
  - `replace_all` support
  - clear ambiguity error when `old_string` is non-unique and `replace_all` is false
- Implemented OpenAI `apply_patch` (v4a-style) parsing and execution:
  - `*** Begin Patch` / `*** End Patch` envelope validation
  - `Add File`, `Delete File`, `Update File`, optional `Move to`
  - hunk parsing with `@@` headers and line prefixes (` `, `-`, `+`)
  - multi-operation, multi-file patch application with operation summary output
- Extended `ExecutionEnvironment` in `crates/forge-agent/src/execution.rs` with:
  - `delete_file(path)`
  - `move_file(from, to)`
  - local implementation support in `LocalExecutionEnvironment`
- Added default-profile convenience constructors using the new registries in `crates/forge-agent/src/profiles.rs`:
  - `OpenAiProviderProfile::with_default_tools(...)`
  - `AnthropicProviderProfile::with_default_tools(...)`
  - `GeminiProviderProfile::with_default_tools(...)`

**Tests**
- Added provider tool-variant coverage:
  - OpenAI includes `apply_patch` and excludes `edit_file`
  - Anthropic/Gemini include `edit_file` and exclude `apply_patch`
- Added `edit_file` ambiguity behavior test.
- Added `apply_patch` parse-failure test.
- Added successful multi-file `apply_patch` test covering add/update/rename/delete.

**Validation**
- `cargo test -p forge-agent` passed.
