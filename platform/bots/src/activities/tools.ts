import { randomUUID } from "node:crypto";
import { and, count, desc, eq, gte } from "drizzle-orm";
import { LightspeedClient } from "@lightspeed/agent-client";
import { schema, type Db } from "@lightspeed/platform-db";
import type { Client } from "@temporalio/client";
import {
  BotConfigError,
  createTrigger,
  deleteTrigger,
  findTriggerByName,
  redactTriggerSecrets,
  triggerCreateInput,
  triggerUpdateInput,
  updateTrigger,
  webhookIngestPath,
  type BotTriggerRow,
  type TriggerCreateInput,
  type TriggerUpdateInput,
} from "../config.js";
import {
  BOT_BRIEF_PUT_TOOL_ID,
  BOT_CONFIG_SIGNAL,
  BOT_CONTROLLER_WORKFLOW,
  BOT_EMIT_TOOL_ID,
  BOT_EVENT_LIST_TOOL_ID,
  BOT_EVENT_READ_TOOL_ID,
  BOT_FILTER_TEST_TOOL_ID,
  BOT_STATUS_TOOL_ID,
  BOT_TRIGGER_DELETE_TOOL_ID,
  BOT_TRIGGER_LIST_TOOL_ID,
  BOT_TRIGGER_PUT_TOOL_ID,
  BOTS_WORKFLOW_TASK_QUEUE,
  botKeyedSessionId,
  botWorkflowId,
  type BotEvent,
  type BotEventDocumentV1,
  type BotStartV1,
} from "../contracts/bots.js";
import { allocateBotEventSeq, renderAdmittedEvent, wakeBotController } from "../events.js";
import {
  DEFAULT_READ_BUDGET,
  largestBranches,
  renderValue,
  resolvePath,
} from "../rendering.js";
import { evaluateFilter, type FilterContext } from "../webhooks.js";

export interface BotToolActivitiesConfig {
  db: Db;
  endpoint: string;
  temporal: Client;
  /** Public origin of the platform, for ingest URLs handed to the bot. */
  baseUrl?: string | null;
  fetch?: typeof fetch;
}

/** Controller-side state the activity cannot read from the database. */
export interface BotControllerSummary {
  sessions: { sessionId: string; label: string; kind: string }[];
  activeDeliveries: { id: string; eventCount: number; sessionId: string }[];
  buffers: { key: string; count: number; flushAtMs: number }[];
  runsToday: number;
  eventsProcessed: number;
}

export interface ExecuteBotToolInput {
  universeId: string;
  botId: string;
  botName: string;
  sessionId: string;
  toolId: string;
  args: unknown;
  controller: BotControllerSummary;
}

export type ExecuteBotToolResult =
  | { ok: true; payloadRef: string | null }
  | { ok: false; message: string; errorRef: string | null };

export interface BotToolActivities {
  executeBotTool(input: ExecuteBotToolInput): Promise<ExecuteBotToolResult>;
}

const MAX_SAMPLE_EVENTS = 50;

export function createBotToolActivities(config: BotToolActivitiesConfig): BotToolActivities {
  const clientFor = (universeId: string) =>
    new LightspeedClient({
      endpoint: config.endpoint,
      ...(config.fetch === undefined ? {} : { fetch: config.fetch }),
      headers: { "x-lightspeed-universe": universeId },
    });

  async function putJson(universeId: string, value: unknown): Promise<string> {
    const stored = await clientFor(universeId).call("blobs/put", {
      blobs: [{ bytesBase64: Buffer.from(JSON.stringify(value), "utf8").toString("base64") }],
    });
    const ref = stored.result.blobs?.[0]?.blobRef;
    if (!ref) throw new Error("blobs/put returned no ref");
    return ref;
  }

  async function putText(universeId: string, value: string): Promise<string> {
    const stored = await clientFor(universeId).call("blobs/put", {
      blobs: [{ bytesBase64: Buffer.from(value, "utf8").toString("base64") }],
    });
    const ref = stored.result.blobs?.[0]?.blobRef;
    if (!ref) throw new Error("blobs/put returned no ref");
    return ref;
  }

  async function readJson(universeId: string, blobRef: string): Promise<unknown> {
    const response = await clientFor(universeId).call("blobs/read", { blobRef });
    return JSON.parse(Buffer.from(response.result.bytesBase64, "base64").toString("utf8")) as unknown;
  }

  async function recordSelfConfig(botId: string, detail: string, eventId?: string): Promise<void> {
    await config.db.insert(schema.botActivity).values({
      botId,
      kind: "self_configured",
      eventId: eventId ?? null,
      runId: null,
      detail,
    });
  }

  function ingestUrl(trigger: BotTriggerRow): string {
    const path = webhookIngestPath(trigger);
    return config.baseUrl ? `${config.baseUrl.replace(/\/$/, "")}${path}` : path;
  }

  function triggerView(trigger: BotTriggerRow) {
    const redacted = redactTriggerSecrets(trigger);
    return {
      id: trigger.id,
      name: trigger.name,
      kind: trigger.kind,
      spec: redacted.spec,
      filter: trigger.filter,
      route: trigger.route,
      coalesce: trigger.coalesce,
      deliver: trigger.deliver,
      enabled: trigger.enabled,
      ...(trigger.kind === "webhook" ? { ingestUrl: ingestUrl(trigger) } : {}),
    };
  }

  async function loadBot(botId: string) {
    const [row] = await config.db
      .select({ bot: schema.bots, lightspeedUniverseId: schema.universes.lightspeedUniverseId })
      .from(schema.bots)
      .innerJoin(schema.universes, eq(schema.bots.universeId, schema.universes.id))
      .where(eq(schema.bots.id, botId))
      .limit(1);
    if (!row) throw new BotConfigError("bot not found", 404);
    return row;
  }

  async function recentEnvelopes(universeId: string, botId: string, limit: number) {
    const rows = await config.db
      .select()
      .from(schema.botEvents)
      .where(eq(schema.botEvents.botId, botId))
      .orderBy(desc(schema.botEvents.receivedAt))
      .limit(Math.min(Math.max(limit, 1), MAX_SAMPLE_EVENTS));
    const results = [];
    for (const row of rows) {
      let document: BotEventDocumentV1 | null = null;
      try {
        document = (await readJson(universeId, row.ref)) as BotEventDocumentV1;
      } catch {
        document = null;
      }
      results.push({ row, document });
    }
    return results;
  }

  async function execute(input: ExecuteBotToolInput): Promise<unknown> {
    const { bot, lightspeedUniverseId } = await loadBot(input.botId);
    const deps = { db: config.db, temporal: config.temporal };
    const args = (input.args ?? {}) as Record<string, unknown>;

    // Defense in depth: the gated tools are not declared to sessions of a
    // bot without the grant, but the fresh row is authoritative — a stale
    // pre-toggle session must not mutate configuration either.
    if (
      !bot.selfConfig &&
      (input.toolId === BOT_TRIGGER_PUT_TOOL_ID ||
        input.toolId === BOT_TRIGGER_DELETE_TOOL_ID ||
        input.toolId === BOT_BRIEF_PUT_TOOL_ID)
    ) {
      throw new BotConfigError(
        "self-configuration is disabled for this bot; an operator can enable it in the bot's settings",
        403,
      );
    }
    if (!bot.selfEmit && input.toolId === BOT_EMIT_TOOL_ID) {
      throw new BotConfigError(
        "self-emitted events are disabled for this bot; an operator can enable them in the bot's settings",
        403,
      );
    }
    switch (input.toolId) {
      case BOT_STATUS_TOOL_ID: {
        return {
          bot: {
            name: bot.name,
            enabled: bot.enabled,
            profileId: bot.profileId,
            brief: bot.brief,
            runsPerDay: bot.runsPerDay,
            runsToday: input.controller.runsToday,
            breaker: bot.breaker,
            routedSessionTtlMs: bot.routedSessionTtlMs,
            selfConfig: bot.selfConfig,
            selfEmit: bot.selfEmit,
            eventsProcessed: input.controller.eventsProcessed,
          },
          sessions: input.controller.sessions,
          activeDeliveries: input.controller.activeDeliveries,
          buffers: input.controller.buffers,
        };
      }
      case BOT_TRIGGER_LIST_TOOL_ID: {
        const triggers = await config.db
          .select()
          .from(schema.botTriggers)
          .where(eq(schema.botTriggers.botId, bot.id))
          .orderBy(schema.botTriggers.name);
        return { triggers: triggers.map(triggerView) };
      }
      case BOT_TRIGGER_PUT_TOOL_ID: {
        const flat = parseTriggerPutArgs(args);
        const existing = await findTriggerByName(config.db, bot.id, flat.name);
        let trigger: BotTriggerRow;
        if (existing === undefined) {
          const parsed = triggerCreateInput.safeParse(flat.create);
          if (!parsed.success) throw new BotConfigError("validation failed", 400, parsed.error.issues);
          trigger = await createTrigger(deps, {
            bot,
            universeId: lightspeedUniverseId,
            input: parsed.data,
          });
          await recordSelfConfig(bot.id, `created ${trigger.kind} trigger ${trigger.name}`);
        } else {
          if (existing.kind !== flat.create.kind) {
            throw new BotConfigError(
              `trigger ${flat.name} is a ${existing.kind}; delete it before changing its kind`,
              409,
            );
          }
          const parsed = triggerUpdateInput.safeParse(flat.update);
          if (!parsed.success) throw new BotConfigError("validation failed", 400, parsed.error.issues);
          trigger = await updateTrigger(deps, {
            bot,
            universeId: lightspeedUniverseId,
            existing,
            input: parsed.data,
          });
          await recordSelfConfig(bot.id, `updated ${trigger.kind} trigger ${trigger.name}`);
        }
        return { trigger: triggerView(trigger), created: existing === undefined };
      }
      case BOT_TRIGGER_DELETE_TOOL_ID: {
        const name = requireString(args.name, "name");
        const existing = await findTriggerByName(config.db, bot.id, name);
        if (existing === undefined) throw new BotConfigError(`no trigger named ${name}`, 404);
        await deleteTrigger(deps, { bot, universeId: lightspeedUniverseId, existing });
        await recordSelfConfig(bot.id, `deleted ${existing.kind} trigger ${existing.name}`);
        return { deleted: true, name };
      }
      case BOT_FILTER_TEST_TOOL_ID: {
        const filter = requireString(args.filter, "filter");
        const limit = typeof args.limit === "number" ? args.limit : 20;
        const samples = await recentEnvelopes(lightspeedUniverseId, bot.id, limit);
        const results = samples.map(({ row, document }) => {
          const context: FilterContext = {
            event: {
              id: row.eventId,
              kind: row.kind,
              source: row.source,
              occurredAt: row.occurredAt.toISOString(),
            },
            data: document?.data,
            headers: document?.headers ?? {},
          };
          const outcome = evaluateFilter(filter, context);
          return {
            ...(row.seq === null ? {} : { seq: row.seq }),
            eventId: row.eventId,
            kind: row.kind,
            source: row.source,
            summary: document?.summary ?? null,
            matched: outcome.matched,
            ...(outcome.error === undefined ? {} : { error: outcome.error }),
          };
        });
        return {
          filter,
          sampled: results.length,
          matched: results.filter((result) => result.matched).length,
          errors: results.filter((result) => result.error !== undefined).length,
          results,
        };
      }
      case BOT_EVENT_LIST_TOOL_ID: {
        const limit = typeof args.limit === "number" ? args.limit : 20;
        const samples = await recentEnvelopes(lightspeedUniverseId, bot.id, limit);
        return {
          events: samples.map(({ row, document }) => ({
            ...(row.seq === null ? {} : { seq: row.seq }),
            eventId: row.eventId,
            kind: row.kind,
            source: row.source,
            occurredAt: row.occurredAt.toISOString(),
            receivedAt: row.receivedAt.toISOString(),
            session: row.session,
            summary: document?.summary ?? null,
          })),
        };
      }
      case BOT_EVENT_READ_TOOL_ID: {
        const seq = args.seq;
        if (!Number.isSafeInteger(seq) || (seq as number) < 1) {
          throw new BotConfigError("seq must be a positive integer (the event's #N)", 400);
        }
        const [row] = await config.db
          .select()
          .from(schema.botEvents)
          .where(and(eq(schema.botEvents.botId, bot.id), eq(schema.botEvents.seq, seq as number)))
          .limit(1);
        if (!row) {
          throw new BotConfigError(
            bot.eventSeq > 0
              ? `no event #${seq}; this bot's events run #1..#${bot.eventSeq}`
              : `no event #${seq}; this bot has no events yet`,
            404,
          );
        }
        let document: BotEventDocumentV1 | null = null;
        try {
          document = (await readJson(lightspeedUniverseId, row.ref)) as BotEventDocumentV1;
        } catch {
          document = null;
        }
        if (document === null) {
          throw new BotConfigError(`the stored document for event #${seq} could not be read`, 502);
        }
        const envelope: Record<string, unknown> = {
          seq: row.seq,
          eventId: row.eventId,
          kind: row.kind,
          source: row.source,
          occurredAt: row.occurredAt.toISOString(),
          receivedAt: row.receivedAt.toISOString(),
          ...(row.session === null ? {} : { session: row.session }),
          summary: document.summary,
          ...(document.correlationId == null ? {} : { correlationId: document.correlationId }),
          ...(document.links === undefined ? {} : { links: document.links }),
          ...(document.data === undefined ? {} : { data: document.data }),
          ...(document.headers === undefined ? {} : { headers: document.headers }),
        };
        const path = typeof args.path === "string" && args.path.length > 0 ? args.path : null;
        const target = path === null ? envelope : resolvePath(envelope, path);
        if (target === undefined) {
          throw new BotConfigError(
            `path "${path}" not found in event #${seq}; top-level keys: ${Object.keys(envelope).join(", ")}`,
            400,
          );
        }
        const requested =
          typeof args.maxBytes === "number" && Number.isSafeInteger(args.maxBytes)
            ? args.maxBytes
            : DEFAULT_READ_BUDGET;
        const maxBytes = Math.min(Math.max(requested, 256), 65_536);
        const json = JSON.stringify(target);
        if (json.length <= maxBytes) {
          return { seq: row.seq, ...(path === null ? {} : { path }), value: target };
        }
        // Never a silent cut: report the size, a pruned preview, and the
        // largest branches so the narrowing follow-up call is obvious.
        return {
          seq: row.seq,
          ...(path === null ? {} : { path }),
          truncated: true,
          bytes: json.length,
          preview: renderValue(target, { maxBytes }).text,
          largest: largestBranches(target).map((branch) => ({
            ...branch,
            path: path === null ? branch.path : `${path}.${branch.path}`,
          })),
          hint: "narrow with path or raise maxBytes (max 65536)",
        };
      }
      case BOT_BRIEF_PUT_TOOL_ID: {
        const brief = requireString(args.brief, "brief").trim();
        if (brief.length > 20_000) throw new BotConfigError("brief is too long", 400);
        const [updated] = await config.db
          .update(schema.bots)
          .set({ brief })
          .where(eq(schema.bots.id, bot.id))
          .returning();
        if (!updated) throw new BotConfigError("bot not found", 404);
        const start: BotStartV1 = {
          version: 1,
          universeId: lightspeedUniverseId,
          botId: updated.id,
          botName: updated.name,
          profileId: updated.profileId,
          brief: updated.brief,
          runsPerDay: updated.runsPerDay,
          routedSessionTtlMs: updated.routedSessionTtlMs,
          enabled: updated.enabled,
        };
        await config.temporal.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
          workflowId: botWorkflowId(start.universeId, start.botName),
          taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
          args: [start],
          signal: BOT_CONFIG_SIGNAL,
          signalArgs: [start],
        });
        await recordSelfConfig(bot.id, "rewrote its brief");
        return { brief, appliesAt: "next idle boundary" };
      }
      case BOT_EMIT_TOOL_ID: {
        const kind = requireString(args.kind, "kind");
        const summary = requireString(args.summary, "summary");
        // Loop breaker: even a granted bot cannot feed itself unbounded
        // events. The bot's breaker rate applies when set; otherwise a
        // fixed ceiling. The refusal is a tool error the model reads.
        const cap = bot.breaker ?? { fires: 60, windowMs: 60 * 60 * 1000 };
        const since = new Date(Date.now() - cap.windowMs);
        const [recentSelf] = await config.db
          .select({ value: count() })
          .from(schema.botEvents)
          .where(
            and(
              eq(schema.botEvents.botId, bot.id),
              eq(schema.botEvents.source, "bot:self"),
              gte(schema.botEvents.receivedAt, since),
            ),
          );
        if (Number(recentSelf?.value ?? 0) >= cap.fires) {
          throw new BotConfigError(
            `self-emission rate exceeded (${cap.fires} events in ${Math.round(cap.windowMs / 1000)}s); wait before emitting again`,
            429,
          );
        }
        const sessionKey = typeof args.sessionKey === "string" && args.sessionKey ? args.sessionKey : null;
        const document: BotEventDocumentV1 = {
          version: 1,
          kind,
          source: "bot:self",
          occurredAt: new Date().toISOString(),
          summary,
          ...(args.data === undefined || args.data === null ? {} : { data: args.data }),
        };
        const seq = await allocateBotEventSeq(config.db, bot.id);
        const ref = await putJson(lightspeedUniverseId, document);
        const promptRef = await putText(
          lightspeedUniverseId,
          renderAdmittedEvent(seq, document),
        );
        const eventId = `self-${randomUUID()}`;
        const session =
          sessionKey === null
            ? undefined
            : { sessionId: botKeyedSessionId(bot.name, sessionKey), label: sessionKey };
        await config.db.insert(schema.botEvents).values({
          botId: bot.id,
          eventId,
          seq,
          triggerId: null,
          kind,
          source: "bot:self",
          occurredAt: new Date(document.occurredAt),
          ref,
          promptRef,
          session: session ?? null,
        });
        const start: BotStartV1 = {
          version: 1,
          universeId: lightspeedUniverseId,
          botId: bot.id,
          botName: bot.name,
          profileId: bot.profileId,
          brief: bot.brief,
          runsPerDay: bot.runsPerDay,
          routedSessionTtlMs: bot.routedSessionTtlMs,
          enabled: bot.enabled,
        };
        const event: BotEvent = {
          version: 1,
          id: eventId,
          ref,
          seq,
          promptRef,
          ...(session === undefined ? {} : { session }),
        };
        await wakeBotController({
          db: config.db,
          temporal: config.temporal,
          start,
          event,
          stored: true,
        });
        await recordSelfConfig(bot.id, `emitted ${kind}: ${summary.slice(0, 120)}`, eventId);
        return { eventId };
      }
      default:
        throw new BotConfigError(`unknown bot tool ${input.toolId}`, 400);
    }
  }

  return {
    async executeBotTool(input) {
      try {
        const result = await execute(input);
        const payloadRef = await putJson(input.universeId, result ?? {});
        return { ok: true, payloadRef };
      } catch (error) {
        // Validation and config failures are final answers for the model;
        // only unexpected infrastructure errors propagate for retry.
        if (error instanceof BotConfigError) {
          const message = error.issues
            ? `${error.message}: ${JSON.stringify(error.issues).slice(0, 800)}`
            : error.message;
          const errorRef = await putJson(input.universeId, { error: message }).catch(() => null);
          return { ok: false, message, errorRef };
        }
        throw error;
      }
    },
  };
}

/** Flatten the model-facing trigger_put arguments into the config inputs. */
export function parseTriggerPutArgs(args: Record<string, unknown>): {
  name: string;
  create: TriggerCreateInput | Record<string, unknown>;
  update: TriggerUpdateInput | Record<string, unknown>;
} {
  const name = requireString(args.name, "name");
  const kind = args.kind;
  if (kind !== "schedule" && kind !== "webhook" && kind !== "poll") {
    throw new BotConfigError("kind must be schedule, webhook, or poll", 400);
  }
  const enabled = typeof args.enabled === "boolean" ? args.enabled : undefined;
  if (kind === "poll") {
    const url = nullableString(args.url);
    const environmentId = nullableString(args.environmentId);
    const argv = Array.isArray(args.argv)
      ? args.argv.filter((entry): entry is string => typeof entry === "string" && entry.length > 0)
      : null;
    const intervalMs = nullableInteger(args.intervalMs);
    if (intervalMs === null) throw new BotConfigError("intervalMs is required for poll triggers", 400);
    const cursorId = nullableString(args.cursorId);
    const watermarkField = nullableString(args.watermarkField);
    if ((cursorId === null) === (watermarkField === null)) {
      throw new BotConfigError("set exactly one of cursorId or watermarkField", 400);
    }
    let source: Record<string, unknown>;
    if (url !== null) {
      if (environmentId !== null || argv !== null) {
        throw new BotConfigError("set url (http) or environmentId+argv (exec), not both", 400);
      }
      source = { kind: "http", url };
    } else {
      if (environmentId === null || argv === null || argv.length === 0) {
        throw new BotConfigError(
          "a poll source needs url (http) or environmentId plus argv (exec)",
          400,
        );
      }
      const cwd = nullableString(args.cwd);
      source = { kind: "exec", environmentId, argv, ...(cwd === null ? {} : { cwd }) };
    }
    const spec = {
      source,
      intervalMs,
      items: nullableString(args.items),
      cursor:
        cursorId !== null
          ? { kind: "idSet", id: cursorId }
          : { kind: "watermark", field: watermarkField },
    };
    const common = pollWebhookCommon(args, enabled);
    return {
      name,
      create: { name, kind, spec, ...common },
      update: { spec, ...common },
    };
  }
  if (kind === "schedule") {
    const spec = {
      cron: nullableString(args.cron),
      at: nullableString(args.at),
      timezone: nullableString(args.timezone) ?? "UTC",
      summary: nullableString(args.summary) ?? "",
    };
    return {
      name,
      create: { name, kind, spec, ...(enabled === undefined ? {} : { enabled }) },
      update: { spec, ...(enabled === undefined ? {} : { enabled }) },
    };
  }
  const verification = nullableString(args.verification);
  const secret = nullableString(args.secret);
  let verificationInput: Record<string, unknown>;
  let preset: "github" | null = null;
  if (verification === "github") {
    if (!secret) throw new BotConfigError("github verification needs a secret", 400);
    preset = "github";
    verificationInput = {
      scheme: "hmac-sha256",
      secret,
      header: "x-hub-signature-256",
      prefix: "sha256=",
    };
  } else if (verification === "hmac-sha256") {
    if (!secret) throw new BotConfigError("hmac-sha256 verification needs a secret", 400);
    verificationInput = { scheme: "hmac-sha256", secret, header: "x-signature-256" };
  } else {
    verificationInput = { scheme: "token" };
  }
  const common = pollWebhookCommon(args, enabled);
  return {
    name,
    create: { name, kind, spec: { verification: verificationInput, preset }, ...common },
    update: { spec: { verification: verificationInput, preset }, ...common },
  };
}

/** Filter/route/coalesce/deliver fields shared by webhook and poll kinds. */
function pollWebhookCommon(
  args: Record<string, unknown>,
  enabled: boolean | undefined,
): Record<string, unknown> {
  const routePolicy = nullableString(args.routePolicy);
  const route =
    routePolicy === "perKey"
      ? { policy: "perKey", key: nullableString(args.routeKey) }
      : routePolicy === "perEvent"
        ? { policy: "perEvent" }
        : null;
  const debounceMs = nullableInteger(args.debounceMs);
  const coalesce =
    debounceMs === null
      ? null
      : {
          debounceMs,
          maxWaitMs: nullableInteger(args.maxWaitMs) ?? debounceMs,
          maxCount: nullableInteger(args.maxCount) ?? 50,
        };
  const whenBusy = nullableString(args.whenBusy);
  const deliver = whenBusy && whenBusy !== "queue" ? { whenBusy } : null;
  const filter = nullableString(args.filter);
  return { filter, route, coalesce, deliver, ...(enabled === undefined ? {} : { enabled }) };
}

function requireString(value: unknown, label: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new BotConfigError(`${label} is required`, 400);
  }
  return value;
}

function nullableString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function nullableInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) ? value : null;
}
