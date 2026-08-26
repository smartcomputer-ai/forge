import { describe, expect, it } from "vitest";
import { createFakeDeliveryActivities } from "../src/activities/fake-delivery.js";
import { parseToolOperation, validateDeliveryResult } from "../src/contracts/delivery.js";

describe("channel delivery contracts", () => {
  it("decodes tool arguments as message numbers", () => {
    expect(
      parseToolOperation("channels.message_send.v1", { text: "hello", replyTo: 41 }),
    ).toEqual({ type: "send", text: "hello", replyTo: 41 });
    expect(
      parseToolOperation("channels.message_send.v1", { text: "hello", replyTo: null }),
    ).toEqual({ type: "send", text: "hello", replyTo: null });
    expect(
      parseToolOperation("channels.message_edit.v1", { message: 42, text: "fixed" }),
    ).toEqual({ type: "edit", message: 42, text: "fixed" });
    expect(
      parseToolOperation("channels.message_react.v1", { message: 42, emoji: "👍" }),
    ).toEqual({ type: "react", message: 42, emoji: "👍" });
    expect(() =>
      parseToolOperation("channels.message_send.v1", { text: "hello", replyTo: "41" }),
    ).toThrow("message number");
    expect(() => parseToolOperation("channels.message_react.v1", { message: 0, emoji: "x" })).toThrow(
      "message number",
    );
    expect(() => parseToolOperation("unknown", {})).toThrow("unsupported pushed channel tool");
  });

  it("uses the invocation id as the fake provider idempotency key", async () => {
    const activities = createFakeDeliveryActivities();
    const result = await activities.deliverChannelMessage({
      version: 1,
      invocationId: `wti:sha256:${"a".repeat(64)}`,
      idempotencyKey: `wti:sha256:${"a".repeat(64)}`,
      route: { provider: "telegram", accountId: "primary", chatId: "123" },
      operation: { type: "send", text: "hello" },
    });
    expect(validateDeliveryResult(result, "telegram").messageIds).toEqual([
      `fake:wti:sha256:${"a".repeat(64)}`,
    ]);
  });

  it("rejects empty or cross-provider receipts", () => {
    expect(() =>
      validateDeliveryResult({ version: 1, provider: "whatsapp", messageIds: ["42"] }, "telegram"),
    ).toThrow("does not match");
    expect(() =>
      validateDeliveryResult({ version: 1, provider: "telegram", messageIds: [] }, "telegram"),
    ).toThrow("1 to 32");
  });
});
