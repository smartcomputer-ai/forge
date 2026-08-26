import { ApplicationFailure } from "@temporalio/common";
import { and, eq, sql } from "drizzle-orm";
import { schema, type Db } from "@lightspeed/platform-db";
import type {
  AssertTriggerActiveInput,
  ControlPlaneActivities,
} from "../contracts/control-plane.js";

export function createControlPlaneActivities(db: Db): ControlPlaneActivities {
  return {
    async assertTriggerActive(input) {
      if (!(await triggerIsActive(db, input))) {
        throw ApplicationFailure.nonRetryable(
          `chat trigger ${input.triggerId} no longer serves this conversation`,
          "InactiveChatTrigger",
        );
      }
    },
  };
}

async function triggerIsActive(db: Db, input: AssertTriggerActiveInput): Promise<boolean> {
  const [row] = await db
    .select({
      enabled: schema.botTriggers.enabled,
      spec: schema.botTriggers.spec,
      botEnabled: schema.bots.enabled,
      universeStatus: schema.universes.status,
      accountEnabled: schema.channelAccounts.enabled,
      pairingKey: schema.channelPairings.key,
    })
    .from(schema.botTriggers)
    .innerJoin(schema.bots, eq(schema.bots.id, schema.botTriggers.botId))
    .innerJoin(schema.universes, eq(schema.universes.id, schema.bots.universeId))
    .innerJoin(
      schema.channelAccounts,
      and(
        eq(schema.channelAccounts.id, sql`(${schema.botTriggers.spec}->>'channelAccountId')::uuid`),
        eq(schema.channelAccounts.provider, input.route.provider),
        eq(schema.channelAccounts.accountId, input.route.accountId),
      ),
    )
    .leftJoin(
      schema.channelPairings,
      and(
        eq(schema.channelPairings.triggerId, schema.botTriggers.id),
        eq(schema.channelPairings.channelAccountId, schema.channelAccounts.id),
        eq(schema.channelPairings.chatId, input.route.chatId),
      ),
    )
    .where(and(eq(schema.botTriggers.id, input.triggerId), eq(schema.botTriggers.kind, "chat")))
    .limit(1);
  if (row === undefined) {
    return false;
  }
  const spec = row.spec as { matchScope?: "direct" | "group" | null; pairingCode?: string | null };
  return (
    row.enabled &&
    row.botEnabled &&
    row.accountEnabled &&
    row.universeStatus === "active" &&
    (spec.matchScope == null || spec.matchScope === input.scope) &&
    (spec.pairingCode == null || row.pairingKey !== null)
  );
}
