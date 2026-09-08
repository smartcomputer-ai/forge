# Run Lightspeed locally

The local launcher starts the complete Lightspeed product: the web app,
runtime, databases, storage, and Temporal. This walkthrough takes you from a
source checkout to a model response in a persistent session.

## Before you start

You need:

- A Lightspeed source checkout, with a terminal open at its root.
- Rust and Cargo through rustup. The repository's `rust-toolchain.toml`
  selects the required compiler version.
- Node.js 24 or newer, including npm.
- Docker running, with Docker Compose v2.
- A native build toolchain and the Protocol Buffers compiler (`protoc`),
  including its standard `.proto` include files, for the Rust dependencies.
- An OpenAI or Anthropic API key with access to a suitable model.

If someone has already deployed Lightspeed for you, open that installation and
continue at [Configure a model](#configure-a-model). Use your assigned account
and a universe where you are an owner or admin, or use a platform administrator
account. Those permissions are needed to manage integrations and sessions.

## Start the local product

From the repository root:

```bash
./dev.sh --no-envd
```

The launcher installs missing npm dependencies, starts infrastructure in
Docker, applies database migrations, and builds and starts the application
processes on your machine. The first startup can take a while because it
compiles the Rust runtime. Leave this terminal running and wait for the
readiness checks to finish.

The `--no-envd` flag leaves the local execution daemon disabled. The first
walkthrough uses only the model and Lightspeed's persistent files. You can
[connect compute](../environments/bring-your-own-compute.md) when a task needs
processes or a shell. Running plain `./dev.sh` also starts a local daemon;
sessions still need an environment configured before they can use it.

The launcher reads a root `.env` if one exists. A fresh installation can start
without deployment-wide model credentials because you will add a key through
the web app below.

## Sign in and create a universe

Open [http://localhost:5173/app/](http://localhost:5173/app/). The launcher
prints the development account. Unless you have overridden it, use:

| Field | Development value |
| --- | --- |
| Email | `admin@lightspeed.dev` |
| Password | `lightspeed-dev-password` |

These credentials belong to the local development setup. A deployed
installation uses an administrator account configured by its operator.

On a fresh installation, the app shows **No universes yet**. Choose **New
universe**, enter `Getting started`, and choose **Create**. The universe holds
the sessions, credentials, profiles, and workspaces you create during these
walkthroughs. If you already have a suitable universe, select it instead.

## Configure a model

In your universe, open **Settings → Integrations → Add integration**. Choose
**OpenAI (API key)** or **Anthropic (API key)**, enter your key in **API key**,
and choose **Save key**. Wait for a model list to appear under **Available
models**, then choose **Done**.

Use an API-key integration for this walkthrough. The Codex and Claude Code
subscription integrations serve those coding agents inside execution
environments; they do not supply model inference for a Lightspeed session.

If model discovery reports an error, resolve it here before creating the
session. A saved key needs access to the model you will select.

## Start a conversation

1. Open **Sessions** and choose the plus button labeled **New session**.
2. Enter `First conversation` as the **Name** and leave **Profile** at
   **No profile (engine defaults)**.
3. Choose **Customize setup…**. Under **Model configuration → Model**, select
   a conversational model from the provider you just connected.
4. Choose **Create session**.

Select the model explicitly. Adding a credential does not change the
deployment's default model, so leaving **Deployment default** selected can
send the request to a different provider.

Send a short message:

```text
Give me three questions to ask when reviewing an incident report.
```

An assistant answer confirms that the web app, runtime, and provider
connection work together. Refresh the page and open the same session: the
conversation should still be there. The answer completed one run; another
message will start a new run in this session.

## Stop and return later

In another terminal at the repository root, inspect the local stack:

```bash
./dev.sh status
```

Press Ctrl+C in the launcher terminal, or run `./dev.sh stop`, to stop the
application processes. The Docker infrastructure remains available. To stop
both the application processes and infrastructure:

```bash
./dev.sh down
```

Ordinary shutdown keeps the data volumes. Run `./dev.sh --no-envd` again when
you want to continue. The reset and volume-removal commands in the development
guide erase local state; they are for deliberately starting over.

## If something fails

| Symptom | What to check |
| --- | --- |
| Docker or Compose cannot be reached | Start Docker and check `docker compose version`. |
| The Rust build cannot find `protoc` or an imported standard protobuf file | Install the compiler and its standard include files. If the build cannot locate the include directory, set `PROTOC_INCLUDE` to that directory. |
| A port is already in use | Check `./dev.sh status` and the conflicting process. Service ports and overrides are listed in the [development guide](../../../scripts/dev/README.md). |
| The launcher warns that no provider key is configured | Continue to **Settings → Integrations** and add the key there. The warning alone does not prevent startup. |
| The session reports missing credentials or model access | Check both the integration and the model selected for this session. They must refer to the same provider. |
| Sign-in fails with the displayed defaults | Check the credentials printed by the launcher; an existing `.env` or existing user account may use different values. |

Continue with [Build your first agent](first-agent.md) to give the agent a
reusable profile and a workspace it can edit. For local profiles, ports, and
configuration options, see the [development guide](../../../scripts/dev/README.md)
and [environment-variable reference](../../variables.md).
