import { Hono } from "hono";
import { and, count, desc, eq, getTableColumns, lt, or } from "drizzle-orm";
import { z } from "zod";
import { schema } from "@lightspeed/platform-db";
import { BOT_STATE_QUERY, botWorkflowId, type BotEventDocumentV1 } from "@lightspeed/bots/contracts";
import {
  BotConfigError,
  botNameInput,
  breakerInput,
  canManageRole,
  createTrigger,
  deleteTrigger,
  reconcileBotSchedules,
  redactTriggerSecrets,
  triggerCreateInput,
  triggerUpdateInput,
  updateTrigger,
} from "@lightspeed/bots/config";
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
} from "./bot-common.js";

const { bots, botTriggers, botEvents, botActivity } = schema;

const botName = botNameInput;
const botCreateSchema = z.object({
  name: botName,
  profileId: z.string().trim().min(1),
  brief: z.string().trim().min(1).max(20_000).nullish(),
  runsPerDay: z.number().int().min(1).max(10_000).nullish(),
  breaker: breakerInput.nullish(),
  routedSessionTtlMs: z.number().int().min(60_000).max(8_640_000_000).nullish(),
});
const botUpdateSchema = z
  .object({
    profileId: z.string().trim().min(1).optional(),
    brief: z.string().trim().min(1).max(20_000).nullable().optional(),
    runsPerDay: z.number().int().min(1).max(10_000).nullable().optional(),
    breaker: breakerInput.nullable().optional(),
    routedSessionTtlMs: z.number().int().min(60_000).max(8_640_000_000).nullable().optional(),
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
const historyCursorSchema = z.object({
  at: z.string().datetime({ offset: true }),
  id: z.string().uuid(),
});
const DEFAULT_HISTORY_LIMIT = 50;
const MAX_HISTORY_LIMIT = 100;

export function botRoutes(ctx: AppContext) {
  const byUniverse = new Hono<{ Variables: ApiVariables }>();
  const byId = new Hono<{ Variables: ApiVariables }>();

  byUniverse.get("/:id/bots", async (c) => {
    const access = await universeForSession(ctx, c, c.req.param("id"), false);
    if (!access) return c.json({ error: "not found" }, 404);
    const rows = await ctx.db
      .select({
        ...getTableColumns(bots),
        triggerCount: count(botTriggers.id),
      })
      .from(bots)
      .leftJoin(botTriggers, eq(botTriggers.botId, bots.id))
      .where(eq(bots.universeId, access.universe.id))
      .groupBy(bots.id)
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
        routedSessionTtlMs: body.data.routedSessionTtlMs ?? null,
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
          routedSessionTtlMs: found.bot.routedSessionTtlMs,
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
    await reconcileBotSchedules({ db: ctx.db, temporal }, bot, universeId);
  }

  byId.get("/:id/triggers", async (c) => {
    const found = await botForSession(c, c.req.param("id"), false);
    if (!found) return c.json({ error: "not found" }, 404);
    const triggers = await ctx.db
      .select()
      .from(botTriggers)
      .where(eq(botTriggers.botId, found.bot.id))
      .orderBy(botTriggers.name);
    const manage = canManageRole(found.access.role);
    return c.json({ triggers: manage ? triggers : triggers.map(redactTriggerSecrets) });
  });

  byId.post("/:id/triggers", async (c) => {
    const found = await botForSession(c, c.req.param("id"), true);
    if (!found) return c.json({ error: "not found" }, 404);
    const body = await parseBody(c, triggerCreateInput);
    if (!body.ok) return body.response;
    try {
      const temporal = await getTemporal();
      const trigger = await createTrigger(
        { db: ctx.db, temporal },
        { bot: found.bot, universeId: found.access.universe.lightspeedUniverseId, input: body.data },
      );
      return c.json({ trigger }, 201);
    } catch (error) {
      return configErrorResponse(c, error);
    }
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
    const body = await parseBody(c, triggerUpdateInput);
    if (!body.ok) return body.response;
    try {
      const temporal = await getTemporal();
      const trigger = await updateTrigger(
        { db: ctx.db, temporal },
        {
          bot: found.bot,
          universeId: found.access.universe.lightspeedUniverseId,
          existing,
          input: body.data,
        },
      );
      return c.json({ trigger });
    } catch (error) {
      return configErrorResponse(c, error);
    }
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
      await deleteTrigger(
        { db: ctx.db, temporal },
        { bot: found.bot, universeId: found.access.universe.lightspeedUniverseId, existing },
      );
      return c.json({ deleted: true });
    } catch (error) {
      return configErrorResponse(c, error);
    }
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
    // it never coalesces, so it delivers promptly and exactly once. The
    // original document is read back so the replay's rendering carries the
    // real payload, not a stub.
    const replayId = `replay-${crypto.randomUUID()}`;
    let document: BotEventDocumentV1 = {
      version: 1,
      kind: stored.kind,
      source: stored.source,
      occurredAt: stored.occurredAt.toISOString(),
      summary: `replay of ${stored.eventId}`,
    };
    try {
      const engine = engineClientFor(ctx, found.access.universe);
      const raw = await engine.call("blobs/read", { blobRef: stored.ref });
      document = JSON.parse(
        Buffer.from(raw.result.bytesBase64, "base64").toString("utf8"),
      ) as BotEventDocumentV1;
    } catch {
      // Unreadable document: fall back to the envelope stub above.
    }
    const { event } = await admitBotEvent(ctx, {
      bot: found.bot,
      universe: found.access.universe,
      eventId: replayId,
      document,
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
    const limit = historyLimit(c.req.query("limit"));
    const cursor = decodeHistoryCursor(c.req.query("cursor"));
    if (cursor === undefined) return c.json({ error: "invalid cursor" }, 400);
    const rows = await ctx.db
      .select()
      .from(botEvents)
      .where(
        cursor === null
          ? eq(botEvents.botId, found.bot.id)
          : and(
              eq(botEvents.botId, found.bot.id),
              or(
                lt(botEvents.receivedAt, cursor.at),
                and(eq(botEvents.receivedAt, cursor.at), lt(botEvents.id, cursor.id)),
              ),
            ),
      )
      .orderBy(desc(botEvents.receivedAt), desc(botEvents.id))
      .limit(limit + 1);
    const events = rows.slice(0, limit);
    return c.json({
      events,
      nextCursor: rows.length > limit ? encodeHistoryCursor(events.at(-1)!.receivedAt, events.at(-1)!.id) : null,
    });
  });

  byId.get("/:id/activity", async (c) => {
    const found = await botForSession(c, c.req.param("id"), false);
    if (!found) return c.json({ error: "not found" }, 404);
    const eventId = c.req.query("eventId");
    const limit = historyLimit(c.req.query("limit"));
    const cursor = decodeHistoryCursor(c.req.query("cursor"));
    if (cursor === undefined) return c.json({ error: "invalid cursor" }, 400);
    const baseFilter = eventId === undefined
      ? eq(botActivity.botId, found.bot.id)
      : and(eq(botActivity.botId, found.bot.id), eq(botActivity.eventId, eventId));
    const rows = await ctx.db
      .select()
      .from(botActivity)
      .where(
        cursor === null
          ? baseFilter
          : and(
              baseFilter,
              or(
                lt(botActivity.createdAt, cursor.at),
                and(eq(botActivity.createdAt, cursor.at), lt(botActivity.id, cursor.id)),
              ),
            ),
      )
      .orderBy(desc(botActivity.createdAt), desc(botActivity.id))
      .limit(limit + 1);
    const activity = rows.slice(0, limit);
    return c.json({
      activity,
      nextCursor: rows.length > limit
        ? encodeHistoryCursor(activity.at(-1)!.createdAt, activity.at(-1)!.id)
        : null,
    });
  });

  return { byUniverse, byId };
}

export function historyLimit(raw: string | undefined): number {
  const value = Number(raw ?? DEFAULT_HISTORY_LIMIT);
  return Number.isFinite(value)
    ? Math.min(Math.max(1, Math.floor(value)), MAX_HISTORY_LIMIT)
    : DEFAULT_HISTORY_LIMIT;
}

export function encodeHistoryCursor(at: Date, id: string): string {
  return Buffer.from(JSON.stringify({ at: at.toISOString(), id }), "utf8").toString("base64url");
}

/** `undefined` means the caller supplied a malformed cursor. */
export function decodeHistoryCursor(value: string | undefined): { at: Date; id: string } | null | undefined {
  if (value === undefined || value === "") return null;
  try {
    const decoded: unknown = JSON.parse(Buffer.from(value, "base64url").toString("utf8"));
    const parsed = historyCursorSchema.safeParse(decoded);
    return parsed.success ? { at: new Date(parsed.data.at), id: parsed.data.id } : undefined;
  } catch {
    return undefined;
  }
}

function configErrorResponse(
  c: { json: (body: unknown, status: 400 | 404 | 409 | 502) => Response },
  error: unknown,
): Response {
  if (error instanceof BotConfigError) {
    return c.json(
      { error: error.message, ...(error.issues === undefined ? {} : { issues: error.issues }) },
      error.status,
    );
  }
  return c.json({ error: "bot configuration failed", failure: errorMessage(error) }, 502);
}
