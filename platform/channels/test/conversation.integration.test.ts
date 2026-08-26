import { fileURLToPath } from "node:url";
import { TestWorkflowEnvironment } from "@temporalio/testing";
import { Worker } from "@temporalio/worker";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import {
  sessionWorkflowId,
  type EmissionEnvelope,
} from "@lightspeed/agent-client/workflow";
import { BOT_DELIVERY_SIGNAL, type BotDeliveryReceiptV1 } from "@lightspeed/bots/contracts";
import { createFakeDeliveryActivities } from "../src/activities/fake-delivery.js";
import type {
  EmitChatEventInput,
  ReconcileDeliveryInput,
  ResolveChatHandleInput,
  StoreChatSentInput,
} from "../src/contracts/bridge.js";
import {
  CHANNEL_INBOUND_SIGNAL,
  CHANNEL_CONVERSATION_WORKFLOW,
  CHANNEL_STATE_QUERY,
  CHANNELS_ACTIVITY_TASK_QUEUE,
  CHANNELS_WORKFLOW_TASK_QUEUE,
  type ChannelConversationStartV1,
} from "../src/contracts/channel.js";
import type { ChannelDeliveryCommandV1 } from "../src/contracts/delivery.js";
import type { PrepareChannelMediaInput } from "../src/contracts/media.js";
import { channelConversationIdentity } from "../src/identity/ids.js";
import { channelInboundKey, type ConversationSnapshot } from "../src/workflows/state.js";

const runIntegration = process.env.LIGHTSPEED_CHANNELS_TEMPORAL_INTEGRATION === "1";
const universeId = "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f";
const triggerId = "7f1c4a9e-2b3d-4c5e-8f6a-1b2c3d4e5f60";
const botId = "0b54d227-08a2-45a8-9b3f-6a4c21d1a222";
const routedSessionId = "bot:v1:concierge:k-telegram-primary-123-0123abcd";
const toolsRef = `sha256:${"7".repeat(64)}`;
const receiptRef = `sha256:${"b".repeat(64)}`;

describe.runIf(runIntegration)("Channels conversation workflow", () => {
  let env: TestWorkflowEnvironment;

  beforeAll(async () => {
    env = await TestWorkflowEnvironment.createLocal();
  }, 120_000);

  afterAll(async () => {
    await env.teardown();
  });

  it("emits a chat turn into the bot, answers a pushed send by number, and sends the fallback reply", async () => {
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: CHANNELS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const fakeDelivery = createFakeDeliveryActivities();
    const emitted: EmitChatEventInput[] = [];
    const stored: StoreChatSentInput[] = [];
    const resolvedHandles: ResolveChatHandleInput[] = [];
    const reconciled: ReconcileDeliveryInput[] = [];
    const deliveries: ChannelDeliveryCommandV1[] = [];
    const preparedMedia: PrepareChannelMediaInput[] = [];
    let nextSeq = 17;
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: CHANNELS_ACTIVITY_TASK_QUEUE,
      activities: {
        putChatToolDeclarations: async () => ({ toolsRef, toolIds: ["channels.message_send.v1"] }),
        readJsonBlob: async () => ({ text: `**reply** ${"x".repeat(3_600)}`, replyTo: 17 }),
        putJsonBlob: async () => ({ blobRef: receiptRef }),
        reconcileDelivery: async (input: ReconcileDeliveryInput) => {
          reconciled.push(input);
          return { action: "deliver" as const, text: "final answer in text" };
        },
        assertTriggerActive: async () => undefined,
        emitChatEvent: async (input: EmitChatEventInput) => {
          emitted.push(input);
          const seq = nextSeq;
          nextSeq += 1;
          return { status: "admitted" as const, eventId: `chat:${triggerId}:x`, seq, sessionId: routedSessionId };
        },
        storeChatSent: async (input: StoreChatSentInput) => {
          stored.push(input);
          const seq = nextSeq;
          nextSeq += 1;
          return { seq };
        },
        resolveChatHandle: async (input: ResolveChatHandleInput) => {
          resolvedHandles.push(input);
          return { handle: null, maxSeq: nextSeq - 1 };
        },
        prepareChannelMedia: async (input: PrepareChannelMediaInput) => {
          preparedMedia.push(input);
          return {
            item: {
              type: "media" as const,
              blobRef: `sha256:${"9".repeat(64)}`,
              kind: input.media.kind,
              mime: input.media.mime,
              ...(input.media.name === undefined ? {} : { name: input.media.name }),
            },
          };
        },
        maintainChannelTyping: async () => undefined,
        deliverChannelMessage: async (command: ChannelDeliveryCommandV1) => {
          deliveries.push(command);
          return fakeDelivery.deliverChannelMessage(command);
        },
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const route = { provider: "telegram" as const, accountId: "primary", chatId: "123" };
      const identity = channelConversationIdentity(universeId, route);
      const start: ChannelConversationStartV1 = {
        version: 1,
        universeId,
        triggerId,
        botId,
        botName: "concierge",
        scope: "direct",
        activation: { mode: "dm", triggerPrefixes: ["/ask", "/lightspeed"], mentionNames: [] },
        access: { turn: "conversation", control: "admins" },
        route,
        label: "telegram dm · Lukas",
        deliveryTaskQueue: CHANNELS_ACTIVITY_TASK_QUEUE,
      };
      const holder = await env.client.workflow.start("testHolderWorkflow", {
        workflowId: sessionWorkflowId(universeId, routedSessionId),
        taskQueue: CHANNELS_WORKFLOW_TASK_QUEUE,
      });
      const inbound = {
        version: 1 as const,
        messageId: "42",
        route,
        senderId: "7",
        senderName: "Lukas",
        timestampMs: 1_700_000_000_000,
        text: "hello",
        media: [
          {
            version: 1 as const,
            provider: "telegram" as const,
            fileId: "telegram-file-1",
            kind: "image" as const,
            mime: "image/jpeg",
            name: "photo.jpg",
            byteSize: 123,
          },
        ],
        isDirect: true,
        mentionedBot: false,
        isReplyToBot: false,
        authorization: { turnAllowed: true, controlAllowed: false, memberRole: null },
      };
      const inboundKey = channelInboundKey(inbound);
      const conversation = await env.client.workflow.signalWithStart(CHANNEL_CONVERSATION_WORKFLOW, {
        workflowId: identity.workflowId,
        taskQueue: CHANNELS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: CHANNEL_INBOUND_SIGNAL,
        signalArgs: [inbound],
      });

      // Inbound: one message, one event, carrying media, tools, and the receipt route.
      const afterEmit = await eventually(
        () => conversation.query<ConversationSnapshot>(CHANNEL_STATE_QUERY),
        (snapshot) => snapshot.messages[inboundKey]?.status === "emitted",
      );
      expect(afterEmit.messages[inboundKey]).toMatchObject({ seq: 17, sessionId: routedSessionId });
      expect(afterEmit.handles["17"]).toEqual({
        providerMessageIds: ["42"],
        fromMe: false,
        senderId: "7",
        text: "hello",
      });
      expect(afterEmit.toolsRef).toBe(toolsRef);
      expect(preparedMedia).toHaveLength(1);
      expect(emitted).toHaveLength(1);
      expect(emitted[0]).toMatchObject({
        triggerId,
        botId,
        conversation: { key: identity.conversationKey, label: "telegram dm · Lukas", scope: "direct" },
        message: { messageId: "42", senderName: "Lukas", text: "hello", isDirect: true },
        media: [{ type: "media", kind: "image", mime: "image/jpeg", name: "photo.jpg" }],
        toolsRef,
        notify: { workflowId: identity.workflowId, workflowKind: CHANNEL_CONVERSATION_WORKFLOW, token: inboundKey },
      });

      // A pushed send replying to #17: the number resolves to the provider id
      // before delivery, the send gets its own number, the model gets it back.
      const invocationId = `wti:sha256:${"a".repeat(64)}`;
      const pushed: EmissionEnvelope = {
        emission_id: invocationId,
        producer: { kind: "session", universe_id: universeId, session_id: routedSessionId, log_seq: 1 },
        body: {
          kind: "tool_invocation",
          holder_workflow_id: sessionWorkflowId(universeId, routedSessionId),
          invocation: {
            invocation_id: invocationId,
            tool_id: "channels.message_send.v1",
            semantic_type: "channels.message.send.v1",
            schema_revision: 2,
            binding_fingerprint: "binding:v1:test",
            session_universe_id: universeId,
            session_id: routedSessionId,
            run_id: 1,
            turn_id: 1,
            tool_batch_id: 1,
            tool_call_id: "call_1",
            arguments_ref: `sha256:${"c".repeat(64)}`,
            completion_promises: { reply: "promise_1" },
          },
        },
      };
      await conversation.signal("deliver_emission", pushed);
      const afterSend = await eventually(
        () => conversation.query<ConversationSnapshot>(CHANNEL_STATE_QUERY),
        (snapshot) => snapshot.invocations[invocationId]?.status === "resolved",
      );
      expect(afterSend.invocations[invocationId]).toMatchObject({
        status: "resolved",
        sentSeq: 18,
        providerMessageIds: [`fake:${invocationId}:chunk:1/2`, `fake:${invocationId}:chunk:2/2`],
      });
      expect(afterSend.handles["18"]).toMatchObject({ fromMe: true });
      expect(deliveries[0]?.operation).toMatchObject({
        type: "send",
        replyTo: "42",
        replyContext: { senderId: "7", text: "hello" },
      });
      expect(stored[0]).toMatchObject({ invocationId, replyTo: 17, providerMessageIds: afterSend.invocations[invocationId]?.providerMessageIds });
      const resolutions = await eventually(
        () => holder.query<EmissionEnvelope[]>("holder_state"),
        (emissions) => emissions.length === 1,
      );
      expect(resolutions[0]).toMatchObject({
        producer: { kind: "workflow", workflow_id: identity.workflowId },
        body: {
          kind: "source_resolution",
          promise_id: "promise_1",
          resolution: { kind: "resolved", payload_ref: receiptRef },
        },
      });

      // A duplicate push is deduplicated; a reaction to an unknown number fails the call.
      await conversation.signal("deliver_emission", pushed);
      const unknownInvocationId = `wti:sha256:${"1".repeat(64)}`;
      await conversation.signal("deliver_emission", {
        emission_id: unknownInvocationId,
        producer: { kind: "session", universe_id: universeId, session_id: routedSessionId, log_seq: 3 },
        body: {
          kind: "tool_invocation",
          holder_workflow_id: sessionWorkflowId(universeId, routedSessionId),
          invocation: {
            ...(pushed.body.kind === "tool_invocation" ? pushed.body.invocation : neverInvocation()),
            invocation_id: unknownInvocationId,
            tool_id: "channels.message_react.v1",
            tool_call_id: "call_2",
            completion_promises: { reply: "promise_2" },
          },
        },
      } satisfies EmissionEnvelope);
      // The fake argument blob is a send; the react parser rejects it as
      // missing a message number, which is the failure path under test.
      const afterUnknown = await eventually(
        () => conversation.query<ConversationSnapshot>(CHANNEL_STATE_QUERY),
        (snapshot) => snapshot.invocations[unknownInvocationId]?.status === "failed",
      );
      expect(afterUnknown.duplicateEmissionCount).toBe(1);
      expect(afterUnknown.invocations[unknownInvocationId]?.error).toMatch(/message number/);
      const afterFailure = await eventually(
        () => holder.query<EmissionEnvelope[]>("holder_state"),
        (emissions) => emissions.length === 2,
      );
      expect(afterFailure[1]?.body).toMatchObject({
        kind: "source_resolution",
        promise_id: "promise_2",
        resolution: { kind: "failed" },
      });

      // Receipts from the bot controller: typing while the run is up, then
      // the fallback reply once — a repeated finish changes nothing.
      const receipt = (phase: "started" | "finished", status?: string): BotDeliveryReceiptV1 => ({
        version: 1,
        token: inboundKey,
        phase,
        deliveryId: "delivery-1",
        seqs: [17],
        sessionId: routedSessionId,
        runId: "run_1",
        ...(status === undefined ? {} : { status }),
      });
      await conversation.signal(BOT_DELIVERY_SIGNAL, receipt("started"));
      await eventually(
        () => conversation.query<ConversationSnapshot>(CHANNEL_STATE_QUERY),
        (snapshot) => snapshot.deliveries["delivery-1"]?.status === "started",
      );
      await conversation.signal(BOT_DELIVERY_SIGNAL, receipt("finished", "unresolved"));
      await conversation.signal(BOT_DELIVERY_SIGNAL, receipt("finished", "unresolved"));
      const afterFallback = await eventually(
        () => conversation.query<ConversationSnapshot>(CHANNEL_STATE_QUERY),
        (snapshot) => snapshot.deliveries["delivery-1"]?.fallback?.status === "delivered",
      );
      expect(afterFallback.deliveries["delivery-1"]).toMatchObject({
        status: "finished",
        outcome: "unresolved",
        runId: "run_1",
        fallback: { status: "delivered", seq: 19 },
      });
      expect(reconciled).toEqual([
        { universeId, sessionId: routedSessionId, runId: "run_1", status: "unresolved" },
      ]);
      expect(stored[1]).toMatchObject({ invocationId: "fallback:delivery-1", text: "final answer in text", replyTo: null });
      expect(afterFallback.handles["19"]).toMatchObject({ fromMe: true, text: "final answer in text" });
      expect(deliveries.at(-1)?.operation).toEqual({ type: "send", text: "final answer in text" });

      // A steered delivery only stops typing; nothing is reconciled or sent.
      await conversation.signal(BOT_DELIVERY_SIGNAL, { ...receipt("finished", "steered"), deliveryId: "delivery-2" });
      const afterSteer = await eventually(
        () => conversation.query<ConversationSnapshot>(CHANNEL_STATE_QUERY),
        (snapshot) => snapshot.deliveries["delivery-2"]?.status === "finished",
      );
      expect(afterSteer.deliveries["delivery-2"]?.fallback).toBeUndefined();
      expect(reconciled).toHaveLength(1);
      expect(resolvedHandles).toHaveLength(0);

      const history = await conversation.fetchHistory();
      await Worker.runReplayHistory(
        { workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)) },
        history,
        identity.workflowId,
      );
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);

  it("drops ambient group traffic, answers control commands itself, and refuses foreign sessions", async () => {
    const workflowWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: CHANNELS_WORKFLOW_TASK_QUEUE,
      workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)),
    });
    const emitted: EmitChatEventInput[] = [];
    const deliveries: ChannelDeliveryCommandV1[] = [];
    const activityWorker = await Worker.create({
      connection: env.nativeConnection,
      namespace: env.namespace ?? "default",
      taskQueue: CHANNELS_ACTIVITY_TASK_QUEUE,
      activities: {
        putChatToolDeclarations: async () => ({ toolsRef, toolIds: [] }),
        assertTriggerActive: async () => undefined,
        emitChatEvent: async (input: EmitChatEventInput) => {
          emitted.push(input);
          return { status: "admitted" as const, eventId: "e", seq: emitted.length, sessionId: routedSessionId };
        },
        maintainChannelTyping: async () => undefined,
        readJsonBlob: async () => ({}),
        putJsonBlob: async () => ({ blobRef: receiptRef }),
        deliverChannelMessage: async (command: ChannelDeliveryCommandV1) => {
          deliveries.push(command);
          return createFakeDeliveryActivities().deliverChannelMessage(command);
        },
      },
    });
    const workflowRun = workflowWorker.run();
    const activityRun = activityWorker.run();

    try {
      const route = { provider: "telegram" as const, accountId: "primary", chatId: "-100" };
      const identity = channelConversationIdentity(universeId, route);
      const start: ChannelConversationStartV1 = {
        version: 1,
        universeId,
        triggerId,
        botId,
        botName: "concierge",
        scope: "group",
        activation: { mode: "mention", triggerPrefixes: ["/ask"], mentionNames: ["lightspeed"] },
        access: { turn: "conversation", control: "admins" },
        route,
        label: "telegram group · -100",
        deliveryTaskQueue: CHANNELS_ACTIVITY_TASK_QUEUE,
      };
      const inbound = (messageId: string, text: string, mentionedBot: boolean, controlAllowed = false) => ({
        version: 1 as const,
        messageId,
        route,
        senderId: messageId,
        senderName: `Sender ${messageId}`,
        timestampMs: 1_700_000_000_000 + Number(messageId),
        text,
        isDirect: false,
        mentionedBot,
        isReplyToBot: false,
        authorization: { turnAllowed: true, controlAllowed, memberRole: null },
      });
      const conversation = await env.client.workflow.signalWithStart(CHANNEL_CONVERSATION_WORKFLOW, {
        workflowId: identity.workflowId,
        taskQueue: CHANNELS_WORKFLOW_TASK_QUEUE,
        args: [start],
        signal: CHANNEL_INBOUND_SIGNAL,
        signalArgs: [inbound("1", "ambient chatter", false)],
      });
      await conversation.signal(CHANNEL_INBOUND_SIGNAL, inbound("2", "@lightspeed first", true));
      await conversation.signal(CHANNEL_INBOUND_SIGNAL, inbound("3", "/activation always", false, true));
      await conversation.signal(CHANNEL_INBOUND_SIGNAL, inbound("4", "now active", false));
      await conversation.signal(CHANNEL_INBOUND_SIGNAL, inbound("5", "/status", false));
      // A push from a session that is not this bot's is rejected before any delivery.
      const foreignInvocationId = `wti:sha256:${"f".repeat(64)}`;
      await conversation.signal("deliver_emission", {
        emission_id: foreignInvocationId,
        producer: { kind: "session", universe_id: universeId, session_id: "bot:v1:other:k-x-0123abcd", log_seq: 1 },
        body: {
          kind: "tool_invocation",
          holder_workflow_id: sessionWorkflowId(universeId, "bot:v1:other:k-x-0123abcd"),
          invocation: {
            invocation_id: foreignInvocationId,
            tool_id: "channels.message_send.v1",
            semantic_type: "channels.message.send.v1",
            schema_revision: 2,
            binding_fingerprint: "binding:v1:test",
            session_universe_id: universeId,
            session_id: "bot:v1:other:k-x-0123abcd",
            run_id: 1,
            turn_id: 1,
            tool_batch_id: 1,
            tool_call_id: "call_1",
            arguments_ref: `sha256:${"c".repeat(64)}`,
            completion_promises: { reply: "promise_1" },
          },
        },
      } satisfies EmissionEnvelope);

      const state = await eventually(
        () => conversation.query<ConversationSnapshot>(CHANNEL_STATE_QUERY),
        (snapshot) =>
          snapshot.emittedCount === 2 &&
          snapshot.droppedInboundCount === 1 &&
          snapshot.deniedInboundCount === 1 &&
          snapshot.activation.mode === "always" &&
          Object.values(snapshot.policyResponses).some(
            (response) => response.kind === "control" && response.status === "delivered",
          ) &&
          snapshot.invocations[foreignInvocationId]?.status === "failed",
      );
      expect(emitted.map((event) => event.message.text)).toEqual(["first", "now active"]);
      expect(emitted[0]?.conversation).toMatchObject({ scope: "group", label: "telegram group · -100" });
      // The one control reply quotes the command with the provider id directly;
      // it is Channels-authored, never a bot event. The /status from a sender
      // without control rights is denied silently in a group.
      expect(deliveries).toHaveLength(1);
      expect(deliveries[0]?.operation).toMatchObject({ type: "send", replyTo: "3" });
      expect((deliveries[0]?.operation as { text: string }).text).toContain("Group activation is now always");
      expect(state.invocations[foreignInvocationId]?.error).toMatch(/does not belong/);

      const history = await conversation.fetchHistory();
      await Worker.runReplayHistory(
        { workflowsPath: fileURLToPath(new URL("./workflows.ts", import.meta.url)) },
        history,
        identity.workflowId,
      );
    } finally {
      workflowWorker.shutdown();
      activityWorker.shutdown();
      await Promise.all([workflowRun, activityRun]);
    }
  }, 60_000);
});

function neverInvocation(): never {
  throw new Error("expected a tool invocation fixture");
}

async function eventually<T>(
  read: () => Promise<T>,
  ready: (value: T) => boolean,
): Promise<T> {
  const deadline = Date.now() + 20_000;
  for (;;) {
    const value = await read();
    if (ready(value)) {
      return value;
    }
    if (Date.now() >= deadline) {
      throw new Error("timed out waiting for workflow state");
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
}
