import { describe, expect, it } from "vitest";
import type { AgentProfile } from "@lightspeed/agent-client";
import {
  botDeliveryId,
  botEventSubmissionId,
  botEventTerminalToken,
  botKeyedSessionId,
  botPerEventSessionId,
  botScheduleEventId,
  botScheduleId,
  botSessionId,
  botWorkflowId,
  parseEventResolveArgs,
  resolveBotProfile,
  validateBotEvent,
} from "../src/contracts/bots.js";

const universeId = "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f";

describe("bot contracts", () => {
  it("derives deterministic workflow, session, and delivery identities", () => {
    expect(botWorkflowId(universeId, "triage")).toBe(
      `lightspeed.bots.v1/${universeId}/triage`,
    );
    expect(botSessionId("triage")).toBe("bot:v1:triage");
    expect(botEventSubmissionId("evt-1")).toBe(botEventSubmissionId("evt-1"));
    expect(botEventSubmissionId("evt-1")).not.toBe(botEventSubmissionId("evt-2"));
    expect(botEventTerminalToken("evt-1")).toMatch(/^bot-event-terminal-v1-[0-9a-f]{64}$/);
    expect(botScheduleId(universeId, "triage", "nightly")).toBe(
      `lightspeed.bots.v1/${universeId}/triage/schedule/nightly`,
    );
    expect(botScheduleEventId("trigger-1", "2026-08-20T08:00:00.000Z")).toBe(
      "schedule:trigger-1:2026-08-20T08:00:00.000Z",
    );
    expect(() => botScheduleId(universeId, "triage", "Bad_Name")).toThrow(TypeError);
  });

  it("derives routed session ids that stay readable and collision-free", () => {
    const prSession = botKeyedSessionId("triage", "pr-12");
    expect(prSession).toMatch(/^bot:v1:triage:k-pr-12-[0-9a-f]{8}$/);
    expect(botKeyedSessionId("triage", "pr-12")).toBe(prSession);
    // Keys that slug identically still get distinct sessions via the digest.
    expect(botKeyedSessionId("triage", "a b")).not.toBe(botKeyedSessionId("triage", "a-b"));
    expect(botKeyedSessionId("triage", "ÜÑÎ")).toMatch(/^bot:v1:triage:k-key-[0-9a-f]{8}$/);
    expect(botPerEventSessionId("triage", "evt-1")).toMatch(/^bot:v1:triage:e-[0-9a-f]{12}$/);
    expect(() => botKeyedSessionId("triage", "")).toThrow(TypeError);
  });

  it("derives delivery identities that keep single events stable", () => {
    expect(botDeliveryId(["evt-1"])).toBe("evt-1");
    const batch = botDeliveryId(["b", "a", "c"]);
    expect(batch).toMatch(/^batch-[0-9a-f]{64}$/);
    // Order-insensitive: retries assembling the same set converge.
    expect(botDeliveryId(["c", "a", "b"])).toBe(batch);
    expect(botDeliveryId(["a", "b"])).not.toBe(batch);
    expect(() => botDeliveryId([])).toThrow(TypeError);
  });

  it("validates routed session fields on events", () => {
    const ref = `sha256:${"a".repeat(64)}`;
    validateBotEvent({
      version: 1,
      id: "evt",
      ref,
      session: { sessionId: "bot:v1:triage:k-x-12345678", label: "x" },
    });
    expect(() =>
      validateBotEvent({ version: 1, id: "evt", ref, session: { sessionId: "", label: "x" } }),
    ).toThrow(TypeError);
    expect(() =>
      validateBotEvent({ version: 1, id: "evt", ref, session: { sessionId: "s", label: "" } }),
    ).toThrow(TypeError);
  });

  it("rejects invalid names and events", () => {
    expect(() => botWorkflowId(universeId, "Triage")).toThrow(TypeError);
    expect(() => botSessionId("-bad")).toThrow(TypeError);
    expect(() => validateBotEvent({ version: 1, id: "", ref: `sha256:${"a".repeat(64)}` })).toThrow(
      TypeError,
    );
    expect(() => validateBotEvent({ version: 1, id: "evt", ref: "sha256:short" })).toThrow(
      TypeError,
    );
    validateBotEvent({ version: 1, id: "evt", ref: `sha256:${"a".repeat(64)}` });
  });

  it("parses resolve arguments strictly", () => {
    expect(
      parseEventResolveArgs({ eventId: "evt", outcome: "handled", summary: null }),
    ).toEqual({ eventId: "evt", outcome: "handled", summary: null });
    expect(() =>
      parseEventResolveArgs({ eventId: "evt", outcome: "done", summary: null }),
    ).toThrow(TypeError);
    expect(() => parseEventResolveArgs({ outcome: "handled", summary: null })).toThrow(TypeError);
  });

  it("combines the profile and brief into the applied inline profile", () => {
    const profile: AgentProfile = {
      profileId: "helper",
      displayName: "Helper",
      revision: 3,
      createdAtMs: 0,
      updatedAtMs: 0,
      instructions: { type: "text", text: "Base instructions." },
      environment: { type: "existing", environmentId: "environment_1" },
    };
    const resolved = resolveBotProfile(profile, "Base instructions.", {
      botName: "triage",
      brief: "Watch the queue.",
    });
    expect(resolved.displayName).toBe("Helper");
    expect(resolved.environment).toEqual({ type: "existing", environmentId: "environment_1" });
    expect(resolved.instructions).toMatchObject({ type: "text" });
    const text = (resolved.instructions as { type: "text"; text: string }).text;
    expect(text).toContain("Base instructions.");
    expect(text).toContain("bot triage");
    expect(text).toContain("Watch the queue.");
    expect(text).toContain("bot_event_resolve");
  });
});
