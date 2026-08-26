import { describe, expect, it } from "vitest";
import { sessionWorkflowId } from "@lightspeed/agent-client/workflow";
import {
  channelConversationIdentity,
  channelPairingKey,
  channelDeliveryTaskQueue,
} from "../src/identity/ids.js";

const universeId = "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f";
const defaultUniverseId = "00000000-0000-0000-0000-000000000001";

describe("Channels identities", () => {
  it("derives a stable, opaque workflow id and a readable conversation key", () => {
    const route = {
      provider: "whatsapp" as const,
      accountId: "41790000000@s.whatsapp.net",
      chatId: "family@g.us",
    };
    const first = channelConversationIdentity(universeId, route);
    const retry = channelConversationIdentity(universeId, route);

    expect(retry).toEqual(first);
    expect(first.workflowId).toMatch(
      /^lightspeed\.channels\.v1\/6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f\/whatsapp\/[0-9a-f]{64}$/,
    );
    expect(first.workflowId).not.toContain(route.chatId);
    expect(first.conversationKey).toBe("whatsapp:41790000000@s.whatsapp.net:family@g.us");
    expect(first.deliveryTaskQueue).toMatch(/^lightspeed-channels-delivery-v1-whatsapp-[0-9a-f]{24}$/);
  });

  it("changes identity across tenants, chats, and threads", () => {
    const route = { provider: "telegram" as const, accountId: "primary", chatId: "123" };
    const base = channelConversationIdentity(universeId, route);
    expect(channelConversationIdentity(universeId, { ...route, chatId: "456" }).workflowId).not.toBe(
      base.workflowId,
    );
    expect(
      channelConversationIdentity(universeId, { ...route, threadId: "9" }).conversationKey,
    ).toBe("telegram:primary:123:9");
    expect(
      channelConversationIdentity("123e4567-e89b-42d3-a456-426614174000", route).workflowId,
    ).not.toBe(base.workflowId);
  });

  it("accepts Lightspeed's canonical default universe id", () => {
    const identity = channelConversationIdentity(defaultUniverseId, {
      provider: "telegram",
      accountId: "primary",
      chatId: "123",
    });
    expect(identity.workflowId).toContain(`/${defaultUniverseId}/telegram/`);
  });

  it("composes the Lightspeed holder workflow id for a bot's routed session", () => {
    expect(sessionWorkflowId(universeId, "bot:v1:concierge:k-tg-0123abcd")).toBe(
      `${universeId}/bot:v1:concierge:k-tg-0123abcd`,
    );
    expect(() => sessionWorkflowId(universeId, "bad/session")).toThrow(/invalid session id/);
  });

  it("derives an account-affine delivery queue without tenant context", () => {
    expect(channelDeliveryTaskQueue("telegram", "primary")).toBe(
      channelConversationIdentity(universeId, {
        provider: "telegram",
        accountId: "primary",
        chatId: "123",
      }).deliveryTaskQueue,
    );
  });

  it("derives a stable pairing key without exposing chat identity", () => {
    const route = {
      provider: "whatsapp" as const,
      accountId: "primary",
      chatId: "family@g.us",
    };
    const key = channelPairingKey(route);
    expect(channelPairingKey(route)).toBe(key);
    expect(key).toMatch(/^channels-pairing-v1-[0-9a-f]{64}$/);
    expect(key).not.toContain(route.chatId);
  });
});
