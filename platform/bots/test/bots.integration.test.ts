import { fileURLToPath } from "node:url";
import { ApplicationFailure } from "@temporalio/common";
import { TestWorkflowEnvironment } from "@temporalio/testing";
import { Worker } from "@temporalio/worker";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import {
  sessionWorkflowId,
  type EmissionEnvelope,
} from "@lightspeed/agent-client/workflow";
import {
  BOT_CONFIG_SIGNAL,
  BOT_CONTROLLER_WORKFLOW,
  BOT_EVENT_RESOLVE_TOOL_ID,
  BOT_EVENT_SIGNAL,
  BOT_SESSION_DECLARATION_MISMATCH,
  BOT_SESSION_ROTATE_SIGNAL,
  BOT_STATE_QUERY,
  BOT_STATUS_TOOL_ID,
  BOTS_ACTIVITY_TASK_QUEUE,
  BOTS_WORKFLOW_TASK_QUEUE,
  botDeliveryId,
  botEventTerminalToken,
  botScheduleId,
  botSessionId,
  botWorkflowId,
  type BotEvent,
  type BotStartV1,
} from "../src/contracts/bots.js";
import {
  deleteBotSchedule,
  upsertBotSchedule,
  type BotScheduleSpec,
} from "../src/schedules.js";
import type { BotSnapshot } from "../src/workflows/bot-controller.js";

const runIntegration = process.env.BOTS_TEMPORAL_INTEGRATION === "1";
const universeId = "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f";
const botId = "0b54d227-08a2-45a8-9b3f-6a4c21d1a222";
const botName = "triage";
const eventRef = `sha256:${"a".repeat(64)}`;
const resolveRef = `sha256:${"b".repeat(64)}`;
const defaultUsageActivities = {
  readBotRunUsage: async () => null,
  countBotDescendantSessions: async () => ({ count: 0 }),
};

describe.runIf(runIntegration)("bot controller workflow", () => {
  let env: TestWorkflowEnvironment;

  beforeAll(async () => {
    env = await TestWorkflowEnvironment.createLocal();
  }, 120_000);

  afterAll(async () => {
    await env.teardown();
  });

  it("deduplicates events, drives the session, reconciles resolution, and records activity", async () => {
    const calls: string[] = [];
    const activity: unknown[] = [];
    let runsStarted = 0;
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async () => {
          calls.push("ensureSession");
          return { profileRevision: 4 };
        },
        readBotSessionStatus: async () => ({ status: "idle" }),
        startBotRun: async () => {
          calls.push("startRun");
          runsStarted += 1;
          return { runId: `run_${runsStarted}` };
        },
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations:
            afterSeq === 0
              ? [
                  {
                    invocationId: `wti:sha256:${"d".repeat(64)}`,
                    toolId: BOT_EVENT_RESOLVE_TOOL_ID,
                    runId: "run_1",
                    argumentsRef: resolveRef,
                  },
                ]
              : [],
        }),
        readJsonBlob: async ({ blobRef }: { blobRef: string }) => {
          if (blobRef === resolveRef) {
            // Run-scoped resolution: no id echo in the arguments.
            return { outcome: "handled", summary: "queue drained" };
          }
          throw new Error(`unexpected blob ${blobRef}`);
        },
        recordEventOutcomes: async (input: unknown) => {
          activity.push(input);
          return { updated: 1 };
        },
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName,
        displayName: null,
        profileId: "triage-bot",
        brief: "Watch the queue.",
        runsPerDay: null,
        enabled: true,
      };
      const event: BotEvent = {
        version: 1,
        id: "delivery-1",
        ref: eventRef,
        seq: 17,
        promptRef: `sha256:${"e".repeat(64)}`,
      };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, botName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [event],
      });

      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeDeliveries.some((active) => active.id === event.id),
      );
      await handle.signal(BOT_EVENT_SIGNAL, event);
      await handle.signal("deliver_emission", terminalEmission(1, botEventTerminalToken(event.id)));

      const completed = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 1,
      );
      expect(completed.duplicateEventCount).toBe(1);
      expect(completed.recentEvents[0]).toMatchObject({
        id: event.id,
        seqs: [17],
        outcome: "handled",
        runId: "run_1",
        summary: "queue drained",
      });
      expect(completed.appliedProfileRevision).toBe(4);
      expect(completed.runsToday).toBe(1);
      expect(calls).toEqual(["ensureSession", "startRun"]);

      // The decision lands once on the event row: the read model of this lane.
      expect(activity).toEqual([
        expect.objectContaining({
          eventIds: [event.id],
          outcome: "handled",
          detail: "queue drained",
          deliveryId: event.id,
          runId: "run_1",
        }),
      ]);

      const history = await handle.fetchHistory();
      await Worker.runReplayHistory(
        { workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)) },
        history,
        botWorkflowId(universeId, botName),
      );
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("parks pending events when the daily run budget is exhausted", async () => {
    const budgetBotName = "budgeted";
    let runsStarted = 0;
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async () => ({ profileRevision: 1 }),
        readBotSessionStatus: async () => ({ status: "idle" }),
        startBotRun: async () => {
          runsStarted += 1;
          return { runId: `run_${runsStarted}` };
        },
        countBotDescendantSessions: async () => ({ count: 0 }),
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: budgetBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: 1,
        enabled: true,
      };
      const first: BotEvent = { version: 1, id: "budget-1", ref: eventRef };
      const second: BotEvent = { version: 1, id: "budget-2", ref: eventRef };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, budgetBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [first],
      });
      await handle.signal(BOT_EVENT_SIGNAL, second);
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeDeliveries.some((active) => active.id === first.id),
      );
      await handle.signal(
        "deliver_emission",
        budgetTerminalEmission(budgetBotName, 1, botEventTerminalToken(first.id)),
      );
      const parked = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.controllerStatus === "budget_exhausted",
      );
      expect(parked.eventsProcessed).toBe(1);
      expect(parked.pendingEventCount).toBe(1);
      expect(parked.runsToday).toBe(1);
      expect(parked.descendantsToday).toBe(0);
      expect(runsStarted).toBe(1);
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("counts sub-agent descendants against the daily run budget", async () => {
    const budgetBotName = "budgeted-descendants";
    let runsStarted = 0;
    const counted: { sessionIds: string[]; sinceMs: number }[] = [];
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async () => ({ profileRevision: 1 }),
        readBotSessionStatus: async () => ({ status: "idle" }),
        startBotRun: async () => {
          runsStarted += 1;
          return { runId: `run_${runsStarted}` };
        },
        // The bot's first run delegated two sub-agents (lineage read from
        // core); together with the run itself that spends the budget of 3.
        countBotDescendantSessions: async (input: { sessionIds: string[]; sinceMs: number }) => {
          counted.push(input);
          return { count: runsStarted === 0 ? 0 : 2 };
        },
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: budgetBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: 3,
        enabled: true,
      };
      const first: BotEvent = { version: 1, id: "descendants-1", ref: eventRef };
      const second: BotEvent = { version: 1, id: "descendants-2", ref: eventRef };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, budgetBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [first],
      });
      await handle.signal(BOT_EVENT_SIGNAL, second);
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeDeliveries.some((active) => active.id === first.id),
      );
      await handle.signal(
        "deliver_emission",
        budgetTerminalEmission(budgetBotName, 1, botEventTerminalToken(first.id)),
      );
      const parked = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.controllerStatus === "budget_exhausted",
      );
      expect(parked.eventsProcessed).toBe(1);
      expect(parked.pendingEventCount).toBe(1);
      expect(parked.runsToday).toBe(1);
      expect(parked.descendantsToday).toBe(2);
      expect(runsStarted).toBe(1);
      // The count is scoped to the bot's own sessions and to today.
      const last = counted.at(-1);
      expect(last?.sessionIds).toContain(parked.sessionId);
      expect(new Date(last?.sinceMs ?? 0).toISOString().endsWith("T00:00:00.000Z")).toBe(true);
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("reconciles a config signal after an active main-session delivery finishes", async () => {
    const configBotName = "config-during-run";
    let ensured = 0;
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async () => {
          ensured += 1;
          return { profileRevision: ensured };
        },
        readBotSessionStatus: async () => ({ status: "idle" }),
        startBotRun: async () => ({ runId: "run_1" }),
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: configBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: "before",
        runsPerDay: null,
        enabled: true,
      };
      const session = botSessionId(configBotName);
      const event: BotEvent = { version: 1, id: "config-event", ref: eventRef };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, configBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [event],
      });
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeDeliveries[0]?.runId === "run_1",
      );
      await handle.signal(BOT_CONFIG_SIGNAL, { ...start, brief: "after" });

      // The controller must remain queryable while the config waits for the
      // active main-session lane; configDirty must not make its wake loop hot.
      const during = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeDeliveries[0]?.runId === "run_1",
      );
      expect(during.controllerStatus).toBe("delivering_event");

      await handle.signal(
        "deliver_emission",
        sessionTerminalEmission(session, 1, botEventTerminalToken(event.id)),
      );
      const done = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 1 && ensured === 2,
      );
      expect(done.controllerStatus).toBe("idle");
      expect(done.sessionReady).toBe(true);
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);
  it("routes events into per-key sessions and reconciles their terminals", async () => {
    const routedBotName = "routed";
    const ensured: string[] = [];
    const runs: { sessionId: string }[] = [];
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async ({ sessionId }: { sessionId: string }) => {
          ensured.push(sessionId);
          return { profileRevision: 1 };
        },
        readBotSessionStatus: async () => ({ status: "idle" }),
        startBotRun: async ({ sessionId }: { sessionId: string }) => {
          runs.push({ sessionId });
          return { runId: `run_${runs.length}` };
        },
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: routedBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: null,
        enabled: true,
      };
      const mainSession = botSessionId(routedBotName);
      const keyedA = `${mainSession}:k-pr-12-abcd1234`;
      const keyedB = `${mainSession}:k-pr-13-dcba4321`;
      const first: BotEvent = {
        version: 1,
        id: "routed-1",
        ref: eventRef,
        session: { sessionId: keyedA, label: "pr-12" },
      };
      const second: BotEvent = {
        version: 1,
        id: "routed-2",
        ref: eventRef,
        session: { sessionId: keyedB, label: "pr-13" },
      };
      const third: BotEvent = { version: 1, id: "routed-3", ref: eventRef };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, routedBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [first],
      });
      await handle.signal(BOT_EVENT_SIGNAL, second);
      await handle.signal(BOT_EVENT_SIGNAL, third);

      // Three different target sessions: all three deliveries run as
      // concurrent lanes, each with its own run, none blocking the others.
      const inFlight = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) =>
          state.activeDeliveries.length === 3 &&
          state.activeDeliveries.every((active) => active.runId !== null),
      );
      expect(new Set(inFlight.activeDeliveries.map((active) => active.sessionId))).toEqual(
        new Set([keyedA, keyedB, mainSession]),
      );
      for (const active of inFlight.activeDeliveries) {
        const runNumber = Number(active.runId?.replace("run_", ""));
        await handle.signal(
          "deliver_emission",
          sessionTerminalEmission(active.sessionId, runNumber, botEventTerminalToken(active.id)),
        );
      }

      const done = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 3,
      );
      // The controller created each keyed session exactly once, delivered the
      // routed runs there, and the unrouted event went to the main session.
      expect(ensured.filter((id) => id === keyedA)).toHaveLength(1);
      expect(ensured.filter((id) => id === keyedB)).toHaveLength(1);
      expect(new Set(runs.map((run) => run.sessionId))).toEqual(
        new Set([keyedA, keyedB, mainSession]),
      );
      expect(new Set(done.sessions.map((session) => session.sessionId))).toEqual(
        new Set([mainSession, keyedA, keyedB]),
      );
      expect(done.sessions.find((session) => session.sessionId === keyedA)).toMatchObject({
        label: "pr-12",
        kind: "keyed",
      });
      expect(done.activeDeliveries).toHaveLength(0);
      expect(done.recentEvents.map((event) => event.outcome)).toEqual([
        "unresolved",
        "unresolved",
        "unresolved",
      ]);

      const history = await handle.fetchHistory();
      await Worker.runReplayHistory(
        { workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)) },
        history,
        botWorkflowId(universeId, routedBotName),
      );
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("coalesces events into one batch delivery resolved by delivery id", async () => {
    const coalesceBotName = "coalescing";
    const runs: { deliveryId: string; eventCount: number }[] = [];
    const expectedDeliveryId = botDeliveryId(["c-1", "c-2", "c-3"]);
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async () => ({ profileRevision: 1 }),
        readBotSessionStatus: async () => ({ status: "idle" }),
        startBotRun: async ({
          deliveryId,
          events,
        }: {
          deliveryId: string;
          events: unknown[];
        }) => {
          runs.push({ deliveryId, eventCount: events.length });
          return { runId: `run_${runs.length}` };
        },
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations:
            afterSeq === 0
              ? [
                  {
                    invocationId: `wti:sha256:${"f".repeat(64)}`,
                    toolId: BOT_EVENT_RESOLVE_TOOL_ID,
                    runId: "run_1",
                    argumentsRef: resolveRef,
                  },
                ]
              : [],
        }),
        readJsonBlob: async () => ({ outcome: "handled", summary: "batch triaged" }),
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: coalesceBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: null,
        enabled: true,
      };
      const coalesce = { key: "trigger-x|main", debounceMs: 60_000, maxWaitMs: 120_000, maxCount: 3 };
      const events: BotEvent[] = ["c-1", "c-2", "c-3"].map((id, index) => ({
        version: 1,
        id,
        ref: eventRef,
        seq: index + 1,
        coalesce,
      }));
      const firstEvent = events[0] as BotEvent;
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, coalesceBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [firstEvent],
      });
      // With a 60s debounce, nothing may deliver until maxCount forces the flush.
      const buffered = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.buffers.length === 1,
      );
      expect(buffered.buffers[0]).toMatchObject({ key: coalesce.key, count: 1 });
      expect(runs).toHaveLength(0);
      await handle.signal(BOT_EVENT_SIGNAL, events[1]);
      await handle.signal(BOT_EVENT_SIGNAL, events[2]);

      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeDeliveries.some((active) => active.id === expectedDeliveryId),
      );
      await handle.signal(
        "deliver_emission",
        sessionTerminalEmission(
          botSessionId(coalesceBotName),
          1,
          botEventTerminalToken(expectedDeliveryId),
        ),
      );

      const done = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 1,
      );
      expect(runs).toEqual([{ deliveryId: expectedDeliveryId, eventCount: 3 }]);
      expect(done.recentEvents[0]).toMatchObject({
        id: expectedDeliveryId,
        eventCount: 3,
        seqs: [1, 2, 3],
        outcome: "handled",
        summary: "batch triaged",
      });
      expect(done.buffers).toHaveLength(0);
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("wakes a parked controller to flush one event at its debounce deadline", async () => {
    const deadlineBotName = "coalesce-deadline";
    const runs: string[] = [];
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async () => ({ profileRevision: 1 }),
        readBotSessionStatus: async () => ({ status: "idle" }),
        startBotRun: async ({ deliveryId }: { deliveryId: string }) => {
          runs.push(deliveryId);
          return { runId: "run_1" };
        },
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        readBotRunUsage: async () => null,
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: deadlineBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: null,
        enabled: true,
      };
      const event: BotEvent = {
        version: 1,
        id: "deadline-1",
        ref: eventRef,
        coalesce: {
          key: "trigger-x|main",
          debounceMs: 25,
          maxWaitMs: 1_000,
          maxCount: 8,
        },
      };
      const handle = await env.client.workflow.start(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, deadlineBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
      });
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.controllerStatus === "idle" && state.pendingEventCount === 0,
      );

      await handle.signal(BOT_EVENT_SIGNAL, event);
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) =>
          state.activeDeliveries.some(
            (active) => active.id === event.id && active.runId === "run_1",
          ),
      );
      expect(runs).toEqual([event.id]);

      await handle.signal(
        "deliver_emission",
        sessionTerminalEmission(
          botSessionId(deadlineBotName),
          1,
          botEventTerminalToken(event.id),
        ),
      );
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 1,
      );
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("applies steer and append delivery policies on busy sessions", async () => {
    const policyBotName = "policied";
    const calls: string[] = [];
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async () => ({ profileRevision: 1 }),
        readBotSessionStatus: async () => ({ status: "active" }),
        startBotRun: async () => {
          calls.push("run");
          return { runId: "run_1" };
        },
        steerBotRun: async ({ events }: { events: unknown[] }) => {
          calls.push(`steer:${events.length}`);
          return { steered: true, runId: "run_9" };
        },
        appendBotContext: async ({ events }: { events: unknown[] }) => {
          calls.push(`append:${events.length}`);
        },
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: policyBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: null,
        enabled: true,
      };
      const steered: BotEvent = {
        version: 1,
        id: "p-steer",
        ref: eventRef,
        deliver: { whenBusy: "steer" },
      };
      const appended: BotEvent = {
        version: 1,
        id: "p-append",
        ref: eventRef,
        deliver: { whenBusy: "append" },
      };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, policyBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [steered],
      });
      await handle.signal(BOT_EVENT_SIGNAL, appended);

      const done = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 2,
      );
      // The steer delivery folded into the busy run; the append delivery
      // became keyed context; neither started a run or consumed budget.
      expect(calls).toEqual(["steer:1", "append:1"]);
      expect(done.recentEvents.map((event) => event.outcome)).toEqual(["steered", "appended"]);
      expect(done.runsToday).toBe(0);
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("applies steer and append beside a controller-owned active delivery", async () => {
    const policyBotName = "policy-sidecars";
    const calls: string[] = [];
    let hostActive = false;
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async () => ({ profileRevision: 1 }),
        readBotSessionStatus: async () => ({ status: hostActive ? "running" : "idle" }),
        startBotRun: async () => {
          hostActive = true;
          return { runId: "run_1" };
        },
        steerBotRun: async () => {
          calls.push("steer");
          return { steered: true, runId: "run_1" };
        },
        appendBotContext: async () => {
          calls.push("append");
        },
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: policyBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: null,
        enabled: true,
      };
      const host: BotEvent = { version: 1, id: "host", ref: eventRef };
      const steer: BotEvent = {
        version: 1,
        id: "sidecar-steer",
        ref: eventRef,
        deliver: { whenBusy: "steer" },
      };
      const append: BotEvent = {
        version: 1,
        id: "sidecar-append",
        ref: eventRef,
        deliver: { whenBusy: "append" },
      };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, policyBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [host],
      });
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeDeliveries[0]?.runId === "run_1",
      );
      await handle.signal(BOT_EVENT_SIGNAL, steer);
      await handle.signal(BOT_EVENT_SIGNAL, append);

      const sidecarsDone = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 2,
      );
      expect(calls).toEqual(["steer", "append"]);
      expect(sidecarsDone.activeDeliveries[0]?.id).toBe(host.id);
      expect(sidecarsDone.recentEvents.map((event) => event.outcome)).toEqual([
        "steered",
        "appended",
      ]);

      hostActive = false;
      await handle.signal(
        "deliver_emission",
        sessionTerminalEmission(botSessionId(policyBotName), 1, botEventTerminalToken(host.id)),
      );
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 3 && state.activeDeliveries.length === 0,
      );
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("closes idle routed sessions after the retention window and reopens a new generation", async () => {
    const retentionBotName = "retained";
    const ensured: string[] = [];
    const closed: string[] = [];
    let runsStarted = 0;
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async ({ sessionId }: { sessionId: string }) => {
          ensured.push(sessionId);
          return { profileRevision: 1 };
        },
        readBotSessionStatus: async () => ({ status: "idle" }),
        startBotRun: async () => {
          runsStarted += 1;
          return { runId: `run_${runsStarted}` };
        },
        closeBotSession: async ({ sessionId }: { sessionId: string }) => {
          closed.push(sessionId);
          return { closed: true };
        },
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: retentionBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: null,
        routedSessionTtlMs: 1_500,
        enabled: true,
      };
      const mainSession = botSessionId(retentionBotName);
      const keyed = `${mainSession}:k-issue-7-deadbeef`;
      const session = { sessionId: keyed, label: "issue-7" };
      const first: BotEvent = { version: 1, id: "r-1", ref: eventRef, session };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, retentionBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [first],
      });
      const inFlight = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeDeliveries.some((active) => active.runId !== null),
      );
      const active = inFlight.activeDeliveries[0] as { id: string; sessionId: string; runId: string };
      expect(active.sessionId).toBe(keyed);
      await handle.signal(
        "deliver_emission",
        sessionTerminalEmission(keyed, 1, botEventTerminalToken(active.id)),
      );

      // Idle past the TTL: the controller closes the routed session and
      // drops it from its session list.
      const swept = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.sessions.length === 1 && state.eventsProcessed === 1,
      );
      expect(closed).toEqual([keyed]);
      expect(swept.sessions[0]?.sessionId).toBe(mainSession);

      // A later event for the same key opens a fresh generation instead of
      // reviving the closed session id.
      const second: BotEvent = { version: 1, id: "r-2", ref: eventRef, session };
      await handle.signal(BOT_EVENT_SIGNAL, second);
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) =>
          state.activeDeliveries.some(
            (entry) => entry.id === second.id && entry.sessionId === `${keyed}-g2` && entry.runId !== null,
          ),
      );
      expect(ensured.filter((id) => id !== mainSession)).toEqual([keyed, `${keyed}-g2`]);
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("rotates a routed session whose declaration no longer matches", async () => {
    const rotatedRoutedBotName = "routed-rotated";
    const ensured: string[] = [];
    const runs: string[] = [];
    const mainSession = botSessionId(rotatedRoutedBotName);
    const keyed = `${mainSession}:k-pr-9-cafecafe`;
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async ({ sessionId }: { sessionId: string }) => {
          ensured.push(sessionId);
          if (sessionId === keyed) {
            // The routed session pre-exists under an older declaration.
            throw ApplicationFailure.nonRetryable(
              "created under another declaration",
              BOT_SESSION_DECLARATION_MISMATCH,
            );
          }
          return { profileRevision: 1 };
        },
        readBotSessionStatus: async () => ({ status: "idle" }),
        startBotRun: async ({ sessionId }: { sessionId: string }) => {
          runs.push(sessionId);
          return { runId: `run_${runs.length}` };
        },
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: rotatedRoutedBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: null,
        enabled: true,
      };
      const event: BotEvent = {
        version: 1,
        id: "rr-1",
        ref: eventRef,
        session: { sessionId: keyed, label: "pr-9" },
      };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, rotatedRoutedBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [event],
      });

      // Instead of wedging as run_failed, the delivery lands on the key's
      // next generation.
      const inFlight = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) =>
          state.activeDeliveries.some(
            (active) => active.sessionId === `${keyed}-g2` && active.runId !== null,
          ),
      );
      expect(inFlight.activeDeliveries[0]?.id).toBe(event.id);
      expect(ensured).toContain(keyed);
      expect(ensured).toContain(`${keyed}-g2`);
      expect(runs).toEqual([`${keyed}-g2`]);
      expect(
        inFlight.sessions.some((session) => session.sessionId === `${keyed}-g2`),
      ).toBe(true);

      await handle.signal(
        "deliver_emission",
        sessionTerminalEmission(`${keyed}-g2`, 1, botEventTerminalToken(event.id)),
      );
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 1 && state.activeDeliveries.length === 0,
      );

      const history = await handle.fetchHistory();
      await Worker.runReplayHistory(
        { workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)) },
        history,
        botWorkflowId(universeId, rotatedRoutedBotName),
      );
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("answers pushed bot_* invocations and resolves the session's parked call", async () => {
    const toolBotName = "selfconfig";
    const executed: unknown[] = [];
    const payloadRef = `sha256:${"c".repeat(64)}`;
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async () => ({ profileRevision: 1 }),
        readBotSessionStatus: async () => ({ status: "idle" }),
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        executeBotTool: async (input: unknown) => {
          executed.push(input);
          return { ok: true, payloadRef };
        },
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: toolBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: null,
        enabled: true,
      };
      const mainSession = botSessionId(toolBotName);
      const controller = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, toolBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_CONFIG_SIGNAL,
        signalArgs: [start],
      });
      await eventually(
        () => controller.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.sessionReady,
      );
      // Stand in for the core session workflow that holds the parked call.
      const session = await env.client.workflow.start("fakeSessionWorkflow", {
        workflowId: sessionWorkflowId(universeId, mainSession),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      });

      const promiseId = "promise_9";
      await controller.signal("deliver_emission", {
        emission_id: `wti:sha256:${"7".repeat(64)}`,
        producer: { kind: "session", universe_id: universeId, session_id: mainSession, log_seq: 5 },
        body: {
          kind: "tool_invocation",
          holder_workflow_id: sessionWorkflowId(universeId, mainSession),
          invocation: {
            invocation_id: `wti:sha256:${"7".repeat(64)}`,
            tool_id: BOT_STATUS_TOOL_ID,
            semantic_type: BOT_STATUS_TOOL_ID,
            schema_revision: 2,
            binding_fingerprint: "wtb:test",
            session_universe_id: universeId,
            session_id: mainSession,
            run_id: 1,
            turn_id: 1,
            tool_batch_id: 1,
            tool_call_id: "call_1",
            arguments_ref: eventRef,
            completion_promises: { reply: promiseId },
          },
        },
      });

      const received = await eventually(
        () => session.query<unknown[]>("received"),
        (envelopes) => envelopes.length === 1,
      );
      expect(received[0]).toMatchObject({
        producer: { kind: "workflow", universe_id: universeId, workflow_id: controller.workflowId },
        body: {
          kind: "source_resolution",
          promise_id: promiseId,
          resolution: { kind: "resolved", payload_ref: payloadRef },
        },
      });
      expect(executed).toHaveLength(1);
      expect(executed[0]).toMatchObject({
        toolId: BOT_STATUS_TOOL_ID,
        sessionId: mainSession,
        botName: toolBotName,
        // The summary the tool activity may show the model: labels, no ids.
        controller: { sessions: [{ label: "main", kind: "main" }] },
      });

      // Redelivery of the same invocation is ignored.
      await controller.signal("deliver_emission", {
        emission_id: `wti:sha256:${"7".repeat(64)}`,
        producer: { kind: "session", universe_id: universeId, session_id: mainSession, log_seq: 6 },
        body: { kind: "run_terminal", token: "x", run_id: 9, status: "completed", output_ref: null, failure_message_ref: null },
      });
      const after = await eventually(
        () => controller.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.duplicateEmissionCount === 1,
      );
      expect(after.duplicateEmissionCount).toBe(1);
      expect(executed).toHaveLength(1);
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("rotates the main session when its tool declaration no longer matches", async () => {
    const rotateBotName = "rotated";
    const ensured: string[] = [];
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async ({ sessionId }: { sessionId: string }) => {
          ensured.push(sessionId);
          if (sessionId === botSessionId(rotateBotName)) {
            throw ApplicationFailure.nonRetryable(
              "created under another declaration",
              BOT_SESSION_DECLARATION_MISMATCH,
            );
          }
          return { profileRevision: 3 };
        },
        readBotSessionStatus: async () => ({ status: "idle" }),
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: rotateBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: null,
        enabled: true,
      };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, rotateBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_CONFIG_SIGNAL,
        signalArgs: [start],
      });
      const ready = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.sessionReady,
      );
      const base = botSessionId(rotateBotName);
      expect(ensured).toEqual([base, `${base}-g2`]);
      expect(ready.sessionId).toBe(`${base}-g2`);
      expect(ready.mainGeneration).toBe(2);
      expect(ready.sessions[0]?.sessionId).toBe(`${base}-g2`);
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("rotates an operator-selected main session to a fresh generation", async () => {
    const operatorBotName = "operator-rotated";
    const ensured: string[] = [];
    const closed: string[] = [];
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async ({ sessionId }: { sessionId: string }) => {
          ensured.push(sessionId);
          return { profileRevision: 1 };
        },
        closeBotSession: async ({ sessionId }: { sessionId: string }) => {
          closed.push(sessionId);
          return { closed: true };
        },
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: operatorBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: null,
        enabled: true,
      };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, operatorBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_CONFIG_SIGNAL,
        signalArgs: [start],
      });
      const base = botSessionId(operatorBotName);
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.sessionReady && state.sessionId === base,
      );

      await handle.signal(BOT_SESSION_ROTATE_SIGNAL, { version: 1, sessionId: base });
      const rotated = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.sessionReady && state.sessionId === `${base}-g2`,
      );
      expect(closed).toEqual([base]);
      expect(ensured).toEqual([base, `${base}-g2`]);
      expect(rotated.sessions[0]?.sessionId).toBe(`${base}-g2`);
      expect(rotated.rotatingSessionIds).toEqual([]);

      const history = await handle.fetchHistory();
      await Worker.runReplayHistory(
        { workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)) },
        history,
        botWorkflowId(universeId, operatorBotName),
      );
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("reconciles schedules and fires them through the admission activity", async () => {
    const admissions: unknown[] = [];
    const polls: unknown[] = [];
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        admitScheduleEvent: async (input: unknown) => {
          admissions.push(input);
          return { admitted: true, eventId: "schedule:test", duplicate: false };
        },
        pollBotTrigger: async (input: unknown) => {
          polls.push(input);
          return { polled: true, baselined: true, admitted: 0, filtered: 0 };
        },
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    const spec: BotScheduleSpec = {
      universeId,
      botId,
      fire: "schedule",
      botName: "scheduled",
      triggerId: "3fbc2b1e-0f6f-4a83-b0d6-92c07d4d1333",
      triggerName: "nightly",
      cron: "0 3 * * *",
      timezone: "UTC",
      paused: false,
    };
    try {
      await upsertBotSchedule(env.client, spec);
      // Upsert must be idempotent and apply config changes in place.
      await upsertBotSchedule(env.client, { ...spec, cron: "30 3 * * *" });

      const handle = env.client.schedule.getHandle(
        botScheduleId(universeId, spec.botName, spec.triggerName),
      );
      const described = await handle.describe();
      expect(described.state.paused).toBe(false);

      await handle.trigger();
      await eventually(
        () => Promise.resolve(admissions.length),
        (count) => count === 1,
      );
      const admission = admissions[0] as { botId: string; triggerId: string; scheduledAt: string };
      expect(admission.botId).toBe(botId);
      expect(admission.triggerId).toBe(spec.triggerId);
      expect(new Date(admission.scheduledAt).getTime()).not.toBeNaN();

      await upsertBotSchedule(env.client, { ...spec, paused: true });
      expect((await handle.describe()).state.paused).toBe(true);

      await deleteBotSchedule(env.client, universeId, spec.botName, spec.triggerName);
      // Deleting an absent schedule stays a no-op.
      await deleteBotSchedule(env.client, universeId, spec.botName, spec.triggerName);

      // Poll triggers ride the same schedule machinery with an interval
      // spec and their own fire workflow + activity.
      const pollSpec: BotScheduleSpec = {
        universeId,
        botId,
        fire: "poll",
        botName: "scheduled",
        triggerId: "7abc2b1e-0f6f-4a83-b0d6-92c07d4d1444",
        triggerName: "feed",
        intervalMs: 300_000,
        paused: false,
      };
      await upsertBotSchedule(env.client, pollSpec);
      const pollHandle = env.client.schedule.getHandle(
        botScheduleId(universeId, pollSpec.botName, pollSpec.triggerName),
      );
      await pollHandle.trigger();
      await eventually(
        () => Promise.resolve(polls.length),
        (count) => count === 1,
      );
      const poll = polls[0] as { botId: string; triggerId: string; scheduledAt: string };
      expect(poll.botId).toBe(botId);
      expect(poll.triggerId).toBe(pollSpec.triggerId);
      expect(new Date(poll.scheduledAt).getTime()).not.toBeNaN();
      await deleteBotSchedule(env.client, universeId, pollSpec.botName, pollSpec.triggerName);
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);
  it("publishes the directory before a delivery and sends receipts when it finishes", async () => {
    const fedBotName = "federated";
    const calls: string[] = [];
    const directories: unknown[] = [];
    const receipts: unknown[] = [];
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async () => ({ profileRevision: 1 }),
        readBotSessionStatus: async () => ({ status: "idle" }),
        readBotRunUsage: async () => null,
        publishBotDirectory: async (input: unknown) => {
          calls.push("directory");
          directories.push(input);
          return { entries: 1 };
        },
        startBotRun: async () => {
          calls.push("run");
          return { runId: `run_${calls.filter((call) => call === "run").length}` };
        },
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations:
            afterSeq === 0
              ? [
                  {
                    invocationId: `wti:sha256:${"d".repeat(64)}`,
                    toolId: BOT_EVENT_RESOLVE_TOOL_ID,
                    runId: "run_1",
                    argumentsRef: resolveRef,
                  },
                ]
              : [],
        }),
        readJsonBlob: async () => ({ outcome: "handled", summary: "root cause found" }),
        sendBotReceipts: async (input: unknown) => {
          calls.push("receipts");
          receipts.push(input);
          return { sent: 1 };
        },
        recordEventOutcomes: async () => ({ updated: 0 }),
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: fedBotName,
        displayName: null,
        profileId: "triage-bot",
        brief: null,
        runsPerDay: null,
        emit: true,
        enabled: true,
      };
      // An addressed event that asked for a receipt, two hops from the world.
      const ask: BotEvent = { version: 1, id: "ask-1", ref: eventRef, seq: 5, hops: 2, reply: true };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, fedBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [ask],
      });
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeDeliveries.some((active) => active.id === ask.id),
      );
      await handle.signal(
        "deliver_emission",
        sessionTerminalEmission(botSessionId(fedBotName), 1, botEventTerminalToken(ask.id)),
      );
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 1,
      );
      await eventually(async () => calls, (seen) => seen.includes("receipts"));
      // The directory lands before the run; the receipt carries the outcome
      // and the delivery's hop count.
      expect(calls).toEqual(["directory", "run", "receipts"]);
      expect(directories[0]).toMatchObject({ universeId, botId, sessionId: botSessionId(fedBotName) });
      expect(receipts[0]).toEqual({
        universeId,
        botId,
        deliveryId: ask.id,
        eventIds: [ask.id],
        status: "handled",
        summary: "root cause found",
        hops: 2,
      });

      // An event that did not ask gets no receipt.
      const plain: BotEvent = { version: 1, id: "plain-1", ref: eventRef, seq: 6 };
      await handle.signal(BOT_EVENT_SIGNAL, plain);
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeDeliveries.some((active) => active.id === plain.id),
      );
      await handle.signal(
        "deliver_emission",
        sessionTerminalEmission(botSessionId(fedBotName), 2, botEventTerminalToken(plain.id)),
      );
      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 2,
      );
      expect(receipts).toHaveLength(1);
      expect(calls.filter((call) => call === "directory")).toHaveLength(2);

      const history = await handle.fetchHistory();
      await Worker.runReplayHistory(
        { workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)) },
        history,
        botWorkflowId(universeId, fedBotName),
      );
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("carries conversation tools into a routed session, sends delivery receipts, and keeps the chat open", async () => {
    const chatBotName = "concierge";
    const ensured: Array<{ sessionId: string; toolsRef: string | null | undefined }> = [];
    const started: Array<{ sessionId: string; events: BotEvent[] }> = [];
    const receipts: Array<{ eventIds: string[]; receipt: Record<string, unknown> }> = [];
    let closes = 0;
    const toolsRef = `sha256:${"7".repeat(64)}`;
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
      activities: {
        ...defaultUsageActivities,
        ensureBotSession: async (input: { sessionId: string; toolsRef?: string | null }) => {
          ensured.push({ sessionId: input.sessionId, toolsRef: input.toolsRef });
          return {
            profileRevision: 1,
            carriedToolIds: input.toolsRef ? ["channels.message_send.v1"] : [],
          };
        },
        readBotSessionStatus: async () => ({ status: "idle" }),
        startBotRun: async (input: { sessionId: string; events: BotEvent[] }) => {
          started.push({ sessionId: input.sessionId, events: input.events });
          return { runId: `run_${started.length}` };
        },
        // The run answered through the conversation's own tool and never
        // called bot_event_resolve.
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations:
            afterSeq === 0
              ? [
                  {
                    invocationId: `wti:sha256:${"9".repeat(64)}`,
                    toolId: "channels.message_send.v1",
                    runId: "run_1",
                    argumentsRef: `sha256:${"c".repeat(64)}`,
                  },
                ]
              : [],
        }),
        readJsonBlob: async () => ({}),
        recordEventOutcomes: async () => ({ updated: 0 }),
        sendDeliveryReceipts: async (input: { eventIds: string[]; receipt: Record<string, unknown> }) => {
          receipts.push(input);
          return { sent: 1, skipped: 0 };
        },
        closeBotSession: async () => {
          closes += 1;
          return { closed: true };
        },
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const start: BotStartV1 = {
        version: 1,
        universeId,
        botId,
        botName: chatBotName,
        displayName: "Concierge",
        profileId: "concierge-bot",
        brief: null,
        runsPerDay: null,
        // The bot closes idle routed sessions quickly; the chat trigger says never.
        routedSessionTtlMs: 1_000,
        enabled: true,
      };
      const conversationSession = `${botSessionId(chatBotName)}:k-telegram-primary-123-0123abcd`;
      const event: BotEvent = {
        version: 1,
        id: "chat-1",
        ref: eventRef,
        seq: 17,
        promptRef: `sha256:${"e".repeat(64)}`,
        session: { sessionId: conversationSession, label: "telegram dm · Lukas", ttlMs: null },
        media: [{ blobRef: `sha256:${"9".repeat(64)}`, kind: "image", mime: "image/jpeg", name: "photo.jpg" }],
        tools: toolsRef,
        notify: true,
      };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, chatBotName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [event],
      });

      const inFlight = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeDeliveries.some((active) => active.runId === "run_1"),
      );
      expect(inFlight.activeDeliveries[0]?.sessionId).toBe(conversationSession);
      // The routed session was created with the conversation's declarations
      // and the run input carried the attachment.
      expect(ensured.find((entry) => entry.sessionId === conversationSession)?.toolsRef).toBe(toolsRef);
      expect(started[0]?.events[0]?.media).toEqual(event.media);
      const startedReceipt = await eventually(
        async () => receipts,
        (list) => list.some((entry) => entry.receipt.phase === "started"),
      );
      expect(startedReceipt[0]).toMatchObject({
        eventIds: ["chat-1"],
        receipt: { phase: "started", deliveryId: "chat-1", sessionId: conversationSession, runId: "run_1", seqs: [17] },
      });

      await handle.signal(
        "deliver_emission",
        sessionTerminalEmission(conversationSession, 1, botEventTerminalToken(event.id)),
      );
      const done = await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.eventsProcessed === 1,
      );
      // A reply through the carried tool counts as handled without bot_event_resolve.
      expect(done.recentEvents[0]).toMatchObject({ id: "chat-1", outcome: "handled", seqs: [17] });
      const finishedReceipt = await eventually(
        async () => receipts,
        (list) => list.some((entry) => entry.receipt.phase === "finished"),
      );
      expect(finishedReceipt.find((entry) => entry.receipt.phase === "finished")?.receipt).toMatchObject({
        deliveryId: "chat-1",
        sessionId: conversationSession,
        runId: "run_1",
        status: "handled",
      });
      expect(done.sessions.find((session) => session.sessionId === conversationSession)).toMatchObject({
        label: "telegram dm · Lukas",
        kind: "keyed",
        ttlMs: null,
        carriedToolIds: ["channels.message_send.v1"],
      });

      // Well past the bot's own retention: the conversation stays open.
      await new Promise((resolve) => setTimeout(resolve, 2_500));
      const later = await handle.query<BotSnapshot>(BOT_STATE_QUERY);
      expect(later.sessions.map((session) => session.sessionId)).toContain(conversationSession);
      expect(closes).toBe(0);

      const history = await handle.fetchHistory();
      await Worker.runReplayHistory(
        { workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)) },
        history,
        botWorkflowId(universeId, chatBotName),
      );
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);
});

function terminalEmission(runId: number, token: string): EmissionEnvelope {
  return sessionTerminalEmission(botSessionId(botName), runId, token);
}

function budgetTerminalEmission(name: string, runId: number, token: string): EmissionEnvelope {
  return sessionTerminalEmission(botSessionId(name), runId, token);
}

function sessionTerminalEmission(
  sessionId: string,
  runId: number,
  token: string,
): EmissionEnvelope {
  return {
    emission_id: `emission:sha256:${(runId + sessionId.length * 1000).toString(16).padStart(64, "0")}`,
    producer: {
      kind: "session",
      universe_id: universeId,
      session_id: sessionId,
      log_seq: runId,
    },
    body: {
      kind: "run_terminal",
      token,
      run_id: runId,
      status: "completed",
      output_ref: null,
      failure_message_ref: null,
    },
  };
}

async function eventually<T>(read: () => Promise<T>, ready: (value: T) => boolean): Promise<T> {
  const deadline = Date.now() + 20_000;
  for (;;) {
    const value = await read();
    if (ready(value)) return value;
    if (Date.now() >= deadline) throw new Error("timed out waiting for workflow state");
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
}
