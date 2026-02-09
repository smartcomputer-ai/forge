# P17: Local Execution Environment

**Status**
- Planned (2026-02-09)

**Goal**
Implement `LocalExecutionEnvironment` as the required runtime backend for all core tools.

**Scope**
- Implement filesystem operations (`read_file`, `write_file`, `file_exists`, `list_directory`).
- Implement command execution with process-group handling and timeout escalation (SIGTERM -> wait 2s -> SIGKILL).
- Implement environment variable filtering policy defaults and overrides.
- Implement `grep`/`glob` capabilities with `ripgrep` preference and fallback behavior.

**Out of Scope**
- Docker/Kubernetes/WASM/SSH environments (extension points only).

**Deliverables**
- `ExecutionEnvironment` trait/interface and local implementation.
- Timeout-safe command runner with duration measurement and partial-output return on timeout.
- Tests for timeout behavior, env var filtering, and basic file/search operations.

**Acceptance**
- Default command timeout is 10 seconds, with per-call override support.
- Max timeout clamping enforced by session config.
- Sensitive env vars (`*_API_KEY`, `*_SECRET`, `*_TOKEN`, `*_PASSWORD`, `*_CREDENTIAL`) excluded by default.

