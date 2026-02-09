# P13: Unified Spec Gap Closure
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Close the remaining implementation gaps after the first pass of `spec/01-unified-llm-spec.md` for the currently active providers.

**Gap Audit Summary (before this milestone)**
1. High-level API did not expose cancellation or timeout controls, and execution paths did not enforce total/per-step timeout budgets.
2. Tool definitions were not validated against spec constraints (`name` pattern/length and root object schema).
3. Tool-call arguments were parsed but not schema-validated before execute handlers.
4. OpenAI adapters did not map unified `response_format` into native request payloads.
5. OpenAI adapters did not preserve `Retry-After` into provider errors.
6. Anthropic fallback path for structured output (`json`/`json_schema`) was not injected when native schema mode is unavailable.

**Implemented**
1. Added `GenerateOptions.timeout` and `GenerateOptions.abort_signal`, wired into `generate()` and `stream()` execution paths.
2. Added timeout enforcement and abort checks in retry wrappers (`complete_with_retry` and `stream_with_retry`) with total/per-step effective timeout calculation.
3. Added tool validation before execution:
   - Name validation: `[a-zA-Z][a-zA-Z0-9_]*` and max length 64.
   - Parameter schema root validation: must be object schema.
4. Added pre-execution tool argument validation:
   - Requires object payload.
   - Validates required keys and primitive type compatibility against declared JSON schema properties.
5. Added OpenAI Responses and OpenAI-compatible request translation for `response_format`:
   - `text`
   - `json`
   - `json_schema` (with strict support)
6. Added OpenAI `Retry-After` parsing and propagation into `ProviderError.retry_after`.
7. Added Anthropic structured-output fallback instruction injection into system blocks for `json` and `json_schema` response formats.
8. Added regression tests for new behaviors:
   - high-level timeout enforcement and tool-name validation,
   - OpenAI response_format mapping and retry-after propagation,
   - Anthropic response-format schema hint injection.
9. Added a public low-level async retry helper (`retry_async`) in the SDK error/retry layer for callers using low-level `Client` methods directly.
10. Enforced adapter `stream_read` timeouts in OpenAI/OpenAI-compatible/Anthropic streaming paths.
11. Normalized stream-failure behavior to emit `StreamEventType::Error` as the canonical stream error signal.
12. Added optional provider lifecycle/capability hooks in the adapter contract (`initialize`, `close`, `supports_tool_choice`) and integrated registration-time initialization in `Client`.
13. Updated `stream_object()` to consume real streaming deltas and replay real stream events instead of synthesizing a single text chunk from `generate_object()`.
14. Added README guidance for low-level retry usage and stream error-event handling.

**Files Changed**
- `crates/forge-llm/src/high_level.rs`
- `crates/forge-llm/src/openai.rs`
- `crates/forge-llm/src/anthropic.rs`
- `crates/forge-llm/src/errors.rs`
- `crates/forge-llm/src/stream.rs`
- `crates/forge-llm/src/provider.rs`
- `crates/forge-llm/src/client.rs`
- `crates/forge-llm/README.md`

**Validation**
- `cargo test -p forge-llm` passed after changes.
