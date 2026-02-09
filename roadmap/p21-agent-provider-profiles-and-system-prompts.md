# P21: Provider Profiles + Layered System Prompt Construction

**Status**
- Planned (2026-02-09)

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

