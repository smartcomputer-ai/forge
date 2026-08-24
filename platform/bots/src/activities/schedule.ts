import { and, count, eq, gte } from "drizzle-orm";
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
import { allocateBotEventSeq, renderAdmittedEvent } from "../events.js";

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
  | {
      admitted: false;
      reason: "trigger_missing" | "trigger_disabled" | "bot_disabled" | "breaker_tripped";
    };

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
      // The flood breaker guards schedules exactly like webhooks: a trigger
      // firing beyond the bot's rate is disabled (and its Temporal Schedule
      // dropped) until a human re-enables it. Misconfigured or runaway
      // schedules must not burn the run budget unattended.
      const breaker = row.bot.breaker;
      if (breaker) {
        const since = new Date(Date.now() - breaker.windowMs);
        const [recent] = await config.db
          .select({ value: count() })
          .from(schema.botEvents)
          .where(
            and(
              eq(schema.botEvents.triggerId, row.trigger.id),
              gte(schema.botEvents.receivedAt, since),
            ),
          );
        if (Number(recent?.value ?? 0) >= breaker.fires) {
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
          await config.db.insert(schema.botActivity).values({
            botId: row.bot.id,
            kind: "breaker_tripped",
            eventId: null,
            runId: null,
            detail: `schedule trigger ${row.trigger.name} exceeded ${breaker.fires} fires in ${Math.round(breaker.windowMs / 1000)}s and was disabled`,
          });
          return { admitted: false, reason: "breaker_tripped" };
        }
      }
      const spec = row.trigger.spec as {
        cron?: string | null;
        at?: string | null;
        timezone: string;
        summary: string;
      };

      // The prompt carries everything the session needs in a few lines; the
      // machine envelope keeps only what filters and replay can use.
      const document: BotEventDocumentV1 = {
        version: 1,
        kind: "schedule",
        source: `schedule:${row.trigger.name}`,
        occurredAt: input.scheduledAt,
        summary: spec.summary,
        data: {
          trigger: row.trigger.name,
          ...(spec.cron == null ? {} : { cron: spec.cron }),
          ...(spec.at == null ? {} : { at: spec.at }),
          timezone: spec.timezone,
          scheduledAt: input.scheduledAt,
        },
      };
      const seq = await allocateBotEventSeq(config.db, row.bot.id);
      const prompt = renderAdmittedEvent(seq, document);
      const client = new LightspeedClient({
        endpoint: config.endpoint,
        ...(config.fetch === undefined ? {} : { fetch: config.fetch }),
        headers: { "x-lightspeed-universe": row.lightspeedUniverseId },
      });
      const stored = await client.call("blobs/put", {
        blobs: [
          { bytesBase64: Buffer.from(JSON.stringify(document), "utf8").toString("base64") },
          { bytesBase64: Buffer.from(prompt, "utf8").toString("base64") },
        ],
      });
      let ref = stored.result.blobs?.[0]?.blobRef;
      let promptRef = stored.result.blobs?.[1]?.blobRef;
      if (!ref || !promptRef) throw new Error("event document storage returned no ref");

      const eventId = botScheduleEventId(row.trigger.id, input.scheduledAt);
      let eventSeq: number | null = seq;
      const inserted = await config.db
        .insert(schema.botEvents)
        .values({
          botId: row.bot.id,
          eventId,
          seq,
          triggerId: row.trigger.id,
          kind: "schedule",
          source: `schedule:${row.trigger.name}`,
          occurredAt: new Date(input.scheduledAt),
          ref,
          promptRef,
        })
        .onConflictDoNothing()
        .returning();
      if (inserted.length === 0) {
        // A retried fire reuses the stored row's identity so #N stays stable.
        const [existing] = await config.db
          .select()
          .from(schema.botEvents)
          .where(and(eq(schema.botEvents.botId, row.bot.id), eq(schema.botEvents.eventId, eventId)))
          .limit(1);
        if (existing) {
          ref = existing.ref;
          promptRef = existing.promptRef ?? undefined;
          eventSeq = existing.seq;
        }
      }

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
      const event: BotEvent = {
        version: 1,
        id: eventId,
        ref,
        ...(eventSeq === null ? {} : { seq: eventSeq }),
        ...(promptRef === undefined ? {} : { promptRef }),
      };
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
