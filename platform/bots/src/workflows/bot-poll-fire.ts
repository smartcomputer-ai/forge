import { proxyActivities, workflowInfo } from "@temporalio/workflow";
import type { BotActivities } from "../activities/index.js";
import { BOTS_ACTIVITY_TASK_QUEUE, type BotPollFireInputV1 } from "../contracts/bots.js";

// A fire covers one fetch or one environment job (which may first wake a
// sleeping environment: `environment_not_ready` retries land here) plus
// per-item admissions; the retry budget absorbs wake latency without
// overlapping the next fire badly (overlap policy SKIP drops collisions).
const activities = proxyActivities<BotActivities>({
  taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
  startToCloseTimeout: "240 seconds",
  retry: { maximumAttempts: 6 },
});

/**
 * One execution per poll fire. The trigger row is re-read by the activity,
 * so this workflow carries only identities plus the nominal fire time.
 */
export async function botPollFireWorkflowV1(input: BotPollFireInputV1): Promise<void> {
  if (input.version !== 1) throw new TypeError("unsupported poll fire version");
  const attribute = workflowInfo().searchAttributes["TemporalScheduledStartTime"]?.[0];
  const scheduledAt =
    attribute instanceof Date
      ? attribute.toISOString()
      : typeof attribute === "string"
        ? attribute
        : new Date(Date.now()).toISOString();
  await activities.pollBotTrigger({
    botId: input.botId,
    triggerId: input.triggerId,
    scheduledAt,
  });
}
