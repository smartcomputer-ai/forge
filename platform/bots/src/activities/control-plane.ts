import { schema, type Db } from "@lightspeed/platform-db";

export interface BotActivityEntry {
  kind: string;
  eventId?: string;
  runId?: string;
  detail?: string;
}

export interface RecordBotActivityInput {
  botId: string;
  entries: BotActivityEntry[];
}

export interface BotControlPlaneActivities {
  recordBotActivity(input: RecordBotActivityInput): Promise<void>;
}

export function createBotControlPlaneActivities(db: Db): BotControlPlaneActivities {
  return {
    async recordBotActivity({ botId, entries }) {
      if (entries.length === 0) return;
      await db.insert(schema.botActivity).values(
        entries.map((entry) => ({
          botId,
          kind: entry.kind,
          eventId: entry.eventId ?? null,
          runId: entry.runId ?? null,
          detail: entry.detail ?? null,
        })),
      );
    },
  };
}
