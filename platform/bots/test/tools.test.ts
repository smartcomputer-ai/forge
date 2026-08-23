import { describe, expect, it } from "vitest";
import { parseTriggerPutArgs } from "../src/activities/tools.js";
import { BotConfigError, scheduleSpecInput, triggerCreateInput } from "../src/config.js";
import {
  BOT_EMIT_TOOL_ID,
  BOT_EVENT_RESOLVE_TOOL_ID,
  BOT_TOOL_REPLY_DEADLINE_MS,
  BOT_TOOLS_REVISION,
  botWorkflowTools,
  lightspeedSessionWorkflowId,
} from "../src/contracts/bots.js";

const universeId = "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f";

describe("bot tool declarations", () => {
  it("declares the self-configuration tools as joined pushed tools bound to the controller", () => {
    const receiver = { workflowId: "wf", workflowKind: "botControllerWorkflowV1" };
    const refs = (names: readonly string[]) =>
      Object.fromEntries(names.map((name) => [name, `sha256:${"a".repeat(64)}`]));
    const tools = botWorkflowTools(
      receiver,
      refs([
        "eventResolveInput",
        "statusInput",
        "triggerPutInput",
        "triggerDeleteInput",
        "filterTestInput",
        "eventsReadInput",
        "briefPutInput",
        "emitInput",
      ]) as never,
      refs([
        "eventResolve",
        "status",
        "triggerPut",
        "triggerDelete",
        "filterTest",
        "eventsRead",
        "briefPut",
        "emit",
      ]) as never,
    );
    expect(tools).toHaveLength(8);
    for (const tool of tools) expect(tool.definition.revision).toBe(BOT_TOOLS_REVISION);
    const resolve = tools.find((tool) => tool.definition.toolId === BOT_EVENT_RESOLVE_TOOL_ID);
    expect(resolve?.target).toMatchObject({ type: "bound", dispatch: "pull" });
    expect(resolve?.completion).toEqual({ type: "accepted" });
    const emit = tools.find((tool) => tool.definition.toolId === BOT_EMIT_TOOL_ID);
    expect(emit?.target).toMatchObject({ dispatch: "push" });
    expect(emit?.completion).toEqual({ type: "accepted" });
    const joined = tools.filter((tool) => tool.completion.type === "joined");
    expect(joined).toHaveLength(6);
    for (const tool of joined) {
      expect(tool.completion).toMatchObject({ deadlineAfterMs: BOT_TOOL_REPLY_DEADLINE_MS });
      expect(tool.target).toMatchObject({ dispatch: "push" });
    }
  });

  it("derives the core session workflow id for replies", () => {
    expect(lightspeedSessionWorkflowId(universeId, "bot:v1:triage")).toBe(
      `${universeId}/bot:v1:triage`,
    );
    expect(() => lightspeedSessionWorkflowId(universeId, "a/b")).toThrow(TypeError);
  });
});

describe("bot_trigger_put argument mapping", () => {
  it("maps a github webhook with per-key routing and coalescing", () => {
    const flat = parseTriggerPutArgs({
      name: "prs",
      kind: "webhook",
      verification: "github",
      secret: "s3cret-key-1",
      routePolicy: "perKey",
      routeKey: null,
      filter: 'event.kind == "pull_request.opened"',
      debounceMs: 30_000,
      maxWaitMs: null,
      maxCount: null,
      whenBusy: "steer",
      enabled: null,
    });
    const parsed = triggerCreateInput.parse(flat.create);
    expect(parsed).toMatchObject({
      name: "prs",
      kind: "webhook",
      spec: {
        preset: "github",
        verification: { scheme: "hmac-sha256", header: "x-hub-signature-256", prefix: "sha256=" },
      },
      route: { policy: "perKey" },
      coalesce: { debounceMs: 30_000, maxWaitMs: 30_000, maxCount: 50 },
      deliver: { whenBusy: "steer" },
    });
  });

  it("maps schedules, requires secrets for signed schemes, and rejects bad kinds", () => {
    const schedule = parseTriggerPutArgs({
      name: "nightly",
      kind: "schedule",
      cron: "0 3 * * *",
      at: null,
      timezone: "Europe/Zurich",
      summary: "Triage overnight issues",
    });
    expect(triggerCreateInput.parse(schedule.create)).toMatchObject({
      kind: "schedule",
      spec: { cron: "0 3 * * *", timezone: "Europe/Zurich" },
    });
    expect(() =>
      parseTriggerPutArgs({ name: "x", kind: "webhook", verification: "hmac-sha256", secret: null }),
    ).toThrow(BotConfigError);
    expect(() => parseTriggerPutArgs({ name: "x", kind: "poll" })).toThrow(BotConfigError);
  });
});

describe("schedule spec validation", () => {
  it("requires exactly one of cron or at, and a future at", () => {
    expect(scheduleSpecInput.safeParse({ cron: "* * * * *", summary: "s" }).success).toBe(true);
    expect(scheduleSpecInput.safeParse({ cron: "* * * * *", at: "2030-01-01T00:00:00Z", summary: "s" }).success).toBe(false);
    expect(scheduleSpecInput.safeParse({ summary: "s" }).success).toBe(false);
    expect(scheduleSpecInput.safeParse({ at: "2030-01-01T00:00:00Z", summary: "s" }).success).toBe(true);
    expect(scheduleSpecInput.safeParse({ at: "2020-01-01T00:00:00Z", summary: "s" }).success).toBe(false);
  });
});
