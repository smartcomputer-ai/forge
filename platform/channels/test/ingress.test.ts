import type { WorkflowClient } from "@temporalio/client";
import { describe, expect, it, vi } from "vitest";
import {
  CHANNEL_INBOUND_SIGNAL,
  CHANNEL_CONVERSATION_WORKFLOW,
  CHANNELS_WORKFLOW_TASK_QUEUE,
  conversationLabel,
  type ChannelConversationStartV1,
  type NormalizedInboundV1,
} from "../src/contracts/channel.js";
import { channelConversationIdentity } from "../src/identity/ids.js";
import { signalInbound } from "../src/ingress/signal.js";
import { channelConversationSearchAttributes } from "../src/contracts/search-attributes.js";

const universeId = "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f";
const route = { provider: "telegram" as const, accountId: "primary", chatId: "123" };
const identity = channelConversationIdentity(universeId, route);
const start: ChannelConversationStartV1 = {
  version: 1,
  universeId,
  triggerId: "7f1c4a9e-2b3d-4c5e-8f6a-1b2c3d4e5f60",
  botId: "0b54d227-08a2-45a8-9b3f-6a4c21d1a222",
  botName: "concierge",
  scope: "direct",
  activation: { mode: "dm", triggerPrefixes: ["/ask", "/lightspeed"], mentionNames: [] },
  access: { turn: "conversation", control: "admins" },
  route,
  label: "telegram dm · Lukas",
  deliveryTaskQueue: identity.deliveryTaskQueue,
};
const inbound: NormalizedInboundV1 = {
  version: 1,
  messageId: "42",
  route,
  senderId: "7",
  senderName: "Lukas",
  timestampMs: 1_700_000_000_000,
  text: "hello",
  isDirect: true,
  mentionedBot: false,
  isReplyToBot: false,
};
const admitted = {
  ...inbound,
  authorization: { turnAllowed: true, controlAllowed: false, memberRole: null },
} as const;

describe("signalInbound", () => {
  it("uses deterministic identity and signal-with-start", async () => {
    const signalWithStart = vi.fn(async () => ({ signaledRunId: "run-1" }));
    const client = { signalWithStart } as unknown as WorkflowClient;

    await expect(signalInbound(client, start, admitted)).resolves.toEqual({
      workflowId: identity.workflowId,
      signaledRunId: "run-1",
    });
    expect(signalWithStart).toHaveBeenCalledWith(CHANNEL_CONVERSATION_WORKFLOW, {
      workflowId: identity.workflowId,
      taskQueue: CHANNELS_WORKFLOW_TASK_QUEUE,
      args: [start],
      signal: CHANNEL_INBOUND_SIGNAL,
      signalArgs: [admitted],
      typedSearchAttributes: channelConversationSearchAttributes(start),
    });
  });

  it("rejects identity drift before contacting Temporal", async () => {
    const signalWithStart = vi.fn();
    const client = { signalWithStart } as unknown as WorkflowClient;

    await expect(
      signalInbound(client, { ...start, deliveryTaskQueue: "wrong-queue" }, admitted),
    ).rejects.toThrow("deliveryTaskQueue must be");
    await expect(
      signalInbound(client, start, { ...admitted, route: { ...route, chatId: "999" } }),
    ).rejects.toThrow("must match the conversation route");
    expect(signalWithStart).not.toHaveBeenCalled();
  });

  it("labels conversations by counterpart, never by a provider message id", () => {
    expect(conversationLabel(inbound)).toBe("telegram dm · Lukas");
    expect(
      conversationLabel({ ...inbound, isDirect: false, route: { ...route, chatId: "-100", threadId: "3" } }),
    ).toBe("telegram group · -100 · thread 3");
  });
});
