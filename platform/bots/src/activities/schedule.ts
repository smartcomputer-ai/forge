import { eq } from "drizzle-orm";
import { LightspeedClient } from "@lightspeed/agent-client";
import { schema, type Db } from "@lightspeed/platform-db";
import type { Client } from "@temporalio/client";
import { deleteBotSchedule } from "../schedules.js";
import {
  BOT_CONTROLLER_WORKFLOW,
  BOT_EVENT_SIGNAL,
  BOTS_WORKFLOW_TASK_QUEUE,
  botScheduleEventId,
  botWorkflowId,
  type BotEvent,
  type BotEventDocumentV1,
  type BotStartV1,
} from "../contracts/bots.js";

export interface BotScheduleActivitiesConfig {
  db: Db;
  endpoint: string;
  temporal: Client;
  fetch?: typeof fetch;
}

export interface AdmitScheduleEventInput {
  botId: string;
  triggerId: string;
  scheduledAt: string;
}

export type AdmitScheduleEventResult =
  | { admitted: true; eventId: string; duplicate: boolean }
  | { admitted: false; reason: "trigger_missing" | "trigger_disabled" | "bot_disabled" };

export interface BotScheduleActivities {
  admitScheduleEvent(input: AdmitScheduleEventInput): Promise<AdmitScheduleEventResult>;
}

export function createBotScheduleActivities(
  config: BotScheduleActivitiesConfig,
): BotScheduleActivities {
  return {
    async admitScheduleEvent(input) {
      const [row] = await config.db
        .select({
          trigger: schema.botTriggers,
          bot: schema.bots,
          lightspeedUniverseId: schema.universes.lightspeedUniverseId,
        })
        .from(schema.botTriggers)
        .innerJoin(schema.bots, eq(schema.botTriggers.botId, schema.bots.id))
        .innerJoin(schema.universes, eq(schema.bots.universeId, schema.universes.id))
        .where(eq(schema.botTriggers.id, input.triggerId))
        .limit(1);
      if (!row || row.bot.id !== input.botId) return { admitted: false, reason: "trigger_missing" };
      if (row.trigger.kind !== "schedule") return { admitted: false, reason: "trigger_missing" };
      if (!row.trigger.enabled) return { admitted: false, reason: "trigger_disabled" };
      if (!row.bot.enabled) return { admitted: false, reason: "bot_disabled" };
      const spec = row.trigger.spec as {
        cron?: string | null;
        at?: string | null;
        timezone: string;
        summary: string;
      };

      const document: BotEventDocumentV1 = {
        version: 1,
        kind: "schedule",
        source: `schedule:${row.trigger.name}`,
        occurredAt: input.scheduledAt,
        summary: spec.summary,
        data: {
          triggerId: row.trigger.id,
          triggerName: row.trigger.name,
          cron: spec.cron ?? null,
          at: spec.at ?? null,
          timezone: spec.timezone,
          scheduledAt: input.scheduledAt,
        },
      };
      const client = new LightspeedClient({
        endpoint: config.endpoint,
        ...(config.fetch === undefined ? {} : { fetch: config.fetch }),
        headers: { "x-lightspeed-universe": row.lightspeedUniverseId },
      });
      const stored = await client.call("blobs/put", {
        blobs: [{ bytesBase64: Buffer.from(JSON.stringify(document), "utf8").toString("base64") }],
      });
      const ref = stored.result.blobs?.[0]?.blobRef;
      if (!ref) throw new Error("event document storage returned no ref");

      const eventId = botScheduleEventId(row.trigger.id, input.scheduledAt);
      const inserted = await config.db
        .insert(schema.botEvents)
        .values({
          botId: row.bot.id,
          eventId,
          triggerId: row.trigger.id,
          kind: "schedule",
          source: `schedule:${row.trigger.name}`,
          occurredAt: new Date(input.scheduledAt),
          ref,
        })
        .onConflictDoNothing()
        .returning();

      const start: BotStartV1 = {
        version: 1,
        universeId: row.lightspeedUniverseId,
        botId: row.bot.id,
        botName: row.bot.name,
        profileId: row.bot.profileId,
        brief: row.bot.brief,
        runsPerDay: row.bot.runsPerDay,
        enabled: row.bot.enabled,
      };
      const event: BotEvent = { version: 1, id: eventId, ref };
      await config.temporal.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
        workflowId: botWorkflowId(start.universeId, start.botName),
        taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: BOT_EVENT_SIGNAL,
        signalArgs: [event],
      });
      if (spec.at) {
        // One-shot: it has fired; disable the trigger and drop the schedule so
        // it cannot fire again and reads as spent in the UI.
        await config.db
          .update(schema.botTriggers)
          .set({ enabled: false })
          .where(eq(schema.botTriggers.id, row.trigger.id));
        await deleteBotSchedule(
          config.temporal,
          row.lightspeedUniverseId,
          row.bot.name,
          row.trigger.name,
        ).catch(() => undefined);
      }
      return { admitted: true, eventId, duplicate: inserted.length === 0 };
    },
  };
}
