# P157 — Native MCP in Mixed Tool Batches

**Status**

- Proposed 2026-09-03 after diagnosing a production run of
  `bot:v1:requirements-engineer-and-ux-g3` on ls.bot. Two injected Notion MCP
  calls failed when the model emitted them beside a managed workflow tool in
  one turn, then succeeded unchanged when retried alone. Later native MCP
  calls also succeeded in a parallel-only batch.

## Problem

A model turn may mix an admitted workflow tool with injected native MCP
calls. The engine accepts the calls and the provider-facing toolset exposes
all of them, but the execution boundary cannot run that combination.

Any batch containing a workflow tool or `await` takes the batch-unit path.
That path sends the original `ToolInvocationBatchRequest` to
`SessionTools::invoke_batch`, where ordinary calls fall through to the local
inline tool runtime. Injected MCP names are not local built-ins, so the calls
fail with:

```text
unsupported tool capability: unknown tool: mcp_notion__notion-fetch
```

Native MCP routing is currently materialized only on the ordinary per-call
path. There, `remote_mcp_call_runtime` resolves the injected name and adds a
`RemoteMcpCallRuntime` to `ToolInvocationCallRequest`; the worker activity
then dispatches through `NativeMcpExecutor`. The batch-unit request carries
no equivalent per-call routing facts, and its activity never consults the
native MCP executor.

This is not a Notion, authentication, rate-limit, or general parallelism
failure. In the production session:

- the first batch contained `bot_event_read` and two
  `mcp_notion__notion-fetch` calls;
- both MCP calls failed within the same append with the identical local
  `unknown tool` error;
- the same fetch argument blobs succeeded on the next two turns when each
  call used the per-call path;
- three later Notion searches completed successfully in one parallel native
  MCP batch.

The user-visible symptom therefore depends on the sibling call: native MCP
works until any call in the same model turn requires batch-unit execution.

## Cursory Solution Direction

Make batch-unit execution aware of native MCP rather than allowing injected
MCP calls to fall through to `InlineToolRuntime`.

One likely shape is to materialize the same remote-MCP runtime facts used by
the per-call path for every call before starting the batch activity. The
batch activity can then route annotated MCP calls through the existing
`NativeMcpExecutor` while keeping workflow-tool calls on their current
promise/emission path. The implementation should reuse the per-call result,
asset, approval, timeout, and error projection behavior rather than create a
second MCP protocol adapter.

Splitting a mixed model batch into a workflow-tool unit and separate per-call
activities is another option, but it needs careful treatment of batch-scoped
promise allocation, workflow-tool emission ordering, cancellation, partial
completion, approval continuation, and deterministic replay. Preserving one
logical batch and teaching its activity to dispatch each call by kind appears
like the smaller conceptual change.

Until this is fixed, disabling provider parallel tool use or instructing a
profile not to combine managed workflow tools with MCP calls avoids the bad
path, but neither is a correctness solution.

## Acceptance

- A single model turn containing one managed workflow tool and two injected
  native MCP calls completes all three calls through their intended runtimes.
- The MCP calls receive the same target, authorization, allowlist, approval,
  timeout, asset handling, and model-visible result behavior as the existing
  per-call path.
- A real MCP failure is reported as that server/transport failure; an
  injected MCP name never reaches the inline runtime as an unknown tool.
- Existing workflow-tool-only, `await`, native-MCP-only parallel, approval,
  cancellation, and deterministic replay coverage remains green.

## Non-Goals

- Changing the provider's decision to emit parallel tool calls.
- Making Notion-specific routing or retry policy.
- Changing workflow-tool promise semantics merely to work around the missing
  native MCP dispatch case.
