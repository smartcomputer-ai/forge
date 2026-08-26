import type { ChannelProvider, ChannelRoute } from "./channel.js";
import { CHANNEL_EDIT_TOOL_ID, CHANNEL_REACT_TOOL_ID, CHANNEL_SEND_TOOL_ID } from "./tools.js";

/**
 * What the model asked for: messages by number (`#N`). The conversation
 * workflow resolves numbers to provider ids before anything reaches a
 * provider worker.
 */
export type ChannelToolOperation =
  | { type: "send"; text: string; replyTo: number | null }
  | { type: "edit"; message: number; text: string }
  | { type: "react"; message: number; emoji: string };

/** What a provider worker executes: provider message ids and their direction. */
export type ChannelDeliveryOperation =
  | {
      type: "send";
      text: string;
      replyTo?: string | null;
      replyContext?: { senderId: string; text: string };
    }
  | { type: "edit"; messageId: string; text: string }
  | { type: "react"; messageId: string; emoji: string; fromMe: boolean };

export interface ChannelDeliveryCommandV1 {
  version: 1;
  invocationId: string;
  idempotencyKey: string;
  route: ChannelRoute;
  operation: ChannelDeliveryOperation;
}

export interface ChannelDeliveryResultV1 {
  version: 1;
  provider: ChannelProvider;
  messageIds: string[];
}

export interface ChannelDeliveryActivities {
  deliverChannelMessage(command: ChannelDeliveryCommandV1): Promise<ChannelDeliveryResultV1>;
}

export class ChannelDeliveryError extends Error {
  constructor(
    message: string,
    readonly retryable: boolean,
  ) {
    super(message);
    this.name = "ChannelDeliveryError";
  }
}

export function parseToolOperation(toolId: string, value: unknown): ChannelToolOperation {
  const args = record(value, "tool arguments");
  switch (toolId) {
    case CHANNEL_SEND_TOOL_ID: {
      const text = nonEmptyString(args.text, "text");
      const replyTo = args.replyTo === undefined || args.replyTo === null ? null : handle(args.replyTo, "replyTo");
      return { type: "send", text, replyTo };
    }
    case CHANNEL_EDIT_TOOL_ID:
      return {
        type: "edit",
        message: handle(args.message, "message"),
        text: nonEmptyString(args.text, "text"),
      };
    case CHANNEL_REACT_TOOL_ID:
      return {
        type: "react",
        message: handle(args.message, "message"),
        emoji: nonEmptyString(args.emoji, "emoji"),
      };
    default:
      throw new TypeError(`unsupported pushed channel tool: ${toolId}`);
  }
}

export function validateDeliveryResult(
  value: ChannelDeliveryResultV1,
  expectedProvider: ChannelProvider,
): ChannelDeliveryResultV1 {
  if (value.version !== 1 || value.provider !== expectedProvider) {
    throw new TypeError("delivery result does not match the command provider");
  }
  if (
    !Array.isArray(value.messageIds) ||
    value.messageIds.length === 0 ||
    value.messageIds.length > 32 ||
    value.messageIds.some((id) => typeof id !== "string" || id.length === 0)
  ) {
    throw new TypeError("delivery result must contain 1 to 32 message ids");
  }
  return value;
}

export function isDeliveryIdempotencyKey(command: ChannelDeliveryCommandV1): boolean {
  return (
    command.idempotencyKey === command.invocationId ||
    command.idempotencyKey.startsWith(`${command.invocationId}:chunk:`)
  );
}

function record(value: unknown, name: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TypeError(`${name} must be an object`);
  }
  return value as Record<string, unknown>;
}

function nonEmptyString(value: unknown, name: string): string {
  if (typeof value !== "string" || value.length === 0) {
    throw new TypeError(`${name} must be a non-empty string`);
  }
  return value;
}

function handle(value: unknown, name: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 1) {
    throw new TypeError(`${name} must be a message number (the #N of a message)`);
  }
  return value;
}
