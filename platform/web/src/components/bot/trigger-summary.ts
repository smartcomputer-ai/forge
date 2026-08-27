import type {
  BotChatSpec,
  BotInboxSpec,
  BotPollSpec,
  BotScheduleSpec,
  BotTrigger,
  BotWebhookSpec,
} from "@/api";
import { cronBuilderFromExpression, cronFromBuilder, type CronBuilderState } from "./cron-builder";

const WEEKDAY_NAMES = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

function timeLabel(time: string): string {
  const match = /^(\d{2}):(\d{2})$/.exec(time);
  return match ? `${match[1]}:${match[2]}` : time;
}

/** A cron expression in words when the builder can express it, else the expression itself. */
export function describeCron(cron: string, timezone?: string | null): string {
  const trimmed = cron.trim();
  if (!trimmed) return "no schedule";
  const state: CronBuilderState = cronBuilderFromExpression(trimmed);
  const exact = cronFromBuilder(state) === trimmed;
  const zone = timezone && timezone !== "UTC" ? ` ${timezone}` : timezone === "UTC" ? " UTC" : "";
  if (!exact) return `${trimmed}${zone}`;
  switch (state.frequency) {
    case "minutes":
      return `Every ${state.interval} minutes`;
    case "hourly":
      return state.minute === 0 ? "Every hour" : `Every hour at :${String(state.minute).padStart(2, "0")}`;
    case "daily":
      return `Every day at ${timeLabel(state.time)}${zone}`;
    case "weekdays":
      return `Weekdays at ${timeLabel(state.time)}${zone}`;
    case "weekly":
      return `${WEEKDAY_NAMES[state.weekday] ?? "Weekly"}s at ${timeLabel(state.time)}${zone}`;
    case "monthly":
      return `Monthly on day ${state.monthday} at ${timeLabel(state.time)}${zone}`;
  }
}

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** One line saying what wakes the bot, in the person's words. */
export function triggerSummary(trigger: BotTrigger): string {
  switch (trigger.kind) {
    case "schedule": {
      const spec = trigger.spec as BotScheduleSpec;
      if (spec.at) return `Once, at ${new Date(spec.at).toLocaleString()}`;
      return describeCron(spec.cron ?? "", spec.timezone);
    }
    case "webhook": {
      const spec = trigger.spec as BotWebhookSpec;
      const source = spec.preset === "github" ? "GitHub webhook" : "Webhook";
      const verified = spec.verification.scheme === "hmac-sha256" ? "signed" : "URL token";
      return `${source} · ${verified}`;
    }
    case "poll": {
      const spec = trigger.spec as BotPollSpec;
      const every = `every ${Math.max(1, Math.round(spec.intervalMs / 60_000))} min`;
      return spec.source.kind === "http"
        ? `Checks ${hostOf(spec.source.url)} ${every}`
        : `Runs ${spec.source.argv[0] ?? "a command"} ${every}`;
    }
    case "chat": {
      const spec = trigger.spec as BotChatSpec;
      const account = trigger.channelAccount
        ? `${trigger.channelAccount.provider} · ${trigger.channelAccount.displayName}`
        : "a messaging account";
      const scope =
        spec.matchScope === "direct" ? "direct messages" : spec.matchScope === "group" ? "groups" : "all chats";
      return `${account} · ${scope}${spec.pairingCode === null ? "" : " · pairing required"}`;
    }
    case "bot": {
      const spec = trigger.spec as BotInboxSpec;
      return spec.from === undefined
        ? "Messages from any bot in this universe"
        : spec.from.length === 0
          ? "Messages from no bot yet"
          : `Messages from ${spec.from.join(", ")}`;
    }
  }
}

export interface DeliveryShape {
  routePolicy: "bot" | "perKey" | "perEvent";
  routeKey: string;
  filter: string;
  whenBusy: "queue" | "steer" | "append";
  /** Seconds, as typed; empty means no coalescing. */
  debounceSeconds: string;
  maxWaitSeconds: string;
  ttlMode: "inherit" | "forever" | "hours";
  ttlHours: string;
}

/**
 * The Advanced disclosure, closed: what routing, batching, busy handling,
 * and retention do, as one sentence. Reads the same for a form and a saved
 * trigger, so a person learns the vocabulary before opening the fields.
 */
export function deliverySentence(shape: DeliveryShape, chat = false): string {
  const parts: string[] = [];
  if (shape.routePolicy === "bot") parts.push("to Main");
  else if (shape.routePolicy === "perKey") {
    parts.push(
      chat
        ? "one thread per conversation"
        : shape.routeKey.trim()
          ? `one thread per ${shape.routeKey.trim()}`
          : "one thread per key",
    );
  } else parts.push(chat ? "one thread per message" : "one thread per event");
  if (shape.filter.trim()) parts.push("filtered");
  const debounce = Number(shape.debounceSeconds);
  if (shape.debounceSeconds.trim() !== "" && debounce > 0) {
    const wait = Number(shape.maxWaitSeconds);
    const bound = shape.maxWaitSeconds.trim() !== "" && wait > debounce ? wait : debounce;
    parts.push(`batches for up to ${bound % 1 === 0 ? bound : bound.toFixed(1)}s`);
  } else parts.push("no batching");
  parts.push(
    shape.whenBusy === "steer"
      ? "steers a busy run"
      : shape.whenBusy === "append"
        ? "context only when busy"
        : "queues when busy",
  );
  if (shape.routePolicy !== "bot") {
    if (shape.ttlMode === "forever") parts.push("threads kept");
    else if (shape.ttlMode === "hours" && shape.ttlHours.trim()) parts.push(`threads close after ${shape.ttlHours.trim()}h idle`);
  }
  return parts.join(" · ");
}

export function deliveryShapeOf(trigger: BotTrigger): DeliveryShape {
  return {
    routePolicy: trigger.route?.policy ?? (trigger.kind === "chat" ? "perKey" : "bot"),
    routeKey: trigger.route?.policy === "perKey" ? (trigger.route.key ?? "") : "",
    filter: trigger.filter ?? "",
    whenBusy: trigger.deliver?.whenBusy ?? "queue",
    debounceSeconds: trigger.coalesce ? String(trigger.coalesce.debounceMs / 1000) : "",
    maxWaitSeconds: trigger.coalesce ? String(trigger.coalesce.maxWaitMs / 1000) : "",
    ttlMode: trigger.sessionTtlMs === null ? "inherit" : trigger.sessionTtlMs === 0 ? "forever" : "hours",
    ttlHours: trigger.sessionTtlMs ? String(Math.round(trigger.sessionTtlMs / 3_600_000)) : "",
  };
}
