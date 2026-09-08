# Product documentation structure

Status: proposed navigation and page scope. This is the index plan; page content
and the documentation renderer are subsequent work.

Use `docs/documentation/` as the product documentation root. The existing
`docs/roadmap/` and `docs/spec/` can continue to hold implementation plans and
design history. A dedicated publishing root makes the reader-facing manual easy
to navigate and gives a future Markdown documentation tool a clear input
directory.

Organize the manual around reader tasks: get a first agent running, use the
product, provide compute, operate a deployment, integrate or extend it, understand
the internals, and contribute. Keep exact configuration and contract details in
reference pages. These are different reading modes; a first-run tutorial should
not require reading the architecture or an exhaustive configuration table.

## Proposed index

The comments below describe page scope, not additional pages. This is a compact
target structure for the first manual: keep related topics together initially
and split pages only when their content warrants it. The tree is proposed; these
files have not been scaffolded.

```text
docs/documentation/
├── index.md                              # What Lightspeed is; capabilities; choose a reading path
│
├── getting-started/
│   ├── concepts.md                       # Universes, sessions, runs, profiles, bots, workspaces, environments
│   ├── quickstart.md                     # Prerequisites → local stack → sign in → configure a model
│   └── first-agent.md                    # Complete one task; inspect its tools, output, and persisted session
│
├── using-lightspeed/
│   ├── sessions-and-runs.md              # Web app and CLI/TUI; history, steering, queueing, cancellation, fork/clone
│   ├── models-and-credentials.md         # Model selection; provider connections, API keys, OAuth, compatible endpoints
│   ├── profiles-and-instructions.md      # Reusable setups, capability grants, instructions, limits, applying profiles
│   ├── workspaces-and-skills.md          # Persistent VFS files, workspace links, prompts, skill discovery/activation
│   ├── tools-and-mcp.md                  # Built-in tools and web access; connect MCP, discover tools, handle approvals
│   ├── bots-and-triggers.md              # Create a bot; schedules, webhooks, pollers, routing, activity, lifecycle
│   ├── subagents-and-federation.md       # Delegate with profiles and limits; coordinate independent bots
│   └── chat-channels.md                  # Telegram/WhatsApp accounts, pairing, access, routing to bots, media
│
├── environments/
│   ├── overview.md                      # When compute is needed; VFS vs real files; three environment sources
│   ├── bring-your-own-compute.md         # Outbound envd registration and keys; persistent/ephemeral identities; direct endpoints
│   ├── incus-vms.md                      # Set up the included provider, images/templates, universe bindings; cluster option
│   ├── using-environments.md            # Create/select/share; profile provisioning; sessions, bots, and sub-agents
│   ├── processes-and-jobs.md             # Run commands; background jobs; inspect output, await, cancel
│   ├── credentials.md                   # Bind and inject credentials into compute and jobs
│   ├── power-and-cleanup.md             # Readiness, pause/stop/wake, idle policies, closing, retention and ownership
│   └── networking-and-ingress.md        # Connectivity requirements; provider-supported application endpoints
│
├── deployment/
│   ├── overview.md                      # Components, dependencies, full product vs runtime-only, deployment topology
│   ├── self-hosting.md                   # Install a coherent release; configure dependencies; migrate; start and verify
│   ├── authentication-and-tenancy.md    # Platform users/memberships, universes, gateway auth, keys, network trust boundaries
│   ├── configuration.md                 # Configure runtime, Platform, storage, connectors, Configurator; secrets and URLs
│   ├── operations.md                    # Health, logs, metrics, Temporal inspection, worker roles/scaling, storage retention
│   ├── upgrades-and-recovery.md          # Release compatibility, database migrations, backup/restore, recovery constraints
│   └── troubleshooting.md               # Startup, auth, model calls, stalled runs, compute connectivity, channel delivery
│
├── integrating-and-extending/
│   ├── api-and-typescript.md            # Authenticate; start a session/run; follow events and terminal results; client examples
│   ├── configurator-mcp.md              # Manage Lightspeed from another MCP client; deployment and access
│   ├── workflow-tools.md                # Managed sessions, external controllers, durable tools, replies and cancellation
│   ├── custom-tools-and-model-providers.md # Extension choices; tool adapters and native LLM integrations
│   ├── environment-providers.md         # Implement the public environment protocol; control/data boundaries and conformance
│   └── channel-connectors.md            # Add a chat provider through the connector boundary
│
├── how-it-works/
│   ├── architecture.md                  # Components and ownership: clients, Platform, runtime, Temporal, stores, compute
│   ├── agent-loop-and-durability.md     # Commands → events → state → intents; replay, activities, long-lived sessions
│   ├── context-and-storage.md           # Provider-native content, compaction, prompt caching, CAS, VFS, projections, collection
│   └── tools-and-controller-workflows.md # Tool execution; managed sessions; bots, channels, and sub-agents as workflows
│
├── development/
│   ├── local-development.md             # Repository map; dev.sh profiles; Rust/TypeScript loops; demo backend
│   ├── testing-and-evaluation.md        # Unit/integration/replay tests, generated checks, explicit live tests, evaluations
│   ├── changing-contracts.md            # API and workflow schema ownership; generation; database migration changes
│   └── contributing-and-releasing.md   # Contribution workflow, architectural rules, release construction and publication
│
└── reference/
    ├── api.md                           # Entry point to the generated JSON-RPC contract and TypeScript types
    ├── environment-variables.md         # Published view of the authoritative component-variable reference
    ├── session-and-profile-config.md    # Exact config fields, feature grants, defaults, limits, profile/environment intents
    ├── cli.md                           # User CLI, server administration, Platform CLI, daemon/provider commands
    ├── tools.md                         # Capabilities and logical tool identities; provider-dependent exposed names
    └── protocols.md                     # Generated workflow contract and environment control/data/registration protocol
```

## Navigation and page boundaries

The site home should offer four immediate paths: **Try Lightspeed**, **Use an
existing installation**, **Deploy Lightspeed**, and **Build with Lightspeed**.
The sidebar follows the section order above. Getting started establishes one
working path; environments and advanced features follow when the reader needs
them. The concepts page also serves as the initial glossary.

The most important Lightspeed-specific boundaries to preserve in the writing:

- **Session, run, and bot:** explain these terms before asking the reader to
  configure triggers or delegate. Sub-agents and bot federation belong in one
  introductory comparison, with separate procedures inside the page.
- **VFS and execution environments:** show that persistent VFS files work
  without a machine and that an environment has a separate real filesystem.
  Files do not synchronize automatically. Environment ownership, selection,
  sharing, and lifecycle deserve their own guide.
- **Using compute and implementing a provider:** the environments section is
  for users and operators. The provider protocol and implementation guide live
  under extensions and reference.
- **MCP in both directions:** using an MCP server as an agent tool belongs in
  the usage guide. Controlling Lightspeed through Configurator MCP belongs in
  the integration guide.
- **Local setup and deployment:** the quickstart uses `dev.sh`; self-hosting
  explains release artifacts, services, credentials, migrations, and network
  exposure. Release construction belongs in development.
- **Configuration guidance and exact fields:** task pages explain choices and
  use small examples. Reference owns exhaustive settings and schemas.

Use the web app as the main first-agent walkthrough because it exposes the
complete product. Include CLI alternatives where supported; API consumers get
a separate end-to-end path. Each task guide should finish with a way to verify
the result and a short troubleshooting section. The deployment troubleshooting
page can index those sections alongside cross-service failure diagnosis.

## Repository material behind the index

These are writing inputs and sources to verify, rather than a proposal to
publish every existing document unchanged. Roadmaps contain implementation
history and deferred ideas; current code and generated contracts settle the
supported behavior.

| Proposed area | Existing material to use |
| --- | --- |
| Introduction and getting started | [Product overview and capabilities](../../README.md), [development launcher guide](../../scripts/dev/README.md), [Platform and web app](../../platform/README.md), and [launcher implementation](../../scripts/dev/stack.mjs). |
| Sessions, models, profiles, workspaces, skills | [Client boundary and capability model](../design.md), [public API reference](../../crates/api/contract/api-reference.md), [profile DTOs](../../crates/api/src/profiles.rs), [session UI](../../platform/web/src/pages/SessionsPage.tsx), [CLI commands](../../crates/cli/src/main.rs), and [skill implementation](../../crates/tools/src/skills/mod.rs). |
| MCP and provider credentials | [Auth guide](../../crates/auth/README.md), [MCP approvals](p144-mcp-approvals.md), [native MCP execution](p145-native-mcp-execution.md), and [MCP discovery and tool search](p150-scalable-mcp-discovery-and-tool-search.md). |
| Bots, delegation, federation, and channels | [Bots design and implementation history](p130-bots.md), [sub-agent design](p134-subagents.md), [bot federation](p135-bot-federation.md), [channels as bot triggers](p139-channels-as-bot-triggers.md), [bot lifecycle and environments](p140-bot-environments-and-bot-lifecycle.md), [Bots domain](../../crates/bots/src/lib.rs), and [connector host guide](../../platform/connectors/README.md). |
| Environments | [Environment specification](../spec/04-environments.md), [Incus provider guide](../../crates/environment-provider-incus/README.md), [outbound daemon registration](p148-key-based-outbound-environment-registration.md), [profile provisioning](p125-profile-provisioned-environments.md), and [environment API types](../../crates/api/src/environments.rs). |
| Deployment and operation | [Build and release](../releasing.md), [release manifest schema](../../release/release-manifest.schema.json), [environment variables](../variables.md), [tenant isolation and authentication](../multi-tenancy.md), [Platform guide](../../platform/README.md), and [runtime role instructions](../../scripts/dev/README.md#manual-runtime-roles). |
| Integration and extension | [TypeScript client guide](../../clients/typescript/README.md), [Configurator MCP guide](../../platform/configurator-mcp/README.md), [workflow contract](../../crates/temporal-workflow/contract/workflow-contract.md), [environment protocol](../../crates/environment-protocol/src/lib.rs), [tool packages](../../crates/tools/src/lib.rs), and [LLM clients](../../crates/llm-clients/README.md). |
| How it works | [Design walkthrough](../design.md), [engine guide](../../crates/engine/README.md), [environment specification](../spec/04-environments.md), and the bot/channel/delegation designs above. |
| Development and reference | [Repository contribution guidance](../../AGENTS.md), [contribution policy](../../CONTRIBUTING.md), [development guide](../../scripts/dev/README.md), [evaluation harness](../../crates/eval/README.md), [API exporter](../../crates/api/src/bin/export-schema.rs), [workflow exporter](../../crates/temporal-workflow/src/bin/export-workflow-contract.rs), and [profile configuration reference generator](../../platform/scripts/generate-config-reference.mjs). |

## First writing pass

Write in this order while retaining the target navigation above:

1. **A complete first-use path:** home, concepts, quickstart, first agent,
   sessions/runs, models/credentials, and profiles/instructions. Connect the
   existing API and configuration references.
2. **A complete self-hosting and compute path:** deployment overview and
   installation, authentication, operations and recovery, environment overview,
   daemon registration, Incus, and environment selection/lifecycle.
3. **The remaining capabilities and builder paths:** VFS/skills, MCP,
   bots/channels/delegation, API and workflow integration, extensions,
   architecture, and contributor guides.

The main writing gaps are coherent tutorials, an end-to-end deployment guide,
and operational recovery procedures. Their presence in the proposed index is a
writing requirement, not a claim that a tested deployment or recovery recipe
already exists. Establish one verified self-hosting recipe first; add
orchestrator-specific recipes when the repository actually supports them.

Existing guidance also needs reconciliation during authoring. For example, the
development README currently says the full/runtime profiles require deployment
API keys, while the launcher permits startup without them and points users to
per-universe credentials in the UI. The quickstart must follow the executable
behavior. Similarly, the design document's final pointer to an AGENTS crate
inventory is stale; the workspace manifest owns the current crate list.

## Existing docs and publishing

Keep the root README as the short product overview with a documentation entry
link. As product pages are written, consolidate user-facing prose from
`docs/design.md`, `docs/multi-tenancy.md`, and the relevant parts of the component
guides into the manual. Retain useful repository-local pointers at their old
locations. Component READMEs can continue to explain local implementation and
maintenance details.

Preserve one authoritative source for each reference. In particular,
`docs/variables.md` remains authoritative until deliberately migrated, and API
and workflow contracts remain generated from Rust. The reference pages above
should link to or include those sources; a later site build can stage generated
artifacts into its output without creating hand-maintained copies. If a source
moves, update its generator and consumers in the same change.

Use ordinary Markdown, relative links, stable descriptive filenames, and
existing repository images. Configure ordering and site navigation when the
renderer is selected. Publish completed pages as they land; keep this plan and
unfinished page placeholders out of the product sidebar.


## Writing Style

The voice reference is Lukas's hand-authored
[Agent OS design document](https://github.com/smartcomputer-ai/agent-os/blob/pre-next/docs/design/design.md).
Use it for exposition and voice. Lightspeed's current code and contracts remain
the sources for technical claims.

The distinctive quality is patient explanation by someone who has built the
system. The document develops a few primitives, follows their consequences,
and makes abstractions concrete through examples. It is conversational, uses
definitions and transitions freely, and acknowledges tradeoffs. Confidence
comes from making the mechanism understandable. Preserve that reasoning and
rhythm while editing accidental repetition and errors.

### Who reads this

Buyers and their engineers include architects, platform and security teams,
and compliance officers, often at enterprises and banks. They need enough
detail to evaluate the claims. The manual also serves people installing,
using, integrating, and developing Lightspeed. Match the page to their task:

| Page type | What the writing should accomplish |
| --- | --- |
| Product overview and evaluation | State the capability, explain its mechanism, and establish its practical value and limits. |
| Architecture and concepts | Build an understanding of how the pieces fit together and why the design takes this shape. |
| Tutorials and task guides | Help the reader complete a task, verify the result, and understand the decisions they must make. |
| Reference | Make exact behavior, fields, defaults, prerequisites, and errors easy to find. |

Assume technical competence, but introduce Lightspeed-specific vocabulary.
Knowing what a workflow engine is does not tell someone what a Lightspeed
session owns or how a bot differs from a sub-agent. Explain familiar concepts
briefly when their precise meaning matters to the argument.

### Voice

- State what Lightspeed does and explain why. Use concrete subjects and verbs:
  the gateway admits a command, the core emits an intent, an activity calls a
  provider. Avoid corporate enthusiasm and unsupported superlatives such as
  "world-class" or "rock-solid".
- Name Lightspeed or the responsible component. Reserve "we" for reasoning
  with the reader; use "you" for the reader's actions and choices. Keep the
  house preference against "we believe" and company "our", even though the
  historical sample sometimes uses them.
- Sound like an engineer explaining the system to a peer. Contractions and
  occasional informal asides are welcome. Directness can be warm; it does not
  require every sentence to sound like a specification.
- Put the main point early. A paragraph can start with a constraint, an
  observation, or the next step in the explanation. Develop the point with
  evidence and consequences instead of repeatedly increasing its intensity.
- Name real products and mechanisms when they clarify the explanation:
  Temporal, PostgreSQL, S3, Telegram, WhatsApp, vLLM. Comparisons must describe
  a specific, verified difference.
- Show arithmetic when scale or cost is the point. State assumptions and
  distinguish illustrative numbers from measured results. Prices need a
  dated source; performance figures need conditions. An idle agent's low
  compute use does not establish that its total operating cost is zero.
- State implemented behavior, limitations, and intended future work distinctly.
  A design goal is not a guarantee. Put material qualifications beside the
  claim they qualify.

### How to develop an explanation

Start with the problem or property that matters, introduce the smallest model
needed to explain it, and follow one concrete operation through that model.
Then explain what follows from the mechanism: what it enables, what it costs,
or what the reader must account for. This is a useful progression, not a
mandatory template for every page.

- Build on what the reader has just learned. Introduce a new component when
  its job becomes clear. If an explanation depends on storage, establish the
  relevant storage model before describing execution against it.
- Use causal connections: because, so, therefore, which means. A second
  sentence should explain, demonstrate, or qualify the first. Restate an
  abstraction when the restatement gives the reader a more concrete view.
- Define a term near its first meaningful use, preferably through what it
  does. Keep the same names through the explanation, code, and diagram so the
  reader can follow the same objects throughout.
- Guide the reader when the dependency or change of level needs explaining.
  A short transition into an example or a reminder of an earlier invariant
  is useful. Remove generic announcements that add no information. Address
  likely objections in the prose without turning each paragraph into a
  rhetorical question and answer.
- Use familiar systems as analogies, then identify where the analogy stops
  helping. Follow with the actual data flow, code, or behavior. An analogy
  should make the implementation easier to understand.
- Give code and diagrams a job. Introduce what to notice, keep the example
  focused, and explain the result afterward. Show a lower-level mechanism
  when it helps explain what a convenient API handles for the user.
- Use connected paragraphs for reasoning, numbered steps for procedures and
  sequences, and lists or tables for parallel facts. A short recap can make a
  long explanation easier to retain; it should collect the model the reader
  has now learned.

### Examples from the voice reference

These brief excerpts illustrate specific moves, not phrases to repeat in every
page. The technical subject matter belongs to the historical document.

| Excerpt | What to carry into Lightspeed documentation |
| --- | --- |
| "Let's first consider Git." | Begin with a familiar model. [Object store section](https://github.com/smartcomputer-ai/agent-os/blob/pre-next/docs/design/design.md#grit-object-store). |
| "To make it a bit more concrete" | Move from design to a worked example. [Prototype section](https://github.com/smartcomputer-ai/agent-os/blob/pre-next/docs/design/design.md#python-prototype). |
| "Or another way to look at it:" | Restate a relationship from another useful angle. [Data structure section](https://github.com/smartcomputer-ai/agent-os/blob/pre-next/docs/design/design.md#grit-data-structure). |
| "there is no magic" | Expose what the abstraction does underneath. [Low-level API section](https://github.com/smartcomputer-ai/agent-os/blob/pre-next/docs/design/design.md#low-level-api). |

The following paragraphs are newly written Lightspeed examples, based on the
[current design](../design.md) and [environment model](../spec/04-environments.md).
They demonstrate how to apply the guide rather than quote the older document.

An explanation that starts with a task and follows it to a boundary:

> An agent can keep notes and edit files in its VFS workspace without an
> operating system attached. Suppose it now needs to run a test suite. That
> requires an execution environment, which provides a real filesystem and
> processes. The session selects an environment, and process tools run there.
> The files in the VFS remain separate, so anything the test needs from that
> workspace must be transferred explicitly.

An explanation that connects a design constraint to a mechanism:

> Temporal records activity inputs and results in its workflow history. If
> every model response and tool result passed through that history in full,
> a long-running session would accumulate a large amount of data. Lightspeed
> stores those payloads in content-addressed storage and passes references
> between the workflow and its activities. The activity can load the bytes
> when it needs them, while the workflow records the smaller reference. This
> keeps the data crossing that boundary small as the conversation grows.

### Sentences and punctuation

- Mix short statements with longer explanatory sentences. Keep a connected
  cause and consequence together when that makes the reasoning easier to
  follow. Break a sentence when it starts carrying a separate argument.
- Use a colon to introduce an explanation, example, or compact enumeration.
  Use ordinary conjunctions when the relationship needs to be spelled out.
- Em dashes are occasional asides, spaced ( — ). Prefer a full stop when the
  aside becomes a second argument. Documentation has no mandatory dash count.
- Parentheses can hold a small clarification. Put important limits in the
  main sentence or a separate sentence where readers will see them.
- Use normal punctuation in documentation. A comma splice may occasionally
  suit short product copy, but should not become the default sentence rhythm.
- Use italics sparingly for emphasis and bold for useful scanning, including
  UI labels and list lead-ins. There is no required italicized word per page.
  Use backticks for code identifiers, commands, paths, and literal values.
- Use American spelling. Write round rhetorical quantities in words
  ("a thousand agents", "weeks to months") and arithmetic and specifications
  in digits ("3,200 machine-hours", "2 vCPUs").

### Short product copy

Landing and short topic copy can be tighter than the manual. Lead with the
claim and follow it with the mechanism or concrete consequence that earns it.
Prefer positive statements of capability. Define terms only when needed for
that claim, and keep navigation announcements out of the opening.

Keep emphasis restrained: usually one stressed word is enough for a short
piece, and at most one or two spaced em-dash asides. A "Rests on:" label can
introduce supporting mechanisms if the page's format calls for it; it is not a
required ending for documentation. These are choices for short copy, not
restrictions on explaining a complex system.
