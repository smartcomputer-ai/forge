import { describe, expect, it } from "vitest";
import { LightspeedRpcError } from "@lightspeed/agent-client";
import { sessionWorkflowId } from "@lightspeed/agent-client/workflow";
import { parseTriggerPutArgs } from "../src/activities/tools.js";
import {
  deliveryInputItems,
  isBotSessionDeclarationMismatch,
  steerInputItems,
} from "../src/activities/lightspeed.js";
import { BotConfigError, pollSpecInput, scheduleSpecInput, triggerCreateInput } from "../src/config.js";
import {
  BOT_EMIT_TOOL_ID,
  BOT_EVENT_RESOLVE_TOOL_ID,
  BOT_TOOL_REPLY_DEADLINE_MS,
  BOT_TOOLS_REVISION,
  botWorkflowTools,
} from "../src/contracts/bots.js";

const universeId = "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f";

describe("bot tool declarations", () => {
  it("declares the self-configuration tools as joined pushed tools bound to the controller", () => {
    const receiver = { workflowId: "wf", workflowKind: "botControllerWorkflowV1" };
    const refs = (names: readonly string[]) =>
      Object.fromEntries(names.map((name) => [name, `sha256:${"a".repeat(64)}`]));
    const toolRefs = {
      schemas: refs([
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
      descriptions: refs([
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
    };
    const tools = botWorkflowTools(receiver, toolRefs.schemas, toolRefs.descriptions, {
      selfConfig: true,
      emit: true,
    });
    expect(tools).toHaveLength(10);
    for (const tool of tools) expect(tool.definition.revision).toBe(BOT_TOOLS_REVISION);
    const resolve = tools.find((tool) => tool.definition.toolId === BOT_EVENT_RESOLVE_TOOL_ID);
    expect(resolve?.target).toMatchObject({ type: "bound", dispatch: "pull" });
    expect(resolve?.completion).toEqual({ type: "accepted" });
    // bot_emit is joined: the model reads the stored #N or the rate-cap refusal.
    const emit = tools.find((tool) => tool.definition.toolId === BOT_EMIT_TOOL_ID);
    expect(emit?.target).toMatchObject({ dispatch: "push" });
    expect(emit?.completion).toMatchObject({ type: "joined" });
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
    expect(joined).toHaveLength(9);
    for (const tool of joined) {
      expect(tool.completion).toMatchObject({ deadlineAfterMs: BOT_TOOL_REPLY_DEADLINE_MS });
      expect(tool.target).toMatchObject({ dispatch: "push" });
    }
  });

  it("withholds the mutating tools without the self-configuration grant", () => {
    const receiver = { workflowId: "wf", workflowKind: "botControllerWorkflowV1" };
    const refs = (names: readonly string[]) =>
      Object.fromEntries(names.map((name) => [name, `sha256:${"a".repeat(64)}`]));
    const schemas = refs([
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
    ]) as never;
    const descriptions = refs([
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
    ]) as never;
    // Default (no options) is the fully ungated set: self-modification and
    // self-emission are both opt-in.
    for (const tools of [
      botWorkflowTools(receiver, schemas, descriptions),
      botWorkflowTools(receiver, schemas, descriptions, { selfConfig: false, emit: false }),
    ]) {
      const ids = new Set(tools.map((tool) => tool.definition.toolId));
      expect(tools).toHaveLength(6);
      expect(ids.has("lightspeed.bots.trigger.put.v1")).toBe(false);
      expect(ids.has("lightspeed.bots.trigger.delete.v1")).toBe(false);
      expect(ids.has("lightspeed.bots.brief.put.v1")).toBe(false);
      expect(ids.has("lightspeed.bots.emit.v1")).toBe(false);
      // Read-only and event tools stay: inspect yes, mutate no.
      expect(ids.has("lightspeed.bots.trigger.list.v1")).toBe(true);
      expect(ids.has("lightspeed.bots.event.resolve.v1")).toBe(true);
    }
    // The grants are independent.
    const emitOnly = new Set(
      botWorkflowTools(receiver, schemas, descriptions, { emit: true }).map(
        (tool) => tool.definition.toolId,
      ),
    );
    expect(emitOnly.has("lightspeed.bots.emit.v1")).toBe(true);
    expect(emitOnly.has("lightspeed.bots.trigger.put.v1")).toBe(false);
  });

  it("derives the core session workflow id for replies", () => {
    expect(sessionWorkflowId(universeId, "bot:v1:triage")).toBe(
      `${universeId}/bot:v1:triage`,
    );
    expect(() => sessionWorkflowId(universeId, "a/b")).toThrow(TypeError);
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
      grantId: "github-webhook-secret",
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

  it("maps schedules, requires grant references for signed schemes, and rejects bad kinds", () => {
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
      parseTriggerPutArgs({ name: "x", kind: "webhook", verification: "hmac-sha256", grantId: null }),
    ).toThrow(BotConfigError);
    expect(() => parseTriggerPutArgs({ name: "x", kind: "poll" })).toThrow(BotConfigError);
  });
});

describe("cel save-time validation", () => {
  it("rejects unparseable filters and route keys where they are written", () => {
    const webhook = (extra: Record<string, unknown>) => ({
      name: "hook",
      kind: "webhook",
      ...extra,
    });
    expect(triggerCreateInput.safeParse(webhook({ filter: 'event.kind == "x"' })).success).toBe(
      true,
    );
    const broken = triggerCreateInput.safeParse(webhook({ filter: 'event.kind == ' }));
    expect(broken.success).toBe(false);
    expect(JSON.stringify(broken.error?.issues)).toContain("invalid CEL");
    expect(
      triggerCreateInput.safeParse(
        webhook({ route: { policy: "perKey", key: "data.pr.number" } }),
      ).success,
    ).toBe(true);
    expect(
      triggerCreateInput.safeParse(webhook({ route: { policy: "perKey", key: "data..x" } }))
        .success,
    ).toBe(false);
  });
});

describe("poll trigger mapping and validation", () => {
  it("maps a poll trigger with id-set dedupe and delivery policy", () => {
    const flat = parseTriggerPutArgs({
      name: "issues",
      kind: "poll",
      url: "https://api.example.com/issues",
      grantId: "issues-api-key",
      authHeader: "x-api-key",
      authScheme: "",
      intervalMs: 300_000,
      items: "data.issues",
      cursorId: "id",
      whenBusy: "steer",
      filter: "data.state == \"open\"",
    });
    const parsed = triggerCreateInput.parse(flat.create);
    expect(parsed).toMatchObject({
      name: "issues",
      kind: "poll",
      spec: {
        source: {
          kind: "http",
          url: "https://api.example.com/issues",
          auth: { grantId: "issues-api-key", header: "x-api-key", scheme: "" },
        },
        intervalMs: 300_000,
        items: "data.issues",
        cursor: { kind: "idSet", id: "id" },
      },
      deliver: { whenBusy: "steer" },
    });
  });

  it("maps an exec poll so a bot can register its own authored poller", () => {
    const flat = parseTriggerPutArgs({
      name: "orders",
      kind: "poll",
      environmentId: "environment_1",
      argv: ["./poll-orders.sh", "--json"],
      cwd: "/srv/app",
      intervalMs: 300_000,
      watermarkField: "updated_at",
    });
    const parsed = triggerCreateInput.parse(flat.create);
    expect(parsed).toMatchObject({
      kind: "poll",
      spec: {
        source: {
          kind: "exec",
          environmentId: "environment_1",
          argv: ["./poll-orders.sh", "--json"],
          cwd: "/srv/app",
        },
        cursor: { kind: "watermark", field: "updated_at" },
      },
    });
    expect(() =>
      parseTriggerPutArgs({
        name: "x",
        kind: "poll",
        url: "https://a.example.com",
        environmentId: "environment_1",
        argv: ["./x"],
        intervalMs: 60_000,
        cursorId: "id",
      }),
    ).toThrow(/not both/);
    expect(() =>
      parseTriggerPutArgs({ name: "x", kind: "poll", environmentId: "environment_1", intervalMs: 60_000, cursorId: "id" }),
    ).toThrow(/needs url/);
  });

  it("requires exactly one dedupe discipline and a sane interval", () => {
    expect(() =>
      parseTriggerPutArgs({ name: "x", kind: "poll", url: "https://a", intervalMs: 60_000 }),
    ).toThrow(BotConfigError);
    expect(() =>
      parseTriggerPutArgs({
        name: "x",
        kind: "poll",
        url: "https://a",
        intervalMs: 60_000,
        cursorId: "id",
        watermarkField: "updatedAt",
      }),
    ).toThrow(BotConfigError);
    // The zod layer rejects sub-minute intervals and non-http sources.
    const flat = parseTriggerPutArgs({
      name: "x",
      kind: "poll",
      url: "https://a.example.com/feed",
      intervalMs: 5_000,
      cursorId: "id",
    });
    expect(triggerCreateInput.safeParse(flat.create).success).toBe(false);
    expect(
      pollSpecInput.safeParse({
        source: { kind: "exec", environmentId: "environment_1", argv: ["./check.sh"] },
        intervalMs: 120_000,
        cursor: { kind: "watermark", field: "updatedAt" },
      }).success,
    ).toBe(true);
    expect(
      pollSpecInput.safeParse({
        source: { kind: "http", url: "ftp://nope" },
        intervalMs: 120_000,
        cursor: { kind: "idSet", id: "id" },
      }).success,
    ).toBe(false);
  });

  it("rejects credential material in ordinary HTTP headers", () => {
    expect(
      pollSpecInput.safeParse({
        source: {
          kind: "http",
          url: "https://api.example.com/items",
          headers: { Authorization: "Bearer plaintext" },
        },
        intervalMs: 60_000,
        cursor: { kind: "idSet", id: "id" },
      }).success,
    ).toBe(false);
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
