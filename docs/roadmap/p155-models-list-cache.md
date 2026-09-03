# P155 — `models/list` Discovery Cache

**Status**

- Proposed 2026-09-03; moved out of [P153](p153-session-metadata.md), where
  it was recorded as an aside during the first Harbor evaluation
  ([P149](p149-harbor-end-to-end-agent-evaluation.md)).

## Problem

`models/list` queries every provider live on each call. An evaluation job
that starts 12 trials at once issues 12 provider catalog requests within
seconds, and the first hosted trial hit a transient provider 500 that way.

## Decision

- A per-universe, per-provider cache of the discovery result with a short
  time to live, 10 to 30 seconds, collapses the burst to one upstream
  request without making the catalog stale in any way a client could
  notice. This is a real TTL: it counts from insertion, unlike session
  retention ([P154](p154-session-retention.md)).
- Failures are cached too, with a shorter negative TTL, so an outage is
  retried but not hammered.
- Credentials differ per universe, so the cache key is the universe and
  the provider.

## Acceptance

- Twelve concurrent `models/list` calls in one universe produce one
  upstream request per provider.
- A provider failure is returned to callers inside the negative TTL and
  retried after it.
- Two universes never share an entry.
