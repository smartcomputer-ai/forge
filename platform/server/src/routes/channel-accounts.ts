import type {
  ChannelAccountCreateParams,
  ChannelAccountListParams,
  ChannelAccountPutParams,
  ChannelPairingListParams,
} from "@lightspeed/agent-client";
import { Hono } from "hono";
import type { AppContext, ApiVariables } from "../context.js";
import { isPlatformAdmin } from "../context.js";
import { engineClientFor, operatorClientFor, withGateway } from "./gateway.js";
import { universeForSession } from "./universes.js";

/// Channel accounts are universe resources in the core (`channels/*`,
/// P142): a provider account (a Telegram bot token, a WhatsApp number)
/// belongs to exactly one universe, whose owners/admins manage it. The
/// platform passes through with membership checks, exactly like the bot
/// routes.

async function jsonBody(c: {
  req: { json: () => Promise<unknown> };
}): Promise<Record<string, unknown> | null> {
  try {
    const body = await c.req.json();
    return typeof body === "object" && body !== null
      ? (body as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

/// Deployment-wide listing for the admin page and the connector-host
/// operator view: every enabled account across universes, from the
/// core's operator scope. Platform-admin only.
export function channelAccountAdminRoutes(ctx: AppContext) {
  const app = new Hono<{ Variables: ApiVariables }>();

  app.get("/", async (c) => {
    if (!isPlatformAdmin(c.get("session"))) {
      return c.json({ error: "platform admin required" }, 403);
    }
    return withGateway(c, async () => {
      const client = operatorClientFor(ctx);
      const response = await client.call("operator/channels/accounts/list", {
        includeDisabled: true,
      });
      return c.json(response.result);
    });
  });

  return app;
}

/// Universe-scoped account and pairing routes, mounted under
/// `/universes`.
export function channelUniverseRoutes(ctx: AppContext) {
  const app = new Hono<{ Variables: ApiVariables }>();

  app.get("/:id/channel-accounts", async (c) => {
    const access = await universeForSession(ctx, c, c.req.param("id"), false);
    if (!access) {
      return c.json({ error: "not found" }, 404);
    }
    return withGateway(c, async () => {
      const client = engineClientFor(ctx, access.universe);
      const response = await client.call("channels/accounts/list", {
        provider: c.req.query("provider"),
      } as unknown as ChannelAccountListParams);
      return c.json(response.result);
    });
  });

  app.post("/:id/channel-accounts", async (c) => {
    const access = await universeForSession(ctx, c, c.req.param("id"), true);
    if (!access) {
      return c.json({ error: "not found" }, 404);
    }
    const body = await jsonBody(c);
    const account = body?.account;
    if (!body || typeof account !== "object" || account === null) {
      return c.json({ error: "invalid body" }, 400);
    }
    return withGateway(c, async () => {
      const client = engineClientFor(ctx, access.universe);
      const response = await client.call("channels/accounts/create", {
        account,
      } as unknown as ChannelAccountCreateParams);
      return c.json(response.result, 201);
    });
  });

  app.get("/:id/channel-accounts/:accountId", async (c) => {
    const access = await universeForSession(ctx, c, c.req.param("id"), false);
    if (!access) {
      return c.json({ error: "not found" }, 404);
    }
    return withGateway(c, async () => {
      const client = engineClientFor(ctx, access.universe);
      const response = await client.call("channels/accounts/read", {
        accountId: c.req.param("accountId"),
      });
      return c.json(response.result);
    });
  });

  app.put("/:id/channel-accounts/:accountId", async (c) => {
    const access = await universeForSession(ctx, c, c.req.param("id"), true);
    if (!access) {
      return c.json({ error: "not found" }, 404);
    }
    const body = await jsonBody(c);
    const account = body?.account;
    if (!body || typeof account !== "object" || account === null) {
      return c.json({ error: "invalid body" }, 400);
    }
    return withGateway(c, async () => {
      const client = engineClientFor(ctx, access.universe);
      const response = await client.call("channels/accounts/put", {
        account: { ...account, accountId: c.req.param("accountId") },
        expectedRevision: body.expectedRevision,
      } as unknown as ChannelAccountPutParams);
      return c.json(response.result);
    });
  });

  app.delete("/:id/channel-accounts/:accountId", async (c) => {
    const access = await universeForSession(ctx, c, c.req.param("id"), true);
    if (!access) {
      return c.json({ error: "not found" }, 404);
    }
    return withGateway(c, async () => {
      const client = engineClientFor(ctx, access.universe);
      const response = await client.call("channels/accounts/delete", {
        accountId: c.req.param("accountId"),
      });
      return c.json(response.result);
    });
  });

  app.get("/:id/channel-pairings", async (c) => {
    const access = await universeForSession(ctx, c, c.req.param("id"), false);
    if (!access) {
      return c.json({ error: "not found" }, 404);
    }
    return withGateway(c, async () => {
      const client = engineClientFor(ctx, access.universe);
      const response = await client.call("channels/pairings/list", {
        accountId: c.req.query("accountId"),
        botId: c.req.query("botId"),
      } as unknown as ChannelPairingListParams);
      return c.json(response.result);
    });
  });

  app.delete("/:id/channel-pairings/:accountId/:chatId", async (c) => {
    const access = await universeForSession(ctx, c, c.req.param("id"), true);
    if (!access) {
      return c.json({ error: "not found" }, 404);
    }
    return withGateway(c, async () => {
      const client = engineClientFor(ctx, access.universe);
      const response = await client.call("channels/pairings/delete", {
        accountId: c.req.param("accountId"),
        chatId: c.req.param("chatId"),
      });
      return c.json(response.result);
    });
  });

  return app;
}
