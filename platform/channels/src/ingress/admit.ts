import type { WorkflowClient } from "@temporalio/client";
import { conversationLabel, type NormalizedInboundV1 } from "../contracts/channel.js";
import type {
  ChannelControlPlane,
  ResolvedChatTrigger,
} from "../control-plane/chat-triggers.js";
import { channelConversationIdentity } from "../identity/ids.js";
import { signalInbound, type SignalInboundResult } from "./signal.js";

export type AdmitInboundResult =
  | { status: "unbound" }
  | { status: "pairing_required" }
  | { status: "pairing_pending" }
  | { status: "paired"; trigger: ResolvedChatTrigger }
  | {
      status: "admitted";
      trigger: ResolvedChatTrigger;
      workflowId: string;
      signaledRunId: string;
    };

/**
 * Resolve the chat trigger for a provider message and hand the message to
 * its conversation workflow by signal-with-start. Provider acknowledgement
 * is safe only after this resolves.
 */
export async function admitInbound(
  client: WorkflowClient,
  controlPlane: ChannelControlPlane,
  inbound: NormalizedInboundV1,
): Promise<AdmitInboundResult> {
  const decision = await controlPlane.admit(inbound);
  if (decision.status !== "bound") {
    return decision;
  }
  const trigger = decision.trigger;
  const identity = channelConversationIdentity(trigger.universeId, inbound.route);
  const signaled: SignalInboundResult = await signalInbound(
    client,
    {
      version: 1,
      universeId: trigger.universeId,
      triggerId: trigger.triggerId,
      botId: trigger.botId,
      botName: trigger.botName,
      scope: inbound.isDirect ? "direct" : "group",
      activation: trigger.activation,
      access: trigger.access,
      route: inbound.route,
      label: conversationLabel(inbound),
      deliveryTaskQueue: identity.deliveryTaskQueue,
    },
    { ...inbound, authorization: trigger.authorization },
  );
  return { status: "admitted", trigger, ...signaled };
}

export const PAIRING_REQUIRED_REPLY =
  "This chat is not paired yet. Send the pairing code to connect it.";
export const PAIRING_CONFIRMED_REPLY =
  "Paired. You can now message Lightspeed from this chat.";
