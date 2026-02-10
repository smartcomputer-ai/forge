# P29: Attractor State, Checkpoint/Fidelity, and Advanced Handlers (Spec 03 §5 + advanced §4)
_Complete_

**Status**
- Completed (2026-02-10)

**Goal**
Implement production-grade runtime state behavior: context/artifacts, checkpoint/resume semantics, fidelity/thread resolution, and advanced handlers (`parallel`, `fan_in`, `stack.manager_loop`) on top of storage abstractions.

**Source**
- Spec of record: `spec/03-attractor-spec.md` (Sections 4.8-4.11, 5, 11.6, 11.7)
- Storage extension: `spec/04-cxdb-integration-spec.md` (Sections 3.3, 3.4, 5.1, 5.2)

**Context**
- P28 delivers core execution and baseline handlers.
- P27.1 provides `forge-turnstore` abstractions and deterministic local backends.
- This phase hardens state and recovery behavior, which is required for unattended factory loops.
- CXDB adapter is still optional and deferred to post-P31 milestones.

## Scope
- Implement context and artifact store behavior.
- Implement checkpoint serialization and resume logic through storage contracts.
- Implement fidelity resolution and full->summary degrade-on-resume rule.
- Implement advanced handlers:
  - `parallel`
  - `parallel.fan_in`
  - `stack.manager_loop`
- Implement loop restart behavior.
- Persist run graph metadata (DOT source hash/ref + normalized snapshot ref) through storage layer.

## Out of Scope
- HTTP server mode and remote host orchestration UX.
- CXDB-authoritative resume path.
- Typed projection/renderer loading concerns.

## Priority 0 (Must-have)

### [x] G1. Context store and artifact store implementation
- Work:
  - Implement thread-safe context map with serializable snapshot behavior.
  - Implement artifact store:
    - memory-backed for small payloads
    - filesystem-backed above threshold
  - Enforce namespace/key conventions where practical.
- Files:
  - `crates/forge-attractor/src/context.rs`
  - `crates/forge-attractor/src/artifacts.rs`
- DoD:
  - Context updates propagate correctly across stages.
  - Large artifacts are file-backed with stable references.
- Completed:
  - Added `ContextStore` with thread-safe read/write access, serializable snapshots, isolated cloning, update merge support, and key validation.
  - Added `ArtifactStore` with threshold-based in-memory vs filesystem-backed persistence and stable `artifact://<id>` references.
  - Wired runtime traversal to use `ContextStore` snapshots and mutation APIs so context propagation remains deterministic across stage boundaries.
  - Added deterministic unit tests covering context snapshot/clone behavior and artifact threshold/file lifecycle behavior.

### [x] G2. Checkpoint save/load and resume semantics (store-aware)
- Work:
  - Implement checkpoint model:
    - current node
    - completed nodes
    - context snapshot
    - retry counters
    - metadata timestamps/ids
  - Save checkpoint after each completed node.
  - Implement resume flow through storage abstraction.
  - Enforce fidelity degrade on first resumed hop if prior node was `full`.
- Files:
  - `crates/forge-attractor/src/checkpoint.rs`
  - `crates/forge-attractor/src/resume.rs`
  - `crates/forge-attractor/src/runner.rs`
- DoD:
  - Crash/restart resume reproduces deterministic continuation behavior.
- Completed:
  - Added checkpoint model + persistence module (`checkpoint.rs`) including:
    - current node and next node
    - completed node order
    - per-node retry counters
    - serialized context/log snapshots
    - node outcome metadata and terminal status/failure metadata
    - checkpoint metadata IDs/timestamps/sequence
  - Added resume module (`resume.rs`) to load checkpoint state, rebuild runtime state, and validate resume targets.
  - Extended runtime config with `resume_from_checkpoint` and wired runner resume flow through storage-aware lifecycle events.
  - Runner now persists full checkpoint snapshots to checkpoint file paths after each completed node and emits enriched checkpoint event summaries through storage abstraction.
  - Implemented first-hop fidelity degrade marker on resume when prior checkpointed node fidelity was `full` (`summary:high` override marker in context for one hop).
  - Added deterministic tests for checkpoint round-trip, resume continuation parity, and one-hop degrade marker behavior.

### [x] G3. Fidelity and thread resolution engine
- Work:
  - Implement fidelity precedence:
    1) incoming edge
    2) target node attr
    3) graph default
    4) fallback default
  - Implement `thread_id` resolution for `full` mode with precedence and derived fallback.
  - Integrate with codergen backend thread continuity.
- Files:
  - `crates/forge-attractor/src/fidelity.rs`
  - `crates/forge-attractor/src/backends/forge_agent.rs`
- DoD:
  - Fidelity/thread behavior is deterministic and test-covered.
- Completed:
  - Added centralized fidelity/thread resolver module in `crates/forge-attractor/src/fidelity.rs`:
    - fidelity precedence: incoming edge -> target node -> graph default -> `compact`
    - thread key precedence for `full`: node `thread_id` -> edge `thread_id` -> graph-level thread (`thread_id`/`default_thread_id`) -> node class-derived fallback -> previous node id
  - Integrated resolver into runner so each hop writes deterministic runtime context keys:
    - `internal.fidelity.mode`
    - `internal.fidelity.thread_key` (for `full`)
    - `thread_key` (for backend continuity)
  - Enforced non-`full` behavior to clear thread reuse keys, ensuring fresh-session semantics.
  - Updated resume/checkpoint fidelity integration to use resolved runtime fidelity values and one-hop degrade override behavior.
  - Updated `forge_agent` adapter thread continuity behavior to honor resolved fidelity context (`full` only) and clear thread when fidelity is non-`full`.
  - Added deterministic unit coverage for resolver precedence and runner/backend integration behavior.

### [x] G4. Advanced handlers: `parallel`, `parallel.fan_in`, `stack.manager_loop`
- Work:
  - `parallel`:
    - branch context cloning
    - concurrency execution model
    - join policies (`all_success`, `any_success`, `quorum`, `ignore`)
  - `parallel.fan_in`:
    - branch result aggregation
    - merge outcome behavior
  - `stack.manager_loop`:
    - child pipeline observe/steer/wait loop
    - stop-condition evaluation
    - polling interval + timeout behavior
- Files:
  - `crates/forge-attractor/src/handlers/parallel.rs`
  - `crates/forge-attractor/src/handlers/parallel_fan_in.rs`
  - `crates/forge-attractor/src/handlers/stack_manager_loop.rs`
- DoD:
  - Parallel + fan-in + manager loop behavior satisfies Section 11.6 coverage expectations.
- Completed:
  - Added `parallel` handler (`crates/forge-attractor/src/handlers/parallel.rs`) with:
    - branch-context cloning semantics
    - bounded batch concurrency model (`max_parallel`)
    - join policies: `all_success`, `any_success`, `quorum`, `ignore`
    - deterministic `parallel.results` and branch summary context updates
  - Added `parallel.fan_in` handler (`crates/forge-attractor/src/handlers/parallel_fan_in.rs`) with:
    - candidate aggregation from `parallel.results`
    - deterministic ranking/selection (status rank -> score -> id)
    - best-candidate context projection (`parallel.fan_in.*`)
  - Added `stack.manager_loop` handler (`crates/forge-attractor/src/handlers/stack_manager_loop.rs`) with:
    - observe/steer/wait action model
    - stop-condition evaluation via condition engine
    - polling interval + max-cycle timeout semantics
  - Registered all advanced handlers in core registry wiring (`crates/forge-attractor/src/handlers/mod.rs`).
  - Added deterministic unit tests for parallel, fan-in, and manager-loop behavior.

### [x] G5. Loop restart and run lineage hardening
- Work:
  - Implement `loop_restart=true` edge behavior:
    - stop current run
    - start a fresh run lineage attempt
    - persist lineage metadata through storage interfaces
  - Harden artifact durability guarantees.
- Files:
  - `crates/forge-attractor/src/runner.rs`
  - `crates/forge-attractor/src/runtime.rs`
- DoD:
  - Loop restart behavior is deterministic and visible in logs/events.
- Completed:
  - Implemented `loop_restart=true` edge handling in runner:
    - current attempt finalizes with `status="restarted"` event payload
    - runner launches a fresh lineage attempt beginning at the edge target node
  - Added lineage-aware run attempts:
    - root run id + per-attempt run id suffix (`:attempt:N`)
    - lineage context metadata (`internal.lineage.root_run_id`, `internal.lineage.attempt`, `internal.lineage.parent_run_id`)
    - lineage metadata emitted through storage-backed run events
  - Added restart safety limit via `RunConfig.max_loop_restarts` to avoid unbounded restart loops.
  - Hardened artifact/log durability setup by preparing per-attempt fresh log directories and `artifacts/` directories before execution.
  - Added deterministic runtime test for loop-restart lineage behavior and fresh attempt log-root creation.

## Priority 1 (Strongly recommended)

### [x] G6. Resume/fidelity/parallel regression suite (Section 11.7 focus)
- Work:
  - Add deterministic tests for:
    - checkpoint round-trip
    - resume parity
    - fidelity degrade-on-resume
    - artifact threshold behavior
    - parallel/fan-in join policies
  - Run tests against in-memory and filesystem storage backends.
- Files:
  - `crates/forge-attractor/tests/state_and_resume.rs`
  - `crates/forge-attractor/tests/fidelity.rs`
  - `crates/forge-attractor/tests/parallel.rs`
- DoD:
  - State and resume semantics are robust under repeated runs.
- Completed:
  - Added integration regression suite files:
    - `crates/forge-attractor/tests/state_and_resume.rs`
    - `crates/forge-attractor/tests/fidelity.rs`
    - `crates/forge-attractor/tests/parallel.rs`
  - Added deterministic checkpoint/resume parity coverage with manual checkpoint continuation validation.
  - Added fidelity regression coverage for precedence resolution and resume first-hop degrade behavior.
  - Added parallel/fan-in join-policy and aggregation regression coverage.
  - Executed state/resume storage-path coverage against both in-memory and filesystem turnstore backends.
  - Persisted run graph metadata through storage abstraction:
    - `forge.attractor.dot_source` turns with content hash + size metadata.
    - `forge.attractor.graph_snapshot` turns with normalized graph snapshot hash + size metadata.
  - Added checkpoint metadata fields carrying DOT/source and normalized snapshot hash/ref pointers for resume/query continuity.

## Deliverables
- Stable state layer: context, artifacts, checkpoint/resume.
- Fidelity/thread resolution.
- Advanced handler support for parallel and supervisory workflows.
- Regression coverage for state-heavy runtime behavior.

## Execution order
1. G1 context/artifacts
2. G2 checkpoint/resume
3. G3 fidelity/thread resolution
4. G4 advanced handlers
5. G5 loop restart hardening
6. G6 regression suite

## Exit criteria for this file
- Runtime can recover from interruption using checkpoint state.
- Fidelity and thread reuse behavior follows spec precedence rules.
- Parallel and supervisory flows are stable and test-backed.
- Storage-backed run lineage metadata is available for post-P31 CXDB projection.
