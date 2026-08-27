import { Hono } from "hono";
import { eq } from "drizzle-orm";
import { schema } from "@lightspeed/platform-db";
import type { BotWebhookTriggerSpec } from "@lightspeed/platform-db/schema";
import type { BotEventDocumentV1 } from "@lightspeed/bots/contracts";
import { constantTimeEquals, extractWebhookEvent, verifyWebhook } from "@lightspeed/bots/webhooks";
import { admitTriggerEvent } from "@lightspeed/bots/admission";
import { GrantLeaseCache } from "@lightspeed/bots/credentials";
import type { AppContext } from "../context.js";
import { admissionDeps, checkTriggerBreaker, errorMessage } from "./bot-common.js";
import { engineClientFor } from "./gateway.js";

const MAX_BODY_BYTES = 1024 * 1024;
const webhookLeaseCache = new GrantLeaseCache();

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

    // Do not turn an untrusted path probe into a credential lease/audit event.
    if (!constantTimeEquals(spec.token, c.req.param("token"))) {
      return c.json({ error: "not found" }, 404);
    }

    const rawBody = Buffer.from(await c.req.arrayBuffer());
    if (rawBody.byteLength > MAX_BODY_BYTES) {
      return c.json({ error: "payload too large" }, 413);
    }
    const rawHeaders: Record<string, string> = {};
    c.req.raw.headers.forEach((value, name) => {
      rawHeaders[name] = value;
    });

    let signingSecret: string | undefined;
    if (spec.verification.scheme === "hmac-sha256") {
      try {
        signingSecret = await webhookLeaseCache.lease(
          engineClientFor(ctx, row.universe, "service_account:lightspeed-platform"),
          {
            cacheScope: row.universe.lightspeedUniverseId,
            grantId: spec.verification.grantId,
            ...(spec.verification.audience === undefined
              ? {}
              : { audience: spec.verification.audience }),
          },
        );
      } catch (error) {
        return c.json(
          { error: "webhook verification unavailable", failure: errorMessage(error) },
          503,
        );
      }
    }
    const verified = verifyWebhook(
      spec,
      c.req.param("token"),
      rawBody,
      rawHeaders,
      signingSecret,
    );
    if (!verified.ok) {
      // Token mismatch is indistinguishable from an unknown endpoint;
      // signature failures on a known endpoint get an explicit 401.
      if (verified.reason === "unknown endpoint") return c.json({ error: "not found" }, 404);
      return c.json({ error: "verification failed", failure: verified.reason }, 401);
    }
    // A closed bot is gone for good: tell the sender to stop, not to retry.
    if (row.bot.closedAt !== null) {
      return c.json({ error: "bot is closed" }, 410);
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
      // The shared trigger pipeline: filter (refuse on miss, nothing
      // stored), route, coalesce, delivery policy, store-then-wake.
      const admitted = await admitTriggerEvent(await admissionDeps(ctx, row.universe), {
        bot: row.bot,
        trigger: row.trigger,
        universeId: row.universe.lightspeedUniverseId,
        eventId: extraction.eventId,
        document,
        ...(extraction.promptData === undefined ? {} : { promptData: extraction.promptData }),
      });
      if (admitted.filtered) {
        return c.json(
          { eventId: extraction.eventId, filtered: true, ...(admitted.error === undefined ? {} : { filterError: admitted.error }) },
          202,
        );
      }
      return c.json({ eventId: admitted.event.id, duplicate: admitted.duplicate }, 202);
    } catch (error) {
      return c.json({ error: "event admission failed", failure: errorMessage(error) }, 502);
    }
  });

  return hooks;
}
