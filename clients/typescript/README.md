# Lightspeed TypeScript Client

Generated TypeScript client for the Lightspeed JSON-RPC gateway.

The public API types and typed method map are generated from the committed
contract artifacts in `crates/api/contract/`. The hand-written code is limited to the
JSON-RPC transport and small workflow helpers.

## Install

Tagged Lightspeed releases publish `@lightspeed-ai/agent-client` to npm. In-tree
consumers use the repository workspace directly.

```bash
npm install @lightspeed-ai/agent-client
```

## Use

```ts
import { LightspeedClient } from "@lightspeed-ai/agent-client";

const lightspeed = new LightspeedClient("http://127.0.0.1:18080/rpc");

const session = await lightspeed.call("session/start", {
  sessionId: "session_123",
  cwd: null,
  config: null,
});

const run = await lightspeed.startRun(
  session.result.session.id,
  [{ type: "text", text: "summarize this repository" }],
);

const terminal = await lightspeed.awaitRun(session.result.session.id, run.result.run.id);

console.log(terminal.state.status, terminal.cursor);
```

Raw calls return the full `AgentApiOutcome<...>` envelope, including any
notifications. JSON-RPC failures throw `LightspeedRpcError` with `code`, `message`,
`kind`, and raw `data` preserved.

`METHOD_INFO` exposes the canonical Rust-authored scope, summary, and
operational description for every method. The generated `rpc.*` helpers carry
the same text as JSDoc, while parameter and result field documentation comes
from the generated schema types.

## Regenerate

```bash
npm install
npm run check --workspace @lightspeed-ai/agent-client
```

`npm run check:generated` regenerates `src/generated/*` and the packaged
schemas, and fails if the committed generated output is stale.

Workflow receivers import `@lightspeed-ai/agent-client/workflow`. That subpath
contains generated emission/start-on-call types and manifest-owned constants,
plus Temporal-sandbox-safe parsing, id derivation, workflow-id, recipe, and
reply helpers. It has no Temporal package dependency.
