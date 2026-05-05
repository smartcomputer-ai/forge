# forge-agent

`forge-agent` is the Forge-native agent core SDK crate.

The current implementation target is
[`spec/04-new-agent-spec.md`](../../spec/04-new-agent-spec.md). The crate is
being rebuilt as a deterministic core model plus runner/adapter contracts. The
first cut focuses on the core SDK contracts; it does not yet implement the full
agent loop, tool execution, Temporal workflows, CXDB persistence, or CLI UI.
Host shell/filesystem/process tools are provided by runner/tool packages, not
by this core crate.

## Core Modules

- `ids`: durable ids and allocation helpers
- `lifecycle`: session/run/turn lifecycle states and transition rules
- `config`: session, run, turn, and extension configuration records
- `refs`: artifact and transcript references for large payloads
- `transcript`: transcript ledger and message records
- `context`: context windows, token counts, pressure, and compaction records
- `turn`: turn inputs, plans, reports, and resolved turn context snapshots
- `tooling`: tool specs, profiles, observed calls, and planned calls
- `batch`: active tool-batch state and per-call statuses
- `effects`: effect intents, receipts, and stream frames
- `events`: input, lifecycle, effect, and observation events
- `state`: session/run state, pending effects, queues, forks, and rewrites
- `trace`: bounded run trace records
- `projection`: stable CLI/JSONL/web projection items
- `subagent`: parent/child session metadata

## Deferred Surfaces

Hooks, approval flows, permission grants, sandbox policy review, and dynamic
tool loading are future SDK extension surfaces. They are not part of the
first-cut core model.

## Build

```bash
cargo build -p forge-agent
```

## Tests

Run deterministic tests:

```bash
cargo test -p forge-agent
```
