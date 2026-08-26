import type { ChannelRoute } from "./channel.js";

export interface AssertTriggerActiveInput {
  triggerId: string;
  route: ChannelRoute;
  scope: "direct" | "group";
}

export interface ControlPlaneActivities {
  /** Fails non-retryably when the chat trigger no longer serves this conversation. */
  assertTriggerActive(input: AssertTriggerActiveInput): Promise<void>;
}
