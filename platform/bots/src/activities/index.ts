export * from "./lightspeed.js";
export * from "./control-plane.js";

import type { BotControlPlaneActivities } from "./control-plane.js";
import type { BotLightspeedActivities } from "./lightspeed.js";

export type BotActivities = BotLightspeedActivities & BotControlPlaneActivities;
