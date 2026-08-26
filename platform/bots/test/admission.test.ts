import { describe, expect, it } from "vitest";
import {
  BotAdmissionRefusal,
  botEventIdFor,
  deliveryHops,
  directoryEntriesFor,
  nextHops,
  receiptDocument,
  receiptEventId,
  renderBotDirectory,
  persistBotEventNotify,
  restoreBotEventNotifyToken,
  resolveInbox,
  type InboxTarget,
} from "../src/admission.js";
import type { BotTriggerRow } from "../src/config.js";
import { MAX_BOT_HOPS } from "../src/contracts/bots.js";
import { renderEventPrompt } from "../src/rendering.js";

const botUuid = "0b54d227-08a2-45a8-9b3f-6a4c21d1a222";
const triggerUuid = "7f1c4a9e-2b3d-4c5e-8f6a-1b2c3d4e5f60";

function inbox(overrides: Partial<BotTriggerRow> = {}): BotTriggerRow {
  return {
    id: triggerUuid,
    botId: botUuid,
    name: "inbox",
    kind: "bot",
    spec: {},
    filter: null,
    route: null,
    coalesce: null,
    deliver: null,
    cursor: null,
    sessionTtlMs: null,
    enabled: true,
    disabledReason: null,
    disabledAt: null,
    lastFilterError: null,
    lastFilterErrorAt: null,
    createdAt: new Date("2026-08-26T10:00:00Z"),
    updatedAt: new Date("2026-08-26T10:00:00Z"),
    ...overrides,
  };
}

function target(overrides: Partial<InboxTarget> = {}): InboxTarget {
  return { bot: { name: "infra", enabled: true }, inbox: inbox(), ...overrides };
}

function refusalCode(fn: () => unknown): string | null {
  try {
    fn();
    return null;
  } catch (error) {
    if (error instanceof BotAdmissionRefusal) return error.code;
    throw error;
  }
}

describe("addressed emit admission", () => {
  const sender = { name: "triage" };

  it("resolves the target's inbox when it accepts the sender", () => {
    expect(resolveInbox(sender, "infra", target()).name).toBe("inbox");
    expect(
      resolveInbox(sender, "infra", target({ inbox: inbox({ spec: { from: ["ops", "triage"] } }) })).name,
    ).toBe("inbox");
  });

  it("refuses with a typed code for every admission failure", () => {
    expect(refusalCode(() => resolveInbox(sender, "ghost", null))).toBe("unknown_bot");
    expect(refusalCode(() => resolveInbox(sender, "infra", target({ bot: { name: "infra", enabled: false } })))).toBe(
      "bot_disabled",
    );
    expect(refusalCode(() => resolveInbox(sender, "infra", target({ inbox: null })))).toBe("no_inbox");
    expect(refusalCode(() => resolveInbox(sender, "infra", target({ inbox: inbox({ enabled: false }) })))).toBe(
      "no_inbox",
    );
    expect(
      refusalCode(() => resolveInbox(sender, "infra", target({ inbox: inbox({ kind: "webhook" }) }))),
    ).toBe("no_inbox");
    expect(
      refusalCode(() => resolveInbox(sender, "infra", target({ inbox: inbox({ spec: { from: ["ops"] } }) }))),
    ).toBe("not_accepted");
  });

  it("derives one event id per (sender, invocation) so retries converge", () => {
    const first = botEventIdFor(botUuid, `wti:sha256:${"a".repeat(64)}`);
    expect(first).toBe(botEventIdFor(botUuid, `wti:sha256:${"a".repeat(64)}`));
    expect(first).not.toBe(botEventIdFor(botUuid, `wti:sha256:${"b".repeat(64)}`));
    expect(first).toMatch(/^bot:[0-9a-f-]{36}:[0-9a-f]{64}$/);
  });

  it("bounds exchanges by hops", () => {
    expect(nextHops(0)).toBe(1);
    expect(nextHops(MAX_BOT_HOPS - 1)).toBe(MAX_BOT_HOPS);
    expect(refusalCode(() => nextHops(MAX_BOT_HOPS))).toBe("loop_cut");
    expect(deliveryHops([{}, { hops: 3 }, { hops: 1 }])).toBe(3);
    expect(deliveryHops([])).toBe(0);
  });
});

describe("bot directory", () => {
  const bots = [
    { name: "triage", enabled: true, description: "me", inbox: { enabled: true, spec: {} } },
    { name: "infra", enabled: true, description: "Investigates incidents.", inbox: { enabled: true, spec: { from: ["triage"] } } },
    { name: "comms", enabled: true, description: null, inbox: { enabled: true, spec: {} } },
    { name: "ops", enabled: true, description: "Not for me.", inbox: { enabled: true, spec: { from: ["comms"] } } },
    { name: "paused", enabled: false, description: "Disabled.", inbox: { enabled: true, spec: {} } },
    { name: "deaf", enabled: true, description: "No inbox.", inbox: null },
    { name: "closed", enabled: true, description: "Inbox paused.", inbox: { enabled: false, spec: {} } },
  ];

  it("lists only bots whose inbox accepts the reader, never the reader itself", () => {
    expect(directoryEntriesFor("triage", bots)).toEqual([
      { botId: "comms", description: null },
      { botId: "infra", description: "Investigates incidents." },
    ]);
    expect(directoryEntriesFor("comms", bots).map((entry) => entry.botId)).toEqual(["ops", "triage"]);
  });

  it("renders one line per bot and says so when nobody listens", () => {
    const text = renderBotDirectory(directoryEntriesFor("triage", bots));
    expect(text).toContain("- comms\n");
    expect(text).toContain("- infra — Investigates incidents.");
    expect(text).not.toContain("ops");
    expect(renderBotDirectory([])).toBe("No other bot accepts events from you right now.");
    expect(text).not.toMatch(/[0-9a-f]{32}/);
  });
});

describe("receipts", () => {
  it("round-trips opaque tokens through PostgreSQL-safe jsonb", () => {
    const token = ["telegram", "primary", "6071843755", "", "304"].join("\0");
    const persisted = persistBotEventNotify({
      workflowId: "lightspeed.channels.v1/x",
      workflowKind: "channelConversationWorkflowV1",
      token,
    });

    expect(persisted).toMatchObject({ tokenEncoding: "base64url-v1" });
    expect(JSON.stringify(persisted)).not.toContain("\\u0000");
    expect(restoreBotEventNotifyToken(persisted)).toBe(token);
    expect(
      restoreBotEventNotifyToken({
        workflowId: "legacy",
        workflowKind: "legacyWorkflowV1",
        token: "legacy-safe-token",
      }),
    ).toBe("legacy-safe-token");
  });

  it("builds a deterministic receipt from the delivery outcome", () => {
    const document = receiptDocument({
      answering: "infra",
      askedSeq: 17,
      status: "handled",
      summary: "root cause: bad deploy",
      occurredAt: "2026-08-26T10:00:00.000Z",
      hops: 2,
    });
    expect(document).toMatchObject({
      kind: "bot.reply",
      source: "bot:infra",
      summary: "root cause: bad deploy",
      data: { status: "handled" },
      sender: { bot: "infra" },
      hops: 2,
      inReplyTo: { bot: "infra", seq: 17 },
    });
    expect(
      receiptDocument({ answering: "infra", askedSeq: 3, status: "appended", summary: null, occurredAt: "t", hops: 1 })
        .summary,
    ).toBe("#3 at infra finished appended");
    const id = receiptEventId(botUuid, "batch-1", "bot:x:y");
    expect(id).toBe(receiptEventId(botUuid, "batch-1", "bot:x:y"));
    expect(id).not.toBe(receiptEventId(botUuid, "batch-2", "bot:x:y"));
  });

  it("renders the correlation as #N at the answering bot, never an id", () => {
    const prompt = renderEventPrompt({
      seq: 12,
      kind: "bot.reply",
      source: "bot:infra",
      occurredAt: "2026-08-26T10:00:00.000Z",
      summary: "root cause: bad deploy",
      data: { status: "handled" },
      inReplyTo: { bot: "infra", seq: 17 },
    });
    expect(prompt).toContain("event #12 · bot.reply · bot:infra");
    expect(prompt).toContain("reply to your #17 at infra");
    expect(prompt).not.toMatch(/[0-9a-f]{32}/);
  });
});
