import { and, eq, sql } from "drizzle-orm";
import type { WorkflowClient } from "@temporalio/client";
import { schema, type Db } from "@lightspeed/platform-db";
import {
  BOT_CONTROLLER_WORKFLOW,
  BOT_EVENT_SIGNAL,
  BOTS_WORKFLOW_TASK_QUEUE,
  botWorkflowId,
  type BotEvent,
  type BotEventDocumentV1,
  type BotStartV1,
} from "./contracts/bots.js";
import { renderEventPrompt } from "./rendering.js";

/**
 * Allocate the next per-bot event sequence number (#N). A dedicated counter
 * column keeps allocation race-free under concurrent webhook admissions;
 * a duplicate admission wastes its number, which only leaves a gap.
 */
export async function allocateBotEventSeq(db: Db, botId: string): Promise<number> {
  const [row] = await db
    .update(schema.bots)
    .set({ eventSeq: sql`${schema.bots.eventSeq} + 1` })
    .where(eq(schema.bots.id, botId))
    .returning({ seq: schema.bots.eventSeq });
  if (!row) throw new Error(`bot ${botId} not found while allocating an event seq`);
  return row.seq;
}

/**
 * The model-facing rendering of one admitted event. The stored document is
 * the machine envelope; `promptData`, when a preset projected the payload,
 * replaces `document.data` in the rendering only. Headers are never
 * rendered — they exist for verification and filters and stay readable via
 * bot_event_read.
 */
export function renderAdmittedEvent(
  seq: number,
  document: BotEventDocumentV1,
  promptData?: unknown,
): string {
  return renderEventPrompt({
    seq,
    kind: document.kind,
    source: document.source,
    occurredAt: document.occurredAt,
    summary: document.summary,
    data: promptData ?? document.data,
    ...(document.correlationId === undefined ? {} : { correlationId: document.correlationId }),
    ...(document.links === undefined ? {} : { links: document.links }),
    ...(document.inReplyTo === undefined ? {} : { inReplyTo: document.inReplyTo }),
  });
}

/** The slice of a Temporal client that waking a controller needs. */
export interface BotWakeClient {
  workflow: Pick<WorkflowClient, "signalWithStart">;
}

/**
 * Wake the bot controller with a stored event (signal-with-start, so a
 * missing controller is created on the way).
 *
 * The controller dedupes by event id, so waking it again for an event whose
 * row already existed is harmless — and it is how a retried admission heals
 * a wake that failed after the row was stored. When this call stored the row
 * itself and the wake fails, the row is removed before rethrowing: the caller
 * sees a plain failure with nothing left behind, and its retry (Temporal, the
 * webhook sender, the model) admits the event from scratch instead of
 * leaving a stranded duplicate that nothing will ever deliver.
 */
export async function wakeBotController(input: {
  db: Db;
  temporal: BotWakeClient;
  start: BotStartV1;
  event: BotEvent;
  /** True when the caller inserted the event row in this admission. */
  stored: boolean;
}): Promise<void> {
  try {
    await input.temporal.workflow.signalWithStart(BOT_CONTROLLER_WORKFLOW, {
      workflowId: botWorkflowId(input.start.universeId, input.start.botName),
      taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
      args: [input.start],
      signal: BOT_EVENT_SIGNAL,
      signalArgs: [input.event],
    });
  } catch (error) {
    if (input.stored) {
      try {
        await input.db
          .delete(schema.botEvents)
          .where(
            and(
              eq(schema.botEvents.botId, input.start.botId),
              eq(schema.botEvents.eventId, input.event.id),
            ),
          );
      } catch {
        // The wake failure is the error worth reporting; a row that could not
        // be discarded is healed by the next admission of the same event id.
      }
    }
    throw error;
  }
}
