# Transient Provider Errors Should Not Fail A Long Agentic Run

**Status**

- Later / reliability follow-up in `llm-runtime`'s generation path.
- Discovered in production on 2026-08-04 during the first end-to-end
  foundry job (ls.bot managed session developing a pack on the
  `hz02-devbox` environment).
- Deliberately outside P114's scope: P114 bounds *tool* activities; this
  is the LLM generation activity.

## Incident

Session `foundry:v1:hello`, run 1, turn 19 failed with:

```text
core agent LLM generation failed
error=core agent I/O failed: provider call failed: openai:responses
provider HTTP error 503: Our servers are currently overloaded.
```

OpenAI's status page showed an active load incident with mitigation in
progress. One transient upstream 503 discarded an 18-turn agentic run
mid-flight. The session itself survived (event-sourced state is fine) and
a manual `session/runs/start` resumed the work, but the run — and with it
the controller's job — failed, requiring an operator to notice, diagnose,
and resubmit.

## Analysis

The generation activity treats a provider HTTP failure as terminal for
the run. That is correct for auth, validation, and content errors — the
request will never succeed — but wrong for capacity and transport
failures, which are expected to clear within seconds to minutes. Long
agentic runs make the asymmetry expensive: the cost of one lost turn's
retry is trivial next to the cost of failing a run that represents an
hour of durable work and forcing a human resubmission loop.

## Direction

- Classify provider errors in the generation activity: 408/429/5xx and
  connection-level failures (reset, timeout, DNS) are retryable;
  authentication, invalid-request, and content/policy errors stay
  terminal exactly as today.
- Bounded backoff for the retryable class — a few attempts spread over
  one to five minutes, honoring `Retry-After` when present — before
  declaring `runFailed`. No unlimited retries; the run must still reach a
  terminal state on a genuinely down provider.
- The run-terminal failure message must state that retries were
  exhausted (attempt count, elapsed time), so callers can distinguish
  "provider was down for a while" from "this request is broken".
- Controllers (Channels, foundry) then need no provider-retry logic of
  their own; today the foundry would otherwise grow a controller-level
  resubmit-once backstop, which is the wrong layer.
