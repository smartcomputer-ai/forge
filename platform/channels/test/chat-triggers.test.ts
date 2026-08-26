import type { WorkflowClient } from "@temporalio/client";
import { describe, expect, it, vi } from "vitest";
import {
  selectChatTrigger,
  planChannelAdmission,
  type ChatTriggerCandidate,
  type ChannelControlPlane,
} from "../src/control-plane/chat-triggers.js";
import type { NormalizedInboundV1 } from "../src/contracts/channel.js";
import { admitInbound } from "../src/ingress/admit.js";

const candidate: ChatTriggerCandidate = {
  triggerId: "123e4567-e89b-42d3-a456-426614174000",
  triggerName: "family-chat",
  botId: "0b54d227-08a2-45a8-9b3f-6a4c21d1a222",
  botName: "concierge",
  botEnabled: true,
  channelAccountId: "123e4567-e89b-42d3-a456-426614174001",
  accountProvider: "telegram",
  accountId: "primary",
  universeId: "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f",
  universeName: "Family",
  universeActive: true,
  enabled: true,
  matchScope: "group",
  priority: 100,
  pairingRequired: true,
  pairingCode: "PairCode123",
  paired: true,
  activation: { group: "mention", triggerPrefixes: ["/ask"] },
  access: { turn: "members", control: "admins" },
  memberRole: "admin",
};

const inbound: NormalizedInboundV1 = {
  version: 1,
  messageId: "42",
  route: { provider: "telegram", accountId: "primary", chatId: "-100123" },
  senderId: "7",
  senderName: "Lukas",
  timestampMs: 1_700_000_000_000,
  text: "hello",
  isDirect: false,
  mentionedBot: true,
  isReplyToBot: false,
};

describe("chat trigger admission", () => {
  it("selects only active, matching, paired triggers of enabled bots in priority order", () => {
    expect(
      selectChatTrigger([{ ...candidate, paired: false }, candidate], inbound.route, false),
    ).toMatchObject({
      triggerName: "family-chat",
      botName: "concierge",
      universeId: candidate.universeId,
      authorization: { turnAllowed: true, controlAllowed: true, memberRole: "admin" },
    });
    expect(selectChatTrigger([candidate], inbound.route, true)).toBeNull();
    expect(selectChatTrigger([{ ...candidate, universeActive: false }], inbound.route, false)).toBeNull();
    expect(selectChatTrigger([{ ...candidate, botEnabled: false }], inbound.route, false)).toBeNull();
    expect(
      selectChatTrigger([candidate], { ...inbound.route, accountId: "secondary" }, false),
    ).toBeNull();
    expect(
      selectChatTrigger(
        [
          { ...candidate, triggerName: "later", priority: 200 },
          { ...candidate, triggerName: "first", priority: 10 },
        ],
        inbound.route,
        false,
      ),
    ).toMatchObject({ triggerName: "first" });
  });

  it("resolves the chat trigger before signal-with-start", async () => {
    const resolved = selectChatTrigger([candidate], inbound.route, false);
    if (resolved === null) {
      throw new Error("expected fixture trigger to resolve");
    }
    const resolver: ChannelControlPlane = {
      resolve: vi.fn(async () => resolved),
      admit: vi.fn(async () => ({ status: "bound" as const, trigger: resolved })),
      pairingRequired: vi.fn(async () => false),
    };
    const signalWithStart = vi.fn(async (..._args: unknown[]) => ({ signaledRunId: "run-1" }));
    const client = { signalWithStart } as unknown as WorkflowClient;

    const result = await admitInbound(client, resolver, inbound);

    expect(result).toMatchObject({
      status: "admitted",
      trigger: { triggerName: "family-chat" },
      signaledRunId: "run-1",
    });
    expect(signalWithStart).toHaveBeenCalledOnce();
    const options = signalWithStart.mock.calls[0]?.[1];
    expect(options).toMatchObject({
      args: [
        {
          triggerId: candidate.triggerId,
          botId: candidate.botId,
          botName: "concierge",
          scope: "group",
          activation: { mode: "mention", triggerPrefixes: ["/ask"], mentionNames: [] },
          access: { turn: "members", control: "admins" },
          route: inbound.route,
          label: "telegram group · -100123",
        },
      ],
      signalArgs: [inbound],
    });
    expect(JSON.stringify(options)).not.toContain("sessionKey");
    expect(JSON.stringify(options)).not.toContain("profileId");
  });

  it("does not create a workflow for unbound traffic", async () => {
    const resolver: ChannelControlPlane = {
      resolve: vi.fn(async () => null),
      admit: vi.fn(async () => ({ status: "unbound" as const })),
      pairingRequired: vi.fn(async () => false),
    };
    const signalWithStart = vi.fn();
    const client = { signalWithStart } as unknown as WorkflowClient;
    await expect(admitInbound(client, resolver, inbound)).resolves.toEqual({ status: "unbound" });
    expect(signalWithStart).not.toHaveBeenCalled();
  });

  it("consumes exact pairing codes before any workflow is created", async () => {
    expect(
      planChannelAdmission([{ ...candidate, paired: false }], {
        ...inbound,
        text: "PairCode123",
      }),
    ).toMatchObject({ status: "pair", candidate: { triggerName: "family-chat" } });
    expect(
      planChannelAdmission([{ ...candidate, paired: false }], {
        ...inbound,
        text: "wrong",
      }),
    ).toEqual({ status: "pairing_required" });
    expect(
      planChannelAdmission([{ ...candidate, paired: false }], {
        ...inbound,
        text: "ambient group traffic",
        mentionedBot: false,
      }),
    ).toEqual({ status: "pairing_pending" });
    expect(
      planChannelAdmission([{ ...candidate, pairingRequired: false, pairingCode: null, paired: false }], inbound),
    ).toMatchObject({ status: "bound" });
  });

  it("returns pairing responses without signaling Temporal", async () => {
    const resolver: ChannelControlPlane = {
      resolve: vi.fn(async () => null),
      admit: vi.fn(async () => ({ status: "pairing_required" as const })),
      pairingRequired: vi.fn(async () => true),
    };
    const signalWithStart = vi.fn();
    const client = { signalWithStart } as unknown as WorkflowClient;
    await expect(admitInbound(client, resolver, inbound)).resolves.toEqual({
      status: "pairing_required",
    });
    expect(signalWithStart).not.toHaveBeenCalled();
  });
});
