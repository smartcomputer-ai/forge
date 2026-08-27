import { Hono, type Context } from "hono";
import { and, count, desc, eq, getTableColumns, inArray, isNull, lt, or } from "drizzle-orm";
import { z } from "zod";
import { LightspeedRpcError } from "@lightspeed/agent-client";
import { schema } from "@lightspeed/platform-db";
import {
  BOT_SESSION_ROTATE_SIGNAL,
  BOT_STATE_QUERY,
  botWorkflowId,
  type BotEventDocumentV1,
  type BotSessionRotateV1,
} from "@lightspeed/bots/contracts";
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
  webhookIngestPath,
} from "@lightspeed/bots/config";
import type { BotSnapshot } from "@lightspeed/bots/workflows";
import {
  GrantReferenceError,
  validateRetrievableGrant,
} from "@lightspeed/bots/credentials";
import type { AppContext, ApiVariables } from "../context.js";
import { parseBody } from "../http.js";
import { engineClientFor } from "./gateway.js";
import { universeForSession } from "./universes.js";
import {
  admitBotEvent,
  botStart,
  errorMessage,
  getTemporal,
  signalBotConfig,
  type BotRow,
  type BotTriggerRow,
} from "./bot-common.js";

const { bots, botTriggers, botEvents, channelAccounts } = schema;

/// A bot's authored id (`botId` on the wire, `bots.name` in the row) is
/// immutable and universe-unique, like a profile id; `displayName` is the
/// mutable label. The uuid row key never leaves the database.
const botIdInput = botNameInput;
const displayNameInput = z.string().trim().min(1).max(200);
const descriptionInput = z.string().trim().min(1).max(500);
const botCreateSchema = z.object({
  botId: botIdInput,
  displayName: displayNameInput.nullish(),
  description: descriptionInput.nullish(),
  profileId: z.string().trim().min(1),
  brief: z.string().trim().min(1).max(20_000).nullish(),
  runsPerDay: z.number().int().min(1).max(10_000).nullish(),
  breaker: breakerInput.nullish(),
  routedSessionTtlMs: z.number().int().min(60_000).max(8_640_000_000).nullish(),
  selfConfig: z.boolean().optional(),
  emit: z.boolean().optional(),
  /** Create the `inbox` trigger (kind bot) so other bots can address this one. */
  acceptsBotEvents: z.boolean().optional(),
});
const botUpdateSchema = z
  .object({
    displayName: displayNameInput.nullable().optional(),
    description: descriptionInput.nullable().optional(),
    profileId: z.string().trim().min(1).optional(),
    brief: z.string().trim().min(1).max(20_000).nullable().optional(),
    runsPerDay: z.number().int().min(1).max(10_000).nullable().optional(),
    breaker: breakerInput.nullable().optional(),
    routedSessionTtlMs: z.number().int().min(60_000).max(8_640_000_000).nullable().optional(),
    selfConfig: z.boolean().optional(),
    emit: z.boolean().optional(),
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
/** How long a close waits for the controller's teardown before answering `completed: false`. */
const BOT_CLOSE_WAIT_MS = 30_000;
const BOT_LIST_COLLATOR = new Intl.Collator("en", { sensitivity: "base", numeric: true });

type BotContext = Context<{ Variables: ApiVariables }>;

/** Wire shape of a bot: the authored id as `botId`; the row key stays inside. */
export function botView<T extends BotRow>(row: T) {
  const { id: _id, name, ...rest } = row;
  return { botId: name, ...rest };
}

/** Sort the navigation list by its visible label, never by the private row id. */
export function compareBotListItems(
  left: Pick<BotRow, "displayName" | "name">,
  right: Pick<BotRow, "displayName" | "name">,
): number {
  const byLabel = BOT_LIST_COLLATOR.compare(
    left.displayName ?? left.name,
    right.displayName ?? right.name,
  );
  return byLabel !== 0 ? byLabel : BOT_LIST_COLLATOR.compare(left.name, right.name);
}

/**
 * Wire shape of a trigger: addressed by name. The row key survives only
 * inside the webhook ingest path, which is a capability URL and opaque on
 * purpose.
 */
export function triggerView(
  trigger: BotTriggerRow,
  channelAccount?: { id: string; provider: string; accountId: string; displayName: string } | null,
) {
  const { id: _id, botId: _botId, ...rest } = trigger;
  return {
    ...rest,
    ...(trigger.kind === "webhook" ? { ingestPath: webhookIngestPath(trigger) } : {}),
    ...(trigger.kind === "chat" ? { channelAccount: channelAccount ?? null } : {}),
  };
}

/// Bot routes are universe-scoped and addressed by the authored id:
/// /universes/:id/bots/:botId/…, triggers by /triggers/:triggerName.
export function botRoutes(ctx: AppContext) {
  const byUniverse = new Hono<{ Variables: ApiVariables }>();

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
      .groupBy(bots.id);
    rows.sort(compareBotListItems);
    return c.json({ bots: rows.map(botView) });
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
        name: body.data.botId,
        displayName: body.data.displayName ?? null,
        description: body.data.description ?? null,
        profileId: body.data.profileId,
        brief: body.data.brief ?? null,
        runsPerDay: body.data.runsPerDay ?? null,
        breaker: body.data.breaker ?? null,
        routedSessionTtlMs: body.data.routedSessionTtlMs ?? null,
        selfConfig: body.data.selfConfig ?? false,
        emit: body.data.emit ?? false,
      })
      .onConflictDoNothing()
      .returning();
    if (!bot) return c.json({ error: "a bot with that id already exists" }, 409);

    try {
      await signalBotConfig(botStart(bot, access.universe.lightspeedUniverseId));
      if (body.data.acceptsBotEvents === true) {
        const temporal = await getTemporal();
        await createTrigger(triggerConfigDeps(access.universe, temporal), {
          bot,
          universeId: access.universe.lightspeedUniverseId,
          input: triggerCreateInput.parse({ name: "inbox", kind: "bot" }),
        });
      }
    } catch (error) {
      await ctx.db.delete(bots).where(eq(bots.id, bot.id));
      return c.json(
        { error: "failed to start the bot controller", failure: errorMessage(error) },
        502,
      );
    }
    return c.json({ bot: botView(bot) }, 201);
  });

  async function botForUniverse(c: BotContext, write: boolean) {
    const universeId = c.req.param("id") ?? "";
    const botId = c.req.param("botId") ?? "";
    const access = await universeForSession(ctx, c, universeId, write);
    if (!access) return null;
    const [bot] = await ctx.db
      .select()
      .from(bots)
      .where(and(eq(bots.universeId, access.universe.id), eq(bots.name, botId)))
      .limit(1);
    return bot ? { bot, access } : null;
  }

  /** Wire views with each chat trigger's account attached (provider, id, label). */
  async function triggerViews(triggers: BotTriggerRow[]) {
    const accountIds = [
      ...new Set(
        triggers
          .filter((trigger) => trigger.kind === "chat")
          .map((trigger) => (trigger.spec as { channelAccountId: string }).channelAccountId),
      ),
    ];
    const accounts =
      accountIds.length === 0
        ? []
        : await ctx.db
            .select({
              id: channelAccounts.id,
              provider: channelAccounts.provider,
              accountId: channelAccounts.accountId,
              displayName: channelAccounts.displayName,
            })
            .from(channelAccounts)
            .where(inArray(channelAccounts.id, accountIds));
    const byId = new Map(accounts.map((account) => [account.id, account]));
    return triggers.map((trigger) =>
      triggerView(
        trigger,
        trigger.kind === "chat"
          ? (byId.get((trigger.spec as { channelAccountId: string }).channelAccountId) ?? null)
          : undefined,
      ),
    );
  }

  async function triggerForBot(bot: BotRow, name: string): Promise<BotTriggerRow | null> {
    const [trigger] = await ctx.db
      .select()
      .from(botTriggers)
      .where(and(eq(botTriggers.botId, bot.id), eq(botTriggers.name, name)))
      .limit(1);
    return trigger ?? null;
  }

  byUniverse.get("/:id/bots/:botId", async (c) => {
    const found = await botForUniverse(c, false);
    if (!found) return c.json({ error: "not found" }, 404);
    return c.json({ bot: botView(found.bot) });
  });

  /**
   * Terminal close. The row is marked first so every later step is
   * retry-safe and admission refuses from here on; then triggers are
   * disabled (`bot_closed`, schedules paused), and the controller is told to
   * tear down — archive what is pending, force-close its sessions, record
   * the closed sessions — and complete. The route waits a bounded time for
   * that completion; `completed: false` means the teardown is still running
   * (or the controller is unreachable) and a repeated close re-signals it.
   * The bot's environment is untouched: it is a universe resource.
   */
  byUniverse.post("/:id/bots/:botId/close", async (c) => {
    const found = await botForUniverse(c, true);
    if (!found) return c.json({ error: "not found" }, 404);
    const result = await closeBot(found.bot, found.access.universe.lightspeedUniverseId);
    return c.json({ bot: botView(result.bot), completed: result.completed });
  });

  /**
   * Erase. An open bot is closed first (and its teardown awaited); then the
   * sessions the controller recorded are deleted from the core, every
   * schedule is dropped, and the row goes (triggers, events, and pairings
   * cascade), which frees the name. Environments are universe resources and
   * stay.
   */
  byUniverse.delete("/:id/bots/:botId", async (c) => {
    const found = await botForUniverse(c, true);
    if (!found) return c.json({ error: "not found" }, 404);
    const universeId = found.access.universe.lightspeedUniverseId;
    const closed = await closeBot(found.bot, universeId);
    if (!closed.completed) {
      return c.json(
        { error: "the bot's controller has not finished closing; retry the delete shortly" },
        409,
      );
    }
    const engine = engineClientFor(ctx, found.access.universe);
    let sessionsDeleted = 0;
    for (const sessionId of closed.bot.closedSessions ?? []) {
      try {
        await engine.call("session/delete", { sessionId });
        sessionsDeleted += 1;
      } catch (error) {
        // Never created (a rotation that was never ensured) or already gone.
        if (error instanceof LightspeedRpcError && error.kind === "not_found") continue;
        return c.json(
          { error: `failed to delete session ${sessionId}`, failure: errorMessage(error) },
          502,
        );
      }
    }
    const temporal = await getTemporal();
    const triggers = await ctx.db
      .select()
      .from(botTriggers)
      .where(eq(botTriggers.botId, found.bot.id));
    for (const trigger of triggers) {
      try {
        await deleteTrigger(triggerConfigDeps(found.access.universe, temporal), {
          bot: closed.bot,
          universeId,
          existing: trigger,
        });
      } catch (error) {
        return c.json(
          { error: `failed to delete trigger ${trigger.name}`, failure: errorMessage(error) },
          502,
        );
      }
    }
    await ctx.db.delete(bots).where(eq(bots.id, found.bot.id));
    return c.json({ deleted: true, sessionsDeleted });
  });

  async function closeBot(
    bot: BotRow,
    universeId: string,
  ): Promise<{ bot: BotRow; completed: boolean }> {
    let current = bot;
    if (current.closedAt === null) {
      const [marked] = await ctx.db
        .update(bots)
        .set({ closedAt: new Date(), enabled: false })
        .where(and(eq(bots.id, bot.id), isNull(bots.closedAt)))
        .returning();
      if (marked) current = marked;
      else {
        const [reread] = await ctx.db.select().from(bots).where(eq(bots.id, bot.id)).limit(1);
        if (reread) current = reread;
      }
    }
    await ctx.db
      .update(botTriggers)
      .set({ enabled: false, disabledReason: "bot_closed", disabledAt: new Date() })
      .where(and(eq(botTriggers.botId, current.id), eq(botTriggers.enabled, true)));
    await reconcileSchedules(current, universeId);
    // The config carries `closed`; signal-with-start makes this restart-safe:
    // a controller that already completed runs the teardown again and exits.
    await signalBotConfig(botStart(current, universeId));
    const temporal = await getTemporal();
    const handle = temporal.workflow.getHandle(botWorkflowId(universeId, current.name));
    const completed = await Promise.race([
      handle.result().then(
        () => true,
        () => false,
      ),
      new Promise<boolean>((resolve) => {
        setTimeout(() => resolve(false), BOT_CLOSE_WAIT_MS).unref();
      }),
    ]);
    const [reread] = await ctx.db.select().from(bots).where(eq(bots.id, current.id)).limit(1);
    return { bot: reread ?? current, completed };
  }

  byUniverse.patch("/:id/bots/:botId", async (c) => {
    const found = await botForUniverse(c, true);
    if (!found) return c.json({ error: "not found" }, 404);
    const body = await parseBody(c, botUpdateSchema);
    if (!body.ok) return body.response;
    if (found.bot.closedAt !== null) {
      // A closed bot is history: labels may change, nothing that would bring
      // it back or reconfigure a controller that no longer runs.
      const labelsOnly = Object.keys(body.data).every(
        (key) => key === "displayName" || key === "description",
      );
      if (!labelsOnly) return c.json({ error: "bot is closed" }, 409);
      const [bot] = await ctx.db.update(bots).set(body.data).where(eq(bots.id, found.bot.id)).returning();
      if (!bot) return c.json({ error: "not found" }, 404);
      return c.json({ bot: botView(bot) });
    }
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
          displayName: found.bot.displayName,
          description: found.bot.description,
          profileId: found.bot.profileId,
          brief: found.bot.brief,
          runsPerDay: found.bot.runsPerDay,
          breaker: found.bot.breaker,
          routedSessionTtlMs: found.bot.routedSessionTtlMs,
          selfConfig: found.bot.selfConfig,
          emit: found.bot.emit,
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
    return c.json({ bot: botView(bot) });
  });

  async function reconcileSchedules(bot: BotRow, universeId: string): Promise<void> {
    const temporal = await getTemporal();
    await reconcileBotSchedules({ db: ctx.db, temporal }, bot, universeId);
  }

  function triggerConfigDeps(universe: Parameters<typeof engineClientFor>[1], temporal: Awaited<ReturnType<typeof getTemporal>>) {
    return {
      db: ctx.db,
      temporal,
      validateGrant: async (grantId: string) => {
        try {
          await validateRetrievableGrant(engineClientFor(ctx, universe), grantId);
        } catch (error) {
          if (error instanceof GrantReferenceError) {
            throw new BotConfigError(error.message, 400);
          }
          throw new BotConfigError("could not validate the credential with Lightspeed", 502);
        }
      },
    };
  }

  byUniverse.get("/:id/bots/:botId/triggers", async (c) => {
    const found = await botForUniverse(c, false);
    if (!found) return c.json({ error: "not found" }, 404);
    const triggers = await ctx.db
      .select()
      .from(botTriggers)
      .where(eq(botTriggers.botId, found.bot.id))
      .orderBy(botTriggers.name);
    const manage = canManageRole(found.access.role);
    return c.json({
      triggers: await triggerViews(manage ? triggers : triggers.map(redactTriggerSecrets)),
    });
  });

  byUniverse.post("/:id/bots/:botId/triggers", async (c) => {
    const found = await botForUniverse(c, true);
    if (!found) return c.json({ error: "not found" }, 404);
    const body = await parseBody(c, triggerCreateInput);
    if (!body.ok) return body.response;
    try {
      const temporal = await getTemporal();
      const trigger = await createTrigger(
        triggerConfigDeps(found.access.universe, temporal),
        { bot: found.bot, universeId: found.access.universe.lightspeedUniverseId, input: body.data },
      );
      return c.json({ trigger: (await triggerViews([trigger]))[0] }, 201);
    } catch (error) {
      return configErrorResponse(c, error);
    }
  });

  byUniverse.patch("/:id/bots/:botId/triggers/:triggerName", async (c) => {
    const found = await botForUniverse(c, true);
    if (!found) return c.json({ error: "not found" }, 404);
    const existing = await triggerForBot(found.bot, c.req.param("triggerName") ?? "");
    if (!existing) return c.json({ error: "not found" }, 404);
    const body = await parseBody(c, triggerUpdateInput);
    if (!body.ok) return body.response;
    try {
      const temporal = await getTemporal();
      const trigger = await updateTrigger(
        triggerConfigDeps(found.access.universe, temporal),
        {
          bot: found.bot,
          universeId: found.access.universe.lightspeedUniverseId,
          existing,
          input: body.data,
        },
      );
      return c.json({ trigger: (await triggerViews([trigger]))[0] });
    } catch (error) {
      return configErrorResponse(c, error);
    }
  });

  byUniverse.delete("/:id/bots/:botId/triggers/:triggerName", async (c) => {
    const found = await botForUniverse(c, true);
    if (!found) return c.json({ error: "not found" }, 404);
    const existing = await triggerForBot(found.bot, c.req.param("triggerName") ?? "");
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

  byUniverse.get("/:id/bots/:botId/state", async (c) => {
    const found = await botForUniverse(c, false);
    if (!found) return c.json({ error: "not found" }, 404);
    const temporal = await getTemporal();
    const handle = temporal.workflow.getHandle(
      botWorkflowId(found.access.universe.lightspeedUniverseId, found.bot.name),
    );
    let state: BotSnapshot;
    try {
      state = await handle.query<BotSnapshot>(BOT_STATE_QUERY);
    } catch (error) {
      return c.json({ error: "bot controller unavailable", failure: errorMessage(error) }, 503);
    }
    // Sub-agent lineage (P134): the controller sees only its own sessions;
    // their delegated descendants are read from core by root. Best effort —
    // a core outage must not hide the controller state.
    const lineage = await botSessionLineage(engineClientFor(ctx, found.access.universe), state);
    return c.json({ state, lineage });
  });

  byUniverse.post("/:id/bots/:botId/sessions/:sessionId/rotate", async (c) => {
    const found = await botForUniverse(c, true);
    if (!found) return c.json({ error: "not found" }, 404);
    const sessionId = c.req.param("sessionId") ?? "";
    const temporal = await getTemporal();
    const handle = temporal.workflow.getHandle(
      botWorkflowId(found.access.universe.lightspeedUniverseId, found.bot.name),
    );
    let state: BotSnapshot;
    try {
      state = await handle.query<BotSnapshot>(BOT_STATE_QUERY);
    } catch (error) {
      return c.json({ error: "bot controller unavailable", failure: errorMessage(error) }, 503);
    }
    if (!state.sessions.some((session) => session.sessionId === sessionId)) {
      return c.json({ error: "session is not managed by this bot" }, 404);
    }
    const request: BotSessionRotateV1 = { version: 1, sessionId };
    try {
      await handle.signal(BOT_SESSION_ROTATE_SIGNAL, request);
    } catch (error) {
      return c.json({ error: "failed to request session rotation", failure: errorMessage(error) }, 502);
    }
    return c.json({ accepted: true, sessionId }, 202);
  });

  byUniverse.post("/:id/bots/:botId/events", async (c) => {
    const found = await botForUniverse(c, true);
    if (!found) return c.json({ error: "not found" }, 404);
    if (found.bot.closedAt !== null) return c.json({ error: "bot is closed" }, 410);
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

  byUniverse.post("/:id/bots/:botId/events/replay", async (c) => {
    const found = await botForUniverse(c, true);
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
    return c.json({ event, original: stored.eventId }, 202);
  });

  byUniverse.get("/:id/bots/:botId/events", async (c) => {
    const found = await botForUniverse(c, false);
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
    const last = events.at(-1);
    // Senders by authored id; the row key and the private return route stay inside.
    const senderIds = [...new Set(events.flatMap((event) => (event.senderBotId === null ? [] : [event.senderBotId])))];
    const senders = new Map(
      senderIds.length === 0
        ? []
        : (await ctx.db.select({ id: bots.id, name: bots.name }).from(bots).where(inArray(bots.id, senderIds))).map(
            (row) => [row.id, row.name] as const,
          ),
    );
    return c.json({
      events: events.map(({ botId: _botId, triggerId: _triggerId, replyTo: _replyTo, senderBotId, ...event }) => ({
        ...event,
        sender: senderBotId === null ? null : (senders.get(senderBotId) ?? null),
      })),
      nextCursor: rows.length > limit && last ? encodeHistoryCursor(last.receivedAt, last.id) : null,
    });
  });

  return { byUniverse };
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
  c: { json: (body: unknown, status: 400 | 403 | 404 | 409 | 429 | 502) => Response },
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

/// Descendant sessions per bot session, keyed by the bot session id: open and
/// lifetime counts plus a bounded child list for the UI.
export interface BotSessionLineage {
  open: number;
  total: number;
  children: Array<{
    id: string;
    displayName: string | null;
    lifecycleStatus: "new" | "open" | "closed";
    profileId: string | null;
    depth: number;
    updatedAtMs: number;
  }>;
}

async function botSessionLineage(
  engine: ReturnType<typeof engineClientFor>,
  state: BotSnapshot,
): Promise<Record<string, BotSessionLineage>> {
  const entries = await Promise.all(
    state.sessions.map(async (session) => {
      try {
        const response = await engine.call("session/list", {
          rootSessionId: session.sessionId,
          limit: 200,
        });
        const sessions = response.result.sessions ?? [];
        const lineage: BotSessionLineage = {
          open: sessions.filter((child) => child.lifecycleStatus !== "closed").length,
          total: sessions.length,
          children: sessions.slice(0, 50).map((child) => ({
            id: child.id,
            displayName: child.displayName ?? null,
            lifecycleStatus: child.lifecycleStatus,
            profileId: child.origin?.agent.profileId ?? null,
            depth: child.origin?.depth ?? 1,
            updatedAtMs: child.updatedAtMs,
          })),
        };
        return [session.sessionId, lineage] as const;
      } catch {
        return null;
      }
    }),
  );
  return Object.fromEntries(entries.filter((entry): entry is NonNullable<typeof entry> => entry !== null));
}
