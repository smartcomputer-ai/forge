# P03: Errors, Retry, Timeouts

**Status**
- Done (2026-02-09)

**Goal**
Implement the unified error taxonomy, retry policy, and timeout structures as specified.

**Scope**
- `SDKError` base type and full error hierarchy.
- `ProviderError` fields and retryability flags.
- Retry policy structs and backoff calculation.
- Timeout config (`TimeoutConfig`, `AdapterTimeout`) and abort signal representation.
- Error helpers for mapping HTTP status codes and message-based classification.

**Out of Scope**
- Provider-specific error parsing.
- Automatic retries in high-level API (handled later).

**Deliverables**
- Error types using `thiserror` with structured fields.
- `RetryPolicy` and a deterministic backoff calculator with optional jitter.
- Unit tests for retryability rules and backoff calculations.

**Acceptance**
- Error types align 1:1 with spec section 6.
- Retryability defaults and status-code mapping conform to the table in section 6.4.
- Backoff tests cover jitter disabled and enabled modes.
