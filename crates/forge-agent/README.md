# forge-agent

`forge-agent` is the Forge-native agent core SDK crate.

The current implementation target is
[`spec/04-new-agent-spec.md`](../../spec/04-new-agent-spec.md). The crate is
being rebuilt as a deterministic core model plus runner/adapter contracts. The
first cut focuses on the core SDK contracts; it does not yet implement the full
agent loop, tool execution, Temporal workflows, CXDB persistence, or CLI UI.
Host shell/filesystem/process tools are provided by runner/tool packages, not
by this core crate.

The core model is journaled, ref-backed, and snapshot-driven: scoped journal
events describe what happened, artifact refs point at large payloads, and
`SessionState` stays a compact control snapshot for runners.

## Module Layout

Implementation files are grouped by layer:

- `model/`: serializable domain contracts such as ids, events, effects,
  transcript records, context records, tool models, and bounded session state
- `loop/`: deterministic loop machinery such as the journal, reducer,
  decider, planner, and local stepper
- `testing/`: deterministic fake stores, fake effect executors, and reusable
  loop fixtures

The crate keeps root-level public re-exports for the main SDK modules, so
callers can continue to use paths such as `forge_agent::events::AgentEvent`,
`forge_agent::state::SessionState`, and `forge_agent::journal::InMemoryJournal`.

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
