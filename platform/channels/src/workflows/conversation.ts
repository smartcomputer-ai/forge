import {
  CancellationScope,
  condition,
  continueAsNew,
  defineQuery,
  defineSignal,
  getExternalWorkflowHandle,
  isCancellation,
  metricMeter,
  proxyActivities,
  setHandler,
  workflowInfo,
} from "@temporalio/workflow";
import {
  DELIVER_EMISSION_SIGNAL,
  type EmissionEnvelope,
  type PromiseResolution,
  type WorkflowToolInvocation,
  parseEmissionEnvelope,
  replyPromiseId as joinedReplyPromiseId,
  sourceResolutionEnvelope,
} from "@lightspeed/agent-client/workflow";
import {
  BOT_DELIVERY_SIGNAL,
  botSessionId,
  type BotDeliveryReceiptV1,
} from "@lightspeed/bots/contracts";
import {
  CHANNEL_INBOUND_SIGNAL,
  CHANNELS_ACTIVITY_TASK_QUEUE,
  CHANNEL_CONVERSATION_WORKFLOW,
  CHANNEL_STATE_QUERY,
  type AdmittedChannelInboundV1,
  type ChannelConversationStartV1,
  parseAdmittedChannelInboundV1,
} from "../contracts/channel.js";
import type {
  BotBridgeActivities,
  ChatHandleV1,
  LightspeedActivities,
} from "../contracts/bridge.js";
import type { ControlPlaneActivities } from "../contracts/control-plane.js";
import {
  type ChannelDeliveryActivities,
  type ChannelDeliveryOperation,
  type ChannelToolOperation,
  parseToolOperation,
  validateDeliveryResult,
} from "../contracts/delivery.js";
import type { ChannelInputItem, ChannelMediaActivities } from "../contracts/media.js";
import type { ChannelPresenceActivities } from "../contracts/presence.js";
import { channelConversationIdentity } from "../identity/ids.js";
import { classifyInbound } from "../policy/activation.js";
import {
  parseChannelControlCommand,
  type ChannelControlCommand,
} from "../policy/control.js";
import { planDeliveryCommands } from "./delivery-plan.js";
import {
  applyEmission,
  applyInbound,
  compactWorkflowState,
  enqueueBoundedInbound,
  initialWorkflowState,
  rememberHandle,
  snapshot,
  type ConversationCarryV1,
  type ConversationSnapshot,
} from "./state.js";

export interface ChannelConversationWorkflowInputV1 extends ChannelConversationStartV1 {
  carry?: ConversationCarryV1;
}

export const deliverEmissionSignal = defineSignal<[unknown]>(DELIVER_EMISSION_SIGNAL);
export const channelInboundSignal = defineSignal<[unknown]>(CHANNEL_INBOUND_SIGNAL);
export const botDeliverySignal = defineSignal<[unknown]>(BOT_DELIVERY_SIGNAL);
export const channelStateQuery = defineQuery<ConversationSnapshot>(CHANNEL_STATE_QUERY);

/** Lane finish statuses that never produced a run to reconcile. */
const NO_RUN_STATUSES = new Set(["steered", "appended"]);

const lightspeed = proxyActivities<LightspeedActivities>({
  taskQueue: CHANNELS_ACTIVITY_TASK_QUEUE,
  startToCloseTimeout: "30 seconds",
  retry: {
    initialInterval: "1 second",
    backoffCoefficient: 2,
    maximumInterval: "30 seconds",
  },
});
const bridge = proxyActivities<BotBridgeActivities>({
  taskQueue: CHANNELS_ACTIVITY_TASK_QUEUE,
  startToCloseTimeout: "30 seconds",
  retry: {
    initialInterval: "1 second",
    backoffCoefficient: 2,
    maximumInterval: "30 seconds",
  },
});
const { assertTriggerActive } = proxyActivities<ControlPlaneActivities>({
  taskQueue: CHANNELS_ACTIVITY_TASK_QUEUE,
  startToCloseTimeout: "15 seconds",
  retry: {
    initialInterval: "1 second",
    backoffCoefficient: 2,
    maximumInterval: "15 seconds",
  },
});

/** A message number the model used that this conversation cannot resolve. */
class UnknownHandleError extends Error {
  constructor(seq: number, maxSeq: number) {
    super(
      maxSeq > 0
        ? `unknown message #${seq} in this conversation; this bot's messages run #1..#${maxSeq}`
        : `unknown message #${seq}; this bot has no messages yet`,
    );
    this.name = "UnknownHandleError";
  }
}

/**
 * One durable conversation: the provider side of a bot's `chat` trigger.
 *
 * Every activated message becomes a bot event through admission; the bot's
 * routed session for this conversation carries this workflow as the receiver
 * of its `message_*` tools; the bot controller reports `started` / `finished`
 * receipts here for typing and the reply fallback. The workflow never creates
 * a session, starts a run, or holds a lifecycle role.
 *
 * Signal handlers only validate and enqueue. The dispatcher consumes inbound,
 * tool, cancellation, and receipt work independently.
 */
export async function channelConversationWorkflowV1(
  start: ChannelConversationWorkflowInputV1,
): Promise<never> {
  validateStart(start);
  if (workflowInfo().workflowType !== CHANNEL_CONVERSATION_WORKFLOW) {
    throw new TypeError(`unexpected workflow type: ${workflowInfo().workflowType}`);
  }

  const info = workflowInfo();
  const identity = channelConversationIdentity(start.universeId, start.route);
  const receiver = { workflowId: info.workflowId, workflowKind: CHANNEL_CONVERSATION_WORKFLOW };
  // Routed sessions of this bot: `bot:v1:<bot>:k-…`, generations included.
  const sessionPrefix = `${botSessionId(start.botName)}:`;
  const workflowMetrics = metricMeter.withTags({
    provider: start.route.provider,
    account_id: start.route.accountId,
    scope: start.scope,
  });
  const inboundMetric = workflowMetrics.createCounter(
    "channels_inbound",
    undefined,
    "Inbound channel messages by workflow outcome.",
  );
  const receiptMetric = workflowMetrics.createCounter(
    "channels_delivery_receipt",
    undefined,
    "Bot delivery receipts and reply fallback outcomes.",
  );
  const promiseMetric = workflowMetrics.createCounter(
    "channels_promise_resolution",
    undefined,
    "Runtime-owned workflow-tool reply outcomes and cancellation facts.",
  );
  const activeMetric = workflowMetrics.createGauge(
    "channels_workflow_active",
    "int",
    undefined,
    "Whether this conversation workflow execution is active.",
  );
  const workflowMetricTags = { workflow_id: info.workflowId, run_id: info.runId };
  activeMetric.set(1, workflowMetricTags);

  const state = initialWorkflowState(start, identity.conversationKey, start.carry);
  const inboundInbox: AdmittedChannelInboundV1[] = [];
  const emissionInbox: EmissionEnvelope[] = [];
  const receiptInbox: BotDeliveryReceiptV1[] = [];
  let backgroundTaskCount = 0;
  const deliveryScopes = new Map<string, CancellationScope>();
  const typingScopes = new Map<string, CancellationScope>();
  const { deliverChannelMessage } = proxyActivities<ChannelDeliveryActivities>({
    taskQueue: start.deliveryTaskQueue,
    startToCloseTimeout: "30 seconds",
    scheduleToCloseTimeout: "110 seconds",
    retry: {
      initialInterval: "1 second",
      backoffCoefficient: 2,
      maximumInterval: "15 seconds",
    },
  });
  const { prepareChannelMedia } = proxyActivities<ChannelMediaActivities>({
    taskQueue: start.deliveryTaskQueue,
    startToCloseTimeout: "90 seconds",
    scheduleToCloseTimeout: "5 minutes",
    retry: {
      initialInterval: "1 second",
      backoffCoefficient: 2,
      maximumInterval: "30 seconds",
    },
  });
  const { maintainChannelTyping } = proxyActivities<ChannelPresenceActivities>({
    taskQueue: start.deliveryTaskQueue,
    startToCloseTimeout: "24 hours",
    heartbeatTimeout: "15 seconds",
    retry: {
      initialInterval: "1 second",
      backoffCoefficient: 2,
      maximumInterval: "15 seconds",
    },
  });

  setHandler(channelInboundSignal, (raw) => {
    try {
      const inbound = parseAdmittedChannelInboundV1(raw);
      if (!enqueueBoundedInbound(state, inboundInbox, inbound)) {
        inboundMetric.add(1, { outcome: "overloaded" });
      }
    } catch (error) {
      state.snapshot.protocolErrors.push(errorMessage(error));
    }
  });
  setHandler(deliverEmissionSignal, (raw) => {
    try {
      emissionInbox.push(parseEmissionEnvelope(raw));
    } catch (error) {
      state.snapshot.protocolErrors.push(errorMessage(error));
    }
  });
  setHandler(botDeliverySignal, (raw) => {
    try {
      receiptInbox.push(parseDeliveryReceipt(raw));
    } catch (error) {
      state.snapshot.protocolErrors.push(errorMessage(error));
    }
  });
  setHandler(channelStateQuery, () => snapshot(state));

  try {
    if (state.snapshot.toolsRef === null) {
      // Content-addressed and receiver-specific: the same bytes for every
      // event of this conversation, so the routed session's declaration
      // fingerprint never drifts.
      const put = await lightspeed.putChatToolDeclarations({
        universeId: start.universeId,
        receiver,
      });
      state.snapshot.toolsRef = put.toolsRef;
    }

    for (;;) {
      if (inboundInbox.length === 0 && emissionInbox.length === 0 && receiptInbox.length === 0) {
        await condition(
          () => inboundInbox.length > 0 || emissionInbox.length > 0 || receiptInbox.length > 0,
        );
      }
      for (const inbound of inboundInbox.splice(0)) {
        const inboundKey = applyInbound(state, inbound);
        if (inboundKey === null) {
          inboundMetric.add(1, { outcome: "deduplicated" });
          continue;
        }
        inboundMetric.add(1, { outcome: "accepted" });
        const control = parseChannelControlCommand(inbound.text);
        if (control !== null) {
          if (!inbound.authorization.controlAllowed) {
            await handleDeniedInbound(inboundKey, inbound);
          } else {
            await handleControlCommand(inboundKey, inbound, control);
          }
          continue;
        }
        if (!inbound.authorization.turnAllowed) {
          await handleDeniedInbound(inboundKey, inbound);
          continue;
        }
        const classification = classifyInbound(inbound, state.snapshot.activation);
        if (classification.kind === "drop") {
          state.snapshot.droppedInboundCount += 1;
          inboundMetric.add(1, { outcome: "dropped" });
          continue;
        }
        let mediaItems: Array<Extract<ChannelInputItem, { type: "media" }>>;
        try {
          mediaItems = await Promise.all(
            (inbound.media ?? []).map(async (media) => {
              const prepared = await prepareChannelMedia({
                universeId: start.universeId,
                route: inbound.route,
                media,
              });
              return prepared.item;
            }),
          );
        } catch (error) {
          state.snapshot.messages[inboundKey] = {
            messageId: inbound.messageId,
            status: "failed",
            error: errorMessage(error),
          };
          state.snapshot.protocolErrors.push(`media ${inbound.messageId}: ${errorMessage(error)}`);
          continue;
        }
        await emitMessage(inboundKey, inbound, classification.text, mediaItems);
      }
      for (const emission of emissionInbox.splice(0)) {
        handleEmission(emission);
      }
      for (const receipt of receiptInbox.splice(0)) {
        handleReceipt(receipt);
      }
      if (
        workflowInfo().continueAsNewSuggested &&
        inboundInbox.length === 0 &&
        emissionInbox.length === 0 &&
        receiptInbox.length === 0 &&
        deliveryScopes.size === 0 &&
        typingScopes.size === 0 &&
        backgroundTaskCount === 0
      ) {
        await continueAsNew<typeof channelConversationWorkflowV1>({
          ...start,
          carry: compactWorkflowState(state),
        });
      }
    }
  } finally {
    activeMetric.set(0, workflowMetricTags);
  }

  /** One activated message → one bot event; the number it gets is its handle. */
  async function emitMessage(
    inboundKey: string,
    inbound: AdmittedChannelInboundV1,
    text: string,
    mediaItems: Array<Extract<ChannelInputItem, { type: "media" }>>,
  ): Promise<void> {
    const toolsRef = state.snapshot.toolsRef;
    if (toolsRef === null) {
      throw new TypeError("tool declarations must be stored before emitting");
    }
    state.snapshot.messages[inboundKey] = { messageId: inbound.messageId, status: "emitting" };
    try {
      const result = await bridge.emitChatEvent({
        universeId: start.universeId,
        triggerId: start.triggerId,
        botId: start.botId,
        conversation: {
          key: identity.conversationKey,
          label: start.label,
          scope: start.scope,
          route: start.route,
        },
        message: {
          messageId: inbound.messageId,
          senderId: inbound.senderId,
          senderName: inbound.senderName,
          memberRole: inbound.authorization.memberRole,
          timestampMs: inbound.timestampMs,
          text,
          isDirect: inbound.isDirect,
          mentionedBot: inbound.mentionedBot,
          isReplyToBot: inbound.isReplyToBot,
        },
        media: mediaItems,
        toolsRef,
        // The token is the inbound key: receipts for a coalesced batch name
        // every message in it, and the delivery id dedupes the fallback.
        notify: { ...receiver, token: inboundKey },
      });
      if (result.status === "refused") {
        state.snapshot.messages[inboundKey] = {
          messageId: inbound.messageId,
          status: "refused",
          error: result.reason,
        };
        inboundMetric.add(1, { outcome: "refused" });
        return;
      }
      state.snapshot.messages[inboundKey] = {
        messageId: inbound.messageId,
        status: result.status === "admitted" ? "emitted" : result.status,
        ...(result.seq === null ? {} : { seq: result.seq }),
        ...(result.sessionId === null ? {} : { sessionId: result.sessionId }),
      };
      if (result.seq !== null) {
        rememberHandle(state, result.seq, {
          providerMessageIds: [inbound.messageId],
          fromMe: false,
          senderId: inbound.senderId,
          text: inbound.text,
        });
      }
      if (result.status === "admitted") state.snapshot.emittedCount += 1;
      inboundMetric.add(1, { outcome: result.status });
    } catch (error) {
      state.snapshot.messages[inboundKey] = {
        messageId: inbound.messageId,
        status: "failed",
        error: errorMessage(error),
      };
      state.snapshot.protocolErrors.push(`emit ${inbound.messageId}: ${errorMessage(error)}`);
      inboundMetric.add(1, { outcome: "failed" });
    }
  }

  function handleEmission(emission: EmissionEnvelope): void {
    const effect = applyEmission(state, emission);
    if (effect.type === "invocation_received") {
      const entry = state.snapshot.invocations[effect.invocationId];
      if (entry === undefined) {
        state.snapshot.protocolErrors.push(`missing invocation ${effect.invocationId}`);
        return;
      }
      let replyPromiseId: string;
      try {
        validateInvocationOwner(entry.invocation);
        replyPromiseId = joinedReplyPromiseId(entry.invocation);
      } catch (error) {
        entry.status = "failed";
        entry.error = errorMessage(error);
        state.snapshot.protocolErrors.push(
          `invalid invocation ${effect.invocationId}: ${errorMessage(error)}`,
        );
        return;
      }
      const cancellation = state.snapshot.cancellations.find((fact) =>
        fact.startsWith(`${effect.invocationId}:`),
      );
      if (cancellation !== undefined) {
        entry.status = "cancelled";
        backgroundTaskCount += 1;
        void CancellationScope.nonCancellable(async () => {
          entry.resolutionEmissionIds = await resolvePromises(
            entry.holderWorkflowId,
            [replyPromiseId],
            { kind: "cancelled" },
          );
          promiseMetric.add(1, { outcome: "cancelled" });
        })
          .catch((error) => {
            entry.error = errorMessage(error);
            state.snapshot.protocolErrors.push(
              `cancel invocation ${effect.invocationId}: ${errorMessage(error)}`,
            );
          })
          .finally(() => {
            backgroundTaskCount -= 1;
          });
        return;
      }
      entry.status = "delivering";
      const scope = new CancellationScope({ cancellable: true });
      deliveryScopes.set(effect.invocationId, scope);
      backgroundTaskCount += 1;
      void scope
        .run(() => processInvocation(entry.invocation, entry.holderWorkflowId, replyPromiseId))
        .then(({ messageIds, resolutionEmissionIds, sentSeq }) => {
          entry.status = "resolved";
          entry.providerMessageIds = messageIds;
          entry.resolutionEmissionIds = resolutionEmissionIds;
          if (sentSeq !== undefined) entry.sentSeq = sentSeq;
        })
        .catch(async (error) => {
          if (isCancellation(error)) {
            entry.status = "cancelled";
            entry.resolutionEmissionIds = await CancellationScope.nonCancellable(() =>
              resolvePromises(entry.holderWorkflowId, [replyPromiseId], { kind: "cancelled" }),
            );
            promiseMetric.add(1, { outcome: "cancelled" });
            return;
          }
          entry.status = "failed";
          entry.error = errorMessage(error);
          state.snapshot.protocolErrors.push(
            `invocation ${effect.invocationId}: ${errorMessage(error)}`,
          );
        })
        .finally(() => {
          deliveryScopes.delete(effect.invocationId);
          backgroundTaskCount -= 1;
        })
        .catch((error) => {
          entry.error = errorMessage(error);
          state.snapshot.protocolErrors.push(
            `invocation cleanup ${effect.invocationId}: ${errorMessage(error)}`,
          );
        });
    }
    if (effect.type === "invocation_cancelled") {
      promiseMetric.add(1, { outcome: "cancellation_received" });
      deliveryScopes.get(effect.invocationId)?.cancel();
    }
  }

  /**
   * The bot controller's word on a delivery: typing while the run is up, and
   * once it finished, the reply fallback when the model answered in text
   * without a messaging tool. One fallback per delivery, whatever the batch.
   */
  function handleReceipt(receipt: BotDeliveryReceiptV1): void {
    const known = state.snapshot.deliveries[receipt.deliveryId];
    if (receipt.phase === "started") {
      if (known !== undefined) return;
      state.snapshot.deliveries[receipt.deliveryId] = {
        status: "started",
        sessionId: receipt.sessionId,
        runId: receipt.runId,
      };
      receiptMetric.add(1, { outcome: "started" });
      startTyping(receipt.deliveryId);
      return;
    }
    if (known?.status === "finished") return;
    const outcome = receipt.status ?? "unknown";
    const record = {
      status: "finished" as const,
      sessionId: receipt.sessionId,
      runId: receipt.runId,
      outcome,
    };
    state.snapshot.deliveries[receipt.deliveryId] = record;
    typingScopes.get(receipt.deliveryId)?.cancel();
    receiptMetric.add(1, { outcome: "finished", status: outcome });
    if (NO_RUN_STATUSES.has(outcome)) return;
    state.snapshot.deliveries[receipt.deliveryId] = {
      ...record,
      fallback: { status: "reconciling" },
    };
    backgroundTaskCount += 1;
    void lightspeed
      .reconcileDelivery({
        universeId: start.universeId,
        sessionId: receipt.sessionId,
        runId: receipt.runId,
        status: outcome,
      })
      .then(async (reconciliation) => {
        if (reconciliation.action === "suppress") {
          setFallback(receipt.deliveryId, { status: "suppressed" });
          receiptMetric.add(1, { outcome: "fallback_suppressed", reason: reconciliation.reason });
          return;
        }
        await assertTriggerActive({
          triggerId: start.triggerId,
          route: start.route,
          scope: start.scope,
        });
        const invocationId = `fallback:${receipt.deliveryId}`;
        const delivered = await deliverPlanned(invocationId, {
          type: "send",
          text: reconciliation.text,
        });
        const stored = await bridge.storeChatSent({
          universeId: start.universeId,
          triggerId: start.triggerId,
          botId: start.botId,
          conversation: { key: identity.conversationKey, label: start.label, route: start.route },
          invocationId,
          text: reconciliation.text,
          providerMessageIds: delivered.messageIds,
          replyTo: null,
        });
        if (stored.seq !== null) {
          rememberHandle(state, stored.seq, {
            providerMessageIds: delivered.messageIds,
            fromMe: true,
            text: reconciliation.text,
          });
        }
        setFallback(receipt.deliveryId, {
          status: "delivered",
          providerMessageIds: delivered.messageIds,
          ...(stored.seq === null ? {} : { seq: stored.seq }),
        });
        receiptMetric.add(1, { outcome: "fallback_delivered" });
      })
      .catch((error) => {
        setFallback(receipt.deliveryId, { status: "failed", error: errorMessage(error) });
        receiptMetric.add(1, { outcome: "fallback_failed" });
        state.snapshot.protocolErrors.push(
          `delivery ${receipt.deliveryId}: ${errorMessage(error)}`,
        );
      })
      .finally(() => {
        backgroundTaskCount -= 1;
      });
  }

  function setFallback(
    deliveryId: string,
    fallback: NonNullable<ConversationSnapshot["deliveries"][string]["fallback"]>,
  ): void {
    const record = state.snapshot.deliveries[deliveryId];
    if (record === undefined) return;
    state.snapshot.deliveries[deliveryId] = { ...record, fallback };
  }

  async function handleDeniedInbound(
    inboundKey: string,
    inbound: AdmittedChannelInboundV1,
  ): Promise<void> {
    state.snapshot.deniedInboundCount += 1;
    inboundMetric.add(1, { outcome: "denied" });
    if (!inbound.isDirect) {
      return;
    }
    await sendPolicyResponse(
      inboundKey,
      inbound,
      "denied",
      "This channel identity is not authorized for this Lightspeed universe.",
    );
  }

  async function handleControlCommand(
    inboundKey: string,
    inbound: AdmittedChannelInboundV1,
    command: ChannelControlCommand,
  ): Promise<void> {
    let text: string;
    if (command.kind === "activation") {
      if (start.scope === "direct") {
        text = "Direct chats are always active; /activation applies to groups.";
      } else {
        state.snapshot.activation.mode = command.mode;
        text = `Group activation is now ${command.mode}.`;
      }
    } else if (command.kind === "activation_help") {
      text = "Usage: /activation mention|always";
    } else {
      text = [
        `bot: ${start.botName}`,
        `activation: ${state.snapshot.activation.mode}`,
        `messages: ${state.snapshot.emittedCount} delivered to the bot`,
        "commands: /activation mention|always, /status",
      ].join("\n");
    }
    await sendPolicyResponse(inboundKey, inbound, "control", text);
  }

  /** Channels-authored replies (pairing, denial, /status) go straight out; they are not chat events. */
  async function sendPolicyResponse(
    inboundKey: string,
    inbound: AdmittedChannelInboundV1,
    kind: "control" | "denied",
    text: string,
  ): Promise<void> {
    const response = { kind, status: "delivering" as const };
    state.snapshot.policyResponses[inboundKey] = response;
    try {
      await assertTriggerActive({
        triggerId: start.triggerId,
        route: start.route,
        scope: start.scope,
      });
      const delivered = await deliverPlanned(`policy:${inbound.messageId}`, {
        type: "send",
        text,
        replyTo: inbound.messageId,
        replyContext: { senderId: inbound.senderId, text: inbound.text },
      });
      state.snapshot.policyResponses[inboundKey] = {
        kind,
        status: "delivered",
        providerMessageIds: delivered.messageIds,
      };
    } catch (error) {
      state.snapshot.policyResponses[inboundKey] = {
        kind,
        status: "failed",
        error: errorMessage(error),
      };
      state.snapshot.protocolErrors.push(`policy response ${inbound.messageId}: ${errorMessage(error)}`);
    }
  }

  function startTyping(deliveryId: string): void {
    if (typingScopes.has(deliveryId)) return;
    const scope = new CancellationScope({ cancellable: true });
    typingScopes.set(deliveryId, scope);
    backgroundTaskCount += 1;
    void scope
      .run(() => maintainChannelTyping({ route: start.route }))
      .catch((error) => {
        if (!isCancellation(error)) {
          state.snapshot.protocolErrors.push(`typing ${deliveryId}: ${errorMessage(error)}`);
        }
      })
      .finally(() => {
        typingScopes.delete(deliveryId);
        backgroundTaskCount -= 1;
      });
  }

  /** A message number → provider ids and direction: the workflow's cache first, the archive second. */
  async function resolveHandle(seq: number): Promise<ChatHandleV1> {
    const cached = state.snapshot.handles[String(seq)];
    if (cached !== undefined) return cached;
    const resolved = await bridge.resolveChatHandle({
      universeId: start.universeId,
      botId: start.botId,
      conversationKey: identity.conversationKey,
      seq,
    });
    if (resolved.handle === null) throw new UnknownHandleError(seq, resolved.maxSeq);
    rememberHandle(state, seq, resolved.handle);
    return resolved.handle;
  }

  /** What the model asked for, in provider terms. */
  async function toDeliveryOperation(
    operation: ChannelToolOperation,
  ): Promise<ChannelDeliveryOperation> {
    switch (operation.type) {
      case "send": {
        if (operation.replyTo === null) return { type: "send", text: operation.text };
        const target = await resolveHandle(operation.replyTo);
        const anchor = target.providerMessageIds[0];
        if (anchor === undefined) throw new UnknownHandleError(operation.replyTo, 0);
        return {
          type: "send",
          text: operation.text,
          replyTo: anchor,
          // Quoting needs the original author and text on WhatsApp; the
          // bot's own messages quote without context.
          ...(target.fromMe || target.senderId === undefined
            ? {}
            : { replyContext: { senderId: target.senderId, text: target.text ?? "" } }),
        };
      }
      case "edit": {
        const target = await resolveHandle(operation.message);
        const anchor = target.providerMessageIds[0];
        if (anchor === undefined) throw new UnknownHandleError(operation.message, 0);
        if (!target.fromMe) {
          throw new TypeError(`message #${operation.message} is not yours to edit; only your own sends can be edited`);
        }
        return { type: "edit", messageId: anchor, text: operation.text };
      }
      case "react": {
        const target = await resolveHandle(operation.message);
        const anchor = target.providerMessageIds[0];
        if (anchor === undefined) throw new UnknownHandleError(operation.message, 0);
        return { type: "react", messageId: anchor, emoji: operation.emoji, fromMe: target.fromMe };
      }
    }
  }

  async function processInvocation(
    invocation: WorkflowToolInvocation,
    holderWorkflowId: string,
    replyPromiseId: string,
  ): Promise<{ messageIds: string[]; resolutionEmissionIds: string[]; sentSeq?: number }> {
    try {
      await assertTriggerActive({
        triggerId: start.triggerId,
        route: start.route,
        scope: start.scope,
      });
      const rawArguments = await lightspeed.readJsonBlob({
        universeId: start.universeId,
        blobRef: invocation.arguments_ref,
      });
      const requested = parseToolOperation(invocation.tool_id, rawArguments);
      const operation = await toDeliveryOperation(requested);
      const delivered = await deliverPlanned(invocation.invocation_id, operation);
      let receiptValue: { sent: number } | { message: number };
      let sentSeq: number | undefined;
      if (requested.type === "send") {
        const stored = await bridge.storeChatSent({
          universeId: start.universeId,
          triggerId: start.triggerId,
          botId: start.botId,
          conversation: { key: identity.conversationKey, label: start.label, route: start.route },
          invocationId: invocation.invocation_id,
          text: requested.text,
          providerMessageIds: delivered.messageIds,
          replyTo: requested.replyTo,
        });
        if (stored.seq === null) {
          throw new Error("the message was sent but could not be recorded; it has no number");
        }
        rememberHandle(state, stored.seq, {
          providerMessageIds: delivered.messageIds,
          fromMe: true,
          text: requested.text,
        });
        sentSeq = stored.seq;
        receiptValue = { sent: stored.seq };
      } else {
        receiptValue = { message: requested.message };
      }
      const receipt = await lightspeed.putJsonBlob({
        universeId: start.universeId,
        value: receiptValue,
      });
      const resolutionEmissionIds = await resolvePromises(holderWorkflowId, [replyPromiseId], {
        kind: "resolved",
        payload_ref: receipt.blobRef,
      });
      promiseMetric.add(1, { outcome: "resolved" });
      return {
        messageIds: delivered.messageIds,
        resolutionEmissionIds,
        ...(sentSeq === undefined ? {} : { sentSeq }),
      };
    } catch (error) {
      if (isCancellation(error)) {
        throw error;
      }
      const errorRef = await putErrorBlob(error);
      await resolvePromises(holderWorkflowId, [replyPromiseId], {
        kind: "failed",
        error_ref: errorRef,
      });
      promiseMetric.add(1, { outcome: "failed" });
      throw error;
    }
  }

  async function deliverPlanned(
    invocationId: string,
    operation: ChannelDeliveryOperation,
  ): Promise<{ provider: ChannelConversationStartV1["route"]["provider"]; messageIds: string[] }> {
    const messageIds: string[] = [];
    for (const command of planDeliveryCommands(invocationId, start.route, operation)) {
      const result = validateDeliveryResult(
        await deliverChannelMessage(command),
        start.route.provider,
      );
      messageIds.push(...result.messageIds);
    }
    return { provider: start.route.provider, messageIds };
  }

  async function putErrorBlob(error: unknown): Promise<string | null> {
    try {
      return (
        await lightspeed.putJsonBlob({
          universeId: start.universeId,
          value: { error: errorMessage(error) },
        })
      ).blobRef;
    } catch {
      return null;
    }
  }

  async function resolvePromises(
    holderWorkflowId: string,
    promiseIds: string[],
    resolution: PromiseResolution,
  ): Promise<string[]> {
    const holder = getExternalWorkflowHandle(holderWorkflowId);
    const emissionIds: string[] = [];
    for (const promiseId of promiseIds) {
      const envelope = sourceResolutionEnvelope({
        universeId: start.universeId,
        producerWorkflowId: info.workflowId,
        holderWorkflowId,
        promiseId,
        resolution,
      });
      await holder.signal(DELIVER_EMISSION_SIGNAL, envelope);
      emissionIds.push(envelope.emission_id);
    }
    return emissionIds;
  }

  /** Only this bot's routed sessions may push here; the core enforces the receiver, this checks the bot. */
  function validateInvocationOwner(invocation: WorkflowToolInvocation): void {
    if (
      invocation.session_universe_id !== start.universeId ||
      !invocation.session_id.startsWith(sessionPrefix)
    ) {
      throw new TypeError("pushed invocation does not belong to this bot's conversation sessions");
    }
  }
}

function parseDeliveryReceipt(value: unknown): BotDeliveryReceiptV1 {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TypeError("delivery receipt must be an object");
  }
  const receipt = value as Record<string, unknown>;
  if (receipt.version !== 1) throw new TypeError("delivery receipt version must be 1");
  if (receipt.phase !== "started" && receipt.phase !== "finished") {
    throw new TypeError("delivery receipt phase is invalid");
  }
  for (const key of ["token", "deliveryId", "sessionId"] as const) {
    if (typeof receipt[key] !== "string" || receipt[key].length === 0) {
      throw new TypeError(`delivery receipt ${key} must be a non-empty string`);
    }
  }
  if (receipt.runId !== null && typeof receipt.runId !== "string") {
    throw new TypeError("delivery receipt runId must be a string or null");
  }
  if (!Array.isArray(receipt.seqs)) throw new TypeError("delivery receipt seqs must be an array");
  return receipt as unknown as BotDeliveryReceiptV1;
}

function validateStart(start: ChannelConversationStartV1): void {
  if (start.version !== 1) {
    throw new TypeError("conversation start version must be 1");
  }
  if (start.scope !== "direct" && start.scope !== "group") {
    throw new TypeError("conversation scope must be direct or group");
  }
  if (
    (start.scope === "direct" && start.activation.mode !== "dm") ||
    (start.scope === "group" && start.activation.mode === "dm")
  ) {
    throw new TypeError("chat activation mode must match the conversation scope");
  }
  if (
    (start.access.turn !== "conversation" && start.access.turn !== "members") ||
    (start.access.control !== "none" &&
      start.access.control !== "members" &&
      start.access.control !== "admins" &&
      start.access.control !== "owners")
  ) {
    throw new TypeError("chat access policy is invalid");
  }
  for (const [key, value] of Object.entries({
    universeId: start.universeId,
    triggerId: start.triggerId,
    botId: start.botId,
    botName: start.botName,
    label: start.label,
    deliveryTaskQueue: start.deliveryTaskQueue,
  })) {
    if (typeof value !== "string" || value.length === 0) {
      throw new TypeError(`${key} must be a non-empty string`);
    }
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
