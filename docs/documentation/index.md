# Lightspeed documentation

Lightspeed runs long-lived agents as durable workflows. Sessions retain their
conversation and execution state across worker restarts, and agents can use
tools, work with persistent files, and react to events. When a task needs an
operating system, the session selects an execution environment that supplies
the machine.

The distinction between the durable session and its compute runs through the
product. You can start with a model and a conversation, add a workspace and a
reusable profile, then connect machines or build bots as the work requires.

## Try Lightspeed

Begin with [Core concepts](getting-started/concepts.md) to understand sessions,
runs, profiles, bots, workspaces, and environments. Then follow
[Run Lightspeed locally](getting-started/quickstart.md) to start the product
and configure a model.

[Build your first agent](getting-started/first-agent.md) is a worked example:
create a release-editor profile, give it source material, and inspect the
persistent file it produces. It also shows how to continue a session and reuse
the profile for another conversation.

## Use an existing installation

Use the universe and account supplied by your operator. The
[model setup instructions](getting-started/quickstart.md#configure-a-model)
and [first-agent walkthrough](getting-started/first-agent.md) apply to an
existing installation too. Managing those resources requires a universe
owner/admin or platform administrator account.

The usage guides build on that first working agent:

- [Sessions and runs](using-lightspeed/sessions-and-runs.md): continue work,
  queue or steer tasks, inspect results, and manage history.
- [Models and credentials](using-lightspeed/models-and-credentials.md): connect
  providers, select model routes, and understand credential defaults.
- [Profiles and instructions](using-lightspeed/profiles-and-instructions.md):
  make a reusable setup and apply changes deliberately.
- [Workspaces and skills](using-lightspeed/workspaces-and-skills.md): share
  persistent files, source project instructions, and add a review skill.
- [Tools and MCP](using-lightspeed/tools-and-mcp.md): grant built-in tools,
  connect external services, and configure approvals.
- [Bots and triggers](using-lightspeed/bots-and-triggers.md): create an ongoing
  agent, deliver events, and inspect its activity.
- [Sub-agents and federation](using-lightspeed/subagents-and-federation.md):
  delegate bounded tasks or coordinate independent bots.
- [Chat channels](using-lightspeed/chat-channels.md): connect Telegram or
  WhatsApp and pair conversations with a bot.

When an agent needs a shell or a machine's files, read
[Environments](environments/overview.md). Connect a machine with
[Bring your own compute](environments/bring-your-own-compute.md) or configure
managed provisioning with [Incus VMs](environments/incus-vms.md). Then use
[Using environments](environments/using-environments.md) and
[Processes and jobs](environments/processes-and-jobs.md) for selection and
execution. [Credentials](environments/credentials.md),
[power and cleanup](environments/power-and-cleanup.md), and
[networking and ingress](environments/networking-and-ingress.md) explain
the access and lifecycle choices around that work.

## Deploy Lightspeed

The [deployment overview](deployment/overview.md) explains the runtime,
Platform, databases, Temporal, and the public/private network boundary.
[Self-host Lightspeed](deployment/self-hosting.md) installs the full web
product using release images built from a pinned source revision and existing
durable infrastructure.

The [environment-variable reference](../variables.md) provides exact settings,
and the [authentication guide](../multi-tenancy.md) explains universe isolation
and gateway modes.

## Build with Lightspeed

The [design walkthrough](../design.md) explains the deterministic core,
provider-native context, storage, and workflow integration. Applications can
use the [JSON-RPC API](../../crates/api/contract/api-reference.md) and
[TypeScript client](../../clients/typescript/README.md). MCP clients can manage
Lightspeed through [Configurator MCP](../../platform/configurator-mcp/README.md).

For repository development, start with the
[development guide](../../scripts/dev/README.md) and
[contribution guide](../../CONTRIBUTING.md). The
[build and release guide](../releasing.md) covers artifact construction and
migration compatibility.
