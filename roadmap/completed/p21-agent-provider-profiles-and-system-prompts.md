# P21: Provider Profiles + Layered System Prompt Construction
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Implement provider-aligned profiles and layered prompt construction for OpenAI, Anthropic, and Gemini.

**Scope**
- Implement `ProviderProfile` interface and concrete profile types.
- Implement layered prompt builder:
  1. provider base instructions
  2. environment context
  3. tool descriptions
  4. project instruction files
  5. user override block
- Implement project-doc discovery from repo root to cwd with 32KB total budget and precedence rules.
- Implement profile-specific file filtering (`AGENTS.md` always; provider-specific files by profile).

**Out of Scope**
- Final parity tuning for provider-native prompt text quality.

**Deliverables**
- Profile types with capability flags (`supports_parallel_tool_calls`, context window, etc.).
- Environment context block generator (git branch/status summary/date/platform/model/cutoff).
- Tests for discovery precedence, byte budget truncation marker, and prompt layer ordering.

**Acceptance**
- Each profile yields a valid system prompt and tool definition list.
- Prompt layering order is deterministic and test-covered.
- Provider-specific instruction loading rules are enforced.

**Implemented**
- Implemented concrete provider profile types in `crates/forge-agent/src/profiles.rs`:
  - `OpenAiProviderProfile`, `AnthropicProviderProfile`, and `GeminiProviderProfile`
  - profile-specific capability defaults and instruction-file loading rules
  - retained `StaticProviderProfile` with profile-aware instruction-file filtering
- Implemented layered prompt construction in `crates/forge-agent/src/profiles.rs`:
  - layer order is now deterministic:
    1. provider base instructions
    2. environment context
    3. tool descriptions
    4. project instructions
    5. user override block (highest priority)
  - added explicit environment/tool/project/override sections in final system prompt text
- Implemented environment context snapshot generation in `crates/forge-agent/src/session.rs`:
  - captures working directory, repository root, git branch, short git status summary, recent commits, platform, OS version, date, model, and optional knowledge cutoff
  - snapshot is created once at session initialization and reused for each request
- Implemented project instruction discovery and filtering in `crates/forge-agent/src/session.rs`:
  - walks from git root (or cwd when not in git) to current working directory
  - root-first then deeper-directory precedence
  - profile-specific file filtering with `AGENTS.md` always included and provider-specific files (`CLAUDE.md`, `GEMINI.md`, `.codex/instructions.md`) gated by profile
  - 32KB aggregate byte budget with truncation marker: `[Project instructions truncated at 32KB]`
- Added `system_prompt_override` to `SessionConfig` in `crates/forge-agent/src/config.rs` and wired it as the final prompt layer.
- Made tool definition order deterministic in `crates/forge-agent/src/tools.rs` by sorting definitions by tool name before prompt rendering.

**Tests**
- Added prompt-layer ordering and provider file-rule tests in `crates/forge-agent/src/profiles.rs`.
- Added project-doc discovery precedence and 32KB truncation-marker tests in `crates/forge-agent/src/session.rs`.
- Added deterministic tool-definition ordering test in `crates/forge-agent/src/tools.rs`.

**Validation**
- `cargo test -p forge-agent` passed.
