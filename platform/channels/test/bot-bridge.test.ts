import { describe, expect, it } from "vitest";
import { renderAdmittedEvent } from "@lightspeed/bots/events";
import {
  CHAT_MESSAGE_KIND,
  CHAT_SENT_KIND,
  chatMessageDocument,
  chatPromptData,
  chatSentDocument,
  handleFromDocument,
} from "../src/activities/bot-bridge.js";
import type { EmitChatEventInput, StoreChatSentInput } from "../src/contracts/bridge.js";

const route = { provider: "whatsapp" as const, accountId: "41790000000@s.whatsapp.net", chatId: "120363012345678901@g.us" };
const conversation = { key: "whatsapp:41790000000@s.whatsapp.net:120363012345678901@g.us", label: "whatsapp group · family", scope: "group" as const, route };

const emit: EmitChatEventInput = {
  universeId: "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f",
  triggerId: "7f1c4a9e-2b3d-4c5e-8f6a-1b2c3d4e5f60",
  botId: "0b54d227-08a2-45a8-9b3f-6a4c21d1a222",
  conversation,
  message: {
    messageId: "3EB0C767D26A1D8E9C4F",
    senderId: "41790000001@s.whatsapp.net",
    senderName: "Alice",
    memberRole: "member",
    timestampMs: 1_700_000_000_000,
    text: "can you look at this?",
    isDirect: false,
    mentionedBot: true,
    isReplyToBot: false,
  },
  media: [{ type: "media", blobRef: `sha256:${"9".repeat(64)}`, kind: "image", mime: "image/jpeg", name: "photo.jpg" }],
  toolsRef: `sha256:${"7".repeat(64)}`,
  notify: { workflowId: "lightspeed.channels.v1/x", workflowKind: "channelConversationWorkflowV1", token: "t" },
};

describe("chat event documents", () => {
  it("puts the message line in the summary and the provider ids only in data", () => {
    const document = chatMessageDocument(emit);
    expect(document.kind).toBe(CHAT_MESSAGE_KIND);
    expect(document.source).toBe("whatsapp:41790000000@s.whatsapp.net");
    expect(document.summary).toBe("Alice (2023-11-14 22:13Z): can you look at this?");
    expect(document.summary).not.toContain("3EB0C767D26A1D8E9C4F");
    expect(document.data).toMatchObject({
      conversation: { key: conversation.key, label: conversation.label, chatId: route.chatId },
      sender: { id: emit.message.senderId, name: "Alice", memberRole: "member" },
      messageId: "3EB0C767D26A1D8E9C4F",
      media: [{ kind: "image", mime: "image/jpeg", name: "photo.jpg" }],
    });
    expect(JSON.stringify(document.data)).not.toContain("sha256:");
  });

  it("renders a terse session prompt while retaining the full filter document", () => {
    const textOnly = { ...emit, media: [] };
    const document = chatMessageDocument(textOnly);

    expect(renderAdmittedEvent(17, document, chatPromptData(textOnly))).toBe(
      "── event #17 · chat.message · whatsapp:41790000000@s.whatsapp.net · 2023-11-14 22:13Z\n" +
        "Alice (2023-11-14 22:13Z): can you look at this?",
    );
    expect(document.data).toMatchObject({
      conversation: { key: conversation.key, chatId: route.chatId },
      sender: { id: emit.message.senderId },
      messageId: emit.message.messageId,
      text: emit.message.text,
      mentionedBot: true,
    });
  });

  it("adds only attachment labels below the message line", () => {
    const document = chatMessageDocument(emit);
    const prompt = renderAdmittedEvent(18, document, chatPromptData(emit));

    expect(prompt).toContain("\nAlice (2023-11-14 22:13Z): can you look at this?\n- [image: photo.jpg]");
    expect(prompt).not.toContain("messageId:");
    expect(prompt).not.toContain("chatId:");
  });

  it("caps a very long message line and points at bot_event_read", () => {
    const document = chatMessageDocument({ ...emit, message: { ...emit.message, text: "y".repeat(5_000) } });
    expect(document.summary.length).toBeLessThan(2_100);
    expect(document.summary).toContain("bot_event_read");
    expect((document.data as { text: string }).text).toHaveLength(5_000);
  });

  it("archives a send with its provider ids and direction", () => {
    const input: StoreChatSentInput = {
      universeId: emit.universeId,
      triggerId: emit.triggerId,
      botId: emit.botId,
      conversation: { key: conversation.key, label: conversation.label, route },
      invocationId: `wti:sha256:${"a".repeat(64)}`,
      text: "Sure — looking now.",
      providerMessageIds: ["BAE5F4C1D2E3A9B7"],
      replyTo: 17,
    };
    const document = chatSentDocument(input);
    expect(document.kind).toBe(CHAT_SENT_KIND);
    expect(document.summary).toBe("sent: Sure — looking now.");
    expect(document.data).toMatchObject({ providerMessageIds: ["BAE5F4C1D2E3A9B7"], fromMe: true, replyTo: 17 });
  });
});

describe("chat handles", () => {
  it("resolves inbound and sent rows of the same conversation, and nothing else", () => {
    expect(handleFromDocument(chatMessageDocument(emit), conversation.key)).toEqual({
      providerMessageIds: ["3EB0C767D26A1D8E9C4F"],
      fromMe: false,
      senderId: emit.message.senderId,
      text: "can you look at this?",
    });
    const sent = chatSentDocument({
      universeId: emit.universeId,
      triggerId: emit.triggerId,
      botId: emit.botId,
      conversation: { key: conversation.key, label: conversation.label, route },
      invocationId: "fallback:delivery-1",
      text: "done",
      providerMessageIds: ["BAE5", "BAE6"],
      replyTo: null,
    });
    expect(handleFromDocument(sent, conversation.key)).toEqual({
      providerMessageIds: ["BAE5", "BAE6"],
      fromMe: true,
      text: "done",
    });
    expect(handleFromDocument(sent, "telegram:primary:123")).toBeNull();
    expect(
      handleFromDocument({ ...sent, kind: "schedule.fire", data: { conversation: { key: conversation.key } } }, conversation.key),
    ).toBeNull();
  });
});
