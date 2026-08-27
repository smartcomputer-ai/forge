import { describe, expect, it } from "vitest";
import type { BotLineage, BotState } from "@/api";
import { conversationTabs } from "./detail";
import { environmentSummary, guardrailsSummary } from "./setup-summary";

function state(partial: Partial<BotState>): BotState {
  return {
    botName: "triage",
    displayName: "Triage",
    profileId: "triage",
    sessionId: "bot:v1:triage",
    sessions: [{ sessionId: "bot:v1:triage", label: "main", kind: "main" }],
    controllerStatus: "idle",
    activeDeliveries: [],
    sessionReady: true,
    pendingEventCount: 0,
    pendingDeliveryCount: 0,
    buffers: [],
    recentEvents: [],
    eventsProcessed: 0,
    duplicateEventCount: 0,
    duplicateEmissionCount: 0,
    appliedProfileRevision: 1,
    runsPerDay: null,
    runsToday: 0,
    descendantsToday: 0,
    lastError: null,
    ...partial,
  };
}

const thread = (id: string, label: string, lastActiveAtMs: number) => ({
  sessionId: id,
  label,
  kind: "keyed" as const,
  lastActiveAtMs,
});

describe("conversationTabs", () => {
  it("is just Main for a bot with one session", () => {
    const tabs = conversationTabs(state({}), undefined, undefined);
    expect(tabs.inline.map((tab) => tab.label)).toEqual(["Main"]);
    expect(tabs.overflow).toEqual([]);
  });
  it("keeps the most recent threads inline and folds the rest, sub-agents included", () => {
    const current = state({
      sessions: [
        { sessionId: "bot:v1:triage", label: "main", kind: "main" },
        thread("t-old", "PR-1", 1),
        thread("t-3", "PR-3", 3),
        thread("t-2", "PR-2", 2),
        thread("t-4", "PR-4", 4),
      ],
      activeDeliveries: [{ id: "d", eventCount: 1, sessionId: "t-4", runId: null }],
    });
    const lineage: BotLineage = {
      "t-4": {
        open: 1,
        total: 1,
        children: [{ id: "sub-1", displayName: "reviewer", lifecycleStatus: "open", profileId: "reviewer", depth: 1, updatedAtMs: 5 }],
      },
    };
    const tabs = conversationTabs(current, lineage, undefined);
    expect(tabs.inline.map((tab) => tab.label)).toEqual(["Main", "PR-4", "PR-3", "PR-2"]);
    expect(tabs.inline[1]?.live).toBe(true);
    expect(tabs.overflow.map((tab) => tab.label)).toEqual(["PR-1", "reviewer"]);
    expect(tabs.overflow[1]?.hint).toBe("sub-agent of PR-4");
  });
  it("always shows the selected conversation inline", () => {
    const current = state({
      sessions: [
        { sessionId: "bot:v1:triage", label: "main", kind: "main" },
        thread("t-1", "PR-1", 1),
        thread("t-2", "PR-2", 2),
        thread("t-3", "PR-3", 3),
        thread("t-4", "PR-4", 4),
      ],
    });
    const tabs = conversationTabs(current, undefined, "t-1");
    expect(tabs.inline.map((tab) => tab.id)).toContain("t-1");
    expect(tabs.overflow.map((tab) => tab.id)).not.toContain("t-1");
    const unknown = conversationTabs(current, undefined, "closed-subagent-session-id");
    expect(unknown.inline.at(-1)?.id).toBe("closed-subagent-session-id");
  });
});

describe("setup summaries", () => {
  it("reads guardrails as one line", () => {
    expect(
      guardrailsSummary(
        { runsPerDay: 50, breaker: { fires: 20, windowMs: 600_000 }, routedSessionTtlMs: 7 * 86_400_000, selfConfig: true, emit: false },
        ["release-shepherd"],
      ),
    ).toBe("50 runs a day · flood 20/10 min · threads close after 7d · can change own triggers · inbox: release-shepherd");
    expect(
      guardrailsSummary({ runsPerDay: null, breaker: null, routedSessionTtlMs: null, selfConfig: false, emit: true }, "off"),
    ).toBe("no daily limit · threads kept · can message bots · no inbox");
  });
  it("names the environment", () => {
    expect(environmentSummary(undefined)).toBe("No environment");
    expect(environmentSummary({ type: "provision", providerId: "incus", templateId: "t", retention: "closeWithSession" })).toBe(
      "A fresh environment per session",
    );
    expect(environmentSummary({ type: "existing", environmentId: "env-1" })).toBe("env-1");
  });
});
