# P05: Core Client + Middleware

**Status**
- Done (2026-02-09)

**Goal**
Implement the provider routing client and middleware system.

**Scope**
- `ProviderAdapter` trait with `complete` and `stream`.
- `Client` with provider registry, default provider, and routing rules.
- `Client.from_env()` with provider env detection and default selection.
- Middleware chain with request and response wrapping for both blocking and streaming.
- Module-level default client with lazy init and `set_default_client`.

**Out of Scope**
- Provider adapters.
- High-level API tooling loops.

**Deliverables**
- `client` module with routing and middleware.
- Tests for middleware order and provider resolution.

**Acceptance**
- Provider routing respects explicit `provider` and defaults.
- `ConfigurationError` raised when no providers configured.
- Middleware order matches spec: request in registration order, response in reverse order.
