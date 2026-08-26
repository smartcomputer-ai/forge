import { describe, expect, it } from "vitest";
import {
  botStatusView,
  eventEnvelopeView,
  eventListRowView,
  filterResultView,
  triggerToolView,
  type BotControllerSummary,
  type BotEventRow,
} from "../src/activities/tool-views.js";
import type { BotRow, BotTriggerRow } from "../src/config.js";
import type { BotEventDocumentV1 } from "../src/contracts/bots.js";
import { renderEventPrompt } from "../src/rendering.js";

const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;
const DIGEST = /[0-9a-f]{32}/i;

/** The §8 guarantee: nothing the model reads carries a uuid or a digest. */
function expectNoIds(value: unknown): void {
  const json = JSON.stringify(value);
  expect(json).not.toMatch(UUID);
  expect(json).not.toMatch(DIGEST);
}

const botUuid = "0b54d227-08a2-45a8-9b3f-6a4c21d1a222";
const triggerUuid = "7f1c4a9e-2b3d-4c5e-8f6a-1b2c3d4e5f60";
const bot: BotRow = {
  id: botUuid,
  universeId: "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f",
  name: "triage",
  displayName: "Triage",
  description: "Routes incidents to the right bot.",
  profileId: "profile_0123456789abcdef0123456789abcdef",
  brief: "Watch the queue.",
  runsPerDay: 20,
  breaker: { fires: 60, windowMs: 3_600_000 },
  routedSessionTtlMs: null,
  eventSeq: 17,
  selfConfig: true,
  selfEmit: true,
  enabled: true,
  createdAt: new Date("2026-08-26T10:00:00Z"),
  updatedAt: new Date("2026-08-26T10:00:00Z"),
};

function eventRow(overrides: Partial<BotEventRow>): BotEventRow {
  return {
    id: "019ec16e-4b0c-7527-8909-c39441bad5a1",
    botId: botUuid,
    eventId: `whk-${"a".repeat(64)}`,
    seq: 12,
    triggerId: triggerUuid,
    kind: "pull_request.opened",
    source: "webhook:github",
    occurredAt: new Date("2026-08-26T10:00:00Z"),
    ref: `sha256:${"b".repeat(64)}`,
    promptRef: `sha256:${"c".repeat(64)}`,
    session: { sessionId: "bot:v1:triage:k-pr-12-0123abcd", label: "pr-12" },
    receivedAt: new Date("2026-08-26T10:00:01Z"),
    ...overrides,
  };
}

const rows: BotEventRow[] = [
  eventRow({}),
  eventRow({ eventId: `poll:${triggerUuid}:${"d".repeat(32)}`, source: "poll:issues", seq: 13 }),
  eventRow({ eventId: `schedule:${triggerUuid}:2026-08-26T08:00:00.000Z`, source: "schedule:nightly", seq: 14, session: null }),
  eventRow({ eventId: `self-${botUuid}`, source: "bot:triage", seq: 15, session: null }),
  eventRow({ eventId: `bot:${botUuid}:${"e".repeat(64)}`, source: "bot:infra", seq: 16 }),
];

const document: BotEventDocumentV1 = {
  version: 1,
  kind: "pull_request.opened",
  source: "webhook:github",
  occurredAt: "2026-08-26T10:00:00.000Z",
  summary: "PR #12 opened",
  data: { number: 12, title: "Fix" },
  headers: { "x-github-event": "pull_request" },
  links: ["https://github.com/acme/repo/pull/12"],
};

const controller: BotControllerSummary = {
  sessions: [
    { label: "main", kind: "main" },
    { label: "pr-12", kind: "keyed" },
  ],
  activeDeliveries: [{ events: [12, 13], session: "pr-12" }],
  buffers: [{ session: "main", count: 3, flushAtMs: 1_800_000_000_000 }],
  runsToday: 4,
  eventsProcessed: 16,
};

describe("model-facing bot tool views", () => {
  it("shows status as the authored id, labels, and #Ns", () => {
    const view = botStatusView(bot, controller);
    expectNoIds(view);
    expect(view.bot.botId).toBe("triage");
    expect(view.bot.displayName).toBe("Triage");
    expect(view.bot).not.toHaveProperty("profileId");
    expect(view.sessions).toEqual(controller.sessions);
    expect(view.activeDeliveries[0]).toEqual({ events: [12, 13], session: "pr-12" });
  });

  it("lists events by #N and session label, never by event id", () => {
    for (const row of rows) {
      const view = eventListRowView(row, document);
      expectNoIds(view);
      expect(view).not.toHaveProperty("eventId");
      expect(view.seq).toBe(row.seq);
      if (row.session === null) expect(view).not.toHaveProperty("session");
      else expect(view.session).toBe(row.session.label);
    }
  });

  it("reads an envelope without ids and keeps the sender's fields", () => {
    for (const row of rows) {
      const view = eventEnvelopeView(row, document);
      expectNoIds(view);
      expect(view).not.toHaveProperty("eventId");
      expect(view.summary).toBe("PR #12 opened");
      expect(view.data).toEqual(document.data);
      expect(view.links).toEqual(document.links);
    }
  });

  it("reports filter results by #N", () => {
    for (const row of rows) {
      const view = filterResultView(row, document, { matched: true });
      expectNoIds(view);
      expect(view).not.toHaveProperty("eventId");
    }
    expect(filterResultView(rows[0]!, null, { matched: false, error: "no such field" })).toMatchObject({
      seq: 12,
      matched: false,
      error: "no such field",
      summary: null,
    });
  });

  it("shows triggers by name with the ingest URL, never the row key", () => {
    const trigger: BotTriggerRow = {
      id: triggerUuid,
      botId: botUuid,
      name: "github",
      kind: "webhook",
      spec: { token: "tok-1234", verification: { scheme: "token" }, preset: "github" },
      filter: 'event.kind.startsWith("pull_request")',
      route: { policy: "perKey", key: "data.number" },
      coalesce: null,
      deliver: null,
      cursor: null,
      enabled: true,
      createdAt: new Date("2026-08-26T10:00:00Z"),
      updatedAt: new Date("2026-08-26T10:00:00Z"),
    };
    const view = triggerToolView(trigger, "https://example.test/api/v1/hooks/bots/opaque/tok-1234");
    expect(view).not.toHaveProperty("id");
    expect(view).not.toHaveProperty("botId");
    expect(view.name).toBe("github");
    expect(view.ingestUrl).toContain("/hooks/bots/");
    expect(triggerToolView({ ...trigger, kind: "schedule" }, null)).not.toHaveProperty("ingestUrl");
  });

  it("renders event prompts without ids", () => {
    for (const row of rows) {
      const prompt = renderEventPrompt({
        seq: row.seq,
        kind: row.kind,
        source: row.source,
        occurredAt: document.occurredAt,
        summary: document.summary,
        data: document.data,
        links: document.links ?? [],
      });
      expect(prompt).not.toMatch(UUID);
      expect(prompt).not.toMatch(DIGEST);
      expect(prompt).toContain(`event #${row.seq}`);
    }
  });
});
