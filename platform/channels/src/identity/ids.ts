import { sha256 } from "@noble/hashes/sha2.js";
import type { ChannelProvider, ChannelRoute } from "../contracts/channel.js";

// Lightspeed's long-lived default universe uses the canonical UUID text shape
// with a zero version nibble (`00000000-0000-0000-0000-000000000001`).
// Universe ids are opaque tenant identifiers here, so validate their shape
// without imposing RFC version/variant bits that Lightspeed itself does not.
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export interface ChannelConversationIdentity {
  /** Temporal workflow id of the conversation workflow. */
  workflowId: string;
  /** The conversation's route key at the bot: one routed session per key. */
  conversationKey: string;
  deliveryTaskQueue: string;
}

/**
 * One conversation workflow per (universe, provider, account, chat, thread).
 * The route key is readable (bots slug it into the session id); the workflow
 * id hashes it so provider ids never appear in Temporal identities.
 */
export function channelConversationIdentity(
  universeId: string,
  route: ChannelRoute,
): ChannelConversationIdentity {
  if (!UUID.test(universeId)) {
    throw new TypeError("universeId must be a UUID");
  }
  requirePart(route.accountId, "accountId");
  requirePart(route.chatId, "chatId");
  const conversationKey = [
    route.provider,
    route.accountId,
    route.chatId,
    ...(route.threadId === undefined ? [] : [route.threadId]),
  ].join(":");
  const routeHash = digest("lightspeed.channels.conversation.v1", [
    universeId.toLowerCase(),
    conversationKey,
  ]);
  return {
    workflowId: `lightspeed.channels.v1/${universeId.toLowerCase()}/${route.provider}/${routeHash}`,
    conversationKey,
    deliveryTaskQueue: channelDeliveryTaskQueue(route.provider, route.accountId),
  };
}

export function channelDeliveryTaskQueue(
  provider: ChannelProvider,
  accountId: string,
): string {
  requirePart(accountId, "accountId");
  const accountHash = digest("lightspeed.channels.account.v1", [provider, accountId]).slice(0, 24);
  return `lightspeed-channels-delivery-v1-${provider}-${accountHash}`;
}

export function channelPairingKey(route: ChannelRoute): string {
  requirePart(route.accountId, "accountId");
  requirePart(route.chatId, "chatId");
  return `channels-pairing-v1-${digest("lightspeed.channels.pairing.v1", [
    route.provider,
    route.accountId,
    route.chatId,
  ])}`;
}

function digest(domain: string, parts: readonly string[]): string {
  const hash = sha256.create();
  for (const part of [domain, ...parts]) {
    const bytes = new TextEncoder().encode(part);
    const length = new Uint8Array(8);
    new DataView(length.buffer).setBigUint64(0, BigInt(bytes.length), false);
    hash.update(length);
    hash.update(bytes);
  }
  return Array.from(hash.digest(), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function requirePart(value: string, name: string): void {
  if (value.length === 0) {
    throw new TypeError(`${name} must not be empty`);
  }
}
