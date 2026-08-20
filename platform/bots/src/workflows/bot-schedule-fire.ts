import { proxyActivities, workflowInfo } from "@temporalio/workflow";
import type { BotActivities } from "../activities/index.js";
import { BOTS_ACTIVITY_TASK_QUEUE, type BotScheduleFireInputV1 } from "../contracts/bots.js";

const activities = proxyActivities<BotActivities>({
  taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
  startToCloseTimeout: "60 seconds",
  retry: { maximumAttempts: 5 },
});

/**
 * One execution per schedule fire. The trigger row is re-read by the
 * admission activity, so this workflow carries only identities plus the
 * nominal fire time from the schedule.
 */
export async function botScheduleFireWorkflowV1(input: BotScheduleFireInputV1): Promise<void> {
  if (input.version !== 1) throw new TypeError("unsupported schedule fire version");
  const attribute = workflowInfo().searchAttributes["TemporalScheduledStartTime"]?.[0];
  const scheduledAt =
    attribute instanceof Date
      ? attribute.toISOString()
      : typeof attribute === "string"
        ? attribute
        : new Date(Date.now()).toISOString();
  await activities.admitScheduleEvent({
    botId: input.botId,
    triggerId: input.triggerId,
    scheduledAt,
  });
}
