import { describe, expect, it } from "vitest";
import type { EmissionEnvelope } from "@lightspeed/agent-client/workflow";
import type {
  ChannelConversationStartV1,
  NormalizedInboundV1,
} from "../src/contracts/channel.js";
import {
  applyEmission,
  applyInbound,
  compactWorkflowState,
  enqueueBoundedInbound,
  initialWorkflowState,
  MAX_CHANNEL_HANDLES,
  MAX_CHANNEL_INBOUND_INBOX,
  rememberHandle,
  snapshot,
} from "../src/workflows/state.js";

const universeId = "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f";
const invocationId = `wti:sha256:${"a".repeat(64)}`;
const sessionId = "bot:v1:concierge:k-telegram-primary-123-0123abcd";
const conversationKey = "telegram:primary:123";

const start: ChannelConversationStartV1 = {
  version: 1,
  universeId,
  triggerId: "7f1c4a9e-2b3d-4c5e-8f6a-1b2c3d4e5f60",
  botId: "0b54d227-08a2-45a8-9b3f-6a4c21d1a222",
  botName: "concierge",
  scope: "direct",
  activation: { mode: "dm", triggerPrefixes: ["/ask", "/lightspeed"], mentionNames: [] },
  access: { turn: "conversation", control: "admins" },
  route: { provider: "telegram", accountId: "primary", chatId: "123" },
  label: "telegram dm · Lukas",
  deliveryTaskQueue: "lightspeed-channels-delivery-v1-telegram-test",
};

const inbound: NormalizedInboundV1 = {
  version: 1,
  messageId: "42",
  route: start.route,
  senderId: "7",
  senderName: "Lukas",
  timestampMs: 1_700_000_000_000,
  text: "hello",
  isDirect: true,
  mentionedBot: false,
  isReplyToBot: false,
};

function invocation(): EmissionEnvelope {
  return {
    emission_id: invocationId,
    producer: { kind: "session", universe_id: universeId, session_id: sessionId, log_seq: 1 },
    body: {
      kind: "tool_invocation",
      holder_workflow_id: `${universeId}/${sessionId}`,
      invocation: {
        invocation_id: invocationId,
        tool_id: "channels.message_send.v1",
        semantic_type: "channels.message.send.v1",
        schema_revision: 2,
        binding_fingerprint: "binding:v1:test",
        session_universe_id: universeId,
        session_id: sessionId,
        run_id: 1,
        turn_id: 1,
        tool_batch_id: 1,
        tool_call_id: "call_1",
        arguments_ref: `sha256:${"c".repeat(64)}`,
        completion_promises: { reply: "promise_1" },
      },
    },
  };
}

describe("conversation workflow state", () => {
  it("deduplicates inbound provider messages", () => {
    const state = initialWorkflowState(start, conversationKey);
    expect(applyInbound(state, inbound)).not.toBeNull();
    expect(applyInbound(state, inbound)).toBeNull();
    expect(snapshot(state)).toMatchObject({ inboundCount: 1, duplicateInboundCount: 1 });
  });

  it("keeps a bounded handle cache keyed by message number", () => {
    const state = initialWorkflowState(start, conversationKey);
    for (let index = 1; index <= MAX_CHANNEL_HANDLES + 4; index += 1) {
      rememberHandle(state, index, { providerMessageIds: [String(index)], fromMe: index % 2 === 0 });
    }
    expect(Object.keys(state.snapshot.handles)).toHaveLength(MAX_CHANNEL_HANDLES);
    expect(state.snapshot.handles["1"]).toBeUndefined();
    expect(state.snapshot.handles["5"]).toEqual({ providerMessageIds: ["5"], fromMe: false });
  });

  it("sheds inbound signals beyond the deterministic workflow inbox ceiling", () => {
    const state = initialWorkflowState(start, conversationKey);
    const inbox: NormalizedInboundV1[] = [];
    for (let index = 0; index < MAX_CHANNEL_INBOUND_INBOX + 2; index += 1) {
      enqueueBoundedInbound(state, inbox, { ...inbound, messageId: String(index) });
    }
    expect(inbox).toHaveLength(MAX_CHANNEL_INBOUND_INBOX);
    expect(state.snapshot.overloadedInboundCount).toBe(2);
  });

  it("deduplicates pushed invocations and refuses a run terminal it does not own", () => {
    const state = initialWorkflowState(start, conversationKey);
    const envelope = invocation();
    expect(applyEmission(state, envelope)).toEqual({
      type: "invocation_received",
      invocationId,
    });
    expect(applyEmission(state, envelope)).toEqual({ type: "none" });
    expect(
      applyEmission(state, {
        emission_id: `emission:sha256:${"e".repeat(64)}`,
        producer: envelope.producer,
        body: {
          kind: "run_terminal",
          token: "turn:42",
          run_id: 1,
          status: "completed",
          output_ref: null,
          failure_message_ref: null,
        },
      }),
    ).toEqual({ type: "none" });

    const view = snapshot(state);
    expect(Object.keys(view.invocations)).toEqual([invocationId]);
    expect(view.duplicateEmissionCount).toBe(1);
    expect(view.protocolErrors).toEqual([
      "conversation received a run terminal it does not own",
    ]);
  });

  it("surfaces cancellation effects independently of invocation delivery", () => {
    const state = initialWorkflowState(start, conversationKey);
    const effect = applyEmission(state, {
      emission_id: `emission:sha256:${"f".repeat(64)}`,
      producer: { kind: "session", universe_id: universeId, session_id: sessionId, log_seq: 2 },
      body: {
        kind: "invocation_cancellation",
        invocation_id: invocationId,
        completion_key: "reply",
        promise_id: "promise_1",
      },
    });
    expect(effect).toEqual({
      type: "invocation_cancelled",
      invocationId,
      promiseId: "promise_1",
    });
  });

  it("compacts finished state while retaining active work and bounded dedup", () => {
    const state = initialWorkflowState(start, conversationKey);
    state.snapshot.toolsRef = `sha256:${"1".repeat(64)}`;
    for (let index = 0; index < 300; index += 1) {
      state.snapshot.messages[`done-${index}`] = { messageId: String(index), status: "emitted", seq: index + 1 };
    }
    state.snapshot.messages.pending = { messageId: "pending", status: "emitting" };
    for (let index = 0; index < 140; index += 1) {
      state.snapshot.deliveries[`d-${index}`] = {
        status: "finished",
        sessionId,
        runId: `run_${index}`,
        outcome: "handled",
        fallback: { status: "suppressed" },
      };
    }
    state.snapshot.deliveries.open = { status: "started", sessionId, runId: "run_x" };
    for (let index = 0; index < 2_100; index += 1) {
      state.seenInboundIds.add(`inbound-${index}`);
      state.seenEmissionIds.add(`emission-${index}`);
    }

    const carry = compactWorkflowState(state);
    expect(Object.keys(carry.snapshot.messages)).toHaveLength(257);
    expect(carry.snapshot.messages.pending?.status).toBe("emitting");
    expect(carry.snapshot.messages["done-0"]).toBeUndefined();
    expect(Object.keys(carry.snapshot.deliveries)).toHaveLength(129);
    expect(carry.snapshot.deliveries.open?.status).toBe("started");
    expect(carry.snapshot.toolsRef).toBe(state.snapshot.toolsRef);
    expect(carry.seenInboundIds).toHaveLength(2_048);
    expect(carry.seenInboundIds[0]).toBe("inbound-52");

    const restored = initialWorkflowState(start, conversationKey, carry);
    restored.snapshot.activation.triggerPrefixes.push("/new");
    expect(carry.snapshot.activation.triggerPrefixes).not.toContain("/new");
    expect(() => initialWorkflowState(start, "telegram:other:1", carry)).toThrow(
      "does not match",
    );
  });
});
