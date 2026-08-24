import { eq, sql } from "drizzle-orm";
import { schema, type Db } from "@lightspeed/platform-db";
import type { BotEventDocumentV1 } from "./contracts/bots.js";
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
  });
}
