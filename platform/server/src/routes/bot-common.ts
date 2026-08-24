import { and, count, eq, gte } from "drizzle-orm";
import { Client, Connection } from "@temporalio/client";
import { schema } from "@lightspeed/platform-db";
import {
  BOT_CONFIG_SIGNAL,
  BOT_CONTROLLER_WORKFLOW,
  BOT_EVENT_SIGNAL,
  BOTS_WORKFLOW_TASK_QUEUE,
  botWorkflowId,
  type BotCoalesceParamsV1,
  type BotEvent,
  type BotEventDocumentV1,
  type BotEventSession,
  type BotStartV1,
  type BotWhenBusyV1,
} from "@lightspeed/bots/contracts";
import { allocateBotEventSeq, renderAdmittedEvent } from "@lightspeed/bots/events";
import type { AppContext } from "../context.js";
import { engineClientFor } from "./gateway.js";

export type BotRow = typeof schema.bots.$inferSelect;
export type BotTriggerRow = typeof schema.botTriggers.$inferSelect;
export type UniverseRow = typeof schema.universes.$inferSelect;

let temporalClient: Promise<Client> | null = null;
export function getTemporal(): Promise<Client> {
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

export function botStart(bot: BotRow, universeId: string): BotStartV1 {
  return {
    version: 1,
    universeId,
    botId: bot.id,
    botName: bot.name,
    profileId: bot.profileId,
    brief: bot.brief,
    runsPerDay: bot.runsPerDay,
    routedSessionTtlMs: bot.routedSessionTtlMs,
    selfConfig: bot.selfConfig,
    selfEmit: bot.selfEmit,
    enabled: bot.enabled,
  };
}

export async function signalBotConfig(config: BotStartV1): Promise<void> {
  const temporal = await getTemporal();
  await temporal.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
    workflowId: botWorkflowId(config.universeId, config.botName),
    taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
    args: [config],
    signal: BOT_CONFIG_SIGNAL,
    signalArgs: [config],
  });
}

/**
 * Store, then wake: the document goes to CAS, the envelope row into the
 * authoritative store, and only then is the controller notified.
 */
export async function admitBotEvent(
  ctx: AppContext,
  input: {
    bot: BotRow;
    universe: UniverseRow;
    eventId: string;
    document: BotEventDocumentV1;
    /** Salient payload projection rendered instead of `document.data`. */
    promptData?: unknown;
    triggerId?: string;
    session?: BotEventSession;
    coalesce?: BotCoalesceParamsV1;
    whenBusy?: BotWhenBusyV1;
    /** Skip the controller signal (archived events: filtered at admission). */
    deliver?: boolean;
    /** Reuse an already-stored document ref (replays). */
    ref?: string;
  },
): Promise<{ event: BotEvent; duplicate: boolean }> {
  const engine = engineClientFor(ctx, input.universe);
  const seq = await allocateBotEventSeq(ctx.db, input.bot.id);
  const prompt = renderAdmittedEvent(seq, input.document, input.promptData);
  const promptBlob = { bytesBase64: Buffer.from(prompt, "utf8").toString("base64") };
  let ref = input.ref;
  let promptRef: string | undefined;
  if (ref === undefined) {
    const stored = await engine.call("blobs/put", {
      blobs: [
        { bytesBase64: Buffer.from(JSON.stringify(input.document), "utf8").toString("base64") },
        promptBlob,
      ],
    });
    ref = stored.result.blobs?.[0]?.blobRef;
    promptRef = stored.result.blobs?.[1]?.blobRef;
  } else {
    const stored = await engine.call("blobs/put", { blobs: [promptBlob] });
    promptRef = stored.result.blobs?.[0]?.blobRef;
  }
  if (!ref || !promptRef) throw new Error("event document storage returned no ref");

  const inserted = await ctx.db
    .insert(schema.botEvents)
    .values({
      botId: input.bot.id,
      eventId: input.eventId,
      seq,
      triggerId: input.triggerId ?? null,
      kind: input.document.kind,
      source: input.document.source,
      occurredAt: new Date(input.document.occurredAt),
      ref,
      promptRef,
      session: input.session ?? null,
    })
    .onConflictDoNothing()
    .returning();
  const duplicate = inserted.length === 0;
  if (duplicate) {
    // Keep #N stable: a re-admitted event reuses the stored row's identity
    // (the freshly allocated seq is wasted, which only leaves a gap).
    const [stored] = await ctx.db
      .select()
      .from(schema.botEvents)
      .where(
        and(eq(schema.botEvents.botId, input.bot.id), eq(schema.botEvents.eventId, input.eventId)),
      )
      .limit(1);
    if (stored) {
      ref = stored.ref;
      promptRef = stored.promptRef ?? undefined;
      return {
        duplicate,
        event: {
          version: 1,
          id: input.eventId,
          ref,
          ...(stored.seq === null ? {} : { seq: stored.seq }),
          ...(promptRef === undefined ? {} : { promptRef }),
          ...(input.session === undefined ? {} : { session: input.session }),
          ...(input.coalesce === undefined ? {} : { coalesce: input.coalesce }),
          ...(input.whenBusy === undefined ? {} : { deliver: { whenBusy: input.whenBusy } }),
        },
      };
    }
  }

  const event: BotEvent = {
    version: 1,
    id: input.eventId,
    ref,
    seq,
    promptRef,
    ...(input.session === undefined ? {} : { session: input.session }),
    ...(input.coalesce === undefined ? {} : { coalesce: input.coalesce }),
    ...(input.whenBusy === undefined ? {} : { deliver: { whenBusy: input.whenBusy } }),
  };
  if (input.deliver !== false) {
    const config = botStart(input.bot, input.universe.lightspeedUniverseId);
    const temporal = await getTemporal();
    await temporal.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
      workflowId: botWorkflowId(config.universeId, config.botName),
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      args: [config],
      signal: BOT_EVENT_SIGNAL,
      signalArgs: [event],
    });
  }
  return { event, duplicate };
}

/**
 * Per-trigger flood breaker: when the bot's breaker rate is exceeded, the
 * trigger is disabled and the delivery rejected. A human re-enables it.
 */
export async function checkTriggerBreaker(
  ctx: AppContext,
  bot: BotRow,
  trigger: BotTriggerRow,
): Promise<{ tripped: boolean }> {
  const breaker = bot.breaker;
  if (!breaker) return { tripped: false };
  const since = new Date(Date.now() - breaker.windowMs);
  const [row] = await ctx.db
    .select({ value: count() })
    .from(schema.botEvents)
    .where(and(eq(schema.botEvents.triggerId, trigger.id), gte(schema.botEvents.receivedAt, since)));
  if (Number(row?.value ?? 0) < breaker.fires) return { tripped: false };
  await ctx.db
    .update(schema.botTriggers)
    .set({ enabled: false })
    .where(eq(schema.botTriggers.id, trigger.id));
  await recordActivity(ctx, bot.id, "breaker_tripped", {
    detail: `trigger ${trigger.name} exceeded ${breaker.fires} events in ${Math.round(breaker.windowMs / 1000)}s and was disabled`,
  });
  return { tripped: true };
}

export async function recordActivity(
  ctx: AppContext,
  botId: string,
  kind: string,
  fields?: { eventId?: string; detail?: string },
): Promise<void> {
  await ctx.db.insert(schema.botActivity).values({
    botId,
    kind,
    eventId: fields?.eventId ?? null,
    runId: null,
    detail: fields?.detail ?? null,
  });
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
