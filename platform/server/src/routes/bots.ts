import { Hono } from "hono";
import { and, desc, eq } from "drizzle-orm";
import { z } from "zod";
import { schema } from "@lightspeed/platform-db";
import { Client, Connection } from "@temporalio/client";
import {
  BOT_CONFIG_SIGNAL,
  BOT_CONTROLLER_WORKFLOW,
  BOT_EVENT_SIGNAL,
  BOT_STATE_QUERY,
  BOTS_WORKFLOW_TASK_QUEUE,
  botWorkflowId,
  type BotEvent,
  type BotEventDocumentV1,
  type BotStartV1,
} from "@lightspeed/bots/contracts";
import {
  deleteBotSchedule,
  upsertBotSchedule,
  type BotScheduleSpec,
} from "@lightspeed/bots/schedules";
import type { BotSnapshot } from "@lightspeed/bots/workflows";
import type { AppContext, ApiVariables } from "../context.js";
import { parseBody } from "../http.js";
import { engineClientFor } from "./gateway.js";
import { universeForSession } from "./universes.js";

const { bots, botTriggers, botEvents, botActivity } = schema;

const botName = z
  .string()
  .regex(/^[a-z0-9][a-z0-9-]*$/, "lowercase alphanumerics and dashes")
  .max(64);
/// Temporal Schedules take classic 5-field crontab or an @-macro; reject
/// Quartz-style expressions (seconds field, `?`) with a message that names
/// the expected shape instead of Temporal's field-range error.
const cronField = z
  .string()
  .trim()
  .min(1)
  .max(200)
  .refine(
    (value) => value.startsWith("@") || (!value.includes("?") && value.split(/\s+/).length === 5),
    "expected 5-field cron (minute hour day month weekday) or an @-macro like @daily",
  );
const triggerCreateSchema = z.object({
  name: botName,
  kind: z.literal("schedule").default("schedule"),
  cron: cronField,
  timezone: z.string().trim().min(1).max(64).default("UTC"),
  summary: z.string().trim().min(1).max(2_000),
  enabled: z.boolean().default(true),
});
const triggerUpdateSchema = z
  .object({
    cron: cronField.optional(),
    timezone: z.string().trim().min(1).max(64).optional(),
    summary: z.string().trim().min(1).max(2_000).optional(),
    enabled: z.boolean().optional(),
  })
  .refine((value) => Object.keys(value).length > 0, "at least one field is required");
const botCreateSchema = z.object({
  name: botName,
  profileId: z.string().trim().min(1),
  brief: z.string().trim().min(1).max(20_000).nullish(),
  runsPerDay: z.number().int().min(1).max(10_000).nullish(),
});
const botUpdateSchema = z
  .object({
    profileId: z.string().trim().min(1).optional(),
    brief: z.string().trim().min(1).max(20_000).nullable().optional(),
    runsPerDay: z.number().int().min(1).max(10_000).nullable().optional(),
    enabled: z.boolean().optional(),
  })
  .refine((value) => Object.keys(value).length > 0, "at least one field is required");
const eventCreateSchema = z.object({
  id: z.string().trim().min(1).max(200).optional(),
  kind: z.string().trim().min(1).max(200),
  source: z.string().trim().min(1).max(200).default("manual"),
  occurredAt: z.string().datetime({ offset: true }).optional(),
  summary: z.string().trim().min(1).max(2_000),
  data: z.unknown().optional(),
  correlationId: z.string().trim().min(1).max(200).nullable().optional(),
  links: z.array(z.string().url()).max(20).optional(),
});

let temporalClient: Promise<Client> | null = null;
function getTemporal(): Promise<Client> {
  temporalClient ??= Connection.connect({
    address: process.env.TEMPORAL_ADDRESS ?? "localhost:7233",
  }).then(
    (connection) =>
      new Client({
        connection,
        namespace: process.env.TEMPORAL_NAMESPACE ?? "default",
      }),
  );
  return temporalClient;
}

export function botRoutes(ctx: AppContext) {
  const byUniverse = new Hono<{ Variables: ApiVariables }>();
  const byId = new Hono<{ Variables: ApiVariables }>();

  byUniverse.get("/:id/bots", async (c) => {
    const access = await universeForSession(ctx, c, c.req.param("id"), false);
    if (!access) return c.json({ error: "not found" }, 404);
    const rows = await ctx.db
      .select()
      .from(bots)
      .where(eq(bots.universeId, access.universe.id))
      .orderBy(bots.name);
    return c.json({ bots: rows });
  });

  byUniverse.post("/:id/bots", async (c) => {
    const access = await universeForSession(ctx, c, c.req.param("id"), true);
    if (!access) return c.json({ error: "not found" }, 404);
    const body = await parseBody(c, botCreateSchema);
    if (!body.ok) return body.response;

    const engine = engineClientFor(ctx, access.universe);
    await engine.call("profiles/read", { profileId: body.data.profileId });

    const [bot] = await ctx.db
      .insert(bots)
      .values({
        universeId: access.universe.id,
        name: body.data.name,
        profileId: body.data.profileId,
        brief: body.data.brief ?? null,
        runsPerDay: body.data.runsPerDay ?? null,
      })
      .onConflictDoNothing()
      .returning();
    if (!bot) return c.json({ error: "a bot with that name already exists" }, 409);

    try {
      await signalConfig(botStart(bot, access.universe.lightspeedUniverseId));
    } catch (error) {
      await ctx.db.delete(bots).where(eq(bots.id, bot.id));
      return c.json(
        { error: "failed to start the bot controller", failure: errorMessage(error) },
        502,
      );
    }
    return c.json({ bot }, 201);
  });

  async function botForSession(
    c: Parameters<typeof universeForSession>[1],
    botIdParam: string,
    write: boolean,
  ) {
    const [bot] = await ctx.db.select().from(bots).where(eq(bots.id, botIdParam)).limit(1);
    if (!bot) return null;
    const access = await universeForSession(ctx, c, bot.universeId, write);
    return access ? { bot, access } : null;
  }

  byId.get("/:id", async (c) => {
    const found = await botForSession(c, c.req.param("id"), false);
    if (!found) return c.json({ error: "not found" }, 404);
    return c.json({ bot: found.bot });
  });

  byId.patch("/:id", async (c) => {
    const found = await botForSession(c, c.req.param("id"), true);
    if (!found) return c.json({ error: "not found" }, 404);
    const body = await parseBody(c, botUpdateSchema);
    if (!body.ok) return body.response;
    if (body.data.profileId !== undefined) {
      await engineClientFor(ctx, found.access.universe).call("profiles/read", {
        profileId: body.data.profileId,
      });
    }
    const [bot] = await ctx.db.update(bots).set(body.data).where(eq(bots.id, found.bot.id)).returning();
    if (!bot) return c.json({ error: "not found" }, 404);
    try {
      await signalConfig(botStart(bot, found.access.universe.lightspeedUniverseId));
      if (body.data.enabled !== undefined && bot.enabled !== found.bot.enabled) {
        await reconcileSchedules(bot, found.access.universe.lightspeedUniverseId);
      }
    } catch (error) {
      await ctx.db
        .update(bots)
        .set({
          profileId: found.bot.profileId,
          brief: found.bot.brief,
          runsPerDay: found.bot.runsPerDay,
          enabled: found.bot.enabled,
        })
        .where(eq(bots.id, found.bot.id));
      await signalConfig(botStart(found.bot, found.access.universe.lightspeedUniverseId)).catch(
        () => undefined,
      );
      return c.json(
        {
          error: "controller reconciliation failed; configuration was not changed",
          failure: errorMessage(error),
        },
        502,
      );
    }
    return c.json({ bot });
  });

  async function reconcileSchedules(bot: BotRow, universeId: string): Promise<void> {
    const temporal = await getTemporal();
    const triggers = await ctx.db
      .select()
      .from(botTriggers)
      .where(eq(botTriggers.botId, bot.id));
    for (const trigger of triggers) {
      await upsertBotSchedule(temporal, scheduleSpec(bot, trigger, universeId));
    }
  }

  byId.get("/:id/triggers", async (c) => {
    const found = await botForSession(c, c.req.param("id"), false);
    if (!found) return c.json({ error: "not found" }, 404);
    const triggers = await ctx.db
      .select()
      .from(botTriggers)
      .where(eq(botTriggers.botId, found.bot.id))
      .orderBy(botTriggers.name);
    return c.json({ triggers });
  });

  byId.post("/:id/triggers", async (c) => {
    const found = await botForSession(c, c.req.param("id"), true);
    if (!found) return c.json({ error: "not found" }, 404);
    const body = await parseBody(c, triggerCreateSchema);
    if (!body.ok) return body.response;
    const [trigger] = await ctx.db
      .insert(botTriggers)
      .values({
        botId: found.bot.id,
        name: body.data.name,
        kind: body.data.kind,
        cron: body.data.cron,
        timezone: body.data.timezone,
        summary: body.data.summary,
        enabled: body.data.enabled,
      })
      .onConflictDoNothing()
      .returning();
    if (!trigger) return c.json({ error: "a trigger with that name already exists" }, 409);
    try {
      const temporal = await getTemporal();
      await upsertBotSchedule(
        temporal,
        scheduleSpec(found.bot, trigger, found.access.universe.lightspeedUniverseId),
      );
    } catch (error) {
      await ctx.db.delete(botTriggers).where(eq(botTriggers.id, trigger.id));
      return c.json(
        { error: "failed to create the schedule", failure: errorMessage(error) },
        502,
      );
    }
    return c.json({ trigger }, 201);
  });

  byId.patch("/:id/triggers/:triggerId", async (c) => {
    const found = await botForSession(c, c.req.param("id"), true);
    if (!found) return c.json({ error: "not found" }, 404);
    const [existing] = await ctx.db
      .select()
      .from(botTriggers)
      .where(and(eq(botTriggers.id, c.req.param("triggerId")), eq(botTriggers.botId, found.bot.id)))
      .limit(1);
    if (!existing) return c.json({ error: "not found" }, 404);
    const body = await parseBody(c, triggerUpdateSchema);
    if (!body.ok) return body.response;
    const [trigger] = await ctx.db
      .update(botTriggers)
      .set(body.data)
      .where(eq(botTriggers.id, existing.id))
      .returning();
    if (!trigger) return c.json({ error: "not found" }, 404);
    try {
      const temporal = await getTemporal();
      await upsertBotSchedule(
        temporal,
        scheduleSpec(found.bot, trigger, found.access.universe.lightspeedUniverseId),
      );
    } catch (error) {
      await ctx.db
        .update(botTriggers)
        .set({
          cron: existing.cron,
          timezone: existing.timezone,
          summary: existing.summary,
          enabled: existing.enabled,
        })
        .where(eq(botTriggers.id, existing.id));
      return c.json(
        {
          error: "schedule reconciliation failed; the trigger was not changed",
          failure: errorMessage(error),
        },
        502,
      );
    }
    return c.json({ trigger });
  });

  byId.delete("/:id/triggers/:triggerId", async (c) => {
    const found = await botForSession(c, c.req.param("id"), true);
    if (!found) return c.json({ error: "not found" }, 404);
    const [existing] = await ctx.db
      .select()
      .from(botTriggers)
      .where(and(eq(botTriggers.id, c.req.param("triggerId")), eq(botTriggers.botId, found.bot.id)))
      .limit(1);
    if (!existing) return c.json({ error: "not found" }, 404);
    try {
      const temporal = await getTemporal();
      await deleteBotSchedule(
        temporal,
        found.access.universe.lightspeedUniverseId,
        found.bot.name,
        existing.name,
      );
    } catch (error) {
      return c.json(
        { error: "failed to delete the schedule; the trigger was kept", failure: errorMessage(error) },
        502,
      );
    }
    await ctx.db.delete(botTriggers).where(eq(botTriggers.id, existing.id));
    return c.json({ deleted: true });
  });

  byId.get("/:id/state", async (c) => {
    const found = await botForSession(c, c.req.param("id"), false);
    if (!found) return c.json({ error: "not found" }, 404);
    const temporal = await getTemporal();
    const handle = temporal.workflow.getHandle(
      botWorkflowId(found.access.universe.lightspeedUniverseId, found.bot.name),
    );
    try {
      const state = await handle.query<BotSnapshot>(BOT_STATE_QUERY);
      return c.json({ state });
    } catch (error) {
      return c.json({ error: "bot controller unavailable", failure: errorMessage(error) }, 503);
    }
  });

  byId.post("/:id/events", async (c) => {
    const found = await botForSession(c, c.req.param("id"), true);
    if (!found) return c.json({ error: "not found" }, 404);
    if (!found.bot.enabled) return c.json({ error: "bot is disabled" }, 409);
    const body = await parseBody(c, eventCreateSchema);
    if (!body.ok) return body.response;
    const occurredAt = body.data.occurredAt ?? new Date().toISOString();
    const document: BotEventDocumentV1 = {
      version: 1,
      kind: body.data.kind,
      source: body.data.source,
      occurredAt,
      summary: body.data.summary,
      ...(body.data.data === undefined ? {} : { data: body.data.data }),
      ...(body.data.correlationId === undefined ? {} : { correlationId: body.data.correlationId }),
      ...(body.data.links === undefined ? {} : { links: body.data.links }),
    };
    const engine = engineClientFor(ctx, found.access.universe);
    const stored = await engine.call("blobs/put", {
      blobs: [{ bytesBase64: Buffer.from(JSON.stringify(document), "utf8").toString("base64") }],
    });
    const ref = stored.result.blobs?.[0]?.blobRef;
    if (!ref) return c.json({ error: "event document storage returned no ref" }, 502);
    const eventId = body.data.id ?? crypto.randomUUID();

    // Store, then wake: the envelope row is authoritative; the signal only
    // notifies the controller that this event id exists.
    const inserted = await ctx.db
      .insert(botEvents)
      .values({
        botId: found.bot.id,
        eventId,
        kind: body.data.kind,
        source: body.data.source,
        occurredAt: new Date(occurredAt),
        ref,
      })
      .onConflictDoNothing()
      .returning();
    const duplicate = inserted.length === 0;

    const event: BotEvent = { version: 1, id: eventId, ref };
    const config = botStart(found.bot, found.access.universe.lightspeedUniverseId);
    const temporal = await getTemporal();
    await temporal.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
      workflowId: botWorkflowId(config.universeId, config.botName),
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      args: [config],
      signal: BOT_EVENT_SIGNAL,
      signalArgs: [event],
    });
    return c.json({ event, document, duplicate }, 202);
  });

  byId.get("/:id/events", async (c) => {
    const found = await botForSession(c, c.req.param("id"), false);
    if (!found) return c.json({ error: "not found" }, 404);
    const events = await ctx.db
      .select()
      .from(botEvents)
      .where(eq(botEvents.botId, found.bot.id))
      .orderBy(desc(botEvents.receivedAt))
      .limit(100);
    return c.json({ events });
  });

  byId.get("/:id/activity", async (c) => {
    const found = await botForSession(c, c.req.param("id"), false);
    if (!found) return c.json({ error: "not found" }, 404);
    const eventId = c.req.query("eventId");
    const activity = await ctx.db
      .select()
      .from(botActivity)
      .where(
        eventId === undefined
          ? eq(botActivity.botId, found.bot.id)
          : and(eq(botActivity.botId, found.bot.id), eq(botActivity.eventId, eventId)),
      )
      .orderBy(desc(botActivity.createdAt))
      .limit(200);
    return c.json({ activity });
  });

  return { byUniverse, byId };
}

type BotRow = typeof bots.$inferSelect;
type TriggerRow = typeof botTriggers.$inferSelect;

function scheduleSpec(bot: BotRow, trigger: TriggerRow, universeId: string): BotScheduleSpec {
  return {
    universeId,
    botId: bot.id,
    botName: bot.name,
    triggerId: trigger.id,
    triggerName: trigger.name,
    cron: trigger.cron,
    timezone: trigger.timezone,
    paused: !(bot.enabled && trigger.enabled),
  };
}

function botStart(bot: BotRow, universeId: string): BotStartV1 {
  return {
    version: 1,
    universeId,
    botId: bot.id,
    botName: bot.name,
    profileId: bot.profileId,
    brief: bot.brief,
    runsPerDay: bot.runsPerDay,
    enabled: bot.enabled,
  };
}

async function signalConfig(config: BotStartV1): Promise<void> {
  const temporal = await getTemporal();
  await temporal.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
    workflowId: botWorkflowId(config.universeId, config.botName),
    taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
    args: [config],
    signal: BOT_CONFIG_SIGNAL,
    signalArgs: [config],
  });
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
