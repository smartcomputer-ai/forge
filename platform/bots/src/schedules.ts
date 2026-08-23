import {
  ScheduleAlreadyRunning,
  ScheduleNotFoundError,
  ScheduleOverlapPolicy,
  type Client,
  type ScheduleOptionsAction,
} from "@temporalio/client";
import {
  BOT_SCHEDULE_FIRE_WORKFLOW,
  BOTS_WORKFLOW_TASK_QUEUE,
  botScheduleId,
  type BotScheduleFireInputV1,
} from "./contracts/bots.js";

/**
 * Missed fires older than this are dropped rather than replayed after an
 * outage; with overlap SKIP the schedule takes at most one catch-up action.
 */
const CATCHUP_WINDOW = "5 minutes";

export interface BotScheduleSpec {
  /** Lightspeed universe id (not the Platform row id). */
  universeId: string;
  botId: string;
  botName: string;
  triggerId: string;
  triggerName: string;
  /** Classic cron; exclusive with `at`. */
  cron?: string | null;
  /** One-shot ISO-8601 instant, expressed as a single calendar spec. */
  at?: string | null;
  timezone: string;
  paused: boolean;
}

const MONTHS = [
  "JANUARY",
  "FEBRUARY",
  "MARCH",
  "APRIL",
  "MAY",
  "JUNE",
  "JULY",
  "AUGUST",
  "SEPTEMBER",
  "OCTOBER",
  "NOVEMBER",
  "DECEMBER",
] as const;

function scheduleSpecOf(spec: BotScheduleSpec) {
  if (spec.at) {
    const when = new Date(spec.at);
    if (Number.isNaN(when.getTime())) throw new TypeError("invalid one-shot instant");
    return {
      calendars: [
        {
          year: when.getUTCFullYear(),
          month: MONTHS[when.getUTCMonth()] as (typeof MONTHS)[number],
          dayOfMonth: when.getUTCDate(),
          hour: when.getUTCHours(),
          minute: when.getUTCMinutes(),
          second: when.getUTCSeconds(),
          comment: `one-shot ${spec.at}`,
        },
      ],
      timezone: "UTC",
    };
  }
  if (!spec.cron) throw new TypeError("schedule needs cron or at");
  return { cronExpressions: [spec.cron], timezone: spec.timezone };
}

/** Create or update the Temporal Schedule for one schedule trigger. */
export async function upsertBotSchedule(client: Client, spec: BotScheduleSpec): Promise<void> {
  const scheduleId = botScheduleId(spec.universeId, spec.botName, spec.triggerName);
  const action = fireAction(spec);
  const scheduleSpec = scheduleSpecOf(spec);
  try {
    await client.schedule.create({
      scheduleId,
      spec: scheduleSpec,
      action,
      policies: { overlap: ScheduleOverlapPolicy.SKIP, catchupWindow: CATCHUP_WINDOW },
      state: { paused: spec.paused },
    });
  } catch (error) {
    if (!(error instanceof ScheduleAlreadyRunning)) throw error;
    const handle = client.schedule.getHandle(scheduleId);
    await handle.update((previous) => ({
      ...previous,
      spec: scheduleSpec,
      action,
      policies: {
        ...previous.policies,
        overlap: ScheduleOverlapPolicy.SKIP,
      },
      state: { ...previous.state, paused: spec.paused },
    }));
  }
}

export async function setBotSchedulePaused(
  client: Client,
  universeId: string,
  botName: string,
  triggerName: string,
  paused: boolean,
  note?: string,
): Promise<void> {
  const handle = client.schedule.getHandle(botScheduleId(universeId, botName, triggerName));
  if (paused) {
    await handle.pause(note);
  } else {
    await handle.unpause(note);
  }
}

/** Delete the Temporal Schedule; already-absent schedules are a no-op. */
export async function deleteBotSchedule(
  client: Client,
  universeId: string,
  botName: string,
  triggerName: string,
): Promise<void> {
  const handle = client.schedule.getHandle(botScheduleId(universeId, botName, triggerName));
  try {
    await handle.delete();
  } catch (error) {
    if (error instanceof ScheduleNotFoundError) return;
    throw error;
  }
}

function fireAction(spec: BotScheduleSpec): ScheduleOptionsAction {
  const input: BotScheduleFireInputV1 = {
    version: 1,
    botId: spec.botId,
    triggerId: spec.triggerId,
  };
  return {
    type: "startWorkflow",
    workflowType: BOT_SCHEDULE_FIRE_WORKFLOW,
    args: [input],
    taskQueue: BOTS_WORKFLOW_TASK_QUEUE,
  };
}
