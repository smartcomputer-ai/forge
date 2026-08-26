export * from "./lightspeed.js";
export * from "./control-plane.js";
export * from "./schedule.js";
export * from "./poll.js";
export * from "./tools.js";
export * from "./federation.js";

import type { BotControlPlaneActivities } from "./control-plane.js";
import type { BotFederationActivities } from "./federation.js";
import type { BotLightspeedActivities } from "./lightspeed.js";
import type { BotPollActivities } from "./poll.js";
import type { BotScheduleActivities } from "./schedule.js";
import type { BotToolActivities } from "./tools.js";

export type BotActivities = BotLightspeedActivities &
  BotControlPlaneActivities &
  BotScheduleActivities &
  BotPollActivities &
  BotToolActivities &
  BotFederationActivities;
