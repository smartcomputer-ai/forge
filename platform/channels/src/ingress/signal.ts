import type { WorkflowClient } from "@temporalio/client";
import {
  CHANNEL_INBOUND_SIGNAL,
  CHANNEL_CONVERSATION_WORKFLOW,
  CHANNELS_WORKFLOW_TASK_QUEUE,
  type AdmittedChannelInboundV1,
  type ChannelConversationStartV1,
  parseAdmittedChannelInboundV1,
} from "../contracts/channel.js";
import { channelConversationIdentity } from "../identity/ids.js";
import { channelConversationSearchAttributes } from "../contracts/search-attributes.js";

type ChannelConversationWorkflow = (start: ChannelConversationStartV1) => Promise<never>;

export interface SignalInboundResult {
  workflowId: string;
  signaledRunId: string;
}

/**
 * Durably admit one already-authorized provider event into Channels.
 * Provider acknowledgement is safe only after this call resolves.
 */
export async function signalInbound(
  client: WorkflowClient,
  start: ChannelConversationStartV1,
  rawInbound: unknown,
): Promise<SignalInboundResult> {
  const inbound = parseAdmittedChannelInboundV1(rawInbound);
  const identity = channelConversationIdentity(start.universeId, start.route);
  assertIdentity(start, inbound, identity);

  const handle = await client.signalWithStart<
    ChannelConversationWorkflow,
    [AdmittedChannelInboundV1]
  >(CHANNEL_CONVERSATION_WORKFLOW, {
    workflowId: identity.workflowId,
    taskQueue: CHANNELS_WORKFLOW_TASK_QUEUE,
    args: [start],
    signal: CHANNEL_INBOUND_SIGNAL,
    signalArgs: [inbound],
    typedSearchAttributes: channelConversationSearchAttributes(start),
  });
  return { workflowId: identity.workflowId, signaledRunId: handle.signaledRunId };
}

function assertIdentity(
  start: ChannelConversationStartV1,
  inbound: AdmittedChannelInboundV1,
  expected: ReturnType<typeof channelConversationIdentity>,
): void {
  if (start.deliveryTaskQueue !== expected.deliveryTaskQueue) {
    throw new TypeError(`deliveryTaskQueue must be ${expected.deliveryTaskQueue}`);
  }
  if (start.scope !== (inbound.isDirect ? "direct" : "group")) {
    throw new TypeError("conversation scope must match the inbound route scope");
  }
  if (
    inbound.route.provider !== start.route.provider ||
    inbound.route.accountId !== start.route.accountId ||
    inbound.route.chatId !== start.route.chatId ||
    inbound.route.threadId !== start.route.threadId
  ) {
    throw new TypeError("inbound route must match the conversation route");
  }
}
