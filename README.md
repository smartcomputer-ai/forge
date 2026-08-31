<p align="center">
  <img src="docs/images/logo.png" alt="Lightspeed logo" width="100">
</p>

# Lightspeed

Lightspeed is a powerful agent harness built for durable workflow engines. It allows you to run complex agents and sub-agents that survive restarts, run for months, and scale to thousands, without needing a dedicated VM for each one.

[Temporal](https://temporal.io/) is fully supported today; others are coming soon: [Restate](https://www.restate.dev/), [Inngest](https://www.inngest.com/), Hatchet, AWS Step Functions, etc. The core is written in Rust. The production data backend is Postgres and optional S3.

## Why?

**The goal of Lightspeed is to build as powerful an agent as Claude Code, Codex, or OpenClaw, but running _outside_ operating systems, thus separating the harness from compute. Plus, making this tenable for workflow engines**.

Concretely, that means the harness—the agent loop, context management, session state—runs as a lightweight durable workflow, while OS-level work (shells, code execution, full file systems) happens on machines the agent attaches to only when a task needs them. The result: thousands of agents managed by a single worker node.

<p align="center">
  <img src="docs/images/readme-why-overview.png" alt="Comparison: on the left, one OS per agent — four large VMs each hosting a single tiny agent; on the right, Lightspeed with dozens of agents packed into one worker node, borrowing VMs and sandboxes via dashed connections only when needed" width="750">
</p>

Frontier agent harnesses like Claude Code, Codex, OpenCode, or OpenClaw are designed to run inside a guest OS and need an entire OS for themselves, which makes them difficult to scale and secure. Hence the emerging pattern to ["separate the harness from compute"](https://openai.com/index/the-next-evolution-of-the-agents-sdk/#:~:text=long%2Drunning%20task.-,Separating%20harness%20from%20compute%20for%20security%2C%20durability%2C%20and%20scale,-Agent%20systems%20should), and to run agents inside workflow engines for durability. This is especially interesting in enterprise or shared deployments, where you cannot easily co-locate agents on the same VM.

But most agent SDKs are not designed for workflow engines: they do not separate the deterministic core from effects such as LLM or tool calls, and they pass too much data between the core workflow logic and the effectful "tasks" or "activities"—e.g. the entire chat history back and forth—which bloats workflow histories.

One caveat we take seriously: frontier models are optimized to the hilt (via RL) assuming they control a full POSIX-compatible OS, so an agent with just MCPs and provider-native tools will underperform one with a real machine. Bridging that gap is a central goal of Lightspeed: agents can borrow compute (dedicated VMs via a bridge daemon, ad-hoc sandboxes, delegated coding-agent jobs) while the harness stays outside the OS.

**What you can build with Lightspeed**:

- An insanely **scalable OpenClaw-style personal assistant**: thousands of users, very low cost (besides tokens)
- A fully **autonomous software factory**: a coordinator that runs sub-agents to build, test, and critique your next feature — and keeps running for weeks
- **Research agents** that spin up compute for long-running experiments, stay live for days, and supervise progress
- ...and much more!

## Quick start

You need Rust with edition 2024 support, Node.js 24 or newer, and Docker with
Compose. Then configure at least one model provider and start the complete
local product:

```bash
cp .env.example .env
# Set OPENAI_API_KEY or ANTHROPIC_API_KEY in .env
./dev.sh
```

When the readiness checks pass, open
[http://localhost:5173/app/](http://localhost:5173/app/) and sign in with the
development account printed by the launcher. The defaults are
`admin@lightspeed.dev` and `lightspeed-dev-password`.

That is the supported happy path. The launcher installs npm dependencies when
needed, starts the local infrastructure and editable application processes,
applies migrations, and waits until the product is ready.

For focused profiles, lifecycle commands, provider-free startup, connector
configuration, manual runtime roles, local service addresses, resets, and live
test setup, see the [development environment guide](scripts/dev/README.md).
Environment variables are documented separately in
[docs/variables.md](docs/variables.md).

## Features

Lightspeed covers the table stakes of a modern agent harness and keeps shipping fast. Everything below works today — run it with the [Quick start](#quick-start).

**Models & providers**

- [x] **OpenAI and Anthropic, provider-native**: reasoning traces, native compaction, advanced tool configs, provider tools, files and images, OAuth login, multiple API keys
- [x] **OpenAI-compatible providers**: OpenRouter, DeepSeek, vLLM, Ollama, and
  similar servers, each configured with its own endpoint and credential
- [x] **Prompt caching**: cache breakpoints and cache keys are placed
  automatically, context is append-only where it counts, and cache usage is
  reported per run

**Agent capabilities**

- [x] **Virtual file system**: dedicated `vfs_*` tools read and edit linked
  snapshots/workspaces without an OS attached
- [x] **Web access**: fetch, search, and extract tools
- [x] **Skills**, automatically cataloged and loaded from linked VFS roots
- [x] **Hosted MCP**, with universe-configured API-key and OAuth identities
  and server-owned tool/approval policy shared by every session selecting that
  MCP server id; the management UI discovers its current tools live without
  storing an inventory
- [x] **Flexible prompt & instruction configuration**
- [x] **Sub-agents**: `agent_run` / `agent_spawn` over allowlisted profiles, supervised by an execution workflow, with root-scoped limits and typed lineage
- [x] **Agent profiles**: reusable session setups, shared across clients and sub-agents;
  a profile can activate an existing environment or provision a fresh one per session

**Bots & channels**

- [x] **Bots**: durable event routers that own their sessions. Triggers fire on
  schedules, webhooks, polls, or chat messages; events are filtered, coalesced,
  and delivered under budgets and flood breakers
- [x] **A numbered event log** per bot: every event gets a `#N` handle and a
  write-once outcome the model records itself
- [x] **Bot federation**: bots address each other through inbox triggers with
  `bot_emit`, bounded by hop and rate limits, with deterministic reply receipts
- [x] **Chat channels**: Telegram and WhatsApp today through a thin connector
  host. A chat pairs to a bot once and keeps that route; replies, media, and
  typing indicators flow both ways
- [x] **Open channel model**: a new chat provider is a new connector, never a
  core change

**Durability & scale**

- [x] **Long-running agents**: sessions that last weeks to months and survive restarts
- [x] **Active-run control**: cancel a run (in-flight model and tool calls are
  aborted, no farewell turn), steer it with a message the model sees at its
  next turn, or queue the next message behind it
- [x] **Session fork & clone**: cheap forks of a running agent's full state, straight from the event-sourced log
- [x] **Managed sessions and workflow-backed tools**: external workflows create
  sessions and add durable tools — deliveries, keyed completions, deadlines,
  cancellation — against the generated
  [workflow contract](crates/temporal-workflow/contract/workflow-contract.md)
- [x] **Eval harness** for regression-testing agent and tool workflows
- [x] **Timers, schedules, wake-ups**
- [x] **One binary, every role**: `lightspeed-server` runs gateway, sessions,
  bots, and channels together by default, or split per role and task type

**Borrowed compute**

- [x] **Dedicated VMs**: sessions run their file and process tools on a selected environment, provisioned by the in-repo Incus provider or attached as an existing machine
- [x] **Power states and idle policy**: environments pause, suspend, or stop on intent or after staged idle timeouts, and wake transparently on next use
- [x] **Environment jobs**: long-running work (downloads, experiments, delegated coding agents) runs as provider-owned jobs, exposed to the model as a default-off grant

**Security & auth**

- [x] **Encrypted secrets**: AEAD-encrypted secret store, plus an OAuth token broker with automatic refresh
- [x] **Credential injection**: secrets reach environments and jobs without ever being exposed to the model
- [x] **Multi-tenant by default**: universes isolate tenants on one deployment,
  and dedicated per-tenant deployments share the same platform
  ([docs](docs/multi-tenancy.md))

**Interfaces**

- [x] **Web app**: manage universes, sessions, profiles, bots, and channels
  from the browser
- [x] **Typed JSON-RPC API**: committed schema contract, generated TypeScript client
- [x] **Configurator MCP**: a configurable universe API surface as generated tools over
  Streamable HTTP
- [x] **CLI** to connect to running agent sessions

The generated [JSON-RPC API reference](crates/api/contract/api-reference.md) is
derived from the same Rust manifest and schemas that drive OpenRPC, the
TypeScript client, and Configurator MCP tool descriptions.

## Design

At the heart of every agent is a carefully engineered state machine that manages what goes into the context window of the LLM.

In Lightspeed, that state machine is an event-sourced, deterministic core: it replays a session's event log into state, decides the next step, and emits effect _intents_ that runtime adapters execute against real LLM providers and tools. The core itself performs no I/O, which is exactly the shape that plays well with durable workflow engines.

Two more decisions make this practical inside a workflow engine:

1) **Minimal provider abstraction.** We extract only the information needed to decide and branch inside the deterministic core; provider-native data stays opaque and blob-backed, instead of being converted into a fake universal LLM message model.
2) **Offloading to CAS.** All data not directly needed by the workflow logic goes to content-addressed storage, so the payloads passed between workflow and activities are extremely thin and the workflow history stays small.

Lightspeed's plugin infrastructure lets external workflows add durable tools
to an agent. A plugin can create and manage a session, provide tools backed by
its own workflows, and rely on Lightspeed to deliver calls, wait for results,
handle timeouts, and cancel work. Plugins stay independent from the core
session worker.

Bots — durable event routers that own managed sessions — and the core of
Channels (chat conversations as bot triggers) run inside the same runtime:
`bots/*` and `channels/*` are ordinary API methods, bot controllers and
conversation workflows are Temporal workflows of the same server, and the
Telegram/WhatsApp bridges are a thin TypeScript connector host that speaks to
the core over `channels/inbound/admit` and three activities on its own task
queue. One `lightspeed-server` process runs every role by default
(`gateway`, `sessions`, `bots`, `channels`); `--roles` selects a subset and
`--task-types workflows|activities` splits a worker role further.

The full design walk-through is in [docs/design.md](docs/design.md).

<p align="center">
  <img src="docs/images/readme-design-overview.png" alt="Lightspeed architecture: clients reach a session workflow holding the deterministic core inside Temporal; thin effect intents and result refs cross to activities that talk to LLM providers and borrowed compute; both sides share a session log and CAS" width="750">
</p>

## Development checks

```bash
cargo test
npm run check
```

## Documentation

- [Design](docs/design.md)
- [Development environment](scripts/dev/README.md)
- [Environment variables](docs/variables.md)
- [Universes, tenant isolation, and gateway authentication](docs/multi-tenancy.md)
- [JSON-RPC API reference](crates/api/contract/api-reference.md)
- [Build and release](docs/releasing.md)
- [Roadmap and design decisions](docs/roadmap/)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md)

## License

[Apache 2.0](LICENSE)
