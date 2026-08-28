/// Deployment-scoped administration: operator environment providers and
/// their per-universe bindings, channel accounts, and connector health.
import { Hono } from "hono";
import type { ChannelAccount, ChannelConnectorStatus, EnvironmentProviderBinding, EnvironmentTemplate } from "@/api";
import type { OperatorEnvironmentProviderView } from "@lightspeed/agent-client";
import type { DemoStore, UniverseState } from "../store";
import { badRequest, conflict, notFound, nowIso, readBody } from "./common";

type ControllerConnection = OperatorEnvironmentProviderView["controllerConnection"];

/// What a fresh binding provisions from when no other universe already
/// lists this provider's templates.
const DEFAULT_TEMPLATES: Array<Omit<EnvironmentTemplate, "providerId" | "bindingId">> = [
  {
    templateId: "dev-small-v1",
    displayName: "Development VM (small)",
    description: "2 vCPU / 4 GiB, Git, Docker, common toolchains, envd.",
    publicIngress: true,
    deprecated: false,
    metadata: { cpu: "2", memory: "4GiB", disk: "40GiB" },
  },
  {
    templateId: "dev-large-v1",
    displayName: "Development VM (large)",
    description: "8 vCPU / 16 GiB, same image as small.",
    publicIngress: true,
    deprecated: false,
    metadata: { cpu: "8", memory: "16GiB", disk: "120GiB" },
  },
];

const CONNECTOR_READY_DELAY_MS = 3_000;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function stringMetadata(value: unknown): Record<string, string> {
  const metadata: Record<string, string> = {};
  if (!isRecord(value)) return metadata;
  for (const [key, entry] of Object.entries(value)) {
    if (typeof entry === "string") metadata[key] = entry;
  }
  return metadata;
}

function parseConnection(value: unknown): ControllerConnection | null {
  if (!isRecord(value)) return null;
  const endpoint = optionalString(value.endpoint);
  const transport = isRecord(value.transport) ? value.transport : null;
  if (!endpoint || !transport) return null;
  switch (transport.type) {
    case "provider": {
      const providerType = optionalString(transport.providerType);
      return providerType ? { endpoint, transport: { type: "provider", providerType } } : null;
    }
    case "webSocket":
    case "http":
    case "stdio":
    case "ssh":
      return { endpoint, transport: { type: transport.type } };
    default:
      return null;
  }
}

/// The admin page addresses universes by their engine id; the platform id
/// resolves too so either spelling works.
function universeByAnyId(store: DemoStore, id: string): UniverseState | null {
  const byPlatformId = store.universe(id);
  if (byPlatformId) return byPlatformId;
  for (const state of store.universes.values()) {
    if (state.universe.lightspeedUniverseId === id) return state;
  }
  return null;
}

/// The engine asks the provider for templates per binding; the demo copies
/// what the provider offers elsewhere (or the default set) so a newly bound
/// universe can provision right away.
function seedTemplates(store: DemoStore, universe: UniverseState, binding: EnvironmentProviderBinding): void {
  let source: Array<Omit<EnvironmentTemplate, "providerId" | "bindingId">> = [];
  for (const other of store.universes.values()) {
    source = other.environmentTemplates.filter((template) => template.providerId === binding.providerId);
    if (source.length > 0) break;
  }
  for (const template of source.length > 0 ? source : DEFAULT_TEMPLATES) {
    universe.environmentTemplates.push({
      ...template,
      providerId: binding.providerId,
      bindingId: binding.bindingId,
    });
  }
}

function parseSettings(value: unknown): ChannelAccount["settings"] {
  return isRecord(value) && typeof value.printQr === "boolean" ? { printQr: value.printQr } : {};
}

function connectorFor(store: DemoStore, account: ChannelAccount): ChannelConnectorStatus | undefined {
  return store.channelsStatus.connectors.find(
    (connector) =>
      connector.health?.provider === account.provider && connector.health.accountId === account.accountId,
  );
}

function setConnectorState(
  store: DemoStore,
  account: ChannelAccount,
  state: NonNullable<ChannelConnectorStatus["health"]>["state"],
): void {
  const connector = connectorFor(store, account);
  if (!connector?.health) return;
  connector.health.state = state;
  connector.health.ingressConnected = state === "ready";
  connector.health.changedAtMs = Date.now();
}

/// A new account gets a connector that comes up after a beat, so the
/// status column moves like a real deployment's.
function addConnector(store: DemoStore, account: ChannelAccount): void {
  store.channelsStatus.connectors.push({
    url: `http://channels-${account.provider}-${store.channelsStatus.connectors.length + 1}.internal:9100/health`,
    reachable: true,
    httpStatus: 200,
    health: {
      version: 1,
      provider: account.provider,
      accountId: account.accountId,
      state: account.enabled ? "starting" : "stopped",
      ingressConnected: false,
      activityWorkerReady: true,
      reconnectAttempts: 0,
      changedAtMs: Date.now(),
    },
  });
  if (account.enabled) {
    setTimeout(() => {
      if (store.channelAccounts.get(account.id)?.enabled) setConnectorState(store, account, "ready");
    }, CONNECTOR_READY_DELAY_MS);
  }
}

export function adminRoutes(store: DemoStore): Hono {
  const app = new Hono();

  app.get("/admin/environment-providers", (c) => c.json([...store.environmentProviders.values()]));

  app.put("/admin/environment-providers/:providerId", async (c) => {
    const providerId = c.req.param("providerId").trim();
    if (!providerId) return badRequest(c, "providerId is required");
    const body = await readBody<{ displayName?: unknown; metadata?: unknown; controllerConnection?: unknown }>(c);
    const controllerConnection = parseConnection(body.controllerConnection);
    if (!controllerConnection) {
      return badRequest(c, "controllerConnection needs an endpoint and a transport");
    }
    const existing = store.environmentProviders.get(providerId);
    const now = Date.now();
    const provider: OperatorEnvironmentProviderView = {
      providerId,
      ...(optionalString(body.displayName) ? { displayName: optionalString(body.displayName) } : {}),
      controllerConnection,
      metadata: stringMetadata(body.metadata),
      createdAtMs: existing?.createdAtMs ?? now,
      updatedAtMs: now,
    };
    store.environmentProviders.set(providerId, provider);
    return c.json(provider);
  });

  app.delete("/admin/environment-providers/:providerId", (c) => {
    const providerId = c.req.param("providerId");
    const provider = store.environmentProviders.get(providerId);
    if (!provider) return notFound(c);
    for (const universe of store.universes.values()) {
      if (universe.providerBindings.some((binding) => binding.providerId === providerId)) {
        return conflict(c, "environment provider is referenced by a universe binding");
      }
    }
    store.environmentProviders.delete(providerId);
    return c.json(provider);
  });

  /// Every platform universe with the bindings the engine reports for it.
  app.get("/admin/environment-provider-bindings", (c) =>
    c.json(
      [...store.universes.values()].map((state) => ({
        universeId: state.universe.id,
        lightspeedUniverseId: state.universe.lightspeedUniverseId,
        name: state.universe.name,
        status: state.universe.status,
        bindings: state.providerBindings,
        error: null,
      })),
    ),
  );

  app.put("/admin/universes/:universeId/environment-provider-bindings/:bindingId", async (c) => {
    const universe = universeByAnyId(store, c.req.param("universeId"));
    if (!universe) return notFound(c);
    const body = await readBody<{
      providerId?: unknown;
      status?: unknown;
      expectedRevision?: unknown;
      metadata?: unknown;
    }>(c);
    const providerId = optionalString(body.providerId);
    if (!providerId) return badRequest(c, "providerId is required");
    if (body.status !== "enabled" && body.status !== "disabled") {
      return badRequest(c, "status must be enabled or disabled");
    }
    if (!store.environmentProviders.has(providerId)) {
      return notFound(c, `environment provider not found: ${providerId}`);
    }
    const bindingId = c.req.param("bindingId");
    const index = universe.providerBindings.findIndex((binding) => binding.bindingId === bindingId);
    const existing = universe.providerBindings[index];
    if (existing && typeof body.expectedRevision === "number" && existing.revision !== body.expectedRevision) {
      return conflict(c, "binding revision conflict");
    }
    const now = Date.now();
    const binding: EnvironmentProviderBinding = {
      bindingId,
      providerId,
      status: body.status,
      revision: (existing?.revision ?? 0) + 1,
      metadata: stringMetadata(body.metadata),
      createdAtMs: existing?.createdAtMs ?? now,
      updatedAtMs: now,
    };
    if (existing) {
      universe.providerBindings[index] = binding;
    } else {
      universe.providerBindings.push(binding);
      seedTemplates(store, universe, binding);
    }
    return c.json(binding);
  });

  app.delete("/admin/universes/:universeId/environment-provider-bindings/:bindingId", (c) => {
    const universe = universeByAnyId(store, c.req.param("universeId"));
    if (!universe) return notFound(c);
    const bindingId = c.req.param("bindingId");
    const index = universe.providerBindings.findIndex((binding) => binding.bindingId === bindingId);
    if (index < 0) return notFound(c);
    const [binding] = universe.providerBindings.splice(index, 1);
    universe.environmentTemplates = universe.environmentTemplates.filter(
      (template) => template.bindingId !== bindingId,
    );
    return c.json(binding);
  });

  app.get("/channel-accounts", (c) => c.json([...store.channelAccounts.values()]));

  app.post("/channel-accounts", async (c) => {
    const body = await readBody<{
      provider?: unknown;
      accountId?: unknown;
      displayName?: unknown;
      settings?: unknown;
      enabled?: unknown;
    }>(c);
    if (body.provider !== "telegram" && body.provider !== "whatsapp") {
      return badRequest(c, "provider must be telegram or whatsapp");
    }
    const accountId = optionalString(body.accountId);
    const displayName = optionalString(body.displayName);
    if (!accountId || !displayName) return badRequest(c, "accountId and displayName are required");
    const at = nowIso();
    const account: ChannelAccount = {
      id: store.nextId("chan"),
      provider: body.provider,
      accountId,
      displayName,
      settings: parseSettings(body.settings),
      enabled: body.enabled !== false,
      createdAt: at,
      updatedAt: at,
    };
    store.channelAccounts.set(account.id, account);
    addConnector(store, account);
    return c.json(account, 201);
  });

  app.patch("/channel-accounts/:id", async (c) => {
    const account = store.channelAccounts.get(c.req.param("id"));
    if (!account) return notFound(c);
    const body = await readBody<{ displayName?: unknown; settings?: unknown; enabled?: unknown }>(c);
    const displayName = optionalString(body.displayName);
    if (displayName) account.displayName = displayName;
    if (body.settings !== undefined) account.settings = parseSettings(body.settings);
    if (typeof body.enabled === "boolean" && body.enabled !== account.enabled) {
      account.enabled = body.enabled;
      setConnectorState(store, account, body.enabled ? "ready" : "stopped");
    }
    account.updatedAt = nowIso();
    return c.json(account);
  });

  app.delete("/channel-accounts/:id", (c) => {
    const account = store.channelAccounts.get(c.req.param("id"));
    if (!account) return notFound(c);
    store.channelAccounts.delete(account.id);
    const connector = connectorFor(store, account);
    if (connector) {
      store.channelsStatus.connectors.splice(store.channelsStatus.connectors.indexOf(connector), 1);
    }
    return c.json({ ok: true });
  });

  app.get("/status/channels", (c) => c.json(store.channelsStatus));

  return app;
}
