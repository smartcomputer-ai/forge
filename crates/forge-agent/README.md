# forge-agent

`forge-agent` is the coding-agent-loop library for Forge, built on top of `forge-llm`.

This crate is the implementation target for `spec/02-coding-agent-loop-spec.md`. The initial foundation includes a public module layout for:

- session orchestration (`session`)
- lifecycle/events (`events`, `turn`, `config`)
- provider profiles (`profiles`)
- tool registry and dispatch primitives (`tools`)
- execution environment abstraction (`execution`)
- truncation/context safeguards (`truncation`)

## Build

```bash
cargo build -p forge-agent
```
