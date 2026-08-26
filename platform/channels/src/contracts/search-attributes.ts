import {
  SearchAttributeType,
  defineSearchAttributeKey,
  type SearchAttributePair,
} from "@temporalio/common";
import type { ChannelConversationStartV1 } from "./channel.js";

export const CHANNEL_UNIVERSE_SEARCH_ATTRIBUTE = defineSearchAttributeKey(
  "LightspeedUniverseId",
  SearchAttributeType.KEYWORD,
);
export const CHANNEL_PROVIDER_SEARCH_ATTRIBUTE = defineSearchAttributeKey(
  "LightspeedChannelProvider",
  SearchAttributeType.KEYWORD,
);
export const CHANNEL_ACCOUNT_SEARCH_ATTRIBUTE = defineSearchAttributeKey(
  "LightspeedChannelAccountId",
  SearchAttributeType.KEYWORD,
);
export const CHANNEL_TRIGGER_SEARCH_ATTRIBUTE = defineSearchAttributeKey(
  "LightspeedBotTriggerId",
  SearchAttributeType.KEYWORD,
);
export const CHANNEL_BOT_SEARCH_ATTRIBUTE = defineSearchAttributeKey(
  "LightspeedBotId",
  SearchAttributeType.KEYWORD,
);

export const CHANNEL_SEARCH_ATTRIBUTE_NAMES = [
  CHANNEL_UNIVERSE_SEARCH_ATTRIBUTE.name,
  CHANNEL_PROVIDER_SEARCH_ATTRIBUTE.name,
  CHANNEL_ACCOUNT_SEARCH_ATTRIBUTE.name,
  CHANNEL_TRIGGER_SEARCH_ATTRIBUTE.name,
  CHANNEL_BOT_SEARCH_ATTRIBUTE.name,
] as const;

export function channelConversationSearchAttributes(
  start: ChannelConversationStartV1,
): SearchAttributePair[] {
  return [
    { key: CHANNEL_UNIVERSE_SEARCH_ATTRIBUTE, value: start.universeId },
    { key: CHANNEL_PROVIDER_SEARCH_ATTRIBUTE, value: start.route.provider },
    { key: CHANNEL_ACCOUNT_SEARCH_ATTRIBUTE, value: start.route.accountId },
    { key: CHANNEL_TRIGGER_SEARCH_ATTRIBUTE, value: start.triggerId },
    { key: CHANNEL_BOT_SEARCH_ATTRIBUTE, value: start.botName },
  ];
}
