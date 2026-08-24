import { describe, expect, it } from "vitest";
import { LightspeedRpcError } from "@lightspeed/agent-client";
import { parseTriggerPutArgs } from "../src/activities/tools.js";
import {
  deliveryInputItems,
  isBotSessionDeclarationMismatch,
  steerInputItems,
} from "../src/activities/lightspeed.js";
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
        "eventListInput",
        "eventReadInput",
        "triggerListInput",
        "briefPutInput",
        "emitInput",
      ]) as never,
      refs([
        "eventResolve",
        "status",
        "triggerPut",
        "triggerDelete",
        "filterTest",
        "eventList",
        "eventRead",
        "triggerList",
        "briefPut",
        "emit",
      ]) as never,
    );
    expect(tools).toHaveLength(10);
    for (const tool of tools) expect(tool.definition.revision).toBe(BOT_TOOLS_REVISION);
    const resolve = tools.find((tool) => tool.definition.toolId === BOT_EVENT_RESOLVE_TOOL_ID);
    expect(resolve?.target).toMatchObject({ type: "bound", dispatch: "pull" });
    expect(resolve?.completion).toEqual({ type: "accepted" });
    const emit = tools.find((tool) => tool.definition.toolId === BOT_EMIT_TOOL_ID);
    expect(emit?.target).toMatchObject({ dispatch: "push" });
    expect(emit?.completion).toEqual({ type: "accepted" });
    expect(emit?.definition.tool.kind).toMatchObject({ type: "function", strict: false });
    // Strict only where the schema has no optional fields; tools with real
    // optionals opt out instead of null-stuffing `required`.
    const strictIds = new Set(
      tools
        .filter((tool) => (tool.definition.tool.kind as { strict?: boolean }).strict === true)
        .map((tool) => tool.definition.toolId),
    );
    expect(strictIds).toEqual(
      new Set([
        "lightspeed.bots.event.resolve.v1",
        "lightspeed.bots.status.v1",
        "lightspeed.bots.trigger.delete.v1",
        "lightspeed.bots.trigger.list.v1",
        "lightspeed.bots.brief.put.v1",
      ]),
    );
    const joined = tools.filter((tool) => tool.completion.type === "joined");
    expect(joined).toHaveLength(8);
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

describe("managed-session declaration conflicts", () => {
  it("recognizes the typed live gateway conflict used to rotate a session", () => {
    expect(
      isBotSessionDeclarationMismatch(
        new LightspeedRpcError({
          code: -32009,
          message:
            "managed-session controller, receiver, or tool declaration conflicts with durable creation state",
          data: {
            kind: "conflict",
            message:
              "managed-session controller, receiver, or tool declaration conflicts with durable creation state",
          },
        }),
      ),
    ).toBe(true);
    expect(
      isBotSessionDeclarationMismatch(
        new LightspeedRpcError({
          code: -32009,
          message: "a run is already active",
          data: { kind: "conflict", message: "a run is already active" },
        }),
      ),
    ).toBe(false);
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

describe("delivery input items", () => {
  const ref = `sha256:${"a".repeat(64)}`;
  const promptRef = `sha256:${"b".repeat(64)}`;

  it("delivers a single event as exactly one rendered item, no framing", () => {
    expect(deliveryInputItems([{ ref, promptRef }])).toEqual([
      { type: "textRef", blobRef: promptRef },
    ]);
    // Events without a rendering (legacy rows) fall back to the envelope.
    expect(deliveryInputItems([{ ref }])).toEqual([{ type: "textRef", blobRef: ref }]);
  });

  it("frames a batch with one header line binding it to one decision", () => {
    const items = deliveryInputItems([
      { ref, promptRef },
      { ref: promptRef, promptRef: ref },
    ]);
    expect(items).toHaveLength(3);
    expect(items[0]).toMatchObject({ type: "text" });
    expect((items[0] as { text: string }).text).toContain("2 events");
    expect((items[0] as { text: string }).text).toContain("resolve the delivery once");
  });

  it("steers with a short note and the renderings", () => {
    const items = steerInputItems([{ ref, promptRef }]);
    expect(items[0]).toMatchObject({ type: "text" });
    expect((items[0] as { text: string }).text).toContain("fold them into your current work");
    expect(items[1]).toEqual({ type: "textRef", blobRef: promptRef });
  });
});
