import { ApplicationFailure } from "@temporalio/common";
import { describe, expect, it } from "vitest";
import {
  deliveryInputItems,
  steerInputItems,
  validateCarriedDeclarations,
} from "../src/activities/lightspeed.js";
import { parseTriggerPutArgs } from "../src/activities/tools.js";
import { triggerToolView } from "../src/activities/tool-views.js";
import type { BotTriggerRow } from "../src/config.js";
import { chatSpecInput, triggerCreateInput } from "../src/config.js";
import {
  chatMessageEventId,
  chatSentEventId,
  validateBotEvent,
  type BotEvent,
} from "../src/contracts/bots.js";
import { computeRouteSession, type FilterContext } from "../src/webhooks.js";

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;
const DIGEST = /[0-9a-f]{32}/i;
const ref = `sha256:${"a".repeat(64)}`;
const triggerUuid = "7f1c4a9e-2b3d-4c5e-8f6a-1b2c3d4e5f60";
const accountUuid = "123e4567-e89b-42d3-a456-426614174001";

const context: FilterContext = {
  event: { id: "chat:x", kind: "chat.message", source: "telegram:primary", occurredAt: "2026-08-26T10:00:00Z" },
  data: { conversation: { key: "telegram:primary:123", label: "telegram dm · Lukas" }, text: "hi" },
  headers: {},
};

describe("chat routing", () => {
  it("routes a conversation to its own keyed session with the conversation label", () => {
    const routed = computeRouteSession("concierge", null, "chat", { eventId: "chat:x", data: context.data }, context);
    expect(routed.session).toMatchObject({ label: "telegram dm · Lukas" });
    expect(routed.session?.sessionId).toMatch(/^bot:v1:concierge:k-telegram-primary-123-[0-9a-f]{8}$/);
    expect(routed.error).toBeUndefined();
    // The main session cannot carry a conversation's reply tools: `bot` is forced to perKey.
    expect(computeRouteSession("concierge", { policy: "bot" }, "chat", { eventId: "chat:x", data: context.data }, context).session?.sessionId)
      .toBe(routed.session?.sessionId);
    expect(
      computeRouteSession("concierge", { policy: "perEvent" }, "chat", { eventId: "chat:x", data: context.data }, context).session?.sessionId,
    ).toMatch(/^bot:v1:concierge:e-/);
  });

  it("derives deterministic ids for inbound messages and sends", () => {
    expect(chatMessageEventId(triggerUuid, "telegram:primary:123", "42")).toBe(
      chatMessageEventId(triggerUuid, "telegram:primary:123", "42"),
    );
    expect(chatMessageEventId(triggerUuid, "telegram:primary:123", "42")).not.toBe(
      chatMessageEventId(triggerUuid, "telegram:primary:456", "42"),
    );
    expect(chatSentEventId(triggerUuid, "wti:1")).toMatch(/^chat-sent:/);
    expect(chatSentEventId(triggerUuid, "wti:1")).toBe(chatSentEventId(triggerUuid, "wti:1"));
  });
});

describe("chat event fields", () => {
  it("validates media, carried tools, notify, and retention on events", () => {
    const event: BotEvent = {
      version: 1,
      id: "chat:1",
      ref,
      seq: 17,
      media: [{ blobRef: ref, kind: "image", mime: "image/jpeg", name: "photo.jpg" }],
      tools: ref,
      notify: true,
      session: { sessionId: "bot:v1:concierge:k-x-0123abcd", label: "x", ttlMs: null },
    };
    validateBotEvent(event);
    expect(() => validateBotEvent({ ...event, tools: "sha256:short" })).toThrow(TypeError);
    expect(() => validateBotEvent({ ...event, media: [{ blobRef: ref, kind: "video" as "image", mime: "x" }] })).toThrow(TypeError);
    expect(() => validateBotEvent({ ...event, media: Array.from({ length: 9 }, () => ({ blobRef: ref, kind: "image" as const, mime: "image/png" })) })).toThrow(TypeError);
    expect(() => validateBotEvent({ ...event, session: { sessionId: "s", label: "x", ttlMs: 10 } })).toThrow(TypeError);
  });

  it("appends attachments after each event's rendering in runs and steers", () => {
    const events: BotEvent[] = [
      { version: 1, id: "a", ref, promptRef: ref, media: [{ blobRef: ref, kind: "image", mime: "image/jpeg", name: "photo.jpg" }] },
      { version: 1, id: "b", ref, promptRef: ref },
    ];
    expect(deliveryInputItems(events)).toEqual([
      { type: "text", text: expect.stringContaining("2 events") },
      { type: "textRef", blobRef: ref },
      { type: "media", blobRef: ref, kind: "image", mime: "image/jpeg", name: "photo.jpg" },
      { type: "textRef", blobRef: ref },
    ]);
    expect(steerInputItems(events.slice(0, 1))).toEqual([
      { type: "text", text: expect.stringContaining("1 more event") },
      { type: "textRef", blobRef: ref },
      { type: "media", blobRef: ref, kind: "image", mime: "image/jpeg", name: "photo.jpg" },
    ]);
  });
});

describe("carried tool declarations", () => {
  const declaration = (name: string) => ({
    definition: { toolId: `channels.${name}.v1`, revision: 2, semanticType: "x", tool: { name, parallelism: "exclusive", kind: { type: "function", inputSchemaRef: ref, descriptionRef: ref, strict: true } } },
    target: { type: "bound", receiver: { workflowId: "w", workflowKind: "k" }, dispatch: "push" },
    completion: { type: "accepted" },
  });

  it("accepts a well-formed array and refuses collisions with bot_* tools", () => {
    expect(validateCarriedDeclarations([declaration("message_send")])).toHaveLength(1);
    expect(() => validateCarriedDeclarations([declaration("bot_status")])).toThrow(ApplicationFailure);
    expect(() => validateCarriedDeclarations([declaration("a"), declaration("a")])).toThrow(ApplicationFailure);
    expect(() => validateCarriedDeclarations({ not: "an array" })).toThrow(ApplicationFailure);
    expect(() => validateCarriedDeclarations([{ definition: {} }])).toThrow(ApplicationFailure);
  });
});

describe("chat triggers for the model", () => {
  const trigger: BotTriggerRow = {
    id: triggerUuid,
    botId: "0b54d227-08a2-45a8-9b3f-6a4c21d1a222",
    name: "family-chat",
    kind: "chat",
    spec: { channelAccountId: accountUuid, matchScope: "direct", activation: null, access: null, pairingCode: "PairCode1234", priority: 100 },
    filter: null,
    route: { policy: "perKey", key: null },
    coalesce: { debounceMs: 400, maxWaitMs: 1_500, maxCount: 8 },
    deliver: null,
    cursor: null,
    sessionTtlMs: 0,
    enabled: true,
    createdAt: new Date("2026-08-26T10:00:00Z"),
    updatedAt: new Date("2026-08-26T10:00:00Z"),
  };

  it("names the account as provider:accountId, never by row key", () => {
    const view = triggerToolView(trigger, null, { provider: "telegram", accountId: "mybot" });
    expect(view.spec).toMatchObject({ channelAccount: "telegram:mybot", pairingCode: "PairCode1234", matchScope: "direct" });
    expect(view).toMatchObject({ sessionTtlMs: 0 });
    expect(JSON.stringify(view)).not.toMatch(UUID);
    expect(JSON.stringify(view)).not.toMatch(DIGEST);
  });

  it("maps bot_trigger_put kind=chat to the chat spec and shared delivery knobs", () => {
    const mapped = parseTriggerPutArgs({
      name: "family-chat",
      kind: "chat",
      channelAccount: "telegram:mybot",
      scope: "direct",
      groupActivation: "always",
      pairing: false,
      sessionTtlMs: 0,
      debounceMs: 400,
    });
    expect(mapped.create).toMatchObject({
      kind: "chat",
      spec: { channelAccount: "telegram:mybot", matchScope: "direct", activation: { group: "always" }, pairingCode: null },
      coalesce: { debounceMs: 400, maxWaitMs: 400, maxCount: 50 },
      sessionTtlMs: 0,
    });
    expect(() => parseTriggerPutArgs({ name: "x", kind: "chat" })).toThrow(/channelAccount/);
    // Once the handle is resolved, the create input validates as a chat trigger.
    const parsed = triggerCreateInput.safeParse({
      ...mapped.create,
      spec: { ...(mapped.create.spec as object), channelAccount: undefined, channelAccountId: accountUuid },
    });
    expect(parsed.success).toBe(true);
    expect(chatSpecInput.safeParse({ channelAccountId: "not-a-uuid" }).success).toBe(false);
  });
});
