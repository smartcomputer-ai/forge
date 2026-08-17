# ls.bot landing page copy


## Hero

**Run thousands of agents for months. Not thousands of VMs.**

**The agent fleet for the enterprise.**

Lightspeed is open-source infrastructure for running long-lived agent fleets
in production. Claude-Code-class agents run as durable workflows on your
engine, borrow real machines only when a task needs one, and stay auditable,
tenant-isolated, and cheap when idle.

Rust core. Temporal runtime. Postgres and S3. Apache 2.0.

[Get started] [GitHub]

[Visual: platform web UI screenshot. Prefer a view that backs the headline —
the session/fleet list showing many long-lived sessions with ages, statuses,
and attached environments. Browser frame, dark mode.]

---

## The problem

Frontier harnesses like Claude Code, Codex, and OpenClaw are excellent, but
they assume they own an operating system. That assumption is what makes them
hard to scale, hard to secure, and impossible to keep alive for long.

- **One OS per agent.** Every agent needs its own VM or container, even while
  it waits for a reply. Cost and attack surface scale with agent count, not
  with work.
- **Agent SDKs are not built for workflow engines.** They mix the agent loop
  with LLM and tool calls, and ship the whole chat history back and forth
  between workflow and activities. Histories bloat, determinism breaks,
  durability is bolted on.
- **MCP-only agents underperform.** Frontier models are trained to drive a full
  POSIX machine. Take the shell away and you take capability away.

---

## What Lightspeed does

Lightspeed separates the harness from compute.

The harness is the agent loop, context management, and session state. In
Lightspeed it runs as a lightweight, deterministic, event-sourced workflow.
Anything that needs an operating system — shells, code execution, full file
systems, long-running jobs — happens on machines the agent borrows for exactly
as long as it needs them.

**Outside the OS.** The agent is a workflow, not a process. Idle agents cost
nothing. Thousands run on a single worker node.

**Durable by construction.** The core replays an event log into state, decides
the next step, and emits thin effect intents. It performs no I/O itself, so it
survives restarts, redeploys, and multi-week lifetimes for free.

**Borrowed compute.** Sessions attach to dedicated VMs, provision fresh ones
per session, run provider-owned jobs for hours-long work, and power machines
down when idle. The model gets a real machine; the harness never lives on it.

[Visual: `docs/images/readme-why-overview.png` — four fat VMs each hosting one
tiny agent vs. dozens of agents packed into one worker, borrowing machines via
dashed lines.]

---

## What you can build

Each use case lists the Lightspeed features it rests on.

### An always-on personal assistant, for thousands of users

One agent per user, reachable over Telegram or WhatsApp, remembering weeks of
context, calling calendars, mail, and internal tools over MCP with each user's
own OAuth identity. Nearly all agents are idle at any moment; in Lightspeed an
idle agent is a suspended workflow, not a running VM.

*Built on: long-running sessions, channels, hosted MCP with universe-owned
auth, agent profiles.*

### An autonomous software factory

A planner agent spawns builders, testers, and reviewers. Each sub-agent
provisions its own VM from a profile, clones the repository, runs the suite,
and critiques the others' work. The fleet keeps going for days and picks up
where it left off after a worker restart. Fork a session to try two
approaches from the same state without paying for the shared prefix twice.

*Built on: sub-agents (fleets), per-session provisioned environments, session
fork and clone, idle power policy.*

### Mapping and documenting the enterprise

Point a fleet at an undocumented landscape: dozens of repositories, servers,
schemas, and cron jobs nobody fully understands. Agents explore machines and
codebases in parallel, reconstruct how the pieces talk to each other, write
the documentation that never existed, and keep it accurate as they continue to
observe. Sub-agents split the estate; a coordinator merges what they find.

*Built on: sub-agents (fleets), environment file and process tools, virtual
file system for the resulting docs, session fork and clone, secret credential injection, idle power
policy.*

### Research agents that supervise long experiments

An agent launches a training run or data pipeline as a job on a GPU box,
checks in on progress, adjusts parameters, and writes up the results when it
finishes — hours or days later. Credentials for the cluster are injected into
the environment and never pass through the model.

*Built on: provider-owned jobs with session supervision, dedicated
environments, credential injection, encrypted secrets.*

### An on-call operations agent

An alert starts a managed session from your incident workflow. The agent reads
dashboards over MCP, inspects the host through the environment daemon,
proposes a fix with a deadline, and reports back to the channel. If nobody
answers, the workflow cancels it cleanly.

*Built on: managed sessions, workflow-backed tools with deadlines and
cancellation, environment process tools, channels.*

### Agents embedded in your own product

You already run Temporal. Give every customer, order, or ticket its own agent:
your workflow starts a managed session, binds tools backed by your own
activities, and receives durable emissions back. No separate agent
infrastructure to run.

*Built on: managed sessions and the generic workflow-tool protocol, typed
JSON-RPC API and TypeScript client, Configurator MCP.*

### Resident agents for the systems you already run

Give every legacy service, batch job, database, and integration a long-lived
agent that lives alongside it. The agent watches its logs and metrics over
MCP, runs health checks and routine maintenance on the host, keeps the runbook
and architecture notes current as the system drifts, and escalates to a human
with a written summary when something looks wrong. It runs for months because
the system does.

*Built on: long-running sessions, dedicated environments and process tools,
hosted MCP, managed sessions and workflow-backed tools, channels.*


