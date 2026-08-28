import { installTemporalMetrics } from "@lightspeed/channels/runtime/temporal-metrics";
import {
  resolvePlatformWorkerRoles,
  type PlatformWorkerLeafRole,
} from "./roles.js";

const command = process.env.LIGHTSPEED_PLATFORM_WORKERS_ROLE ?? process.argv[2];
const roles = resolvePlatformWorkerRoles(command, process.env.LIGHTSPEED_CHANNELS_CONNECTORS);
const channelRoles = roles.filter(
  (role) => role.startsWith("channels-") || role === "telegram" || role === "whatsapp",
);
if (channelRoles.length > 0) {
  installTemporalMetrics(
    roles.length > 1 ? `platform-workers-${command ?? "all"}` : roles[0]!,
    metricsPort(channelRoles),
  );
}

let stopping = false;
const onSignal = () => {
  stopping = true;
};
process.once("SIGINT", onSignal);
process.once("SIGTERM", onSignal);

console.log(`platform-workers: starting ${roles.join(", ")} in one process`);
const running = roles.map((role) => loadRole(role));

try {
  if (running.length === 1) {
    await running[0];
  } else {
    const stoppedRole = await Promise.race(
      running.map((promise, index) => promise.then(() => roles[index])),
    );
    if (!stopping) {
      throw new Error(`Platform workers ${stoppedRole} role stopped while other roles were running`);
    }
    await Promise.all(running);
  }
} finally {
  process.off("SIGINT", onSignal);
  process.off("SIGTERM", onSignal);
}

function loadRole(role: PlatformWorkerLeafRole): Promise<unknown> {
  switch (role) {
    case "channels-workflows":
      return import("../../channels/src/runtime/workflow-worker.js");
    case "channels-activities":
      return import("../../channels/src/runtime/activity-worker.js");
    case "bots-workflows":
      return import("../../bots/src/runtime/workflow-worker.js");
    case "bots-activities":
      return import("../../bots/src/runtime/activity-worker.js");
    case "telegram":
      return import("../../channels/src/runtime/telegram-worker.js");
    case "whatsapp":
      return import("../../channels/src/runtime/whatsapp-worker.js");
  }
}

function metricsPort(channelRolesToRun: readonly PlatformWorkerLeafRole[]): number {
  if (channelRolesToRun.length > 1) return 9_090;
  switch (channelRolesToRun[0]) {
    case "channels-workflows":
      return 9_090;
    case "telegram":
      return 9_091;
    case "whatsapp":
      return 9_092;
    case "channels-activities":
      return 9_093;
    default:
      throw new TypeError("at least one Channels role is required for Channels metrics");
  }
}
