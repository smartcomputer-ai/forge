import { describe, expect, it, vi } from "vitest";
import type { SessionView } from "@lightspeed/agent-client";
import {
  createLightspeedActivities,
  extractAssistantText,
  runUsedMessagingTool,
} from "../src/activities/lightspeed.js";
import {
  CHANNEL_TOOL_DEADLINE_MS,
  CHANNEL_TOOL_DESCRIPTIONS,
  CHANNEL_TOOL_IDS,
  CHANNEL_TOOL_SCHEMAS,
} from "../src/contracts/tools.js";

const universeId = "6f3fac85-1ec8-4c27-9c97-f403355d5e6f";

describe("putChatToolDeclarations", () => {
  it("stores the tool assets and the receiver-bound declaration array in CAS", async () => {
    const requests: Array<{ headers: Headers; body: Record<string, unknown> }> = [];
    const toolAssetCount =
      Object.keys(CHANNEL_TOOL_SCHEMAS).length + Object.keys(CHANNEL_TOOL_DESCRIPTIONS).length;
    const fetch = vi.fn<typeof globalThis.fetch>(async (_input, init) => {
      const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
      requests.push({ headers: new Headers(init?.headers), body });
      const blobs = (body.params as { blobs: unknown[] }).blobs;
      return Response.json({
        id: body.id,
        result: {
          result: {
            blobs: blobs.map((_, index) => ({
              blobRef: blobs.length === 1 ? `sha256:${"d".repeat(64)}` : `blob:tool-asset-${index}`,
            })),
          },
        },
      });
    });
    const activities = createLightspeedActivities({ endpoint: "http://lightspeed.test/rpc", fetch });
    const receiver = {
      workflowId: "lightspeed.channels.v1/workflow",
      workflowKind: "channelConversationWorkflowV1",
    };

    const result = await activities.putChatToolDeclarations({ universeId, receiver });

    expect(result).toEqual({ toolsRef: `sha256:${"d".repeat(64)}`, toolIds: CHANNEL_TOOL_IDS });
    expect(requests).toHaveLength(2);
    expect(requests[0]?.headers.get("x-lightspeed-universe")).toBe(universeId);
    expect((requests[0]?.body.params as { blobs: unknown[] }).blobs).toHaveLength(toolAssetCount);
    const declarationBlob = (requests[1]?.body.params as { blobs: Array<{ bytesBase64: string }> })
      .blobs[0];
    const declarations = JSON.parse(
      Buffer.from(declarationBlob?.bytesBase64 ?? "", "base64").toString("utf8"),
    ) as Array<Record<string, unknown>>;
    expect(declarations).toHaveLength(4);
    for (const tool of declarations.slice(0, 3)) {
      expect(tool).toMatchObject({
        target: { type: "bound", receiver, dispatch: "push" },
        completion: { type: "joined", deadlineAfterMs: CHANNEL_TOOL_DEADLINE_MS },
      });
    }
    expect(declarations[3]).toMatchObject({
      target: { type: "bound", receiver, dispatch: "pull" },
      completion: { type: "accepted" },
    });
  });

  it("reads and writes JSON through universe-scoped CAS calls", async () => {
    const requests: Array<Record<string, unknown>> = [];
    const fetch = vi.fn<typeof globalThis.fetch>(async (_input, init) => {
      const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
      requests.push(body);
      if (body.method === "blobs/read") {
        const bytes = Buffer.from(JSON.stringify({ text: "hello" }), "utf8");
        return Response.json({
          id: body.id,
          result: {
            result: {
              blobRef: `sha256:${"a".repeat(64)}`,
              bytes: bytes.byteLength,
              bytesBase64: bytes.toString("base64"),
            },
          },
        });
      }
      return Response.json({
        id: body.id,
        result: { result: { blobs: [{ blobRef: `sha256:${"b".repeat(64)}`, bytes: 42 }] } },
      });
    });
    const activities = createLightspeedActivities({ endpoint: "http://lightspeed.test/rpc", fetch });

    await expect(
      activities.readJsonBlob({ universeId, blobRef: `sha256:${"a".repeat(64)}` }),
    ).resolves.toEqual({ text: "hello" });
    await expect(activities.putJsonBlob({ universeId, value: { sent: 18 } })).resolves.toEqual({
      blobRef: `sha256:${"b".repeat(64)}`,
    });
    expect(requests.map((request) => request.method)).toEqual(["blobs/read", "blobs/put"]);
  });
});

describe("reconcileDelivery", () => {
  it("reconciles successful messaging tools from the authoritative run log", () => {
    const session = {
      runs: [
        {
          id: "run_1",
          entries: [
            { kind: { type: "message", role: "assistant" }, text: "first" },
            { kind: { type: "message", role: "assistant" }, text: "second" },
            { kind: { type: "toolCall", name: "message_noop", callId: "call-1" } },
            { kind: { type: "toolResult", callId: "call-1", isError: false } },
          ],
        },
      ],
    } as unknown as SessionView;
    expect(runUsedMessagingTool(session, "run_1")).toBe(true);
    expect(extractAssistantText(session, "run_1")).toBe("first\n\nsecond");
  });

  it("sends the assistant text when the run answered without a messaging tool", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>(async () =>
      Response.json({
        id: 1,
        result: {
          result: {
            session: {
              runs: [
                {
                  id: "run_7",
                  entries: [
                    { kind: { type: "message", role: "assistant" }, text: "projected assistant response" },
                  ],
                },
              ],
            },
          },
        },
      }),
    );
    const activities = createLightspeedActivities({ endpoint: "http://lightspeed.test/rpc", fetch });

    await expect(
      activities.reconcileDelivery({
        universeId,
        sessionId: "bot:v1:concierge:k-x-0123abcd",
        runId: "run_7",
        status: "handled",
      }),
    ).resolves.toEqual({ action: "deliver", text: "projected assistant response" });
  });

  it("reports failures and suppresses deliveries without a run", async () => {
    const fetch = vi.fn<typeof globalThis.fetch>();
    const activities = createLightspeedActivities({ endpoint: "http://lightspeed.test/rpc", fetch });
    await expect(
      activities.reconcileDelivery({ universeId, sessionId: "s", runId: "run_1", status: "run_failed" }),
    ).resolves.toEqual({ action: "deliver", text: "I couldn't complete that request." });
    await expect(
      activities.reconcileDelivery({ universeId, sessionId: "s", runId: null, status: "appended" }),
    ).resolves.toEqual({ action: "suppress", reason: "no_run" });
    expect(fetch).not.toHaveBeenCalled();
  });
});
