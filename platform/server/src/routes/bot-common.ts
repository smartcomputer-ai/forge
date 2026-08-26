import { Client, Connection } from "@temporalio/client";
import { schema } from "@lightspeed/platform-db";
import {
  BOT_CONFIG_SIGNAL,
  BOT_CONTROLLER_WORKFLOW,
  BOTS_WORKFLOW_TASK_QUEUE,
  botWorkflowId,
  type BotCoalesceParamsV1,
  type BotEvent,
  type BotEventDocumentV1,
  type BotEventSession,
  type BotStartV1,
  type BotWhenBusyV1,
} from "@lightspeed/bots/contracts";
import {
  botStartFor,
  checkTriggerBreaker as checkTriggerBreakerWith,
  storeBotEvent,
  type AdmissionDeps,
} from "@lightspeed/bots/admission";
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
  return botStartFor(bot, universeId);
}

/** Admission dependencies for a universe: the platform database, Temporal, and the core client. */
export async function admissionDeps(ctx: AppContext, universe: UniverseRow): Promise<AdmissionDeps> {
  return { db: ctx.db, temporal: await getTemporal(), engine: engineClientFor(ctx, universe) };
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
 * Store, then wake, through the shared admission pipeline in the bots
 * package (`storeBotEvent`): CAS document, authoritative row, then the
 * controller signal. Used by the manual event and replay routes; webhook
 * ingest goes through `admitTriggerEvent` for the trigger pipeline.
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
  const { universe, ...rest } = input;
  return storeBotEvent(await admissionDeps(ctx, universe), {
    ...rest,
    universeId: universe.lightspeedUniverseId,
  });
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
  return checkTriggerBreakerWith({ db: ctx.db }, bot, trigger);
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
