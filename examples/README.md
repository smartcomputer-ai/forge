# Forge CLI Example DOT Pipelines

These examples are ordered from simple to more complex and are runnable with `forge-cli`.

## 1) `01-linear-foundation.dot`
A minimal linear pipeline: `start -> plan -> summarize -> exit`.
Use this first to verify parse/execute behavior.

Run:
```bash
cargo run -p forge-cli -- run --dot-file examples/01-linear-foundation.dot
```

## 2) `02-hitl-review-gate.dot`
Adds human-in-the-loop review with `wait.human` and a feedback loop.

- `[A] Approve` routes to ship.
- `[R] Request Changes` routes to refine and back to review.

Run with interactive prompts:
```bash
cargo run -p forge-cli -- run --dot-file examples/02-hitl-review-gate.dot --interviewer console
```

Run non-interactively with queued answers:
```bash
cargo run -p forge-cli -- run --dot-file examples/02-hitl-review-gate.dot --interviewer queue --human-answer A
```

## 3) `03-parallel-triage-and-fanin.dot`
Shows parallel fan-out (`component`), fan-in (`tripleoctagon`), and a final human decision gate.

- Parallel branches: logs/tests/recent changes.
- Fan-in stage consolidates branch outcomes.
- Human selects fast fix or deep fix.

Run with event JSON stream:
```bash
cargo run -p forge-cli -- run --dot-file examples/03-parallel-triage-and-fanin.dot --event-json --interviewer queue --human-answer F
```

## Resume and Inspect (any example)
Use `run` with `--logs-root` first, then inspect/resume from checkpoint.

```bash
cargo run -p forge-cli -- run --dot-file examples/01-linear-foundation.dot --logs-root .tmp/linear
cargo run -p forge-cli -- inspect-checkpoint --checkpoint .tmp/linear/checkpoint.json
cargo run -p forge-cli -- resume --dot-file examples/01-linear-foundation.dot --checkpoint .tmp/linear/checkpoint.json
```
