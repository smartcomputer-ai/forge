import { describe, expect, it } from "vitest";
import type { Db } from "@lightspeed/platform-db";
import {
  BOT_CONTROLLER_WORKFLOW,
  BOT_EVENT_SIGNAL,
  BOTS_WORKFLOW_TASK_QUEUE,
  botWorkflowId,
  type BotEvent,
  type BotStartV1,
} from "../src/contracts/bots.js";
import { wakeBotController, type BotWakeClient } from "../src/events.js";

const start: BotStartV1 = {
  version: 1,
  universeId: "0f1e2d3c-4b5a-4c6d-8e7f-90a1b2c3d4e5",
  botId: "bot-1",
  botName: "ops",
  displayName: null,
  profileId: "profile-1",
  brief: null,
  runsPerDay: 10,
  enabled: true,
};
const event: BotEvent = { version: 1, id: "evt-1", ref: "blob:sha256:abc", seq: 7 };

function fakeDb(behavior: "ok" | "fail" = "ok") {
  const deletes: unknown[] = [];
  const db = {
    delete: (table: unknown) => ({
      where: async (clause: unknown) => {
        if (behavior === "fail") throw new Error("database unavailable");
        deletes.push({ table, clause });
      },
    }),
  };
  return { db: db as unknown as Db, deletes };
}

function fakeTemporal(behavior: "ok" | "fail" = "ok") {
  const calls: { type: unknown; options: Record<string, unknown> }[] = [];
  const temporal = {
    workflow: {
      signalWithStart: async (type: unknown, options: Record<string, unknown>) => {
        calls.push({ type, options });
        if (behavior === "fail") throw new Error("temporal unavailable");
        return {};
      },
    },
  };
  return { temporal: temporal as unknown as BotWakeClient, calls };
}

describe("wakeBotController", () => {
  it("wakes the controller by signal-with-start carrying the event", async () => {
    const { db, deletes } = fakeDb();
    const { temporal, calls } = fakeTemporal();

    await wakeBotController({ db, temporal, start, event, stored: true });

    expect(calls).toHaveLength(1);
    expect(calls[0]?.type).toBe(BOT_CONTROLLER_WORKFLOW);
    expect(calls[0]?.options).toMatchObject({
      workflowId: botWorkflowId(start.universeId, start.botName),
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      args: [start],
      signal: BOT_EVENT_SIGNAL,
      signalArgs: [event],
    });
    expect(deletes).toHaveLength(0);
  });

  it("discards the row it stored when the wake fails, then rethrows", async () => {
    const { db, deletes } = fakeDb();
    const { temporal } = fakeTemporal("fail");

    await expect(
      wakeBotController({ db, temporal, start, event, stored: true }),
    ).rejects.toThrow("temporal unavailable");
    expect(deletes).toHaveLength(1);
  });

  it("keeps an existing row when re-waking a duplicate fails", async () => {
    const { db, deletes } = fakeDb();
    const { temporal } = fakeTemporal("fail");

    await expect(
      wakeBotController({ db, temporal, start, event, stored: false }),
    ).rejects.toThrow("temporal unavailable");
    expect(deletes).toHaveLength(0);
  });

  it("reports the wake failure even when the discard fails", async () => {
    const { db } = fakeDb("fail");
    const { temporal } = fakeTemporal("fail");

    await expect(
      wakeBotController({ db, temporal, start, event, stored: true }),
    ).rejects.toThrow("temporal unavailable");
  });
});
