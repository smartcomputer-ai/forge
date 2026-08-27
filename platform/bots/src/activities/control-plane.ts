import { eq } from "drizzle-orm";
import { schema, type Db } from "@lightspeed/platform-db";
import type { BotEventFinalOutcome } from "../contracts/bots.js";
import { recordEventOutcomes } from "../admission.js";

export interface RecordEventOutcomesInput {
  botId: string;
  eventIds: string[];
  outcome: BotEventFinalOutcome;
  detail: string | null;
  deliveryId: string | null;
  runId: string | null;
}

export interface RecordBotClosedInput {
  botId: string;
  /** Every session the controller closed on the way out, main generations first. */
  sessions: string[];
}

export interface BotControlPlaneActivities {
  /** Write-once outcome on every event row of a finished delivery. */
  recordEventOutcomes(input: RecordEventOutcomesInput): Promise<{ updated: number }>;
  /**
   * The controller's last write before it completes: the sessions it closed,
   * so delete can erase them once the workflow is gone. Union with what an
   * earlier teardown attempt recorded — a retried close must not lose ids.
   */
  recordBotClosed(input: RecordBotClosedInput): Promise<{ sessions: string[] }>;
}

export function createBotControlPlaneActivities(db: Db): BotControlPlaneActivities {
  return {
    async recordEventOutcomes(input) {
      return recordEventOutcomes(db, input.botId, input.eventIds, {
        outcome: input.outcome,
        detail: input.detail,
        deliveryId: input.deliveryId,
        runId: input.runId,
      });
    },

    async recordBotClosed(input) {
      const [existing] = await db
        .select({ closedSessions: schema.bots.closedSessions })
        .from(schema.bots)
        .where(eq(schema.bots.id, input.botId))
        .limit(1);
      const sessions = [...new Set([...(existing?.closedSessions ?? []), ...input.sessions])];
      await db
        .update(schema.bots)
        .set({ closedSessions: sessions })
        .where(eq(schema.bots.id, input.botId));
      return { sessions };
    },
  };
}
