import { describe, expect, it } from "vitest";
import type { BotListItem } from "@/api";
import { rosterLine } from "./BotsPage";
import { BOT_TEMPLATES } from "@/components/bot/templates";
import { capabilitySummary } from "@/components/bot/setup-summary";
import { botIdFrom, templateHighlights, uniqueTriggerName } from "./BotCreatePage";

function bot(partial: Partial<BotListItem>): BotListItem {
  return {
    botId: "triage",
    universeId: "u",
    displayName: "Triage",
    description: null,
    profileId: "triage",
    brief: null,
    runsPerDay: null,
    breaker: null,
    routedSessionTtlMs: null,
    selfConfig: true,
    emit: false,
    enabled: true,
    closedAt: null,
    closedSessions: null,
    createdAt: "2026-08-27T00:00:00Z",
    updatedAt: "2026-08-27T00:00:00Z",
    triggerCount: 0,
    pendingCount: 0,
    lastEvent: null,
    ...partial,
  };
}

describe("rosterLine", () => {
  it("says what the bot is doing from the event log alone", () => {
    expect(rosterLine(bot({}))).toEqual({ text: "Waiting for its first event", tone: "idle" });
    expect(
      rosterLine(
        bot({
          pendingCount: 2,
          lastEvent: {
            seq: 48,
            kind: "github.pull_request",
            source: "webhook",
            outcome: null,
            outcomeDetail: null,
            receivedAt: "2026-08-27T09:00:00Z",
            resolvedAt: null,
            session: null,
          },
        }),
      ),
    ).toEqual({ text: "Working on #48 · github.pull_request", tone: "live" });
    expect(
      rosterLine(
        bot({
          lastEvent: {
            seq: 47,
            kind: "schedule",
            source: "schedule",
            outcome: "handled",
            outcomeDetail: "Digest sent",
            receivedAt: "2026-08-27T09:00:00Z",
            resolvedAt: "2026-08-27T09:01:00Z",
            session: null,
          },
        }),
      ),
    ).toEqual({ text: "#47 handled · Digest sent", tone: "idle" });
  });
  it("puts lifecycle before activity", () => {
    expect(rosterLine(bot({ enabled: false, pendingCount: 3 }))).toEqual({ text: "Paused · 3 waiting", tone: "paused" });
    expect(rosterLine(bot({ closedAt: "2026-08-27T00:00:00Z", pendingCount: 3 }))).toEqual({ text: "Closed", tone: "closed" });
  });
  it("flags a failed last outcome", () => {
    expect(
      rosterLine(
        bot({
          lastEvent: {
            seq: 42,
            kind: "x",
            source: "y",
            outcome: "run_failed",
            outcomeDetail: "environment suspended",
            receivedAt: "2026-08-27T09:00:00Z",
            resolvedAt: "2026-08-27T09:01:00Z",
            session: null,
          },
        }),
      ).tone,
    ).toBe("attention");
  });
});

describe("wizard helpers", () => {
  it("derives an id from the name", () => {
    expect(botIdFrom("Release Shepherd")).toBe("release-shepherd");
    expect(botIdFrom("  Ünïcode! bot ")).toBe("unicode-bot");
  });
  it("keeps wake-up names unique", () => {
    expect(uniqueTriggerName("schedule", [])).toBe("schedule");
    expect(uniqueTriggerName("schedule", ["schedule", "schedule-2"])).toBe("schedule-3");
  });
  it("describes what a template comes with", () => {
    const reviewer = BOT_TEMPLATES.find((template) => template.id === "pr-reviewer")!;
    expect(templateHighlights(reviewer)).toEqual(["GitHub events", "Weekdays at 09:00", "Web"]);
    expect(templateHighlights(BOT_TEMPLATES.find((template) => template.id === "blank")!)).toEqual([]);
    // Every template's suggested name derives to a valid, distinct id.
    const ids = BOT_TEMPLATES.flatMap((template) => (template.suggestedName ? [botIdFrom(template.suggestedName)] : []));
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) expect(id).toMatch(/^[a-z0-9][a-z0-9-]*$/);
  });
  it("summarises capabilities in words", () => {
    expect(
      capabilitySummary({
        model: { model: "claude-opus-5" },
        features: { web: {}, mcp: { servers: [{ serverId: "a" }, { serverId: "b" }] }, subagents: { agents: [{ profileId: "r" }] } },
      }),
    ).toEqual(["claude-opus-5", "Web", "MCP servers (2)", "Sub-agents (1)"]);
  });
});
