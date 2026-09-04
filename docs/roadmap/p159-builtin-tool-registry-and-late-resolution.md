# P159 — Built-in Tool Registry and Late Resolution

**Status**

- Implementation in progress 2026-09-04.
- Simplification is the goal: stable internal identity, one resolver, matching
  execution, and preserved provider transcript. No versioned tool registry.
- Greenfield refactor. Engine records, activity inputs, API projections, and
  internal interfaces may change together. No compatibility aliases, dual
  registries, or legacy schema-materialization path are required.
- Preserve the current model-facing tool behavior, which has been exercised
  through benchmarks. Internal representation is free to change substantially;
  names, descriptions, schemas, defaults, and visible results are the parity
  boundary.
- Builds on the provider presentations in
  [P151](p151-exec-leftover-processes.md), the filesystem boundaries in
  [P113](archive/p113-explicit-vfs-and-environment-tool-domains.md), and the
  execution invariants in [P157](p157-native-mcp-in-mixed-tool-batches.md) and
  [P158](p158-anthropic-hosted-web-tools.md).

## Goal

The engine records the capabilities admitted to a session under stable internal
identities such as `env.run_process` and `vfs.read_file`. The LLM activity resolves
those registrations into the names, schemas, and adapters appropriate for the
actual model of that turn. Returned calls map back to internal identities before
the engine schedules execution.

Built-in descriptions and schemas remain code-owned and are rendered directly
at that boundary. They no longer need to be written to CAS, admitted as schema
refs, and read back for every model request.

```text
Session feature grants and trusted tool declarations
  -> admitted registry: internal identities, definition sources, policies
  -> LLM activity: resolve presentations for the effective turn model
  -> provider-native request plus request-local call bindings
  -> provider response: preserve native transcript, resolve call identities
  -> engine: accept and schedule calls by admitted identity
  -> tool activity: execute using the selected binding and render its result
```

## Current Approach

`crates/tools/src/builtin/mod.rs` already separates logical operations from
presentations. `BuiltinTool` captures domain, operation, surface, variant, and
one-shot behavior. However, `spec_bundle` generates descriptions and schemas as
`ToolDocument`s and wraps their hashes in an engine `FunctionToolSpec`.

`gateway/service/session_toolset.rs` composes that toolset, writes its documents
to CAS, and patches `ToolingState.tools`, keyed by the final `ToolName`. The
LLM adapters then read the same documents to build native provider requests.
The engine never inspects the built-in schema contents. It uses tool membership,
execution policy, parallelism, provider compatibility, and tool choice.

Execution has a separate reconstruction path:
`worker/session_tools.rs::runtime_catalog` builds default toolsets for all three
API kinds and merges their name-keyed bindings. It also generates documents that
this path does not need. This loses the relationship to the presentation that
produced the call. In particular, admission can render a one-shot process tool,
while execution reconstructs `EnvironmentToolsetConfig::basic()`, which enables
continuation. The process adapter uses that option for waiting and timeout
defaults, so preserving only the public name is insufficient.

The current design pins description/schema bytes through CAS, but does not pin
the matching executable adapter. This refactor makes that relationship explicit.

## Decision 1: Registry Identity Is Internal

Keep one admitted, revisioned registry. Key it by an internal tool identity,
with separate types for internal identity and provider-visible name. A built-in
registration identifies an operation; it does not store `Bash`, `exec_command`,
or a provider-specific schema as its identity.

For built-ins, the stable identities should follow the existing logical ids.
VFS and environment operations remain distinct. Workflow registrations retain
their trusted workflow-tool identities; MCP registrations retain server/link
identity. Custom tools may retain an authored name as presentation data without
making that string the execution authority.

The durable registration contains only what admission, scheduling, and later
resolution need:

- Internal identity and definition source.
- Small trusted options that change the admitted contract, including process
  continuation policy and capability-specific restrictions.
- Execution binding or eligibility, parallelism, and the existing execution
  class/retry-safety facts.

The type names below are illustrative; the separation is the contract:

```rust
ToolRegistry = BTreeMap<ToolId, ToolRegistration>

ToolRegistration {
    definition,       // Builtin reference, external definition, or MCP source
    execution,        // Local, workflow-bound, hosted, or admitted MCP policy
    parallelism,
    execution_policy,
}
```

Do not add a second model-name registry to engine state. Provider names and
concrete surfaces are resolved outside the engine. An explicit presentation
override, where supported today, is a resolution input; the provider default is
chosen from the effective turn model, including a run model override.

The engine owns the small protocol types, or they live in a dependency-light
protocol module. The `tools` crate owns the catalog of implementations, schema
builders, and codecs. `engine` must not depend on `tools`, resolve a built-in
schema, or acquire one reducer branch per built-in implementation.

## Decision 2: Definition Source and Execution Are Separate

Being built-in describes where a tool definition comes from. It does not imply
that execution happens inline in a tool activity.

| Tool family | Definition source | Execution remains |
| --- | --- | --- |
| VFS and environment file/process tools | Built-in reference | Domain-specific runtime |
| Concurrency and environment control tools | Built-in reference | Existing control/batch semantics |
| Local web fetch and provider-hosted web tools | Built-in reference with trusted options | Selected local or hosted implementation |
| System subagent and environment-job tools | Built-in reference | Generic workflow-tool binding |
| Externally declared workflow/custom tools | Authored definition refs | Admitted execution binding |
| Remote MCP tools | Admitted server source and runtime discovery | Existing native/provider MCP policy |

Apply this source distinction to all built-in definition producers, including
code-owned workflow-tool definitions, rather than removing schema refs from
filesystem tools while leaving the same detour elsewhere. External workflow
declarations can continue supplying immutable schema refs.

A definition authored in this repository is not automatically a session-runtime
built-in. Bot, channel, and plugin declarations supplied through the generic
workflow protocol may remain external definitions. Do not introduce dependencies
from the engine or LLM resolver onto those application implementations.

Workflow argument/output validation resolves the same definition source as LLM
presentation. It must not require `ToolKind::Function` with an input-schema blob
when the definition is built-in. Completion promises, receiver authorization,
start recipes, binding fingerprints, and reply contracts retain their generic
workflow semantics. A built-in definition never authorizes a new endpoint.

Provider-dependent web resolution must preserve the current routes: hosted
search on OpenAI Responses and Anthropic Messages, hosted fetch on Anthropic,
and guarded local fetch on the other supported routes. Required provider-native
options and request additions are produced with the resolved tool. Hosted calls
remain provider-owned; only client-executable bindings produce schedulable tool
calls. Preserve the small execution facts the engine needs to enforce this
distinction without teaching it provider wire schemas.

## Decision 3: Resolve a Complete Binding Inside the LLM Activity

Resolution consumes the admitted registry snapshot, trusted options, the actual turn model. It produces native
provider tools and a request-local reverse lookup from exposed name to binding.
It is shared by hosted execution and `crates/eval`.

A resolved binding associates:

- The admitted internal registration identity.
- The exposed name, description, input schema, strictness, and provider options.
- The selected argument decoder, result renderer, and presentation variant.
- The admitted execution route and contract options.

The provider request types remain native to their adapters. A common binding
resolver is not a new lowest-common-denominator provider request format.
Built-in rendering returns values directly; it does not create temporary CAS
blobs just to reuse the old function-tool materializer.

### Presentations and expansion

Preserve the existing defaults: OpenAI Responses uses the Codex-like surface,
Anthropic Messages uses the Claude-Code-like surface, and OpenAI Completions
uses Canonical. Use existing explicit presentation overrides where available.
This refactor introduces no new model-name heuristics or prompt tuning.

| Internal identity | Canonical | Codex-like | Claude-Code-like |
| --- | --- | --- | --- |
| `env.run_process` | `run_process` | `exec_command` | `Bash` |
| `env.continue_process` | `continue_process` | `write_stdin` | `BashOutput`, `KillShell` |
| `vfs.read_file` | `vfs_read_file` | `vfs_read_file` | `VfsRead` |

One admitted operation may produce multiple exposed tools. `KillShell` maps to
the continuation operation with its kill variant; the reverse lookup must retain
that variant. When continuation is disabled, omit all continuation exposures and
select the restricted run contract in both presentation and execution.

Resolution may select supported presentations and preserve existing documented
omissions, but cannot add grants. A required capability with no supported
implementation fails with a typed error before provider I/O. Resolving against a
different turn model does not imply support for arbitrary cross-provider replay
of existing provider-native context.

### Names, ordering, and tool choice

Validate the complete exposed namespace after built-ins, custom tools, MCP
expansion, and search helpers have been composed. Duplicate or ambiguous names
fail before the request is sent. Do not silently rename tools, overwrite a
binding, or accept an unadvertised alias from another provider surface.

Registry iteration order is not provider tool order. Preserve the existing
rendered order and placement of provider/MCP helpers using parity fixtures.
Otherwise sorting the new internal ids can change the prompt prefix despite
unchanged descriptions and schemas. Preserve Anthropic cache breakpoints and
other request additions associated with tool ordering.

`ToolChoice::Specific` targets the internal registration. Resolution translates
it to an exposed name. For an operation with several exposures, omission of a
variant selects its designated primary exposure; an explicit variant uses a
stable internal variant identifier. An unavailable choice fails before sending.
The engine validates admitted identity; the resolver validates presentation
availability. Public callers no longer need provider aliases to select a tool.

## Decision 4: Normalize Routing, Preserve Provider History

The response adapter uses the exact reverse lookup built for that request to
resolve each returned client tool call. This also applies to all continuations
within one LLM activity, including Anthropic `pause_turn`.

The engine receives the call id, admitted internal identity, argument/content
refs, and only the provider-neutral facts required for scheduling. Acceptance,
parallelism, workflow lookup, promise controls, and environment batch rules use
that identity. Replace existing checks for public spellings such as `await`
with the corresponding internal identity or admitted execution semantics.

Unknown or unadvertised names remain unavailable calls under the existing tool
error flow. They do not resolve through a global alias table. Malformed tool
arguments remain failures of that call, so one bad call does not discard valid
sibling calls or turn a tool error into an LLM transport failure.

### Use the same resolver for execution

Reuse the existing call records, original provider call, admitted registration,
and originating turn model to select the same argument adapter and result
renderer on execution. Carry the required facts on ordinary activity inputs.
The engine handles internal identities and opaque payload refs; it does not
interpret schemas or provider arguments.

Do not create a separate versioned call envelope or persist a second resolved
catalog. The model cannot choose its grant options or execution destination.
Both per-call and batch-unit execution use the admitted registration and original
turn model, including one-shot policy and the original exposed variant. Retries
and parked batches retain those facts rather than consulting current defaults.

Delete the default all-provider `runtime_catalog` reconstruction path and the
old built-in schema-document builders. A runtime lookup must use the shared
resolver and original call facts, rather than infer policy from a public name.

### Transcript and results

Keep the original provider tool call, exposed name, call id, and argument shape
in provider-native context. Separating dispatch identity must not rewrite past
`Bash` calls into `env.run_process` or re-render old calls using a new model's
surface. Result items pair with the original call ids and use the selected
renderer, preserving current visible text, handles, truncation, and errors.

The current `ObservedToolCall.tool_name` and transcript tool name cannot continue
serving as a single routing/display identity. Change the records and projections
explicitly. API views distinguish internal registration ids from historical
display names; the UI continues to show the name used in the actual exchange.

## Decision 5: Preserve Adjacent Execution Boundaries

MCP inventory remains runtime-owned and is not expanded into engine registry
entries. Its exposed names join the request-local namespace and normalize to the
admitted server identity plus remote tool identity. Search helper calls retain
their admitted server scope. Auth, allowlists, approval decisions, and fresh
credential resolution remain enforced on execution and re-dispatch.

Keep native MCP dispatch identical on per-call and mixed batch paths. Identity
normalization must not send injected calls into the built-in executor or convert
provider-hosted MCP calls into client effects. Preserve existing discovery,
search, tool-count, and name-length behavior while composing the final catalog.

VFS and environments retain separate identities, contexts, and permissions.
Environment selection remains a batch fact, and the selected environment id is
captured for execution as today. No filesystem overlay or implicit sync is
introduced. Workflow-backed tools continue through the generic workflow-tool
protocol, including their joined/explicit-promise and cancellation behavior.

## Replay and Deployment

Built-in definitions and adapters ship together with the runtime. There is no
per-tool definition version, historical implementation registry, or fallback to
the old blob-backed built-in path. Code changes can change the definitions used
by future activity executions; the parity fixtures make those changes explicit.

Event replay reduces recorded internal identities and payload refs without
loading definitions or executing provider codecs. Existing provider-native
transcript remains exact. Reissuing an activity uses the deployed implementation
with its original admitted options and turn model. This is the same deployment
contract as other runtime-owned argument adapters.

## Model-Facing Parity

Before replacing the current path, capture fixtures from it for the supported
provider/feature matrix. The existing executable presentation is the baseline;
do not reconstruct it from older roadmap prose or copy newer upstream harness
definitions as part of this refactor.

The default acceptance is equality of the rendered tool contract and relevant
request fields, including tool order, descriptions, schema details, strictness,
provider options, cache markers, and helper placement. Compare strings exactly
and JSON structurally where object-key order has no meaning; preserve array
order. Freeze representative argument decoding and visible result behavior as
well: defaults, units, process handles, PTY/stdin, polling, kill, one-shot policy,
and error/truncation formatting.

Correcting a demonstrated mismatch between the advertised contract and execution
is allowed, including the one-shot reconstruction issue above. Record the
specific deviation and test it; do not use the refactor to rewrite prompts or
silently change the process substrate. Benchmark parity is behavioral evidence,
not a promise that stochastic scores will be numerically identical.

## Implementation Slices

### Slice 1 — Capture the parity baseline

- Add provider request and tool-result fixtures around the current builders,
  covering VFS, environment tools, one-shot/continuation, concurrency, web,
  workflow tools, and mixed MCP configurations.
- Retain the benchmark profile, model/reasoning configuration, and existing
  baseline artifacts for a representative rerun. Use the evaluation workflow
  described in [P149](p149-harbor-end-to-end-agent-evaluation.md).

### Slice 2 — Internal registry and definition sources

- Replace final-name keys and routing references in `engine` with internal ids;
  adapt tool choice, events, call facts, policy lookup, and workflow definitions.
- Split built-in registration from rendering in `tools::toolset` and all
  code-owned definition producers. Keep externally authored schema refs.
- Change gateway reconciliation to admit registrations without generating or
  storing built-in schema documents. Preserve revision guards and grant checks.

### Slice 3 — Resolution and call normalization

- Share binding resolution between the three LLM adapters and evaluation.
  Build native requests and reverse lookups using the effective turn model.
- Implement expansion, namespace validation, ordering, and specific-tool choice.
- Normalize returned identities, carry admitted call facts through execution,
  and preserve native transcript and continuation behavior.

### Slice 4 — Execution and consumers

- Replace default runtime catalog reconstruction with selected binding dispatch;
  use the same inputs on per-call and batch-unit paths, including re-dispatch.
- Update workflow validation, MCP normalization, promise/environment controls,
  API projections, CLI/web displays, and evaluation harness setup.
- Regenerate API and workflow contracts and TypeScript consumers after their
  source contracts change. Remove obsolete helpers and tests that enforce
  built-in schema storage; retain external-definition storage coverage.
- Update `README.md`, `docs/design.md`, and the relevant feature documentation
  when the implementation changes the architecture. Keep this proposal marked
  proposed until implementation and verification are complete.

## Verification and Acceptance

- A built-in-only toolset can be admitted and rendered with no description or
  schema blobs in CAS. External definitions still load and validate their refs.
- Engine replay and checkpoint rehydration use internal identities and refs,
  with no schema resolution, registry I/O, or provider codec execution.
- One internal registry resolves to the expected three presentations, including
  run model overrides, explicit presentation overrides, and one-to-many tools.
- Golden request/result fixtures preserve the benchmarked surface. Repeated
  resolution preserves ordering and cache prefixes.
- Round trips execute the intended operation and variant. Cover `exec_command`,
  `Bash`, `BashOutput`, `KillShell`, VFS names, one-shot defaults, and invalid
  arguments beside valid calls.
- Cross-surface aliases, name collisions, missing choices, ungranted operations,
  and unsupported presentations fail clearly without dispatching another tool.
- A call survives worker reconstruction, retry, and batch park/resume using its
  original codec/options. Per-call and batch-unit routing remain equivalent.
- Provider-hosted web/MCP calls remain hosted; workflow and native MCP mixtures
  preserve existing approval, promise, cancellation, and completion behavior.
- Transcript replay preserves original exposed names, raw arguments, call ids,
  result pairing, and provider-native blocks after internal identity changes.
- Generated contracts and consumers are current. Scope Rust checks to `engine`,
  `tools`, `llm-runtime`, `temporal-workflow`, `temporal-server`, `api`,
  `api-projection`, and `eval`, then run the TypeScript checks for changed clients.
- Once the developer confirms the environment and credentials are safe, run the
  relevant ignored integration suites with their required serialization and a
  representative benchmark rerun. Record model/config parity and investigate
  material regressions; offline fixtures alone do not establish benchmark parity.

## Out of Scope

- New tool capabilities, different model prompts, new provider presentations,
  or changing process/environment semantics to imitate another harness.
- A plugin loader, versioned tool registry, new call-envelope storage layer,
  remote schema registry, or a second durable expanded catalog.
- Eager normalization of all provider requests into a common wire format.
