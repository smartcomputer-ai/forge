import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { ChannelConversationStartV1 } from "../src/contracts/channel.js";
import {
  CHANNEL_SEARCH_ATTRIBUTE_NAMES,
  channelConversationSearchAttributes,
} from "../src/contracts/search-attributes.js";

describe("conversation workflow search attributes", () => {
  it("indexes every operational routing identity without message data", () => {
    const start = {
      universeId: "universe-1",
      triggerId: "trigger-1",
      botName: "concierge",
      route: { provider: "telegram", accountId: "primary", chatId: "secret-chat" },
    } as ChannelConversationStartV1;

    expect(
      channelConversationSearchAttributes(start).map(({ key, value }) => [key.name, value]),
    ).toEqual([
      ["LightspeedUniverseId", "universe-1"],
      ["LightspeedChannelProvider", "telegram"],
      ["LightspeedChannelAccountId", "primary"],
      ["LightspeedBotTriggerId", "trigger-1"],
      ["LightspeedBotId", "concierge"],
    ]);
    expect(CHANNEL_SEARCH_ATTRIBUTE_NAMES).not.toContain("secret-chat");
  });

  it("registers every Channels index in the local Temporal namespace", () => {
    const bootstrap = readFileSync(
      new URL("../../../scripts/dev/infra/temporal-ensure.sh", import.meta.url),
      "utf8",
    );

    for (const name of CHANNEL_SEARCH_ATTRIBUTE_NAMES) {
      expect(bootstrap).toContain(name);
    }
  });
});
