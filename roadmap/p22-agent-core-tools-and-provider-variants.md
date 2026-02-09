# P22: Core Tools + Provider-Specific Editing Variants

**Status**
- Planned (2026-02-09)

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

