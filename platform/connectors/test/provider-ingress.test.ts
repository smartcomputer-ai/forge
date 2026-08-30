import { describe, expect, it } from "vitest";
import { MAX_IMAGE_BYTES } from "../src/media/validation.js";
import { normalizeTelegramInbound } from "../src/providers/telegram/ingress.js";
import { normalizeWhatsAppInbound } from "../src/providers/whatsapp/ingress.js";

describe("provider ingress normalization", () => {
  it("normalizes Telegram topics, mentions, and replies into the core envelope", () => {
    const text = "@Lightspeed hello";
    expect(
      normalizeTelegramInbound(
        { botId: 99, botUsername: "Lightspeed" },
        {
          messageId: 42,
          chatId: -100123,
          chatType: "supergroup",
          threadId: 7,
          senderId: 12,
          timestampMs: 1_700_000_000_000,
          senderFirstName: "Lukas",
          senderLastName: "Müller",
          text,
          entities: [{ type: "mention", offset: 0, length: 11 }],
          replyToSenderId: 99,
        },
      ),
    ).toEqual({
      messageId: "42",
      chatId: "-100123",
      threadId: "7",
      senderId: "12",
      senderName: "Lukas Müller",
      timestampMs: 1_700_000_000_000,
      text,
      mentionedBot: true,
      isReplyToBot: true,
      isDirect: false,
    });
  });

  it("drops empty and self-authored Telegram messages", () => {
    const base = {
      messageId: 1,
      chatId: 1,
      chatType: "private" as const,
      senderId: 99,
      timestampMs: 1_700_000_000_000,
      text: "hello",
    };
    expect(normalizeTelegramInbound({ botId: 99 }, base)).toBeNull();
    expect(normalizeTelegramInbound({ botId: 99 }, { ...base, senderId: 12, text: "  " })).toBeNull();
  });

  it("normalizes Telegram photos and media-only messages without bytes", () => {
    const inbound = normalizeTelegramInbound(
      { botId: 99 },
      {
        messageId: 2,
        chatId: 1,
        chatType: "private",
        senderId: 12,
        timestampMs: 1_700_000_000_000,
        photos: [
          { fileId: "small", width: 90, height: 90, fileSize: 100 },
          { fileId: "large", width: 1280, height: 720, fileSize: 1_000 },
          { fileId: "too-large", width: 2_000, height: 2_000, fileSize: MAX_IMAGE_BYTES + 1 },
        ],
      },
    );
    expect(inbound).toMatchObject({
      text: "(sent an image)",
      isDirect: true,
      media: [{ fileId: "large", kind: "image", mime: "image/jpeg", name: "photo.jpg", byteSize: 1_000 }],
    });
    expect(inbound?.media).toHaveLength(1);
    expect(JSON.stringify(inbound)).not.toContain("bytesBase64");
    expect(JSON.stringify(inbound)).not.toContain("provider");
  });

  it("normalizes Telegram documents and voice notes by admitted MIME", () => {
    const base = { messageId: 3, chatId: 1, chatType: "private" as const, senderId: 12, timestampMs: 1 };
    expect(
      normalizeTelegramInbound(
        { botId: 99 },
        { ...base, document: { fileId: "doc", fileName: "notes.md", mimeType: "application/octet-stream" } },
      ),
    ).toMatchObject({ text: "(sent a file: notes.md)", media: [{ kind: "document", mime: "text/markdown" }] });
    expect(
      normalizeTelegramInbound({ botId: 99 }, { ...base, voice: { fileId: "v", mimeType: "audio/ogg" } }),
    ).toMatchObject({ text: "(sent a voice note)", media: [{ kind: "audio", mime: "audio/ogg", name: "voice.ogg" }] });
    expect(
      normalizeTelegramInbound(
        { botId: 99 },
        { ...base, document: { fileId: "exe", fileName: "setup.exe", mimeType: "application/octet-stream" } },
      ),
    ).toBeNull();
  });

  it("normalizes WhatsApp group mentions across device-qualified JIDs", () => {
    expect(
      normalizeWhatsAppInbound(
        { ownJids: new Set(["41790000000:4@s.whatsapp.net"]) },
        {
          messageId: "wamid-1",
          remoteJid: "family@g.us",
          participantJid: "41791111111@s.whatsapp.net",
          pushName: "Lukas",
          timestampMs: 1_700_000_000_000,
          text: "@41790000000 hello",
          mentionedJids: ["41790000000@s.whatsapp.net"],
          quotedParticipantJid: "41790000000:7@s.whatsapp.net",
        },
      ),
    ).toEqual({
      messageId: "wamid-1",
      chatId: "family@g.us",
      senderId: "41791111111@s.whatsapp.net",
      senderName: "Lukas",
      timestampMs: 1_700_000_000_000,
      text: "hello",
      mentionedBot: true,
      isReplyToBot: true,
      isDirect: false,
    });
  });

  it("drops own WhatsApp messages and names direct senders by JID user", () => {
    const own = new Set(["41790000000@s.whatsapp.net"]);
    expect(
      normalizeWhatsAppInbound(own.size ? { ownJids: own } : { ownJids: own }, {
        messageId: "w",
        remoteJid: "41791111111@s.whatsapp.net",
        timestampMs: 1,
        text: "hi",
        fromMe: true,
      }),
    ).toBeNull();
    expect(
      normalizeWhatsAppInbound({ ownJids: own }, {
        messageId: "w",
        remoteJid: "41791111111@s.whatsapp.net",
        timestampMs: 1,
        text: "hi",
      }),
    ).toMatchObject({ isDirect: true, senderId: "41791111111@s.whatsapp.net", senderName: "41791111111" });
  });
});
