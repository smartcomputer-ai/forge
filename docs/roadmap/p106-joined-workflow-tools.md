# P106: Joined Workflow Tools And Independent Dispatch

**Status**

- Proposed 2026-07-30 after the first production workflow-tool consumer,
  Channels, exposed a missing completion form in P100/P100b.
- Lightspeed implementation advanced through the projection and adoption-proof
  tranche on 2026-07-30: explicit dispatch, pushed Accepted, engine-native
  Joined completion, Temporal reconstruction, shared declaration readback,
  durable event diagnostics, and the serial live proof suite are implemented.
  The five P106 live boundaries passed serially against the local Temporal and
  PostgreSQL stack on 2026-07-30. The external Channels declaration migration
  remains in its consumer repository.
- P100/P100b remain the durable transport and explicit-Promise substrate.
  P106 revises two P100b decisions: delivery is no longer derived from
  completion, and a workflow tool may durably join one semantic reply without
  requiring the model to call `await`.

## Summary

Workflow tools need three caller-visible completion forms:

1. **Accepted** — the invocation is durably recorded. The caller does not own
   a result and may continue immediately.
2. **Joined** — the invocation is durably recorded and the current tool batch
   is parked until one semantic reply, failure, cancellation, or deadline.
   The reply completes the original workflow-tool call. No Promise handle and
   no `await` call are exposed to the model.
3. **Promises** — the invocation creates explicit, model-addressable handles.
   The caller may overlap work, await a subset, cancel, or detach.

These completion forms are independent of how a bound invocation reaches its
receiver:

- **Pull** records the invocation for consumption at a later receiver-owned
  boundary, currently the managed run-terminal boundary.
- **Push** queues the invocation for delivery as soon as the admitting append
  commits.

Receiver identity is not decoupled from the binding. The trusted binding still
fixes the receiver or start recipe. P106 decouples **dispatch timing** from
**caller completion**, because they answer different questions:

- dispatch: when may the receiver begin work?
- completion: when may the caller continue, and who owns the result?

The initial supported matrix is:

| Target | Dispatch | Accepted | Joined | Promises |
| --- | --- | --- | --- | --- |
| Bound receiver | Pull | yes | no | no |
| Bound receiver | Push | yes | yes | yes |
| Start workflow | immediate start | deferred | yes | yes |

`Start + Accepted` is a valid eventual meaning, but remains deferred until a
consumer requires fire-and-forget workflow start. It needs durable start
failure and cleanup policy without a Promise owner; it is not a fourth
completion form.

## Why This Surfaced Now

P100 and P100b were designed and proved around the primitives Lightspeed
already needed:

- P101 Work reports naturally fit `Bound + Pull + Accepted`;
- environment jobs naturally fit `Start + Promises`, because several jobs may
  overlap and their keyed results are independently awaited or cancelled;
- the generic P100b live proofs intentionally exercised explicit Promise
  creation, push delivery, `await`, cancellation, deadlines, and
  continue-as-new.

At that point there was no concrete consumer for pushed acceptance or for a
single result that the caller always needed immediately. P100b therefore made
the simplifying decision that completion determined delivery:

- `Accepted` implied pull at a run boundary;
- `Promises` implied immediate push;
- waiting was always an explicit subsequent `await` tool call.

Channels is the first real model-driven consumer that sits in the missing
quadrant. A message send, edit, or reaction must be pushed to the provider
controller while the run is active, and the agent should normally wait for one
provider receipt before it finishes. There is no useful scheduling decision
for the model between invocation and receipt.

The production session
`channel:v1:telegram:624461713cac588e88d8f4681804ae6c96b7ea915ca81a18967ef60fb21e877b`
on `hz01` made the cost visible. Five completed messaging runs each followed
the same shape:

1. call `message_send` or `message_react`;
2. receive an acknowledgement containing one Promise;
3. spend a second tool batch calling `await`;
4. receive the provider receipt;
5. generate another assistant turn such as “Done.”

The receipt was confirmation, not information used to choose a later action.
The explicit Promise therefore added a model round, another tool call, and
transcript ceremony without adding concurrency or control.

The Channels tool descriptions currently tell the model to await before
reporting success or ending the run. That instruction is required for
correctness under the P100b contract, not merely presentation:

- the Promise-bearing declaration is what selects immediate push;
- an unresolved run-scoped Promise is cancelled when its run terminates;
- the Channels controller cancels the corresponding provider-delivery scope
  when it observes that cancellation;
- fallback suppression can already observe the successful immediate
  messaging-tool acknowledgement.

Without the explicit `await`, the run can finish, cancel the delivery, and
still suppress the fallback response. The application prompt is compensating
for a missing engine semantic.

This did not show up as a transport or durability failure in P100b tests. The
tests proved exactly the contract that was specified. It surfaced only after
an external application put those primitives behind real model tool calls,
where round economy and the distinction between “agent-controlled future” and
“ordinary blocking tool result” became product behavior.

This is also the intended benefit of the P100/P103 extension boundary. The
first external system found a generic engine gap rather than requiring a
Channels-specific session-worker path.

## Goals

1. Add `Joined` as a durable, engine-native workflow-tool completion form.
2. Let a bound receiver be pushed independently of whether completion is
   `Accepted`, `Joined`, or `Promises`.
3. Complete a joined invocation as the original tool call, with no synthetic
   Promise acknowledgement or model-authored `await` call.
4. Reuse P92/P94 Promise resolution, deadlines, cancellation, wake evaluation,
   and Temporal reconstruction rather than creating a blocking workflow RPC.
5. Preserve P100's trusted binding, producer authorization, deterministic
   identities, atomic admission, retry, deduplication, and replay invariants.
6. Keep explicit Promises unchanged for genuine asynchronous work.
7. Keep the engine deterministic and Temporal-neutral.
8. Make the P103 managed API able to declare the full generic contract needed
   by external applications such as Channels.

## Non-Goals

P106 does not add:

- a blocking cross-workflow activity or synchronous `tool_invoke_batch`;
- model-selectable targets, dispatch modes, completion modes, deadlines, or
  schemas;
- receiver identity independent of the immutable trusted binding;
- implicit joining of existing `Promises` declarations;
- progress events, streaming results, or ordered pushed notification streams;
- workflow return-value decoding as a second completion protocol;
- channel-specific envelopes, receipts, fallback behavior, or worker code;
- a default detach policy for joined calls;
- changes to ordinary local, MCP, Fleet, or environment tools;
- fire-and-forget start-on-call workflows in the first implementation;
- mutation of an existing managed session's workflow-tool bindings.

## Completion Semantics

### Accepted

`Accepted` means only that the invocation and its required dispatch intent are
durably recorded in the producer session.

- The tool result may acknowledge deterministic invocation identity.
- The caller owns no result and receives no Promise.
- Receiver completion, rejection, or later delivery failure does not revise
  the already-completed tool call.
- A pushed Accepted invocation is not cancelled merely because the producing
  run reaches terminal state.
- Force-close may stop retrying only after durable terminal delivery failure
  is recorded according to the existing close policy.

For `Bound + Pull + Accepted`, existing P100 behavior remains unchanged: the
receiver learns the run boundary and pulls authorized invocation pages.

For `Bound + Push + Accepted`, the invocation enters the existing durable
emission queue immediately after commit. This is true fire-and-forget delivery
to the receiver, not a disguised Promise.

### Joined

`Joined` means the tool invocation has exactly one semantic reply and the
calling run cannot make further model progress until that reply reaches a
terminal outcome.

The declaration contains:

- an optional reply schema reference;
- a required non-zero hard deadline.

Admission creates a deterministic internal completion key, reserved as
`reply`, and one ordinary durable workflow Promise. The Promise is
runtime-owned:

- it is visible to operators and source-resolution machinery;
- it is not included in the model-visible tool acknowledgement;
- it is not accepted by model-facing `await`, `cancel`, or `detach` tools;
- it remains run-scoped and is cancelled by run/session cancellation;
- its deadline uses the existing hard Promise deadline machinery.

The original workflow-tool call remains incomplete while the batch is parked.
When the internal Promise resolves:

- a successful, schema-valid payload becomes the original call's normal tool
  result;
- source failure becomes the original call's tool error;
- deadline or cancellation becomes the original call's corresponding terminal
  error outcome;
- the run resumes only after those results are durably appended.

There is no model-visible transport acknowledgement and no synthetic
“Promise resolved” user message. On the next model step, the transcript looks
like a normal tool call followed by its result.

The receiver may resolve the reply before the producer has finished applying
its parked state. The wake path must evaluate already-terminal Promises and
resume deterministically, as P94 already requires for explicit awaits.

### Promises

Existing P100b semantics remain unchanged:

- completion keys are derived from validated arguments;
- Promise identities are returned to the model;
- the run may continue before any Promise resolves;
- the model may await all or a selected subset, cancel owned work, or detach
  where policy permits;
- several independently useful partial results use keyed Promises, not
  `Joined`.

An operation with one reply is not automatically Joined. Use `Promises` when
the caller has a meaningful choice to overlap work, select when to wait,
cancel independently, or transfer ownership.

The declaration rule is therefore:

- no caller-owned result: `Accepted`;
- one result required before the caller can reason again: `Joined`;
- a result whose timing or ownership the caller can usefully control:
  `Promises`.

## Why Dispatch And Completion Are Separate

P100b coupled dispatch and completion because its two known cases happened to
line up. That relationship is accidental:

- Pull versus push concerns receiver scheduling and wake-up.
- Accepted versus Joined versus Promises concerns caller control and result
  ownership.

Keeping them coupled makes valid behaviors impossible or misrepresents them:

- a notification may need immediate push but no result;
- a message send may need immediate push and one ordinary result;
- an approval request may need immediate push but remain explicitly
  asynchronous;
- a work report may remain pull-consumed and require no result.

The separation also avoids semantic overload. `Accepted` must not mean “pull”;
it must mean “the caller no longer owns completion.” `Promises` must not mean
“push”; it must mean “completion is represented by explicit handles.”

Dispatch remains part of `Bound` rather than a top-level field so the type
cannot express meaningless `Start + Pull` declarations. Start targets are
always admitted for immediate deterministic start.

## Contract Changes

The engine contract becomes conceptually:

```rust
pub enum WorkflowToolTarget {
    Bound {
        receiver: WorkflowEndpointRef,
        dispatch: BoundWorkflowToolDispatch,
    },
    Start {
        start: WorkflowStartRef,
    },
}

pub enum BoundWorkflowToolDispatch {
    Pull,
    Push,
}

pub enum WorkflowToolCompletion {
    Accepted,
    Joined {
        reply_schema_ref: Option<BlobRef>,
        /// Required and validated as non-zero at admission.
        deadline_after_ms: u64,
    },
    Promises {
        reply_schema_ref: Option<BlobRef>,
        deadline_after_ms: Option<u64>,
        max_promises: u32,
        key_source: WorkflowPromiseKeySource,
    },
}
```

The P103 managed-session declaration becomes conceptually:

```ts
type WorkflowToolTarget =
  | {
      type: "bound";
      receiver: WorkflowEndpointRef;
      dispatch: "pull" | "push";
    }
  | {
      type: "start";
      start: WorkflowStartRef;
    };

type WorkflowToolCompletion =
  | { type: "accepted" }
  | {
      type: "joined";
      replySchemaRef?: BlobRef;
      deadlineAfterMs: number;
    }
  | {
      type: "promises";
      replySchemaRef?: BlobRef;
      deadlineAfterMs?: number;
      maxPromises: number;
      keySource: WorkflowPromiseKeySource;
    };
```

New admissions must specify bound dispatch explicitly. The server must not
silently derive it from completion for new declarations.

The binding fingerprint domain advances from
`lightspeed.workflow-tool.binding.v3` to v4 and all fingerprint goldens are
repinned. Dispatch and Joined fields participate in canonical encoding.

### Admission Rules

Admission rejects:

- `Bound + Pull + Joined`;
- `Bound + Pull + Promises`;
- `Joined` without a non-zero hard deadline;
- `Joined` with an invalid or unavailable reply schema;
- a lifecycle-controller self-receiver that cannot satisfy the existing
  independent-processing progress contract;
- `Start + Accepted` until that deferred combination is implemented.

`Bound + Push + Accepted` has no completion schema or deadline.

`Bound + Push + Joined` and `Bound + Push + Promises` use the existing
self-receiver progress law when the receiver is also the lifecycle controller:
it must accept and process invocations independently of run-terminal handling
and must not re-enter or await the emitting session.

## Durable Join, Not A Blocking Receiver Call

“Joined” describes the caller's observable semantics. It must not make the
tool activity wait for another workflow.

A blocking implementation would be incorrect because an activity wait is not
the source of truth for session state, cannot safely span long retries or
continue-as-new, obscures cancellation ownership, and makes replay recovery
depend on a live worker call stack.

P106 instead uses the existing two-hop suspension architecture:

1. The tool executor validates arguments and returns deterministic admission
   intent. It performs no workflow I/O.
2. One producer-session append durably creates the internal Promise, records
   the pushed emission or start intent, and parks the tool batch with a typed
   joined-call suspension. It does **not** append `Tool::CallCompleted` for the
   joined call.
3. Only after that append commits may the Temporal workflow dispatch the
   emission or start the target workflow.
4. The receiver resolves or fails the Promise through the existing
   producer-authorized source protocol.
5. The existing wake evaluation observes Promise terminality and proposes a
   typed resume command.
6. The engine validates the claim and atomically appends the original call's
   result, context entry, and batch/run continuation.

The activity that invoked the tool returns quickly at step 1. The session may
remain parked for seconds or minutes, survive worker restart and
continue-as-new, and resume from log-backed state.

This is why receiver dispatch and caller completion must be decoupled in the
contract while still being joined by durable engine state. The receiver is an
independent workflow with its own retries and lifetime; completion is a fact
that the producer consumes later. Conflating them into one synchronous call
would discard the reliability boundary P100 was built to provide.

## Engine Suspension Shape

P94 deliberately made await a typed engine primitive rather than an opaque
tool result. Joined workflow calls introduce a second legitimate reason for a
tool batch to park. The state should generalize without returning to an opaque
resume directive.

One acceptable shape is:

```rust
pub struct ParkedToolBatch {
    pub batch_id: ToolBatchId,
    pub suspension: ToolBatchSuspension,
}

pub enum ToolBatchSuspension {
    AwaitTool {
        call_id: ToolCallId,
        spec: AwaitSpec,
    },
    JoinedWorkflowCalls {
        calls: Vec<JoinedWorkflowCall>,
        spec: AwaitSpec,
    },
}

pub struct JoinedWorkflowCall {
    pub call_id: ToolCallId,
    pub invocation_id: WorkflowToolInvocationId,
    pub promise_id: PromiseId,
}
```

The exact Rust names may differ, but the invariants may not:

- parked state is typed, durable engine state;
- every joined call maps one original call ID to one internal Promise;
- wake claims are reconstructed and validated from engine state;
- the runtime cannot supply arbitrary tool results on resume;
- explicit await keeps its current output and summary behavior;
- joined resume constructs results directly from validated Promise terminal
  values and binds them to the original workflow-tool call IDs.

`ActiveRun.parked_await` should therefore become a typed parked-tool-batch or
parked-suspension field. `ResumeAwait` may similarly become a typed
`ResumeSuspension` command. The rename is justified by a real second engine
suspension source; it must not restore P94's deleted opaque resume surface.

### Atomic Admission Invariant

The current workflow-emission invariant assumes that a successful workflow
tool call completes in the same append as `WorkflowTool::Emitted`. Joined
requires that invariant to become completion-aware:

- Accepted and Promises still require a successful same-append
  `Tool::CallCompleted` before their invocation becomes dispatchable.
- Joined requires an active matching call and a same-append typed parked batch;
  it forbids `Tool::CallCompleted` until resume.

The append for a joined bound call includes, in deterministic order:

1. creation of the runtime-owned Promise;
2. the typed batch-deferred event containing the call-to-Promise mapping;
3. the workflow-tool invocation/emission with its internal reply mapping.

That order lets application of the invocation fact prove that its call is
already durably parked rather than merely trusting that a later proposal will
park it.

For a start target, the corresponding deterministic start intent replaces the
bound emission. No receiver dispatch or workflow start may occur before this
append commits.

### Parallel And Mixed Tool Batches

P106 supports several Joined workflow calls in one provider tool batch. The
batch parks once with `AwaitMode::All`; each original call receives its own
result when all joined replies are terminal. Completed non-suspending calls in
the same batch are durably retained using the existing deferred-batch
behavior.

For the first implementation, a provider batch containing both an explicit
`await` call and one or more Joined workflow calls is invalid because it asks
one batch to express two suspension owners. The engine returns deterministic
tool errors for the suspension-producing calls rather than choosing an
ordering. This restriction may be relaxed later with a typed composite
suspension if a real consumer needs it.

## Delivery, Failure, And Cancellation

### Push Delivery

Both pushed Accepted and pushed Joined invocations use P100b's existing
durable pending-emission reconstruction, bounded retries, receiver
authorization, and invocation deduplication.

- Accepted delivery failure records `WorkflowTool::DeliveryFailed` for
  operators. It does not retroactively fail the completed caller tool call.
- Joined delivery failure fails its internal Promise and therefore completes
  the original caller tool call with an error.
- Promise-bearing delivery failure keeps existing P100b behavior.

Independent per-invocation delivery remains sufficient. P106 does not add
per-receiver FIFO or a high-water notification stream.

### Deadlines

Every Joined declaration has a non-zero hard deadline because the model
cannot cancel, detach, or choose another await. A broken receiver must not park
the managed run forever.

The deadline begins at the same durable point as existing workflow Promises.
Deadline expiry wins and resolves exactly as defined by P92/P100b, including
late duplicate-resolution rejection.

### Cancellation

- Run or session cancellation cancels the runtime-owned Joined Promise and
  emits the existing source cancellation for the exact invocation/key.
- The target workflow or bound receiver remains responsible for domain cleanup
  and start-versus-cancel races.
- A Joined Promise cannot be detached or directly cancelled by a model tool.
- A pushed Accepted invocation has no completion owner and is not cancelled at
  run terminal. Force-close and administrative teardown follow explicit
  delivery-queue policy; they must not silently erase admitted work.

### Early Reply

A receiver or started workflow may resolve the semantic reply and continue
running. The Promise resolution, not target workflow termination, completes
the Joined call. This is an execution-lifecycle policy of the target and does
not require another completion form.

## Projection And Toolset Behavior

The model-visible result for Joined is the semantic reply payload or normal
tool error on the original call. It must not contain:

- the internal Promise ID;
- a transport acknowledgement that asks the model to wait;
- a synthetic await summary;
- instructions to call a concurrency tool.

Operator APIs expose the immutable target, dispatch mode, and deadline through
the same managed workflow-tool declaration DTO accepted at creation. They do
not maintain a second aggregate workflow-tool status view. Per-invocation
diagnostics remain on `session/events/read`, whose durable facts expose:

- original batch and call IDs;
- workflow-tool invocation ID;
- internal Promise ID and source;
- Promise resolution/cancellation and terminal delivery or start failure.

Those operator fields do not grant model ownership.

A session containing only Accepted and Joined workflow tools must not gain
`await`, `cancel`, or `detach` merely because Joined uses an internal Promise.
Concurrency-toolset derivation continues to consider explicit model-owned
Promises and other asynchronous capabilities only.

## Compatibility And Rollout

Workflow-tool bindings are immutable managed-session creation state. Changing
a Channels binding from `Promises` to `Joined`, or adding explicit `dispatch`,
must not mutate an existing session under the same identity.

P106 takes a greenfield cutover. Existing P100/P100b managed sessions and
histories may be retired and recreated; the implementation does not carry a
legacy decoder, derive missing dispatch during replay, or accept a silent
dispatch default. New P103 admission requests must provide explicit bound
dispatch, and the runtime writes only v4 bindings.

Channels adoption uses a new immutable binding/session version where required
by its controller identity scheme. No existing session is reinterpreted
mid-history.

## Implementation Order

P106 lands in three implementation tranches rather than exposing each numbered
design slice independently:

1. **Dispatch foundation — the dispatch portion of P106.1 plus P106.2.** Add
   explicit bound dispatch across engine and P103 contracts, cut fingerprints
   directly to v4, and prove `Bound + Push + Accepted`. `Joined` is not yet
   admitted through the managed API in this tranche because its durable
   suspension path does not exist yet.
2. **Joined vertical slice — the remainder of P106.1 plus P106.3, P106.4, and
   the model-isolation portion of P106.5.** Add runtime-owned Promises, typed
   tool-batch suspension, original-call resume, Temporal reconstruction, and
   exclusion from model-facing concurrency operations before enabling Joined
   admission.
3. **Projection and adoption proof — the remainder of P106.5 plus P106.6.**
   Reuse the managed declaration DTO for readback, retain per-invocation
   diagnostics on the durable event stream, and add the generic Lightspeed live
   proofs. Migrate Channels separately in its consumer repository.

## Implementation Slices

### P106.1 — Contracts And Admission

Implemented 2026-07-30 using the greenfield v4 cutover.

- Add `BoundWorkflowToolDispatch` and `WorkflowToolCompletion::Joined`.
- Add explicit dispatch and Joined decoding to P103 DTOs.
- Enforce the supported matrix and hard-deadline rules.
- Add runtime ownership for internal Joined Promises.
- Advance the binding fingerprint domain to v4 and repin goldens.
- Use the greenfield recreate/cutover path; do not add legacy replay decoding.

### P106.2 — Pushed Accepted

Implemented 2026-07-30 on the durable emission/retry spine.

- Queue `Bound + Push + Accepted` through existing emission reconstruction.
- Keep the caller result as durable acceptance only.
- Do not auto-cancel delivery at run terminal.
- Record terminal delivery failure without revising the caller result.
- Cover restart, continue-as-new, deduplication, and close behavior.

### P106.3 — Engine-Native Join

Implemented 2026-07-30.

- Generalize parked await state to a typed tool-batch suspension.
- Admit Joined invocations without initially completing their calls.
- Atomically create the internal Promise, invocation, dispatch intent, and
  parked mapping.
- Resume original call IDs from validated Promise terminal values.
- Support multiple Joined calls and completed ordinary calls in one batch.
- Reject explicit-await-plus-Joined batches deterministically in v1.

### P106.4 — Temporal Reconstruction And Dispatch

Implemented 2026-07-30 for log-reconstructed parked joins, Promise wake and
deadline paths, post-commit push/start dispatch, and continue-as-new-safe
state. Serial live boundary proofs remain in P106.6.

- Reconstruct parked joins and pending push/start work across workflow replay,
  worker restart, and continue-as-new.
- Reuse existing Promise wake/deadline/source-resolution paths.
- Make already-terminal-before-park replies wake correctly.
- Ensure dispatch starts only after the admitting append commits.
- Preserve self-receiver progress and absent-receiver behavior.

### P106.5 — API, Projection, And Tool Context

Implemented 2026-07-30 using shared managed-declaration readback and the
existing durable event projection; no parallel aggregate workflow-tool status
view is introduced. Generated contract coverage is current.

- Expose the new declarations through managed-session admission and reads.
- Project joined state for operators without granting model ownership.
- Emit normal tool results on original call IDs.
- Exclude internal Joined Promises from concurrency-toolset derivation and
  model-facing Promise operations.

### P106.6 — External Adoption Proof

Implemented and passed serially in the `workflow_tool_plugins_live` suite on
2026-07-30.

- Add a generic Channels-shaped live proof in Lightspeed: pushed bound call,
  one provider-style receipt, no explicit `await`, and completion before the
  lifecycle controller consumes run terminal.
- Prove `Start + Joined` with an early semantic reply.
- Prove `Bound + Push + Accepted` delivery while the run continues and after
  the producing run reaches terminal.
- Migrate Channels declarations separately after P106 lands; that external
  consumer repository is not part of this workspace.

## Required Tests

### Unit And Reducer Tests

- canonical encoding and v4 fingerprint coverage for every valid form;
- rejection of every invalid target/dispatch/completion combination;
- deterministic internal `reply` Promise identity;
- internal Promise rejection by model-facing await/cancel/detach;
- no Joined `Tool::CallCompleted` in the admission append;
- atomic Promise, invocation, dispatch, and parked-state admission;
- successful Joined resolution completes the original call ID;
- reply-schema failure, source failure, cancellation, deadline, and delivery
  failure become deterministic original-call errors;
- early resolution before wake observation;
- first-terminal-writer-wins duplicate resolution;
- multiple Joined calls use all-of wake and preserve call/result mapping;
- mixed completed ordinary calls survive the parked batch;
- explicit await plus Joined rejection;
- pushed Accepted result is not revised by delivery failure;
- pushed Accepted is not cancelled on producing-run terminal;
- concurrency tools are not derived from Joined alone.

### Workflow And Replay Tests

- receiver absent, retrying, duplicated, and terminally failing;
- worker restart at each append/dispatch/resolution boundary;
- continue-as-new while a join is parked and while a push is pending;
- run cancellation before dispatch, during receiver work, and after reply;
- deadline versus reply and cancellation versus reply races;
- self-receiver progress with resolution before run terminal;
- start-on-call deduplication and early reply;
- legacy P100b history replay or explicit versioned cutover, according to the
  selected compatibility path.

### Live Temporal Proof

One serial live suite must demonstrate:

1. `Bound + Push + Joined` produces one ordinary semantic tool result with no
   explicit `await` tool call or Promise acknowledgement in model context.
2. The same lifecycle controller may resolve the reply while the run is
   active and later consume run terminal without re-entrancy.
3. Continue-as-new while parked preserves the original call mapping and
   deadline.
4. `Bound + Push + Accepted` reaches the receiver without keeping the run
   alive and is not abandoned at run terminal.
5. `Start + Joined` resolves from a deterministic started execution.

## Acceptance Criteria

P106 is complete when:

1. Bound dispatch is explicit, fingerprinted, immutable, and no longer derived
   from completion for new admissions.
2. `Accepted`, `Joined`, and `Promises` have the caller semantics defined in
   this document across engine, API, logs, projection, and Temporal runtime.
3. A Joined workflow tool parks only through durable engine state; no activity
   or workflow callback blocks waiting for the receiver.
4. Joined success or failure completes the original tool call and requires no
   model-visible Promise or subsequent `await`.
5. Joined internal Promises cannot be controlled by model-facing concurrency
   tools but retain P92 deadline, cancellation, source authorization, and
   recovery behavior.
6. Pushed Accepted invocations are delivered independently of run terminal and
   do not acquire result ownership.
7. Multiple Joined calls, mixed ordinary results, cancellation races,
   deadlines, retries, restart, replay, and continue-as-new are covered.
8. Existing explicit Promise workflows, including environment jobs, retain
   their current behavior.
9. A generic Channels-shaped live proof closes without an explicit `await`
   tool call and without weakening delivery correctness.

## Follow-On Boundary

The three completion forms are expected to be sufficient:

- many independently usable results are keyed `Promises`;
- a joined multi-step receiver aggregates one semantic reply;
- a receipt with no payload is `Joined` with an empty/void schema;
- progress and streaming are observation protocols, not completion forms;
- fire-and-forget work is `Accepted`, or explicit `Promises` plus detach when
  ownership transfer is intentional;
- target termination versus early reply is target lifecycle policy.

Future work may add ordered pushed notifications, composite suspension within
one provider batch, or `Start + Accepted` when a concrete consumer defines the
required failure and cleanup policy. None requires another caller-visible
completion category.
