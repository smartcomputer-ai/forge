import { eq } from "drizzle-orm";
import { schema } from "@lightspeed/platform-db";
import { scheduleSpecFor } from "@lightspeed/bots/config";
import { upsertBotSchedule } from "@lightspeed/bots/schedules";
import type { AppContext } from "./context.js";
import { errorMessage, getTemporal } from "./routes/bot-common.js";

/**
 * Reconcile every schedule-trigger row to its Temporal Schedule. The rows are
 * authoritative; Temporal state can be lost on a dev reset or drift after a
 * partial failure, and this converges it at server start. Failures are
 * per-trigger and logged, never fatal to boot.
 */
export async function reconcileAllBotSchedules(ctx: Pick<AppContext, "db">): Promise<void> {
  let reconciled = 0;
  let failed = 0;
  const rows = await ctx.db
    .select({
      trigger: schema.botTriggers,
      bot: schema.bots,
      lightspeedUniverseId: schema.universes.lightspeedUniverseId,
    })
    .from(schema.botTriggers)
    .innerJoin(schema.bots, eq(schema.botTriggers.botId, schema.bots.id))
    .innerJoin(schema.universes, eq(schema.bots.universeId, schema.universes.id))
    .where(eq(schema.botTriggers.kind, "schedule"));
  if (rows.length === 0) return;
  const temporal = await getTemporal();
  for (const row of rows) {
    try {
      await upsertBotSchedule(
        temporal,
        scheduleSpecFor(row.bot, row.trigger, row.lightspeedUniverseId),
      );
      reconciled += 1;
    } catch (error) {
      failed += 1;
      console.error(
        `bots: failed to reconcile schedule for ${row.bot.name}/${row.trigger.name}: ${errorMessage(error)}`,
      );
    }
  }
  console.log(`bots: reconciled ${reconciled} schedule(s)${failed ? `, ${failed} failed` : ""}`);
}
