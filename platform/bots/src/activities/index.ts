export * from "./lightspeed.js";
export * from "./control-plane.js";
export * from "./schedule.js";

import type { BotControlPlaneActivities } from "./control-plane.js";
import type { BotLightspeedActivities } from "./lightspeed.js";
import type { BotScheduleActivities } from "./schedule.js";

export type BotActivities = BotLightspeedActivities &
  BotControlPlaneActivities &
  BotScheduleActivities;
