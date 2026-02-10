# P83: Attractor Attribute Policy Completion and Contract Tightening (Post-P82 Predictability)

**Status**
- Planned (2026-02-10)
- Rebaselined on post-migration CXDB-first architecture (2026-02-10)

**Goal**
Ensure DOT graph/node/edge attributes are fully and deterministically enforced at runtime and backend layers, with explicit precedence rules, strict validation, and conformance coverage.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Sections 2.5, 2.6, 2.7, 3, 4.5, 5.4, 11)
- Storage/correlation extension: `spec/04-cxdb-integration-spec.md` (Sections 3.4, 4.4)
- Baseline:
  - `roadmap/p37-turnstore-sunset-and-cxdb-hardening.md`
  - `roadmap/p37-dod-matrix.md`
- Prerequisites:
  - `roadmap/later/p80-attractor-stage-outcome-contract-and-status-ingestion.md`
  - `roadmap/later/p81-attractor-true-parallel-and-fan-in-semantics.md`
  - `roadmap/later/p82-attractor-runtime-control-plane-and-resume-hardening.md`

**Context**
- Complex orchestration graphs rely on many attrs as declarative control surface.
- Partial attr wiring creates silent behavior drift and brittle pipelines.
- We need explicit contract completeness: declared attrs must either be enforced or rejected/warned deterministically.

## Scope
- Complete attr-to-runtime and attr-to-backend policy mapping.
- Define and enforce precedence model (graph defaults, node overrides, edge overrides, runtime overrides).
- Add strict lint/validation for unsupported or ambiguous attrs.
- Expose effective resolved stage policy for observability/debugging.
- Add conformance tests for each enforced attr family.

## Out of Scope
- New provider implementations and provider multiplexing expansion.
- Distributed coordination and worker leasing.
- Host UI renderer/plugin features beyond runtime API exposure.

## Priority 0 (Must-have)

### [ ] G1. Attribute contract inventory and enforcement matrix
- Work:
  - Enumerate all recognized attrs from spec and current implementation.
  - Classify each attr as:
    - enforced
    - partially enforced
    - unsupported
  - Add machine-readable mapping table for runtime/backend enforcement targets.
- Files:
  - `roadmap/p36-attr-matrix.md` (new)
  - `crates/forge-attractor/src/graph.rs`
  - `crates/forge-attractor/src/lint.rs`
- DoD:
  - Every declared attr has explicit implementation status and enforcement target.

### [ ] G2. Runtime attribute enforcement completion
- Work:
  - Ensure runtime attrs are fully honored (examples):
    - retry/max attempts/backoff controls
    - timeout controls
    - fidelity/thread controls
    - loop restart controls
    - goal-gate/retry target controls
  - Add deterministic fallback behavior where attrs are omitted.
- Files:
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/src/retry.rs`
  - `crates/forge-attractor/src/fidelity.rs`
- DoD:
  - Runtime behavior matches declared attr contract with deterministic precedence.

### [ ] G3. Backend policy mapping completion (codergen/tool)
- Work:
  - Ensure backend-facing attrs are fully mapped and enforced (examples):
    - `llm_provider`, `llm_model`, `reasoning_effort`
    - stage execution limits (`max_agent_turns` and related backend options)
    - tool hook and tool policy attrs (when available)
  - Make unsupported backend attrs explicit via validation warnings/errors.
- Files:
  - `crates/forge-attractor/src/backends/forge_agent.rs`
  - `crates/forge-attractor/src/handlers/codergen.rs`
  - `crates/forge-attractor/src/handlers/tool.rs`
- DoD:
  - Backend execution settings are driven by declared attrs without silent drops.

### [ ] G4. Precedence and conflict-resolution contract
- Work:
  - Implement explicit resolution order for overlapping policy inputs:
    - edge > node > graph default > runtime fallback
  - Add conflict diagnostics for contradictory attr combinations.
  - Publish precedence behavior in docs/spec references.
- Files:
  - `crates/forge-attractor/src/fidelity.rs`
  - `crates/forge-attractor/src/lint.rs`
  - `crates/forge-attractor/README.md`
- DoD:
  - Precedence behavior is deterministic, documented, and test-backed.

## Priority 1 (Strongly recommended)

### [ ] G5. Effective policy observability surfaces
- Work:
  - Emit resolved effective policy per stage into runtime events and query APIs.
  - Include provenance metadata (which scope provided each effective value).
- Files:
  - `crates/forge-attractor/src/events.rs`
  - `crates/forge-attractor/src/queries.rs`
- DoD:
  - Hosts can inspect "declared vs effective" policy for each stage.

### [ ] G6. Conformance and regression suite for attrs
- Work:
  - Add deterministic tests validating attr behavior by family:
    - routing/retry/fidelity/timeout
    - backend mapping and limits
    - precedence and conflict cases
  - Add CLI smoke assertions for effective policy visibility.
- Files:
  - `crates/forge-attractor/tests/attr_runtime.rs`
  - `crates/forge-attractor/tests/attr_backend.rs`
  - `crates/forge-attractor/tests/attr_precedence.rs`
  - `crates/forge-cli/tests/effective_policy.rs`
- DoD:
  - Attr enforcement is conformance-tested and protected from regressions.

## Deliverables
- Complete attr enforcement matrix with explicit status.
- Deterministic runtime/backend mapping for declared attrs.
- Precedence/conflict contract and diagnostics.
- Effective policy observability and conformance tests.

## Execution order
1. G1 attr inventory/matrix
2. G2 runtime enforcement completion
3. G3 backend mapping completion
4. G4 precedence/conflict contract
5. G5 effective policy observability
6. G6 conformance/regression suite

## Exit criteria for this file
- No critical attr used by production graphs is silently ignored.
- Effective policy per stage is deterministic and observable.
- Attr behavior and precedence are fully conformance-tested.
