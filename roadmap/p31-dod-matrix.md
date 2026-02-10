# P31 DoD Matrix (Spec 03 Section 11)

Status date: 2026-02-10

Legend:
- `[x]` complete and covered
- `[ ]` gap/deviation tracked

## 11.1 DOT Parsing
- [x] Parser accepts supported DOT subset. Refs: `crates/forge-attractor/tests/dot_parsing.rs`, `crates/forge-attractor/tests/conformance_parsing.rs`
- [x] Graph-level attributes (`goal`, `label`, `model_stylesheet`) extracted. Refs: `crates/forge-attractor/tests/conformance_parsing.rs`, `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] Node attributes including multiline blocks parse. Refs: `crates/forge-attractor/tests/conformance_parsing.rs`
- [x] Edge attributes (`label`, `condition`, `weight`) parse. Refs: `crates/forge-attractor/tests/conformance_parsing.rs`, `crates/forge-attractor/tests/conditions.rs`
- [x] Chained edges expand (`A -> B -> C`). Refs: `crates/forge-attractor/tests/dot_parsing.rs`, `crates/forge-attractor/tests/conformance_parsing.rs`
- [x] Node/edge default blocks apply. Refs: `crates/forge-attractor/tests/dot_parsing.rs`, `crates/forge-attractor/tests/conformance_parsing.rs`
- [x] Subgraph blocks flattened. Refs: `crates/forge-attractor/tests/dot_parsing.rs`
- [x] `class` attribute merge behavior supported for stylesheet targeting. Refs: `crates/forge-attractor/tests/dot_parsing.rs`, `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] Quoted and unquoted values supported. Refs: `crates/forge-attractor/tests/dot_parsing.rs`, `crates/forge-attractor/tests/conformance_parsing.rs`
- [x] `//` and `/* */` comments stripped before parse. Refs: `crates/forge-attractor/tests/conformance_parsing.rs`

## 11.2 Validation and Linting
- [x] Exactly one start node required. Refs: `crates/forge-attractor/src/lint.rs`, `crates/forge-attractor/tests/validation.rs`
- [ ] Exactly one exit node required. Current behavior is at least one terminal required. Refs: `crates/forge-attractor/src/lint.rs` (`rule_terminal_node`)
- [x] Start node has no incoming edges. Refs: `crates/forge-attractor/src/lint.rs`, `crates/forge-attractor/tests/validation.rs`
- [x] Exit node has no outgoing edges. Refs: `crates/forge-attractor/src/lint.rs`, `crates/forge-attractor/tests/validation.rs`
- [x] All nodes reachable from start. Refs: `crates/forge-attractor/src/lint.rs`, `crates/forge-attractor/tests/validation.rs`
- [x] All edges reference valid nodes. Refs: `crates/forge-attractor/src/lint.rs`, `crates/forge-attractor/tests/validation.rs`, `crates/forge-attractor/tests/conformance_parsing.rs`
- [x] Codergen prompt missing warning. Refs: `crates/forge-attractor/src/lint.rs`, `crates/forge-attractor/tests/validation.rs`
- [x] Edge condition syntax validated. Refs: `crates/forge-attractor/tests/conditions.rs`, `crates/forge-attractor/tests/validation.rs`
- [x] `validate_or_raise()` throws on error diagnostics. Refs: `crates/forge-attractor/tests/validation.rs`, `crates/forge-attractor/tests/conformance_parsing.rs`
- [x] Lint results include rule/severity/location/message. Refs: `crates/forge-attractor/src/diagnostics.rs`, `crates/forge-attractor/tests/conformance_parsing.rs`

## 11.3 Execution Engine
- [x] Start node resolved and execution begins there. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/tests/execution_core.rs`
- [x] Handler resolution via shape/type mapping. Refs: `crates/forge-attractor/src/handlers/registry.rs`, `crates/forge-attractor/tests/handlers_core.rs`
- [x] Handler called with node/context/graph and returns Outcome. Refs: `crates/forge-attractor/src/runtime.rs`, `crates/forge-attractor/tests/handlers_core.rs`
- [x] Outcome written to stage `status.json` for codergen paths. Refs: `crates/forge-attractor/src/handlers/codergen.rs`, `crates/forge-attractor/tests/integration_smoke.rs`
- [x] Edge selection follows 5-step priority. Refs: `crates/forge-attractor/src/routing.rs`, `crates/forge-attractor/tests/conditions.rs`, `crates/forge-attractor/tests/execution_core.rs`
- [x] Engine loop executes/advances/repeats. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/tests/execution_core.rs`
- [x] Terminal node stops execution. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/tests/execution_core.rs`
- [x] Pipeline success/fail outcome resolved with gate and routing state. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/tests/execution_core.rs`

## 11.4 Goal Gate Enforcement
- [x] `goal_gate=true` nodes tracked. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/tests/execution_core.rs`
- [x] Gate check occurs before terminal exit. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/tests/execution_core.rs`
- [x] Unsatisfied gate routes to retry target when configured. Refs: `crates/forge-attractor/tests/execution_core.rs`
- [x] Unsatisfied gate without retry target fails pipeline. Refs: `crates/forge-attractor/tests/execution_core.rs`

## 11.5 Retry Logic
- [x] `max_retries > 0` retries RETRY/FAIL outcomes. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/tests/execution_core.rs`, `crates/forge-attractor/tests/events.rs`
- [x] Retry count tracked per node with configured limit. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/tests/execution_core.rs`
- [x] Backoff works (constant/linear/exponential behavior from config). Refs: `crates/forge-attractor/src/retry.rs`, `crates/forge-attractor/tests/retry.rs`
- [x] Jitter applied when configured. Refs: `crates/forge-attractor/tests/retry.rs`
- [x] Post-exhaustion final outcome used for routing. Refs: `crates/forge-attractor/tests/execution_core.rs`

## 11.6 Node Handlers
- [x] Start handler no-op SUCCESS. Refs: `crates/forge-attractor/src/handlers/start.rs`, `crates/forge-attractor/tests/handlers_core.rs`
- [x] Exit handler no-op SUCCESS. Refs: `crates/forge-attractor/src/handlers/exit.rs`, `crates/forge-attractor/tests/handlers_core.rs`
- [x] Codergen expands `$goal`, calls backend, writes prompt/response artifacts. Refs: `crates/forge-attractor/src/handlers/codergen.rs`, `crates/forge-attractor/tests/integration_smoke.rs`
- [x] Wait.human exposes choices and returns routed selection. Refs: `crates/forge-attractor/src/handlers/wait_human.rs`, `crates/forge-attractor/tests/hitl.rs`
- [x] Conditional handler pass-through. Refs: `crates/forge-attractor/src/handlers/conditional.rs`, `crates/forge-attractor/tests/handlers_core.rs`
- [x] Parallel handler fan-out supported. Refs: `crates/forge-attractor/src/handlers/parallel.rs`, `crates/forge-attractor/tests/parallel.rs`
- [x] Fan-in handler consolidates branches. Refs: `crates/forge-attractor/src/handlers/parallel_fan_in.rs`, `crates/forge-attractor/tests/parallel.rs`
- [x] Tool handler executes configured commands. Refs: `crates/forge-attractor/src/handlers/tool.rs`, `crates/forge-attractor/tests/handlers_core.rs`
- [x] Custom handler registration by type string. Refs: `crates/forge-attractor/src/handlers/registry.rs`, `crates/forge-attractor/tests/handlers_core.rs`

## 11.7 State and Context
- [x] Shared key-value context across handlers. Refs: `crates/forge-attractor/src/context.rs`, `crates/forge-attractor/tests/conformance_runtime.rs`
- [x] Handlers return `context_updates`. Refs: `crates/forge-attractor/src/runtime.rs`, `crates/forge-attractor/tests/handlers_core.rs`
- [x] Context updates merged after each node. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/tests/conformance_runtime.rs`
- [x] Checkpoint saved after node completion. Refs: `crates/forge-attractor/src/runner.rs`, `crates/forge-attractor/tests/conformance_state.rs`
- [x] Resume loads state and continues. Refs: `crates/forge-attractor/tests/state_and_resume.rs`, `crates/forge-attractor/tests/conformance_state.rs`
- [x] Stage artifacts written under `{logs_root}/{node_id}`. Refs: `crates/forge-attractor/tests/integration_smoke.rs`

## 11.8 Human-in-the-Loop
- [x] Interviewer interface `ask(question) -> answer`. Refs: `crates/forge-attractor/src/interviewer.rs`
- [x] Question types supported (single-choice/yes-no/free-text/confirmation equivalents). Refs: `crates/forge-attractor/src/interviewer.rs`
- [x] AutoApproveInterviewer picks first/approve behavior for deterministic automation. Refs: `crates/forge-attractor/src/interviewer.rs`, `crates/forge-attractor/tests/interviewer.rs`
- [x] ConsoleInterviewer interactive path. Refs: `crates/forge-attractor/src/interviewer.rs`
- [x] CallbackInterviewer delegates function. Refs: `crates/forge-attractor/tests/interviewer.rs`, `crates/forge-attractor/tests/hitl.rs`
- [x] QueueInterviewer deterministic queued answers. Refs: `crates/forge-attractor/tests/interviewer.rs`, `crates/forge-attractor/tests/hitl.rs`

## 11.9 Condition Expressions
- [x] `=` operator works. Refs: `crates/forge-attractor/tests/conditions.rs`
- [x] `!=` operator works. Refs: `crates/forge-attractor/tests/conditions.rs`
- [x] `&&` conjunction works. Refs: `crates/forge-attractor/tests/conditions.rs`
- [x] `outcome` variable resolves. Refs: `crates/forge-attractor/tests/conditions.rs`
- [x] `preferred_label` variable resolves. Refs: `crates/forge-attractor/tests/conditions.rs`
- [x] `context.*` variables resolve with empty fallback. Refs: `crates/forge-attractor/tests/conditions.rs`
- [x] Empty condition treated as unconditional true. Refs: `crates/forge-attractor/tests/conditions.rs`

## 11.10 Model Stylesheet
- [x] Stylesheet parsed from graph attr. Refs: `crates/forge-attractor/src/stylesheet.rs`, `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] Shape selectors work. Refs: `crates/forge-attractor/tests/stylesheet.rs`, `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] Class selectors work. Refs: `crates/forge-attractor/tests/stylesheet.rs`, `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] Node ID selectors work. Refs: `crates/forge-attractor/tests/stylesheet.rs`, `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] Specificity order enforced. Refs: `crates/forge-attractor/tests/stylesheet.rs`, `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] Explicit node attrs override stylesheet. Refs: `crates/forge-attractor/tests/stylesheet.rs`

## 11.11 Transforms and Extensibility
- [x] AST transforms modify graph between parse/validate. Refs: `crates/forge-attractor/src/transforms.rs`, `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] Transform interface `transform(graph) -> graph`. Refs: `crates/forge-attractor/src/transforms.rs`
- [x] Built-in variable expansion replaces `$goal`. Refs: `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] Custom transforms register/run in order. Refs: `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] HTTP server mode optional/deferred for later phase (not required in this milestone). Refs: `roadmap/p30-attractor-observability-hitl-and-storage-abstractions.md`

## 11.12 Cross-Feature Parity Matrix
- [x] Parse simple linear pipeline. Refs: `crates/forge-attractor/tests/conformance_parsing.rs`, `crates/forge-attractor/tests/execution_core.rs`
- [x] Parse graph-level attrs. Refs: `crates/forge-attractor/tests/conformance_parsing.rs`
- [x] Parse multiline node attrs. Refs: `crates/forge-attractor/tests/conformance_parsing.rs`
- [x] Validate missing start -> error. Refs: `crates/forge-attractor/tests/validation.rs`
- [x] Validate missing exit -> error. Refs: `crates/forge-attractor/tests/validation.rs`
- [x] Validate orphan node flagged. Refs: `crates/forge-attractor/tests/validation.rs`
- [x] Execute linear 3-node pipeline E2E. Refs: `crates/forge-attractor/tests/execution_core.rs`
- [x] Execute conditional branching success/fail paths. Refs: `crates/forge-attractor/tests/execution_core.rs`, `crates/forge-attractor/tests/conditions.rs`
- [x] Execute retry on failure. Refs: `crates/forge-attractor/tests/execution_core.rs`, `crates/forge-attractor/tests/events.rs`
- [x] Goal gate blocks exit unsatisfied. Refs: `crates/forge-attractor/tests/execution_core.rs`
- [x] Goal gate allows exit satisfied. Refs: `crates/forge-attractor/tests/execution_core.rs`, `crates/forge-attractor/tests/conformance_runtime.rs`
- [x] Wait.human routes on selection. Refs: `crates/forge-attractor/tests/hitl.rs`, `crates/forge-attractor/tests/conformance_runtime.rs`
- [x] Edge selection condition wins over weight. Refs: `crates/forge-attractor/tests/conditions.rs`
- [x] Edge selection weight tie-break for unconditional edges. Refs: `crates/forge-attractor/tests/conditions.rs`
- [x] Lexical final fallback works. Refs: `crates/forge-attractor/tests/conditions.rs`
- [x] Context updates visible to downstream nodes. Refs: `crates/forge-attractor/tests/conformance_runtime.rs`
- [x] Checkpoint save/resume parity. Refs: `crates/forge-attractor/tests/conformance_state.rs`, `crates/forge-attractor/tests/state_and_resume.rs`
- [x] Stylesheet shape override application. Refs: `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] Prompt variable expansion `$goal`. Refs: `crates/forge-attractor/tests/conformance_stylesheet.rs`
- [x] Parallel fan-out/fan-in complete. Refs: `crates/forge-attractor/tests/parallel.rs`
- [x] Custom handler registration/execution. Refs: `crates/forge-attractor/tests/handlers_core.rs`
- [x] Pipeline with 10+ nodes completes. Refs: `crates/forge-attractor/tests/parallel.rs` (extended graph coverage)

## 11.13 Integration Smoke Test
- [x] Spec-style deterministic smoke scenario (`plan -> implement -> review -> done`) with success/fail rerouting, goal-gate checks, checkpoint and artifact assertions. Refs: `crates/forge-attractor/tests/integration_smoke.rs`
- [x] Real live-LLM callback smoke path (ignored/env-gated) implemented as single default OpenAI Codex live smoke test. Refs: `crates/forge-attractor/tests/live.rs`
