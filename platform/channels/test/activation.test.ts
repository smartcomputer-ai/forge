import { describe, expect, it } from "vitest";
import type { NormalizedInboundV1 } from "../src/contracts/channel.js";
import {
  classifyInbound,
  formatMessageLine,
  resolveActivationSettings,
} from "../src/policy/activation.js";

const groupInbound: NormalizedInboundV1 = {
  version: 1,
  messageId: "42",
  route: { provider: "telegram", accountId: "primary", chatId: "-100123" },
  senderId: "7",
  senderName: "Lukas",
  timestampMs: 1_700_000_000_000,
  text: "ordinary group traffic",
  isDirect: false,
  mentionedBot: false,
  isReplyToBot: false,
};

describe("Channels activation policy", () => {
  it("forces direct conversations active and ignores group-only settings", () => {
    expect(resolveActivationSettings("direct", { group: "always" })).toEqual({
      mode: "dm",
      triggerPrefixes: ["/ask", "/lightspeed"],
      mentionNames: [],
    });
  });

  it("drops ambient group traffic and activates native mentions", () => {
    const settings = resolveActivationSettings("group", {
      group: "mention",
      mentionNames: ["lightspeed"],
    });
    expect(classifyInbound(groupInbound, settings)).toEqual({ kind: "drop", reason: "ambient" });
    expect(
      classifyInbound(
        { ...groupInbound, text: "@lightspeed, help please", mentionedBot: true },
        settings,
      ),
    ).toEqual({ kind: "userTurn", text: "help please" });
    expect(
      classifyInbound(groupInbound, resolveActivationSettings("group", { group: "always" })),
    ).toEqual({ kind: "userTurn", text: "ordinary group traffic" });
  });

  it("allows explicit prefixes and keeps media-only messages", () => {
    const settings = resolveActivationSettings("group", { triggerPrefixes: ["/ask"] });
    expect(classifyInbound({ ...groupInbound, text: "/ask investigate" }, settings)).toEqual({
      kind: "userTurn",
      text: "investigate",
    });
    expect(classifyInbound({ ...groupInbound, text: "/ask" }, settings)).toEqual({
      kind: "drop",
      reason: "empty-trigger",
    });
    const photo: NormalizedInboundV1["media"] = [
      { version: 1, provider: "telegram", fileId: "f", kind: "image", mime: "image/jpeg" },
    ];
    expect(
      classifyInbound({ ...groupInbound, text: "", media: photo, isDirect: true }, settings),
    ).toEqual({ kind: "userTurn", text: "" });
  });

  it("renders the message line without provider ids", () => {
    const line = formatMessageLine(groupInbound, "ordinary group traffic");
    expect(line).toBe("Lukas (2023-11-14 22:13Z): ordinary group traffic");
    expect(line).not.toContain("-100123");
    expect(line).not.toContain("#42");
  });

  it("rejects malformed durable activation configuration", () => {
    expect(() => resolveActivationSettings("group", { group: "silent" })).toThrow(
      "activation.group",
    );
    expect(() => resolveActivationSettings("group", { mentionNames: [7] })).toThrow(
      "mentionNames",
    );
  });
});
