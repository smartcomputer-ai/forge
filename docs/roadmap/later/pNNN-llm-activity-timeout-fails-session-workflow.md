# LLM Activity Timeouts Must Fail The Run, Not The Session Workflow

**Status**

- DONE 2026-08-23 (same day as the observation): the LLM boundary now
  converts pure timeout chains into a failed generation/compaction result;
  unit tests cover the recognizer against SDK-decoded failure protos and a
  slow live test drives a real `llm_generate` through its schedule-to-close
  budget. See "What changed" at the end.
- Written 2026-08-23 from a live observation on the development stack
  while dogfooding Bots; the gap was in the P116 boundary conversion, not
  in Bots.
- Builds on P116 (typed `llm_provider_transient` retries, exhaustion
  converted to a run failure at the workflow boundary) and P114 (bounded
  retries, boundary failure conversion for tool calls).

## Observation

A bot's main session (`AgentSessionWorkflow`, three days old, 68 runs) had
an `llm_generate` activity in flight when the dev stack was stopped. With no
worker heartbeating, the server expired the activity's retries and then its
15-minute schedule-to-close budget:

```text
ACTIVITY_TASK_TIMED_OUT  scheduledId=586
  message:     Not enough time to schedule next retry before activity
               ScheduleToClose timeout, giving up retrying
  timeoutType: TIMEOUT_TYPE_SCHEDULE_TO_CLOSE
  cause:       activity Heartbeat timeout (TIMEOUT_TYPE_HEARTBEAT)
WORKFLOW_EXECUTION_FAILED  "Activity failed: Activity task timed out"
```

On restart the session workflow processed the timeout and **failed the
whole workflow**. The session projection still showed the run as active,
so the promise reaper logged `stale_active_projection` on every pass, and
the session could only be recovered by hand (`session/close` with
`force: true`).

## Cause

`call_llm_generate` and `call_context_compact`
(`crates/temporal-workflow/src/workflows/session/activity_calls.rs`) convert
an activity failure into a failed generation result only when the failure's
cause chain contains the typed `llm_provider_transient` application failure.
A worker outage produces a pure timeout chain (heartbeat → schedule-to-close)
with no application failure in it, so `llm_transient_exhaustion` returns
`None` and the error takes the deliberate "unrecognized errors propagate so
operational bugs stay visible" path — which here means a routine outage kills
a long-lived session.

Tool batches already behave correctly: `boundary_call_status` in
`tool_batches.rs` turns every non-cancelled activity failure into a failed
call result. The LLM path is the exception.

## Fix

1. **Recognize timeout-class failures at the LLM boundary.** In
   `activity_calls.rs`, treat a timeout failure anywhere in the cause chain
   (heartbeat, start-to-close, schedule-to-close) like exhaustion: put a
   bounded boundary error blob ("LLM generation timed out after N attempts —
   worker unavailable or provider hung") and complete with a failed
   `LlmGenerationResult` / `ContextCompactionResult`. Cancellation stays
   cancellation; genuinely unknown application errors keep propagating, so
   the original intent (surface bugs) survives. Replay-safe: only the
   failure path changes, and histories that already hit it are terminal.
2. **Live test** next to
   `temporal_live_exhausted_llm_retries_fail_the_run_not_the_session`: a
   fake `llm_generate` that hangs; assert the run ends `failed`, the
   session workflow survives as the same execution, and a later run on the
   same session succeeds. Runs under `--ignored --test-threads=1` like the
   rest of the live suite.

   Caveat found while building it: the activity options are workflow
   constants, so the timeout chain cannot be produced faster than the real
   schedule-to-close budget (15 min) — a heartbeat-starved or hung attempt
   is retried until that budget ends either way. The live test therefore
   lives in its own binary so the ordinary live suite stays fast.

## Follow-ups (separate items)

- **Recovery, not just prevention.** The reaper detects
  `stale_active_projection` but only logs it. Either the reaper repairs a
  session whose workflow is terminal (fail the projected run, leave the
  session recoverable) or `session/runs/start` recreates a terminal
  workflow. That makes every "workflow died" class self-healing rather than
  this one.
- **Schedule-to-close budget.** `LLM_SCHEDULE_TO_CLOSE` (15 min) is fine
  once the fix lands: an outage longer than that fails the in-flight run,
  which the caller re-runs, instead of killing the session. No change
  proposed; recorded so the behavior is deliberate.

## Non-goals

- Changing retry policy or heartbeat intervals for provider activities.
- Any Bots-side workaround; the bot controller already tolerates a failed
  session (it rotates or re-ensures), and the fix belongs in the core.

## What changed (2026-08-23)

- `crates/temporal-workflow/src/workflows/session/activity_calls.rs`:
  `llm_transient_exhaustion` became `llm_boundary_failure`, returning
  `TransientExhausted(details)` (as before, and still winning when the
  typed failure sits under a schedule-to-close timeout) or
  `TimedOut { timeout_type, cause }` for a chain made only of timeouts.
  Cancellation still propagates, and a chain carrying any other application
  failure — even under a timeout — still propagates, so the "unknown errors
  stay visible" intent survives. The boundary blob for a timeout reads
  `<op> failed: provider activity timed out (schedule-to-close timeout,
  last attempt hit its heartbeat timeout); the worker was unavailable or the
  provider call hung past its budget`. Both `call_llm_generate` and
  `call_context_compact` use it. Unit tests decode hand-built failure
  protos through `DefaultFailureConverter` + `ActivityExecutionDecodeHint`
  so the recognizer is exercised against the exact shape the SDK hands
  workflow code.
- `FakeLlm::with_stall_switch` (`crates/temporal-server/src/worker/fake.rs`)
  hangs generate while an `AtomicBool` is set; the stall is observable
  through the existing started/abandoned counters.
- `crates/temporal-server/tests/runs_live_slow.rs`:
  `temporal_live_llm_activity_timeout_fails_the_run_not_the_session` —
  stalled provider, first run fails after the budget on a pure timeout
  chain, workflow execution unchanged (`run_id` compared), stall cleared,
  second run completes. ~17 minutes; run explicitly:

  ```bash
  source scripts/dev/env.sh
  cargo test -p temporal-server --test runs_live_slow -- --ignored --test-threads=1
  ```

- Follow-ups above (reaper repair / recreate-on-start, the 15-minute
  budget) are unchanged and still open.
