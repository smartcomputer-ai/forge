import { fileURLToPath } from "node:url";
import { TestWorkflowEnvironment } from "@temporalio/testing";
import { Worker } from "@temporalio/worker";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { EmissionEnvelope } from "../src/contracts/emissions.js";
import {
  BOT_CONTROLLER_WORKFLOW,
  BOT_EVENT_RESOLVE_TOOL_ID,
  BOT_EVENT_SIGNAL,
  BOT_STATE_QUERY,
  BOTS_ACTIVITY_TASK_QUEUE,
  BOTS_WORKFLOW_TASK_QUEUE,
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
            return { eventId: "delivery-1", outcome: "handled", summary: "queue drained" };
          }
          throw new Error(`unexpected blob ${blobRef}`);
        },
        recordBotActivity: async (input: unknown) => {
          activity.push(input);
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
        profileId: "triage-bot",
        brief: "Watch the queue.",
        runsPerDay: null,
        enabled: true,
      };
      const event: BotEvent = { version: 1, id: "delivery-1", ref: eventRef };
      const handle = await env.client.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(universeId, botName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [event],
      });

      await eventually(
        () => handle.query<BotSnapshot>(BOT_STATE_QUERY),
        (state) => state.activeEvent?.id === event.id,
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
        status: "handled",
        runId: "run_1",
        summary: "queue drained",
      });
      expect(completed.appliedProfileRevision).toBe(4);
      expect(completed.runsToday).toBe(1);
      expect(calls).toEqual(["ensureSession", "startRun"]);

      const kinds = activity.flatMap((input) =>
        (input as { entries: { kind: string }[] }).entries.map((entry) => entry.kind),
      );
      expect(kinds).toContain("run_started");
      expect(kinds).toContain("run_completed");

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
        ensureBotSession: async () => ({ profileRevision: 1 }),
        readBotSessionStatus: async () => ({ status: "idle" }),
        startBotRun: async () => {
          runsStarted += 1;
          return { runId: `run_${runsStarted}` };
        },
        readWorkflowToolInvocations: async ({ afterSeq }: { afterSeq: number }) => ({
          nextSeq: afterSeq + 10,
          invocations: [],
        }),
        readJsonBlob: async () => ({}),
        recordBotActivity: async () => undefined,
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
        (state) => state.activeEvent?.id === first.id,
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
      expect(runsStarted).toBe(1);
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);
  it("reconciles schedules and fires them through the admission activity", async () => {
    const admissions: unknown[] = [];
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
        admitScheduleEvent: async (input: unknown) => {
          admissions.push(input);
          return { admitted: true, eventId: "schedule:test", duplicate: false };
        },
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    const spec: BotScheduleSpec = {
      universeId,
      botId,
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
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);
});

function terminalEmission(runId: number, token: string): EmissionEnvelope {
  return budgetTerminalEmission(botName, runId, token);
}

function budgetTerminalEmission(name: string, runId: number, token: string): EmissionEnvelope {
  return {
    emission_id: `emission:sha256:${runId.toString(16).padStart(64, "0")}`,
    producer: {
      kind: "session",
      universe_id: universeId,
      session_id: botSessionId(name),
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
