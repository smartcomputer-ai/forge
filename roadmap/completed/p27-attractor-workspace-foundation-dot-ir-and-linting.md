# P27: Attractor Workspace Foundation, DOT IR, and Linting (Spec 03 §§1-2,7-9)
_Complete_

**Status**
- Done (2026-02-09)

**Goal**
Create the `forge-attractor` foundation crate and implement the DOT front-end pipeline (parse -> IR -> transforms -> validate) for the Attractor DSL subset.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Sections 1, 2, 7, 8, 9, 11.1, 11.2, 11.10, 11.11)
- Vision alignment: `spec/00-vision.md` (orchestrator + run-store role)
- Storage extension constraints: `spec/04-cxdb-integration-spec.md` (filesystem remains authoritative in first phase)
- Prior dependency: `roadmap/completed/p25-agent-attractor-backend-readiness.md`

**Context**
- `forge-llm` and `forge-agent` are complete enough to serve as codergen backend dependencies.
- We will parse DOT using `graphviz-rust` and convert into an internal Attractor IR.
- We will enforce Attractor's strict DOT subset in our validator/normalizer layer (not by trusting full Graphviz grammar acceptance).
- We will not adopt GPL-licensed DOT parser dependencies for this repository.

## Scope
- Add new crate scaffold and public APIs for Attractor graph preparation.
- Implement DOT parse adapter and IR conversion.
- Implement built-in transforms:
  - variable expansion (`$goal`)
  - model stylesheet parsing/application
- Implement core lint diagnostics and `validate_or_raise`.
- Add deterministic unit tests for parser/transform/lint behavior.

## Out of Scope
- Runtime traversal/execution loop.
- Node handler execution.
- Checkpoint/resume and artifact runtime writes.
- HTTP server mode and interviewer runtime behavior.
- CXDB adapter implementation.

## Priority 0 (Must-have)

### [x] G1. Create `forge-attractor` crate and core graph models
- Work:
  - Add `crates/forge-attractor` to workspace.
  - Define core types: `Graph`, `Node`, `Edge`, `Attributes`, `Diagnostic`, `Severity`.
  - Define an internal normalized IR with typed attributes and defaults.
  - Keep parsing and runtime concerns separated by module boundaries.
- Files:
  - `Cargo.toml` (workspace membership)
  - `crates/forge-attractor/Cargo.toml`
  - `crates/forge-attractor/src/lib.rs`
  - `crates/forge-attractor/src/graph.rs`
  - `crates/forge-attractor/src/errors.rs`
- DoD:
  - Crate builds and exposes graph preparation API surface with stable types.
- Completed:
  - Added `forge-attractor` crate and workspace membership.
  - Added normalized graph model with typed attributes, node/edge records, and diagnostics/error surface.

### [x] G2. Implement DOT parser adapter (`graphviz-rust` -> Attractor IR)
- Work:
  - Use `graphviz-rust` for syntactic parse.
  - Convert parser AST into Attractor IR:
    - flatten subgraphs for runtime graph model
    - expand chained edges (`A -> B -> C`)
    - collect graph/node/edge defaults and apply scope rules
  - Capture source-loc-like references where feasible for diagnostics.
- Decision constraints:
  - `dot-parser` (GPL) is excluded.
  - `dot-structures` is used only via parser AST, not as runtime contract.
- Files:
  - `crates/forge-attractor/src/parse.rs`
  - `crates/forge-attractor/src/graph.rs`
- DoD:
  - Parse path handles spec examples and emits normalized IR suitable for validation.
- Completed:
  - Implemented DOT parse adapter over `graphviz-rust`.
  - Added normalization pass to flatten subgraphs into the runtime node/edge model and expand chained edges.

### [x] G3. Enforce Attractor DOT subset and type coercion rules
- Work:
  - Reject unsupported constructs per spec/03 subset:
    - undirected edges (`--`)
    - non-digraph graphs
    - `strict` graphs
    - HTML labels/IDs where disallowed by Attractor profile
  - Implement typed attribute coercion:
    - `String`, `Integer`, `Float`, `Boolean`, `Duration`
  - Support comments and optional semicolons per spec grammar expectations.
- Files:
  - `crates/forge-attractor/src/schema.rs`
  - `crates/forge-attractor/src/parse.rs`
  - `crates/forge-attractor/src/diagnostics.rs`
- DoD:
  - Invalid subset usage yields deterministic error diagnostics.
- Completed:
  - Enforced subset constraints including digraph-only, non-strict, no `--`, no HTML IDs/labels, no ports, and no subgraph vertices in edges.
  - Implemented typed coercion for string/integer/float/boolean/duration values.
  - Added duration literal normalization shim so unquoted duration syntax is accepted through the parser backend.

### [x] G4. Implement transforms pipeline (variable expansion + stylesheet)
- Work:
  - Add transform registry and deterministic ordering:
    1) built-in transforms
    2) custom transforms
  - Implement `VariableExpansionTransform` for `$goal`.
  - Implement `ModelStylesheetTransform` with specificity rules and syntax validation.
  - Ensure explicit node attrs override stylesheet-injected values.
- Files:
  - `crates/forge-attractor/src/transforms.rs`
  - `crates/forge-attractor/src/stylesheet.rs`
- DoD:
  - Transform application matches spec precedence and is covered by unit tests.
- Completed:
  - Added transform trait and ordered built-in transform pipeline.
  - Implemented `$goal` variable expansion and stylesheet parse/apply with selector specificity.
  - Added `prepare_pipeline(...)` API for parse -> transform -> validate workflow.

### [x] G5. Implement lint engine and built-in rule set
- Work:
  - Implement built-in rules from spec/03 Section 7.2.
  - Implement `validate(graph)` and `validate_or_raise(graph)`.
  - Support extensibility via custom lint rule trait.
- Files:
  - `crates/forge-attractor/src/lint.rs`
  - `crates/forge-attractor/src/diagnostics.rs`
- DoD:
  - `validate_or_raise` blocks execution-prep on error-severity diagnostics.
- Completed:
  - Implemented built-in lint rules listed in Section 7.2 and exposed `validate(...)` / `validate_or_raise(...)`.
  - Added condition and stylesheet syntax validation paths used by lints.

## Priority 1 (Strongly recommended)

### [x] G6. Add parse/transform/lint conformance tests for Section 11.1/11.2/11.10/11.11
- Work:
  - Add focused unit tests per rule/transform.
  - Add integration tests for representative DOT fixtures.
- Files:
  - `crates/forge-attractor/tests/dot_parsing.rs`
  - `crates/forge-attractor/tests/validation.rs`
  - `crates/forge-attractor/tests/stylesheet.rs`
- DoD:
  - Core DoD parsing/lint transform matrix is mechanically testable and green.
- Completed:
  - Added module-level and integration tests for parsing, stylesheet behavior, transforms, and lint validation.
  - New integration tests added under:
    - `crates/forge-attractor/tests/dot_parsing.rs`
    - `crates/forge-attractor/tests/validation.rs`
    - `crates/forge-attractor/tests/stylesheet.rs`

## Deliverables
- New crate: `crates/forge-attractor`
- DOT front-end modules:
  - parser adapter
  - normalization/IR conversion
  - transforms
  - lint diagnostics
- Test suites for parser/lint/transform behavior

## Execution order
1. G1 crate + core types
2. G2 parser adapter + IR conversion
3. G3 subset enforcement/type coercion
4. G4 transforms
5. G5 lint engine
6. G6 conformance tests

## Exit criteria for this file
- `forge-attractor` can parse DOT into normalized Attractor IR.
- Required transforms and lint rules execute deterministically.
- Error diagnostics are actionable and stable for downstream runtime use.
