import { randomBytes } from "node:crypto";
import { and, eq } from "drizzle-orm";
import { z } from "zod";
import { schema, type Db } from "@lightspeed/platform-db";
import type { Client } from "@temporalio/client";
import { deleteBotSchedule, upsertBotSchedule, type BotScheduleSpec } from "./schedules.js";

export type BotRow = typeof schema.bots.$inferSelect;
export type BotTriggerRow = typeof schema.botTriggers.$inferSelect;

export const botNameInput = z
  .string()
  .regex(/^[a-z0-9][a-z0-9-]*$/, "lowercase alphanumerics and dashes")
  .max(64);
/// Temporal Schedules take classic 5-field crontab or an @-macro; reject
/// Quartz-style expressions (seconds field, `?`) with a message that names
/// the expected shape instead of Temporal's field-range error.
export const cronInput = z
  .string()
  .trim()
  .min(1)
  .max(200)
  .refine(
    (value) => value.startsWith("@") || (!value.includes("?") && value.split(/\s+/).length === 5),
    "expected 5-field cron (minute hour day month weekday) or an @-macro like @daily",
  );
export const celInput = z.string().trim().min(1).max(2_000);
export const routeInput = z.discriminatedUnion("policy", [
  z.object({ policy: z.literal("bot") }),
  z.object({ policy: z.literal("perKey"), key: celInput.max(500).nullish() }),
  z.object({ policy: z.literal("perEvent") }),
]);
export const scheduleSpecInput = z
  .object({
    cron: cronInput.nullish(),
    at: z.string().datetime({ offset: true }).nullish(),
    timezone: z.string().trim().min(1).max(64).default("UTC"),
    summary: z.string().trim().min(1).max(2_000),
  })
  .refine((value) => Boolean(value.cron) !== Boolean(value.at), "set exactly one of cron or at")
  .refine(
    (value) => !value.at || new Date(value.at).getTime() > Date.now() + 30_000,
    "a one-shot `at` must lie at least 30 seconds in the future",
  );
export const webhookVerificationInput = z.discriminatedUnion("scheme", [
  z.object({ scheme: z.literal("token") }),
  z.object({
    scheme: z.literal("hmac-sha256"),
    secret: z.string().min(8).max(200),
    header: z.string().trim().min(1).max(100),
    prefix: z.string().max(20).optional(),
  }),
]);
export const webhookSpecInput = z.object({
  verification: webhookVerificationInput.default({ scheme: "token" }),
  preset: z.enum(["github"]).nullish(),
});
export const coalesceInput = z
  .object({
    debounceMs: z.number().int().min(1_000).max(604_800_000),
    maxWaitMs: z.number().int().min(1_000).max(604_800_000),
    maxCount: z.number().int().min(2).max(100),
  })
  .refine((value) => value.maxWaitMs >= value.debounceMs, "maxWaitMs must cover debounceMs");
export const deliverInput = z.object({ whenBusy: z.enum(["queue", "steer", "append"]) });
export const breakerInput = z.object({
  fires: z.number().int().min(1).max(100_000),
  windowMs: z.number().int().min(1_000).max(86_400_000),
});
export const triggerCreateInput = z.discriminatedUnion("kind", [
  z.object({
    name: botNameInput,
    kind: z.literal("schedule"),
    spec: scheduleSpecInput,
    enabled: z.boolean().default(true),
  }),
  z.object({
    name: botNameInput,
    kind: z.literal("webhook"),
    spec: webhookSpecInput.default({ verification: { scheme: "token" } }),
    filter: celInput.nullish(),
    route: routeInput.nullish(),
    coalesce: coalesceInput.nullish(),
    deliver: deliverInput.nullish(),
    enabled: z.boolean().default(true),
  }),
]);
export type TriggerCreateInput = z.infer<typeof triggerCreateInput>;
export const triggerUpdateInput = z
  .object({
    spec: z.unknown().optional(),
    filter: celInput.nullable().optional(),
    route: routeInput.nullable().optional(),
    coalesce: coalesceInput.nullable().optional(),
    deliver: deliverInput.nullable().optional(),
    enabled: z.boolean().optional(),
  })
  .refine((value) => Object.keys(value).length > 0, "at least one field is required");
export type TriggerUpdateInput = z.infer<typeof triggerUpdateInput>;

type ScheduleSpecRow = { cron: string | null; at: string | null; timezone: string; summary: string };
type WebhookSpecRow = {
  token: string;
  verification:
    | { scheme: "token" }
    | { scheme: "hmac-sha256"; secret: string; header: string; prefix?: string };
  preset: "github" | null;
};
type RouteRow = { policy: "bot" } | { policy: "perKey"; key: string | null } | { policy: "perEvent" };

/// Zod outputs carry `undefined` for omitted optionals; the row types do not.
function normalizeScheduleSpec(spec: z.infer<typeof scheduleSpecInput>): ScheduleSpecRow {
  return { cron: spec.cron ?? null, at: spec.at ?? null, timezone: spec.timezone, summary: spec.summary };
}

function normalizeWebhookSpec(spec: z.infer<typeof webhookSpecInput>, token: string): WebhookSpecRow {
  const verification =
    spec.verification.scheme === "token"
      ? ({ scheme: "token" } as const)
      : {
          scheme: "hmac-sha256" as const,
          secret: spec.verification.secret,
          header: spec.verification.header,
          ...(spec.verification.prefix === undefined ? {} : { prefix: spec.verification.prefix }),
        };
  return { token, verification, preset: spec.preset ?? null };
}

function normalizeRoute(route: z.infer<typeof routeInput> | null | undefined): RouteRow | null {
  if (route === null || route === undefined) return null;
  if (route.policy === "perKey") return { policy: "perKey", key: route.key ?? null };
  return route;
}

export interface BotConfigDeps {
  db: Db;
  temporal: Client;
}

export class BotConfigError extends Error {
  constructor(
    message: string,
    readonly status: 400 | 404 | 409 | 502,
    readonly issues?: unknown,
  ) {
    super(message);
    this.name = "BotConfigError";
  }
}

export function scheduleSpecFor(bot: BotRow, trigger: BotTriggerRow, universeId: string): BotScheduleSpec {
  const spec = trigger.spec as { cron?: string | null; at?: string | null; timezone: string };
  return {
    universeId,
    botId: bot.id,
    botName: bot.name,
    triggerId: trigger.id,
    triggerName: trigger.name,
    cron: spec.cron ?? null,
    at: spec.at ?? null,
    timezone: spec.timezone,
    paused: !(bot.enabled && trigger.enabled),
  };
}

/** Path of a webhook trigger's ingest endpoint (relative to the platform origin). */
export function webhookIngestPath(trigger: BotTriggerRow): string {
  const spec = trigger.spec as { token: string };
  return `/api/v1/hooks/bots/${trigger.id}/${spec.token}`;
}

export function canManageRole(role: string): boolean {
  return role === "owner" || role === "admin" || role === "platform-admin";
}

/// Members who cannot manage the bot still see trigger shapes, but never the
/// ingest token or signing secret.
export function redactTriggerSecrets(trigger: BotTriggerRow): BotTriggerRow {
  if (trigger.kind !== "webhook") return trigger;
  const spec = trigger.spec as {
    token: string;
    verification: { scheme: string; secret?: string };
    preset?: string | null;
  };
  return {
    ...trigger,
    spec: {
      ...spec,
      token: "",
      verification:
        spec.verification.secret === undefined
          ? spec.verification
          : { ...spec.verification, secret: "" },
    } as BotTriggerRow["spec"],
  };
}

/** Create a trigger; the single code path behind the API and the bot's own tools. */
export async function createTrigger(
  deps: BotConfigDeps,
  args: { bot: BotRow; universeId: string; input: TriggerCreateInput },
): Promise<BotTriggerRow> {
  const { bot, universeId, input } = args;
  const values =
    input.kind === "schedule"
      ? {
          botId: bot.id,
          name: input.name,
          kind: input.kind,
          spec: normalizeScheduleSpec(input.spec),
          filter: null,
          route: null,
          coalesce: null,
          deliver: null,
          enabled: input.enabled,
        }
      : {
          botId: bot.id,
          name: input.name,
          kind: input.kind,
          spec: normalizeWebhookSpec(input.spec, randomBytes(24).toString("hex")),
          filter: input.filter ?? null,
          route: normalizeRoute(input.route),
          coalesce: input.coalesce ?? null,
          deliver: input.deliver ?? null,
          enabled: input.enabled,
        };
  const [trigger] = await deps.db
    .insert(schema.botTriggers)
    .values(values)
    .onConflictDoNothing()
    .returning();
  if (!trigger) throw new BotConfigError("a trigger with that name already exists", 409);
  if (input.kind === "schedule") {
    try {
      await upsertBotSchedule(deps.temporal, scheduleSpecFor(bot, trigger, universeId));
    } catch (error) {
      await deps.db.delete(schema.botTriggers).where(eq(schema.botTriggers.id, trigger.id));
      throw new BotConfigError(`failed to create the schedule: ${errorMessage(error)}`, 502);
    }
  }
  return trigger;
}

export async function updateTrigger(
  deps: BotConfigDeps,
  args: { bot: BotRow; universeId: string; existing: BotTriggerRow; input: TriggerUpdateInput },
): Promise<BotTriggerRow> {
  const { bot, universeId, existing, input } = args;
  if (
    existing.kind === "schedule" &&
    (input.filter !== undefined ||
      input.route !== undefined ||
      input.coalesce !== undefined ||
      input.deliver !== undefined)
  ) {
    throw new BotConfigError(
      "filters, routes, coalescing, and delivery policy apply to webhook triggers",
      400,
    );
  }
  const changes: Partial<BotTriggerRow> = {};
  if (input.enabled !== undefined) changes.enabled = input.enabled;
  if (input.filter !== undefined) changes.filter = input.filter;
  if (input.route !== undefined) changes.route = normalizeRoute(input.route);
  if (input.coalesce !== undefined) changes.coalesce = input.coalesce;
  if (input.deliver !== undefined) changes.deliver = input.deliver;
  if (input.spec !== undefined) {
    if (existing.kind === "schedule") {
      const parsed = scheduleSpecInput.safeParse(input.spec);
      if (!parsed.success) throw new BotConfigError("validation failed", 400, parsed.error.issues);
      changes.spec = normalizeScheduleSpec(parsed.data);
    } else {
      const parsed = webhookSpecInput.safeParse(input.spec);
      if (!parsed.success) throw new BotConfigError("validation failed", 400, parsed.error.issues);
      // The URL token survives spec edits; rotation means a new trigger.
      const token = (existing.spec as { token: string }).token;
      changes.spec = normalizeWebhookSpec(parsed.data, token);
    }
  }
  const [trigger] = await deps.db
    .update(schema.botTriggers)
    .set(changes)
    .where(eq(schema.botTriggers.id, existing.id))
    .returning();
  if (!trigger) throw new BotConfigError("not found", 404);
  if (existing.kind === "schedule") {
    try {
      await upsertBotSchedule(deps.temporal, scheduleSpecFor(bot, trigger, universeId));
    } catch (error) {
      await deps.db
        .update(schema.botTriggers)
        .set({ spec: existing.spec, enabled: existing.enabled })
        .where(eq(schema.botTriggers.id, existing.id));
      throw new BotConfigError(
        `schedule reconciliation failed; the trigger was not changed: ${errorMessage(error)}`,
        502,
      );
    }
  }
  return trigger;
}

export async function deleteTrigger(
  deps: BotConfigDeps,
  args: { bot: BotRow; universeId: string; existing: BotTriggerRow },
): Promise<void> {
  const { bot, universeId, existing } = args;
  if (existing.kind === "schedule") {
    try {
      await deleteBotSchedule(deps.temporal, universeId, bot.name, existing.name);
    } catch (error) {
      throw new BotConfigError(
        `failed to delete the schedule; the trigger was kept: ${errorMessage(error)}`,
        502,
      );
    }
  }
  await deps.db.delete(schema.botTriggers).where(eq(schema.botTriggers.id, existing.id));
}

export async function findTriggerByName(
  db: Db,
  botId: string,
  name: string,
): Promise<BotTriggerRow | undefined> {
  const [row] = await db
    .select()
    .from(schema.botTriggers)
    .where(and(eq(schema.botTriggers.botId, botId), eq(schema.botTriggers.name, name)))
    .limit(1);
  return row;
}

export async function reconcileBotSchedules(
  deps: BotConfigDeps,
  bot: BotRow,
  universeId: string,
): Promise<void> {
  const triggers = await deps.db
    .select()
    .from(schema.botTriggers)
    .where(and(eq(schema.botTriggers.botId, bot.id), eq(schema.botTriggers.kind, "schedule")));
  for (const trigger of triggers) {
    await upsertBotSchedule(deps.temporal, scheduleSpecFor(bot, trigger, universeId));
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
