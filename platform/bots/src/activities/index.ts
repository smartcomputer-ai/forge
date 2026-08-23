export * from "./lightspeed.js";
export * from "./control-plane.js";
export * from "./schedule.js";
export * from "./tools.js";

import type { BotControlPlaneActivities } from "./control-plane.js";
import type { BotLightspeedActivities } from "./lightspeed.js";
import type { BotScheduleActivities } from "./schedule.js";
import type { BotToolActivities } from "./tools.js";

export type BotActivities = BotLightspeedActivities &
  BotControlPlaneActivities &
  BotScheduleActivities &
  BotToolActivities;
