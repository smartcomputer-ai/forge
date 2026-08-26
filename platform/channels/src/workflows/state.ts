import type {
  ChannelConversationStartV1,
  NormalizedInboundV1,
} from "../contracts/channel.js";
import type { ChatHandleV1 } from "../contracts/bridge.js";
import type {
  EmissionEnvelope,
  WorkflowToolInvocation,
} from "@lightspeed/agent-client/workflow";

export interface ReceivedInvocation {
  invocation: WorkflowToolInvocation;
  holderWorkflowId: string;
  producerSessionId: string;
  status: "received" | "delivering" | "resolved" | "failed" | "cancelled";
  providerMessageIds?: string[];
  /** The number the send got (`chat.sent` row), when the tool was a send. */
  sentSeq?: number;
  resolutionEmissionIds?: string[];
  error?: string;
}

export type ApplyEmissionEffect =
  | { type: "invocation_received"; invocationId: string }
  | { type: "invocation_cancelled"; invocationId: string; promiseId: string }
  | { type: "none" };

/** One inbound provider message on its way into the bot. */
export interface ReceivedMessage {
  messageId: string;
  status: "emitting" | "emitted" | "archived" | "duplicate" | "refused" | "failed";
  /** The bot's event number, once admitted: the handle the model uses. */
  seq?: number;
  /** The routed session admission chose (logical base id). */
  sessionId?: string;
  error?: string;
}

/** A bot delivery this conversation was told about through `bot_delivery_v1`. */
export interface DeliveryRecord {
  status: "started" | "finished";
  sessionId: string;
  runId: string | null;
  /** The lane's finish status (`handled`, `run_failed`, `steered`, …). */
  outcome?: string;
  fallback?: {
    status: "reconciling" | "suppressed" | "delivered" | "failed";
    providerMessageIds?: string[];
    seq?: number;
    error?: string;
  };
}

export interface ConversationSnapshot {
  version: 1;
  triggerId: string;
  botName: string;
  conversationKey: string;
  /** CAS ref of this conversation's receiver-bound tool declarations. */
  toolsRef: string | null;
  inboundCount: number;
  duplicateInboundCount: number;
  duplicateEmissionCount: number;
  droppedInboundCount: number;
  overloadedInboundCount: number;
  deniedInboundCount: number;
  emittedCount: number;
  activation: ChannelConversationStartV1["activation"];
  access: ChannelConversationStartV1["access"];
  protocolErrors: string[];
  messages: Record<string, ReceivedMessage>;
  /** Message number → provider ids and direction, both ways. */
  handles: Record<string, ChatHandleV1>;
  deliveries: Record<string, DeliveryRecord>;
  policyResponses: Record<
    string,
    {
      kind: "control" | "denied";
      status: "delivering" | "delivered" | "failed";
      providerMessageIds?: string[];
      error?: string;
    }
  >;
  invocations: Record<string, ReceivedInvocation>;
  cancellations: string[];
}

export interface ChannelWorkflowState {
  snapshot: ConversationSnapshot;
  seenInboundIds: Set<string>;
  seenEmissionIds: Set<string>;
}

export interface ConversationCarryV1 {
  version: 1;
  snapshot: ConversationSnapshot;
  seenInboundIds: string[];
  seenEmissionIds: string[];
}

export const MAX_CHANNEL_HANDLES = 512;
export const MAX_CHANNEL_INBOUND_INBOX = 256;
const MAX_CARRIED_MESSAGES = 256;
const MAX_CARRIED_DELIVERIES = 128;

export function enqueueBoundedInbound<T extends NormalizedInboundV1>(
  state: ChannelWorkflowState,
  inbox: T[],
  inbound: T,
): boolean {
  if (inbox.length >= MAX_CHANNEL_INBOUND_INBOX) {
    state.snapshot.overloadedInboundCount += 1;
    return false;
  }
  inbox.push(inbound);
  return true;
}

export function initialWorkflowState(
  start: ChannelConversationStartV1,
  conversationKey: string,
  carry?: ConversationCarryV1,
): ChannelWorkflowState {
  if (carry !== undefined) {
    if (
      carry.version !== 1 ||
      carry.snapshot.triggerId !== start.triggerId ||
      carry.snapshot.conversationKey !== conversationKey
    ) {
      throw new TypeError("conversation workflow carry does not match the conversation");
    }
    return {
      snapshot: cloneSnapshot(carry.snapshot),
      seenInboundIds: new Set(carry.seenInboundIds),
      seenEmissionIds: new Set(carry.seenEmissionIds),
    };
  }
  return {
    snapshot: {
      version: 1,
      triggerId: start.triggerId,
      botName: start.botName,
      conversationKey,
      toolsRef: null,
      inboundCount: 0,
      duplicateInboundCount: 0,
      duplicateEmissionCount: 0,
      droppedInboundCount: 0,
      overloadedInboundCount: 0,
      deniedInboundCount: 0,
      emittedCount: 0,
      activation: {
        ...start.activation,
        triggerPrefixes: [...start.activation.triggerPrefixes],
        mentionNames: [...start.activation.mentionNames],
      },
      access: { ...start.access },
      protocolErrors: [],
      messages: {},
      handles: {},
      deliveries: {},
      policyResponses: {},
      invocations: {},
      cancellations: [],
    },
    seenInboundIds: new Set(),
    seenEmissionIds: new Set(),
  };
}

/** Keep the newest handles; older numbers still resolve through the activity. */
export function rememberHandle(state: ChannelWorkflowState, seq: number, handle: ChatHandleV1): void {
  state.snapshot.handles[String(seq)] = handle;
  const keys = Object.keys(state.snapshot.handles);
  for (const expired of keys.slice(0, Math.max(0, keys.length - MAX_CHANNEL_HANDLES))) {
    delete state.snapshot.handles[expired];
  }
}

export function compactWorkflowState(state: ChannelWorkflowState): ConversationCarryV1 {
  const compacted = cloneSnapshot(state.snapshot);
  compacted.protocolErrors = compacted.protocolErrors.slice(-32);
  compacted.cancellations = compacted.cancellations.slice(-256);
  compacted.invocations = retainRecent(
    compacted.invocations,
    (invocation) => invocation.status === "received" || invocation.status === "delivering",
    64,
  );
  compacted.messages = retainRecent(
    compacted.messages,
    (message) => message.status === "emitting",
    MAX_CARRIED_MESSAGES,
  );
  compacted.deliveries = retainRecent(
    compacted.deliveries,
    (delivery) => delivery.status === "started" || delivery.fallback?.status === "reconciling",
    MAX_CARRIED_DELIVERIES,
  );
  compacted.policyResponses = retainRecent(
    compacted.policyResponses,
    (response) => response.status === "delivering",
    64,
  );
  compacted.handles = Object.fromEntries(
    Object.entries(compacted.handles).slice(-MAX_CHANNEL_HANDLES),
  );
  return {
    version: 1,
    snapshot: compacted,
    seenInboundIds: [...state.seenInboundIds].slice(-2_048),
    seenEmissionIds: [...state.seenEmissionIds].slice(-2_048),
  };
}

export function applyInbound(
  state: ChannelWorkflowState,
  inbound: NormalizedInboundV1,
): string | null {
  const dedupKey = channelInboundKey(inbound);
  if (state.seenInboundIds.has(dedupKey)) {
    state.snapshot.duplicateInboundCount += 1;
    return null;
  }
  state.seenInboundIds.add(dedupKey);
  state.snapshot.inboundCount += 1;
  return dedupKey;
}

export function channelInboundKey(inbound: NormalizedInboundV1): string {
  return [
    inbound.route.provider,
    inbound.route.accountId,
    inbound.route.chatId,
    inbound.route.threadId ?? "",
    inbound.messageId,
  ].join("\0");
}

export function applyEmission(
  state: ChannelWorkflowState,
  envelope: EmissionEnvelope,
): ApplyEmissionEffect {
  if (state.seenEmissionIds.has(envelope.emission_id)) {
    state.snapshot.duplicateEmissionCount += 1;
    return { type: "none" };
  }
  state.seenEmissionIds.add(envelope.emission_id);

  switch (envelope.body.kind) {
    case "tool_invocation": {
      if (envelope.producer.kind !== "session") {
        state.snapshot.protocolErrors.push("tool invocation must be produced by a session");
        return { type: "none" };
      }
      const invocation = envelope.body.invocation;
      state.snapshot.invocations[invocation.invocation_id] = {
        invocation,
        holderWorkflowId: envelope.body.holder_workflow_id,
        producerSessionId: envelope.producer.session_id,
        status: "received",
      };
      return { type: "invocation_received", invocationId: invocation.invocation_id };
    }
    case "run_terminal":
      // The bot controller is the session's lifecycle controller; a terminal
      // here means a declaration named this workflow where it should not.
      state.snapshot.protocolErrors.push("conversation received a run terminal it does not own");
      return { type: "none" };
    case "invocation_cancellation":
      state.snapshot.cancellations.push(
        `${envelope.body.invocation_id}:${envelope.body.completion_key}`,
      );
      return {
        type: "invocation_cancelled",
        invocationId: envelope.body.invocation_id,
        promiseId: envelope.body.promise_id,
      };
    case "source_resolution":
      state.snapshot.protocolErrors.push("conversation received an unexpected source resolution");
      return { type: "none" };
  }
}

export function snapshot(state: ChannelWorkflowState): ConversationSnapshot {
  return cloneSnapshot(state.snapshot);
}

function cloneSnapshot(source: ConversationSnapshot): ConversationSnapshot {
  return {
    ...source,
    activation: {
      ...source.activation,
      triggerPrefixes: [...source.activation.triggerPrefixes],
      mentionNames: [...source.activation.mentionNames],
    },
    access: { ...source.access },
    protocolErrors: [...source.protocolErrors],
    messages: Object.fromEntries(
      Object.entries(source.messages).map(([key, message]) => [key, { ...message }]),
    ),
    handles: Object.fromEntries(
      Object.entries(source.handles).map(([key, handle]) => [
        key,
        { ...handle, providerMessageIds: [...handle.providerMessageIds] },
      ]),
    ),
    deliveries: Object.fromEntries(
      Object.entries(source.deliveries).map(([key, delivery]) => [
        key,
        {
          ...delivery,
          ...(delivery.fallback === undefined
            ? {}
            : {
                fallback: {
                  ...delivery.fallback,
                  ...(delivery.fallback.providerMessageIds === undefined
                    ? {}
                    : { providerMessageIds: [...delivery.fallback.providerMessageIds] }),
                },
              }),
        },
      ]),
    ),
    policyResponses: Object.fromEntries(
      Object.entries(source.policyResponses).map(([key, response]) => [
        key,
        {
          ...response,
          ...(response.providerMessageIds === undefined
            ? {}
            : { providerMessageIds: [...response.providerMessageIds] }),
        },
      ]),
    ),
    invocations: Object.fromEntries(
      Object.entries(source.invocations).map(([key, invocation]) => [
        key,
        {
          ...invocation,
          invocation: { ...invocation.invocation },
          ...(invocation.providerMessageIds === undefined
            ? {}
            : { providerMessageIds: [...invocation.providerMessageIds] }),
          ...(invocation.resolutionEmissionIds === undefined
            ? {}
            : { resolutionEmissionIds: [...invocation.resolutionEmissionIds] }),
        },
      ]),
    ),
    cancellations: [...source.cancellations],
  };
}

function retainRecent<T>(
  entries: Record<string, T>,
  keep: (entry: T) => boolean,
  limit: number,
): Record<string, T> {
  const list = Object.entries(entries);
  const kept = list.filter(([, entry]) => keep(entry));
  const rest = list.filter(([, entry]) => !keep(entry)).slice(-limit);
  return Object.fromEntries([...rest, ...kept]);
}
