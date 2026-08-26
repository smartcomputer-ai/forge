import type { Db } from "@lightspeed/platform-db";
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

export interface BotControlPlaneActivities {
  /** Write-once outcome on every event row of a finished delivery. */
  recordEventOutcomes(input: RecordEventOutcomesInput): Promise<{ updated: number }>;
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
  };
}
