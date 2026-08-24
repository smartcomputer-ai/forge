import { Hono } from "hono";
import { eq } from "drizzle-orm";
import { schema } from "@lightspeed/platform-db";
import type { BotWebhookTriggerSpec } from "@lightspeed/platform-db/schema";
import type { BotEventDocumentV1 } from "@lightspeed/bots/contracts";
import {
  computeRouteSession,
  evaluateFilter,
  extractWebhookEvent,
  verifyWebhook,
  type FilterContext,
} from "@lightspeed/bots/webhooks";
import type { AppContext } from "../context.js";
import {
  admitBotEvent,
  checkTriggerBreaker,
  errorMessage,
  recordActivity,
} from "./bot-common.js";

const MAX_BODY_BYTES = 1024 * 1024;

/**
 * Public webhook ingress: authenticated by the per-trigger URL token plus the
 * trigger's verification scheme, never by a platform session. Mounted outside
 * the session-auth middleware.
 */
export function botHookRoutes(ctx: AppContext) {
  const hooks = new Hono();

  hooks.post("/bots/:triggerId/:token", async (c) => {
    const [row] = await ctx.db
      .select({
        trigger: schema.botTriggers,
        bot: schema.bots,
        universe: schema.universes,
      })
      .from(schema.botTriggers)
      .innerJoin(schema.bots, eq(schema.botTriggers.botId, schema.bots.id))
      .innerJoin(schema.universes, eq(schema.bots.universeId, schema.universes.id))
      .where(eq(schema.botTriggers.id, c.req.param("triggerId")))
      .limit(1);
    if (!row || row.trigger.kind !== "webhook") return c.json({ error: "not found" }, 404);
    const spec = row.trigger.spec as BotWebhookTriggerSpec;

    const rawBody = Buffer.from(await c.req.arrayBuffer());
    if (rawBody.byteLength > MAX_BODY_BYTES) {
      return c.json({ error: "payload too large" }, 413);
    }
    const rawHeaders: Record<string, string> = {};
    c.req.raw.headers.forEach((value, name) => {
      rawHeaders[name] = value;
    });

    const verified = verifyWebhook(spec, c.req.param("token"), rawBody, rawHeaders);
    if (!verified.ok) {
      // Token mismatch is indistinguishable from an unknown endpoint;
      // signature failures on a known endpoint get an explicit 401.
      if (verified.reason === "unknown endpoint") return c.json({ error: "not found" }, 404);
      return c.json({ error: "verification failed", failure: verified.reason }, 401);
    }
    if (!row.bot.enabled || !row.trigger.enabled) {
      return c.json({ error: "trigger is disabled" }, 409);
    }
    const breaker = await checkTriggerBreaker(ctx, row.bot, row.trigger);
    if (breaker.tripped) {
      return c.json({ error: "trigger disabled by flood breaker" }, 429);
    }

    const extraction = extractWebhookEvent(
      { name: row.trigger.name, spec },
      rawBody,
      rawHeaders,
    );
    const occurredAt = new Date().toISOString();
    const source = `webhook:${row.trigger.name}`;
    const filterContext: FilterContext = {
      event: { id: extraction.eventId, kind: extraction.kind, source, occurredAt },
      data: extraction.data,
      headers: extraction.headers,
    };

    const document: BotEventDocumentV1 = {
      version: 1,
      kind: extraction.kind,
      source,
      occurredAt,
      summary: extraction.summary,
      ...(extraction.data === undefined ? {} : { data: extraction.data }),
      headers: extraction.headers,
    };

    try {
      if (row.trigger.filter !== null) {
        const filtered = evaluateFilter(row.trigger.filter, filterContext);
        if (!filtered.matched) {
          // Archive without delivering so the envelope stays replayable and
          // the activity feed can explain the skip.
          const { duplicate } = await admitBotEvent(ctx, {
            bot: row.bot,
            universe: row.universe,
            eventId: extraction.eventId,
            document,
            ...(extraction.promptData === undefined ? {} : { promptData: extraction.promptData }),
            triggerId: row.trigger.id,
            deliver: false,
          });
          if (!duplicate) {
            await recordActivity(ctx, row.bot.id, filtered.error ? "filter_error" : "filtered", {
              eventId: extraction.eventId,
              detail: filtered.error ?? `filter did not match: ${row.trigger.filter}`,
            });
          }
          return c.json({ eventId: extraction.eventId, filtered: true, duplicate }, 202);
        }
      }

      const routed = computeRouteSession(
        row.bot.name,
        row.trigger.route,
        spec.preset,
        extraction,
        filterContext,
      );
      if (routed.error) {
        await recordActivity(ctx, row.bot.id, "route_fallback", {
          eventId: extraction.eventId,
          detail: routed.error,
        });
      }
      const coalesce = row.trigger.coalesce;
      const { event, duplicate } = await admitBotEvent(ctx, {
        bot: row.bot,
        universe: row.universe,
        eventId: extraction.eventId,
        document,
        ...(extraction.promptData === undefined ? {} : { promptData: extraction.promptData }),
        triggerId: row.trigger.id,
        ...(routed.session === undefined ? {} : { session: routed.session }),
        ...(coalesce === null
          ? {}
          : {
              coalesce: {
                key: `${row.trigger.id}|${routed.session?.sessionId ?? "main"}`,
                ...coalesce,
              },
            }),
        ...(row.trigger.deliver === null ? {} : { whenBusy: row.trigger.deliver.whenBusy }),
      });
      return c.json({ eventId: event.id, duplicate }, 202);
    } catch (error) {
      return c.json({ error: "event admission failed", failure: errorMessage(error) }, 502);
    }
  });

  return hooks;
}
