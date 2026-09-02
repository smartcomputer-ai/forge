# P149 — Harbor End-to-End Agent Evaluation

**Status**

- Proposed 2026-09-01.
- Depends on [P148](p148-key-based-outbound-environment-registration.md)'s
  key-based outbound `lightspeed-envd` registration, server-assigned
  environment identity, correlation metadata, registration receipt, and
  ephemeral disconnect cleanup.
- Uses Harbor as the benchmark orchestrator and Terminal-Bench as the first
  task suite. It does not add a second environment implementation or replace
  the prompt-level harness in `crates/eval`.
- The Harbor adapter, job configurations, and reporting tools live in a
  separate repository. This document records the Lightspeed-side contract,
  cross-repository implementation plan, and acceptance criteria.

## Goal

Measure the complete Lightspeed agent system against another complete agent
system on the same terminal tasks. The first comparison is:

```text
Harbor + Terminal-Bench task + fixed model
├── built-in Codex agent
└── Lightspeed Harbor agent -> hosted Lightspeed -> registered envd
```

Both arms use the same pinned model snapshot, reasoning effort, task image,
verifier, resource envelope, timeout, and task instruction. Each arm keeps its
native prompt, context management, tool loop, retry policy, and terminal/file
tool implementation. The result is an end-to-end product comparison, not an
attempt to attribute a score difference to an individual harness component.

A developer starts Harbor on a laptop or CI runner. Harbor may place task
sandboxes in local Docker or any supported remote environment provider. The
Lightspeed control plane may be a hosted deployment. Every Lightspeed trial
starts `envd` inside the Harbor-owned sandbox; `envd` connects outbound to the
hosted gateway, so neither Harbor nor the sandbox needs an inbound public
address.

The initial acceptance target is a reproducible paired run over a pinned
Terminal-Bench release, producing Harbor-native scores plus enough provenance
and Lightspeed artifacts to diagnose every trial.

## Why Harbor Owns the Evaluation

Harbor already defines the boundaries an end-to-end comparison needs:

- a task supplies one instruction, container image, and verifier;
- an environment provider creates and destroys the sandbox;
- an agent receives the instruction and acts through that environment;
- a trial joins one task, agent, attempt, and environment;
- a job expands the agent/dataset/attempt matrix and controls concurrency; and
- logs, trajectories, rewards, timing, and artifacts are retained together.

Lightspeed should enter at the agent boundary. It should not provision the
benchmark container, reinterpret the verifier, or implement a Harbor
`BaseEnvironment`. Those choices would either give the compared systems
different tasks or replace the real Lightspeed environment path with a bridge.

The integration is therefore a small Python package containing an external
Harbor `BaseAgent`. Harbor calls that agent with its existing
`BaseEnvironment`. The adapter uses the environment only to upload and start
the real `lightspeed-envd`; all model turns and environment operations after
registration run through hosted Lightspeed.

The relevant upstream contracts are Harbor's
[agent interface](https://harborframework.com/docs/agents),
[job configuration](https://harborframework.com/docs/core-concepts/jobs), and
[Terminal-Bench runner](https://harborframework.com/docs/tutorials/running-terminal-bench).
The implementation pins a tested Harbor package version rather than relying on
the current documentation at runtime.

## Scope

### In scope

- a standalone, version-pinned Harbor adapter in a dedicated repository;
- the lifecycle from Harbor trial to outbound `envd` registration, Lightspeed
  session/run, cleanup, and Harbor result;
- local Docker and at least one remote Harbor environment provider;
- a paired Codex/Lightspeed Terminal-Bench configuration using the same model;
- explicit fairness and provenance checks that fail closed;
- token, cost, timing, trajectory, and diagnostic artifact export;
- smoke tasks, oracle preflight, paired reporting, and full-run instructions;
  and
- a path for adding other Harbor datasets without changing the adapter.

### Out of scope

- splitting the score into model, prompt, context manager, or tool-loop
  contributions;
- implementing a host-side `envd` protocol bridge through Harbor's
  `BaseEnvironment`;
- making Lightspeed provision or destroy Harbor sandboxes;
- modifying Terminal-Bench tasks, images, or verifiers;
- maintaining a Harbor fork;
- submitting results to a public leaderboard;
- hosted benchmark scheduling or a benchmark dashboard;
- training, fine-tuning, or reinforcement learning from the trajectories;
- declaring SWE-bench, Aider Polyglot, or another suite part of initial
  acceptance; and
- folding the integration into `crates/eval`, whose purpose remains fast,
  prompt-level and in-process evaluation.

## Repository Boundary

Implement the integration in a dedicated repository, tentatively named
`lightspeed-harbor`. The repository is independently installable and contains:

```text
lightspeed-harbor/
├── README.md
├── pyproject.toml
├── uv.lock
├── src/lightspeed_harbor/
│   ├── __init__.py
│   ├── agent.py
│   ├── artifacts.py
│   ├── client.py
│   ├── envd.py
│   └── provenance.py
├── configs/
│   ├── smoke.local.yaml
│   ├── terminal-bench.local.yaml
│   └── terminal-bench.remote.example.yaml
├── scripts/
│   ├── preflight.py
│   └── report.py
└── tests/
```

`uv.lock` pins Harbor and all adapter dependencies. Dataset refs, task
allowlists, agent settings, environment backend, and attempt counts live in
committed job configuration. Secrets and machine-specific endpoints do not.

The package exposes `lightspeed_harbor.agent:LightspeedAgent` as an importable
Harbor agent. It does not install itself into Harbor or require a source patch.

The ownership split is:

| Repository | Owns |
|---|---|
| Lightspeed | P148 registration/API contracts, `envd`, hosted session behavior, and released client/schema artifacts. |
| `lightspeed-harbor` | Harbor adapter, dependency lock, benchmark configs, preflight, result conversion, reports, and operator instructions. |

The adapter depends only on released Lightspeed interfaces and artifacts. It
must not import Rust workspace internals, read the Lightspeed source tree, or
require a monorepo-relative path. For local development, configuration may
point it at a locally built `envd` binary and local Lightspeed endpoint, but
those are explicit overrides of the same public boundary.

This P149 document remains in the Lightspeed roadmap because its prerequisite
protocol and server work land here. Implementation progress should be tracked
in the adapter repository once it exists, with this document linking the
compatible adapter release or commit rather than mirroring its issue backlog.

## Ownership Boundary

One trial has two control paths which must remain distinct:

```text
developer or CI runner
  |
  +-- Harbor job
        |
        +-- creates task sandbox through BaseEnvironment
        +-- calls LightspeedAgent with the task instruction
        |     |
        |     +-- uploads and starts envd in that sandbox
        |     +-- reads the registration receipt
        |     +-- starts a hosted Lightspeed session and activates that env
        |     +-- starts one run with the unmodified task instruction
        |     +-- waits, records artifacts, and disconnects envd
        |
        +-- runs the task verifier in the same sandbox
        +-- collects reward, logs, and artifacts
        +-- destroys the sandbox
```

Harbor is authoritative for trial identity, sandbox lifecycle, timeout, and
reward. Lightspeed is authoritative for its session, model calls, agent loop,
and environment protocol operations. Closing the registered Lightspeed
environment never deletes the underlying Harbor sandbox. Conversely, Harbor
destroying a sandbox does not require Lightspeed to know how that compute was
provisioned.

## Adapter Contract

### Agent type

Implement `LightspeedAgent` as Harbor's external `BaseAgent`, not
`BaseInstalledAgent`:

- the agent loop runs in hosted Lightspeed rather than as a CLI in the task
  container;
- the adapter itself runs in Harbor's orchestrator process;
- only `lightspeed-envd` runs inside the sandbox; and
- the adapter uses `BaseEnvironment` only for upload, command execution, and
  log retrieval before registration.

The adapter implements Harbor's `name`, `version`, `setup`, and `run`
contracts. It accepts `model_name` from Harbor and returns a populated
`AgentContext`. It must not infer or silently substitute a model.

### Setup phase

`setup(environment)` performs work that does not create durable remote trial
state:

1. Validate the adapter configuration and supported sandbox platform.
2. Select a pinned `envd` release for the sandbox architecture.
3. Verify its checksum on the Harbor host.
4. Upload it into the sandbox through `BaseEnvironment`.
5. Mark it executable and verify `envd --version` as Harbor's
   `environment.default_user`.
6. Create the Lightspeed agent log directory under Harbor's `/logs` contract.

Do not compile `envd` in the task container or download it from the public
internet there. A host-side local path override is useful for development;
release runs use a pinned release URL plus checksum. Initial support may be
Linux amd64 because that covers the first Terminal-Bench images. Linux arm64
is the next target; unsupported combinations fail before a model call.

Setup deliberately does not start `envd`. Starting it inside `run` gives the
adapter one `try/finally` scope covering registration, session execution, and
cleanup. If the adapter process dies outside that scope, P148's ephemeral
disconnect grace remains the final cleanup mechanism.

### Run phase

`run(instruction, environment, context)` performs exactly one Harbor trial:

1. Materialize the P148 registration key into a mode-`0600` temporary file in
   the sandbox. Do not place it in a command line, process environment, log,
   receipt, or artifact; pass only its file path to `envd`.
2. Start `envd` as `environment.default_user` with:
   - the public WSS gateway URL;
   - a task working directory matching Harbor's agent working directory;
   - the filesystem root and operating-system privileges selected by the
     parity policy below;
   - a private state/runtime directory inside the trial sandbox;
   - a registration-receipt output path; and
   - bounded Harbor correlation metadata.
3. Wait for a valid registration receipt. Verify its universe, identity mode,
   and correlation fields, then delete the registration-key file immediately.
4. Resolve Harbor's `model_name` to one explicit Lightspeed model-provider
   record. Validate model id, API kind, reasoning effort, processing tier, and
   output-token settings. Any mismatch or fallback is a configuration error.
5. Start an ordinary Lightspeed session with the committed benchmark profile,
   or an equivalent explicit inline config, and activate the receipt's exact
   `environmentId` through `session/environments/activate`.
6. Append/start the Harbor instruction byte-for-byte. Do not summarize it,
   prepend agent-specific hints, or add success criteria in the adapter.
7. Wait for the run to become terminal while observing Harbor cancellation and
   timeout. The hosted agent performs all terminal and filesystem operations
   through the active registered environment.
8. Convert Lightspeed usage, terminal status, and timings into Harbor's
   `AgentContext`, and export the artifacts described below.
9. In `finally`, cancel an active Lightspeed run, close the session, terminate
   the sandbox `envd` process group, and explicitly close the ephemeral
   registered environment when the API supports immediate close.

The verifier runs after the agent phase through Harbor's own environment
connection. Disconnecting `envd` must therefore leave the sandbox filesystem
and processes intact. Harbor alone tears the sandbox down after verification.

### Correlation

P148 correlation metadata is diagnostic, not identity. Include the bounded
fields available from Harbor, using opaque strings and hashes where necessary:

```text
source=harbor
harborContextId=<globally unique trial/environment context>
harborSessionId=<agent session id>
harborJobId=<job id, when exposed>
harborTrialId=<trial id, when exposed>
harborTaskName=<bounded task name>
harborAttempt=<attempt number>
agent=lightspeed
```

The adapter persists the returned `environmentId`, `incarnationId`,
`daemonId`, and `connectionId` in its trial artifact. It never proposes those
ids to Lightspeed or uses a Harbor id as an authentication claim.

Use the Harbor `context_id` as the principal join key because Harbor documents
it as the link between the trial and environment. If a particular Harbor
version does not expose another listed id through `BaseAgent`, omit that field
rather than discovering it through internal Harbor state.

### Working directory and privileges

The comparison must not accidentally give Lightspeed root access while Codex
runs as the task agent user. Both arms run commands with Harbor's configured
agent user and begin in the same task working directory.

Terminal-Bench agents commonly need access outside the repository working
directory. The initial Lightspeed `envd` filesystem root is therefore `/`
inside the sandbox, subject to the same Unix uid, gid, container mounts, and
container security policy as the Codex process. This grants addressability,
not extra operating-system privilege. Any task-specific difference is a
preflight failure.

## Configuration and Secrets

### Host-side configuration

The adapter process reads these values on the Harbor host:

```text
LIGHTSPEED_API_URL
LIGHTSPEED_API_KEY
LIGHTSPEED_HARBOR_REGISTRATION_KEY
LIGHTSPEED_ENVD_GATEWAY_URL
```

Optional development/release settings select the `envd` artifact path,
release URL, checksum, expected version, benchmark profile, and explicit
Lightspeed provider id.

`LIGHTSPEED_API_KEY` remains host-side and authorizes the adapter to create and
control sessions in one evaluation universe. It is never inserted into the
task sandbox. The hosted Lightspeed deployment owns its model-provider
credential; the Lightspeed sandbox does not receive an OpenAI API key.

### Sandbox registration key

The registration key is the only Lightspeed bootstrap secret inserted into a
task sandbox. Use a dedicated P148 key with:

- the evaluation universe as its only universe;
- ephemeral identity mode, which is the key's policy and not a daemon setting;
- an active-environment limit sized just above configured Harbor concurrency;
- a campaign expiry and operator-visible label;
- a rate limit appropriate to expected trial starts; and
- rotation between benchmark campaigns or after any suspect task.

The same key may register all concurrent trials. P148 makes the daemon key,
not the shared registration key, the identity of each environment. One key per
campaign is therefore the default; one key per job remains an optional tighter
operational policy, not a protocol requirement.

Benchmark code is adversarial from the perspective of credentials: a task can
run commands as the agent user and may inspect that user's processes or files.
Deleting the key file after registration narrows exposure but does not make the
sandbox a trusted secret store. The key consequently carries only bounded
environment-registration authority. It cannot call the Lightspeed API, read
another environment, select an environment id, or retrieve the hosted model
credential.

### Codex credential

The built-in Codex agent receives the provider credential using Harbor's
normal secret mechanism. Keep that secret scoped to the Codex agent. Do not
copy it into the Lightspeed sandbox merely to make job configuration look
symmetric: Lightspeed model calls happen in the hosted service.

## Model and Harness Parity

The comparison answers: *with the same model and task resources, which complete
agent system solves more tasks?* It does not answer which prompt or context
policy caused a difference.

### Required matched settings

Before a job starts, `preflight.py` resolves both agent configurations and
fails unless these fields match:

| Dimension | Rule |
|---|---|
| Model | Same immutable model snapshot/id, not two aliases that may drift. |
| Provider route | Same provider API family and endpoint class where both systems support it. |
| Reasoning | Same reasoning-effort value. |
| Processing | Same service/processing tier, or an explicitly reported unavoidable difference. |
| Output limit | Same per-generation maximum when both interfaces expose it. |
| Instruction | Identical task instruction bytes. |
| Task | Same dataset ref, task definition, image digest, and verifier. |
| Compute | Same CPU, memory, storage, architecture, agent user, mounts, and task timeout. |
| Network | Same task network policy plus only the control endpoints each agent requires. |
| Attempts | Same selected tasks and attempt count. |
| Concurrency | Same per-agent concurrency and comparable provider quota. |

There is no model fallback. If the requested model is unavailable in
Lightspeed, Codex, or the provider account, the job stops before trials begin.
The committed report records the resolved values, not just the input aliases.

When exact equality is impossible—most notably provider processing tiers—the
run may proceed only under a differently named configuration, and the report
must state the mismatch. Such a run is informative but is not the primary
matched comparison.

### Native harness behavior

Keep the mechanisms whose aggregate value is being tested:

- native system instructions;
- native terminal/file tool schemas and output shaping;
- native context compaction and prompt caching;
- native tool scheduling and error recovery;
- native model-call retry behavior; and
- native stopping behavior within the common external timeout.

Do not try to impose a shared turn count if the two systems define a turn
differently. Prefer a common wall-clock limit and report actual tokens, calls,
and cost. A common total spend cap can be added once both integrations enforce
the same semantics; until then, cost is an outcome rather than a matching
constraint.

### Capability surface

The primary track is **terminal-only native harness**:

- each system retains its normal terminal and filesystem interaction;
- browser, web search, MCP servers, retrievable credentials, skills, bots, and
  sub-agents are disabled;
- task-network access follows the dataset's policy; and
- the only additional egress is the agent's required control/model endpoint.

This avoids giving one arm unrelated services while still testing each
harness's actual coding loop. A later **best available system** track may turn
on differentiated capabilities, but it needs a separate config and label and
must not be pooled with the primary result.

### Ordering and provider drift

Run both agents in the same Harbor job matrix when practical. Use equal
per-agent concurrency and interleave their trials so provider load, time of
day, and remote compute conditions do not line up with only one arm. Record
trial start times and provider request ids where available.

For a smoke run, one attempt is enough. For an engineering comparison, use at
least three attempts per task. A result intended for an external claim should
use a predeclared task set and attempt count—normally five—chosen before
examining agent outcomes.

## Harbor Job Configuration

The committed YAML should follow the pinned Harbor version's schema. Its shape
is expected to be:

```yaml
n_attempts: 3
n_concurrent_trials: 8

agents:
  - name: codex
    model_name: openai/<immutable-model-id>
    n_concurrent: 4
    kwargs:
      reasoning_effort: <effort>

  - name: lightspeed
    import_path: lightspeed_harbor.agent:LightspeedAgent
    model_name: openai/<immutable-model-id>
    n_concurrent: 4
    kwargs:
      lightspeed_provider_id: <provider-id>
      profile_id: harbor-terminal
      reasoning_effort: <effort>

datasets:
  - name: terminal-bench/terminal-bench-2-1
    ref: <pinned-ref>

environment:
  type: docker
  delete: true
```

This is an architectural example, not a promise that unimplemented files can
be run verbatim. Slice 1 records the exact schema accepted by the pinned
Harbor dependency. Do not use `latest` for Harbor, the dataset, the model, or
the `envd` artifact.

The supported local invocation becomes:

```bash
cd lightspeed-harbor
uv sync --frozen
uv run python scripts/preflight.py --config configs/smoke.local.yaml
uv run harbor run -c configs/smoke.local.yaml
```

Switching from Docker to a remote provider changes only Harbor's environment
configuration and the permitted gateway egress. It does not change the
Lightspeed agent implementation or create a provider-specific bridge.

## Terminal-Bench Policy

Terminal-Bench is the first suite because its container/verifier contract
directly exercises the environment-backed agent loop. Pin the dataset ref and
the resolved image digest for every task in the campaign manifest.

Before comparing agents:

1. Run Harbor's oracle on the selected tasks against the intended environment
   backend and resource settings.
2. Verify the task image works with the pinned Harbor version and that
   `/logs` artifact collection succeeds.
3. Verify both agents begin with the expected uid, working directory, mounts,
   and network policy.
4. Publish any task exclusions and their reasons before reading agent rewards.

An oracle failure, image-pull failure, or verifier incompatibility discovered
in preflight may exclude a task. An agent timeout, model failure, adapter bug,
gateway failure, or `envd` failure is not an oracle failure and must not be
quietly removed from one arm's denominator.

The first smoke set should contain a few tasks that cover file editing,
long-running commands, process control, and output-heavy terminal interaction.
Selection is for integration coverage, not for an attractive score. The full
run uses the entire declared suite except prepublished oracle exclusions.

## Results and Artifacts

### Primary measure

The primary measure is Harbor's verifier reward aggregated as task success
rate for each agent. Preserve raw per-trial rewards even when a suite exposes
partial credit. For binary Terminal-Bench scoring, report:

- successes / eligible trials;
- success rate by agent;
- paired task-level difference; and
- a confidence interval computed by resampling tasks, keeping attempts for the
  same task together.

The task, not the individual attempt, is the independent sampling unit. A
plain trial-level interval would be overconfident when several attempts share
one task.

### Secondary measures

Record and report, without promoting them to the main ranking:

- input, cached-input, reasoning, and output tokens when available;
- model call count and provider-reported cost;
- Harbor wall time and Lightspeed run time;
- time to first model request and first environment operation;
- environment tool calls, tool errors, and output truncations;
- terminal reason: complete, max turns, timeout, cancel, model error, agent
  error, gateway error, or environment disconnect; and
- setup/registration/cleanup duration and failure class.

Metrics with different provider definitions remain separate fields rather
than being coerced into false equivalence. The report shows missing values.

### Per-trial artifacts

Write bounded, redacted files under Harbor's collected log/artifact paths:

```text
/logs/agent/lightspeed/envd.log
/logs/artifacts/lightspeed/registration.json
/logs/artifacts/lightspeed/run.json
/logs/artifacts/lightspeed/provenance.json
/logs/artifacts/lightspeed/trajectory.json
```

`registration.json` contains receipt ids and non-secret correlation metadata.
`run.json` contains session/run ids, status, usage, timings, and failure
classification. `provenance.json` contains version and configuration digests.
No file contains a registration key, Lightspeed API key, model-provider key,
or authorization header.

Export the Lightspeed trajectory in Harbor's supported ATIF shape when the
adapter can do so losslessly enough for debugging. Until then, keep the raw
Lightspeed event artifact and mark trajectory support unavailable rather than
fabricating tool events. Trajectory conversion is diagnostic and does not
affect the verifier reward.

### Run provenance

Every result bundle must make the run reproducible. Record at least:

- Lightspeed repository commit, server build/version, profile revision and
  digest, provider id/API kind, and resolved model configuration;
- `envd` version, target, and SHA-256 checksum;
- adapter repository commit and locked dependency digest;
- Harbor and Codex agent versions;
- dataset name/ref, selected tasks, task/image digests, and verifier versions;
- Harbor environment provider and resource/network settings;
- attempts, concurrency, timeouts, retry configuration, and exclusions;
- UTC start/end times; and
- the redacted preflight result.

`scripts/report.py` reads Harbor results plus these artifacts and produces a
machine-readable summary and a concise Markdown report. It never queries live
state to reconstruct an old run.

## Failure and Retry Semantics

Classify a trial failure at the boundary where it occurred:

| Class | Examples | Score treatment |
|---|---|---|
| Dataset/preflight | Oracle fails, bad image digest, verifier cannot run before either agent | Exclude only if declared for both arms before result review. |
| Compute infrastructure | Harbor provider cannot create sandbox, image pull outage | Harbor infrastructure retry; retain retry history. |
| Harness setup | Bad `envd` binary, registration rejected, adapter contract error | Count as that agent's failure. |
| Agent execution | Agent timeout, max turns, uncaught model error, bad tool call | Count as that agent's failure. |
| Verification | Agent finishes but verifier returns zero/partial reward | Use verifier reward. |
| Artifact-only | Score exists but optional artifact export fails | Keep score; mark diagnostics incomplete. |

Whole-trial retries are allowed only for a small, explicit list of failures
known to occur before the agent can influence the sandbox, such as a remote
provider failing to allocate it. Do not retry a failed verifier, agent
timeout, provider refusal, gateway error, or environment disconnect into a
better score. Native within-run retries made by Codex or Lightspeed remain
part of each harness.

On Harbor cancellation, the adapter immediately requests Lightspeed run
cancellation and then performs bounded cleanup. Cleanup errors cannot replace
the original failure. If the adapter or host is killed, the `envd` connection
drops and P148's ephemeral cleanup closes the registered environment after its
grace interval. Listing environments by the campaign registration key lets
operators find any leak; the Harbor correlation metadata is diagnostic only.

## Network Policy

Remote and local sandboxes need outbound access to their agent control plane:

- the Codex arm needs its model/provider endpoints;
- the Lightspeed arm needs the configured Lightspeed WSS gateway; and
- neither arm needs the other's credential or control endpoint.

These are agent-phase exceptions, not general task internet access. Preserve
the dataset's network policy and, where Harbor/provider support permits,
allowlist only the required hostnames and TLS ports. The task process shares
the sandbox network namespace with `envd`, but it has no Lightspeed API key;
the constrained registration key file has already been deleted.

Preflight makes a real TLS/WebSocket reachability check from the sandbox
without registering an environment, or registers and immediately cleans a
dedicated smoke environment. DNS, proxy, CA, and clock failures should be
found before expensive model trials.

## Test Strategy

### Unit tests

Use fake Harbor `BaseEnvironment` and fake Lightspeed API/gateway clients to
cover:

- platform/artifact selection and checksum rejection;
- command construction without secret values in argv or logs;
- receipt validation and exact environment-id activation;
- model alias rejection, provider mismatch, and no-fallback behavior;
- instruction byte preservation;
- `AgentContext` usage/cost/status projection;
- cancellation and timeout propagation;
- cleanup ordering and preservation of the Harbor sandbox; and
- redaction and bounded artifact generation.

### Contract and local integration tests

- Run a fake `envd` receipt/process against a real Harbor toy environment.
- Run the real `envd` and a local Lightspeed stack on one deterministic toy
  task whose verifier checks a filesystem mutation.
- Interrupt setup, registration, model execution, environment execution, and
  artifact export independently and assert the documented classification.
- Run two or more concurrent trials with the same registration key and prove
  that receipts identify different environments.
- Restart `envd` inside one still-live trial and prove the same ephemeral
  identity reconnects during grace without creating another environment.

Credentialed tests are explicit/ignored and follow the repository's live-test
rules. They are never silently skipped because an environment variable is
missing.

### Benchmark acceptance

1. Oracle preflight passes for the committed smoke tasks on local Docker.
2. One Codex and one Lightspeed trial use the same model and both reach the
   verifier through Harbor.
3. A paired multi-task smoke job completes with no leaked sessions,
   environments, or local processes.
4. The same smoke config, changing only Harbor environment settings, completes
   on one remote sandbox provider.
5. The report reconstructs every score and comparison from retained files.
6. A full pinned Terminal-Bench job completes with predeclared attempts,
   exclusions, and parity manifest.

## Implementation Slices

### Slice 1 — Adapter skeleton and reproducibility

- Create the dedicated repository with its Python package, lockfile, README,
  CI, and custom `BaseAgent` import path.
- Pin the tested Harbor version and record exact job-schema usage.
- Implement config validation, model mapping, provenance manifest, and fake
  clients/environment tests.
- Add a deterministic toy Harbor task and verify artifact collection.

Exit: Harbor can load the agent, expand a job, and run the adapter against
fakes without a source patch or unpinned dependency.

### Slice 2 — Real outbound environment lifecycle

- Package/select a pinned `envd` release artifact.
- Upload it in `setup`; start, register, validate the receipt, and delete the
  key file in `run`.
- Attach the exact P148 environment to a hosted/local Lightspeed session.
- Implement cancellation, teardown, ephemeral close, and leak reconciliation.
- Exercise concurrent registrations with one campaign key.

Exit: a local Harbor toy task is completed by hosted Lightspeed through the
real `envd`, then verified by Harbor in the unchanged sandbox.

### Slice 3 — Complete trial observability

- Project usage, cost, status, and timings into `AgentContext`.
- Export redacted receipts, run metadata, provenance, and Lightspeed events.
- Add ATIF trajectory conversion where the mapping is faithful.
- Implement the failure taxonomy and invariant-preserving infrastructure retry
  allowlist.

Exit: every toy/smoke trial is diagnosable without querying the live service or
exposing credentials.

### Slice 4 — Paired Terminal-Bench comparison

- Commit the Codex and Lightspeed job matrix with exact model/reasoning parity.
- Implement oracle/parity preflight and a representative smoke allowlist.
- Implement paired task-level reporting and confidence intervals.
- Run the local Docker smoke set and audit task instructions, users, resources,
  network, timeouts, and artifacts for equality.

Exit: one command produces a reproducible paired local comparison rather than
two manually assembled result sets.

### Slice 5 — Remote compute and full campaign

- Validate the unchanged adapter on one Harbor remote environment provider.
- Document operator setup for gateway egress, registration-key policy,
  concurrency, quotas, and cleanup.
- Freeze a full-run manifest, run oracle preflight, then execute the paired
  Terminal-Bench campaign.
- Retain the raw Harbor job directory, parity manifest, exclusions, and report.

Exit: Harbor can be kicked off from a developer machine while remote task
compute connects to hosted Lightspeed, with complete results and no leaked
registered environments.

## Acceptance Invariants

- Harbor creates, verifies, and destroys every task sandbox.
- The adapter is independently installable from its own repository and uses
  only released Lightspeed APIs/contracts and `envd` artifacts.
- The Lightspeed adapter is an external `BaseAgent`; no Harbor fork is needed.
- The Lightspeed arm executes through the canonical `lightspeed-envd`, not a
  host-side environment bridge.
- `envd` connects outbound and receives a server-assigned environment id.
- The adapter activates only the environment named in that trial's validated
  receipt.
- One reusable campaign registration key may safely admit concurrent trials;
  each trial still receives a distinct daemon/environment identity.
- The Lightspeed API key and model credential never enter the task sandbox.
- The task instruction, verifier, image, model snapshot, reasoning effort,
  resources, and timeout are matched and recorded.
- Native harness prompts, context management, tools, and recovery remain part
  of the systems being compared.
- Closing the Lightspeed environment never destroys or changes the Harbor
  sandbox before verification.
- Agent/gateway/`envd` failures are visible outcomes, not silently excluded or
  retried into successes.
- Every reported aggregate is derivable from retained raw trials and a frozen
  provenance manifest.

## Follow-ons

After the Terminal-Bench path is stable, the same adapter can be used with any
Harbor dataset whose agent contract is terminal/environment based. Adding a
suite should normally require only pinned dataset configuration, task-specific
preflight, and report interpretation—not a new Lightspeed transport.

Possible later work includes SWE-bench Verified, Aider Polyglot, a
best-available-capabilities track, scheduled hosted campaigns, and deliberate
Lightspeed ablations. None is required to answer the initial question: whether
Lightspeed plus a fixed GPT model can match or outperform Codex plus that same
model end to end.
