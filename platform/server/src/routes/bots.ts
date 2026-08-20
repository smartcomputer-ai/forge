import { randomBytes } from "node:crypto";
import { Hono } from "hono";
import { and, desc, eq } from "drizzle-orm";
import { z } from "zod";
import { schema } from "@lightspeed/platform-db";
import { BOT_STATE_QUERY, botWorkflowId, type BotEventDocumentV1 } from "@lightspeed/bots/contracts";
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
import {
  admitBotEvent,
  botStart,
  errorMessage,
  getTemporal,
  recordActivity,
  signalBotConfig,
  type BotRow,
  type BotTriggerRow,
} from "./bot-common.js";

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
const celField = z.string().trim().min(1).max(2_000);
const routeInput = z.discriminatedUnion("policy", [
  z.object({ policy: z.literal("bot") }),
  z.object({ policy: z.literal("perKey"), key: celField.max(500).nullish() }),
  z.object({ policy: z.literal("perEvent") }),
]);
const scheduleSpecInput = z.object({
  cron: cronField,
  timezone: z.string().trim().min(1).max(64).default("UTC"),
  summary: z.string().trim().min(1).max(2_000),
});
const webhookVerificationInput = z.discriminatedUnion("scheme", [
  z.object({ scheme: z.literal("token") }),
  z.object({
    scheme: z.literal("hmac-sha256"),
    secret: z.string().min(8).max(200),
    header: z.string().trim().min(1).max(100),
    prefix: z.string().max(20).optional(),
  }),
]);
const webhookSpecInput = z.object({
  verification: webhookVerificationInput.default({ scheme: "token" }),
  preset: z.enum(["github"]).nullish(),
});
const coalesceInput = z
  .object({
    debounceMs: z.number().int().min(1_000).max(604_800_000),
    maxWaitMs: z.number().int().min(1_000).max(604_800_000),
    maxCount: z.number().int().min(2).max(100),
  })
  .refine((value) => value.maxWaitMs >= value.debounceMs, "maxWaitMs must cover debounceMs");
const deliverInput = z.object({ whenBusy: z.enum(["queue", "steer", "append"]) });
const breakerInput = z.object({
  fires: z.number().int().min(1).max(100_000),
  windowMs: z.number().int().min(1_000).max(86_400_000),
});
const triggerCreateSchema = z.discriminatedUnion("kind", [
  z.object({
    name: botName,
    kind: z.literal("schedule"),
    spec: scheduleSpecInput,
    enabled: z.boolean().default(true),
  }),
  z.object({
    name: botName,
    kind: z.literal("webhook"),
    spec: webhookSpecInput.default({ verification: { scheme: "token" } }),
    filter: celField.nullish(),
    route: routeInput.nullish(),
    coalesce: coalesceInput.nullish(),
    deliver: deliverInput.nullish(),
    enabled: z.boolean().default(true),
  }),
]);
const triggerUpdateSchema = z
  .object({
    spec: z.unknown().optional(),
    filter: celField.nullable().optional(),
    route: routeInput.nullable().optional(),
    coalesce: coalesceInput.nullable().optional(),
    deliver: deliverInput.nullable().optional(),
    enabled: z.boolean().optional(),
  })
  .refine((value) => Object.keys(value).length > 0, "at least one field is required");
const botCreateSchema = z.object({
  name: botName,
  profileId: z.string().trim().min(1),
  brief: z.string().trim().min(1).max(20_000).nullish(),
  runsPerDay: z.number().int().min(1).max(10_000).nullish(),
  breaker: breakerInput.nullish(),
});
const botUpdateSchema = z
  .object({
    profileId: z.string().trim().min(1).optional(),
    brief: z.string().trim().min(1).max(20_000).nullable().optional(),
    runsPerDay: z.number().int().min(1).max(10_000).nullable().optional(),
    breaker: breakerInput.nullable().optional(),
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
        breaker: body.data.breaker ?? null,
      })
      .onConflictDoNothing()
      .returning();
    if (!bot) return c.json({ error: "a bot with that name already exists" }, 409);

    try {
      await signalBotConfig(botStart(bot, access.universe.lightspeedUniverseId));
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
      await signalBotConfig(botStart(bot, found.access.universe.lightspeedUniverseId));
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
          breaker: found.bot.breaker,
          enabled: found.bot.enabled,
        })
        .where(eq(bots.id, found.bot.id));
      await signalBotConfig(botStart(found.bot, found.access.universe.lightspeedUniverseId)).catch(
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
      .where(and(eq(botTriggers.botId, bot.id), eq(botTriggers.kind, "schedule")));
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
    const input = body.data;
    const values =
      input.kind === "schedule"
        ? {
            botId: found.bot.id,
            name: input.name,
            kind: input.kind,
            spec: input.spec,
            filter: null,
            route: null,
            enabled: input.enabled,
          }
        : {
            botId: found.bot.id,
            name: input.name,
            kind: input.kind,
            spec: { ...input.spec, token: randomBytes(24).toString("hex") },
            filter: input.filter ?? null,
            route: input.route ?? null,
            coalesce: input.coalesce ?? null,
            deliver: input.deliver ?? null,
            enabled: input.enabled,
          };
    const [trigger] = await ctx.db
      .insert(botTriggers)
      .values(values)
      .onConflictDoNothing()
      .returning();
    if (!trigger) return c.json({ error: "a trigger with that name already exists" }, 409);
    if (input.kind === "schedule") {
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
    if (
      existing.kind === "schedule" &&
      (body.data.filter !== undefined ||
        body.data.route !== undefined ||
        body.data.coalesce !== undefined ||
        body.data.deliver !== undefined)
    ) {
      return c.json(
        { error: "filters, routes, coalescing, and delivery policy apply to webhook triggers" },
        400,
      );
    }

    const changes: Partial<typeof existing> = {};
    if (body.data.enabled !== undefined) changes.enabled = body.data.enabled;
    if (body.data.filter !== undefined) changes.filter = body.data.filter;
    if (body.data.route !== undefined) changes.route = body.data.route;
    if (body.data.coalesce !== undefined) changes.coalesce = body.data.coalesce;
    if (body.data.deliver !== undefined) changes.deliver = body.data.deliver;
    if (body.data.spec !== undefined) {
      if (existing.kind === "schedule") {
        const parsed = scheduleSpecInput.safeParse(body.data.spec);
        if (!parsed.success) {
          return c.json({ error: "validation failed", issues: parsed.error.issues }, 400);
        }
        changes.spec = parsed.data;
      } else {
        const parsed = webhookSpecInput.safeParse(body.data.spec);
        if (!parsed.success) {
          return c.json({ error: "validation failed", issues: parsed.error.issues }, 400);
        }
        // The URL token survives spec edits; rotation means a new trigger.
        const token = (existing.spec as { token: string }).token;
        changes.spec = { ...parsed.data, token };
      }
    }

    const [trigger] = await ctx.db
      .update(botTriggers)
      .set(changes)
      .where(eq(botTriggers.id, existing.id))
      .returning();
    if (!trigger) return c.json({ error: "not found" }, 404);
    if (existing.kind === "schedule") {
      try {
        const temporal = await getTemporal();
        await upsertBotSchedule(
          temporal,
          scheduleSpec(found.bot, trigger, found.access.universe.lightspeedUniverseId),
        );
      } catch (error) {
        await ctx.db
          .update(botTriggers)
          .set({ spec: existing.spec, enabled: existing.enabled })
          .where(eq(botTriggers.id, existing.id));
        return c.json(
          {
            error: "schedule reconciliation failed; the trigger was not changed",
            failure: errorMessage(error),
          },
          502,
        );
      }
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
    if (existing.kind === "schedule") {
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
    const { event, duplicate } = await admitBotEvent(ctx, {
      bot: found.bot,
      universe: found.access.universe,
      eventId: body.data.id ?? crypto.randomUUID(),
      document,
    });
    return c.json({ event, document, duplicate }, 202);
  });

  byId.post("/:id/events/replay", async (c) => {
    const found = await botForSession(c, c.req.param("id"), true);
    if (!found) return c.json({ error: "not found" }, 404);
    if (!found.bot.enabled) return c.json({ error: "bot is disabled" }, 409);
    const body = await parseBody(c, z.object({ eventId: z.string().trim().min(1).max(200) }));
    if (!body.ok) return body.response;
    const [stored] = await ctx.db
      .select()
      .from(botEvents)
      .where(and(eq(botEvents.botId, found.bot.id), eq(botEvents.eventId, body.data.eventId)))
      .limit(1);
    if (!stored) return c.json({ error: "not found" }, 404);

    // A replay is a fresh envelope reusing the stored document and routing;
    // it never coalesces, so it delivers promptly and exactly once.
    const replayId = `replay-${crypto.randomUUID()}`;
    const { event } = await admitBotEvent(ctx, {
      bot: found.bot,
      universe: found.access.universe,
      eventId: replayId,
      document: {
        version: 1,
        kind: stored.kind,
        source: stored.source,
        occurredAt: stored.occurredAt.toISOString(),
        summary: `replay of ${stored.eventId}`,
      },
      ref: stored.ref,
      ...(stored.triggerId === null ? {} : { triggerId: stored.triggerId }),
      ...(stored.session === null ? {} : { session: stored.session }),
    });
    await recordActivity(ctx, found.bot.id, "replayed", {
      eventId: replayId,
      detail: `replay of ${stored.eventId}`,
    });
    return c.json({ event, original: stored.eventId }, 202);
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

function scheduleSpec(bot: BotRow, trigger: BotTriggerRow, universeId: string): BotScheduleSpec {
  const spec = trigger.spec as { cron: string; timezone: string };
  return {
    universeId,
    botId: bot.id,
    botName: bot.name,
    triggerId: trigger.id,
    triggerName: trigger.name,
    cron: spec.cron,
    timezone: spec.timezone,
    paused: !(bot.enabled && trigger.enabled),
  };
}
