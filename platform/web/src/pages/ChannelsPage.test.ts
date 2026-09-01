import { describe, expect, it } from "vitest";
import { channelAccountConnectionState } from "./ChannelsPage";

describe("channel account connection state", () => {
  it("does not call an enabled account connected before its runner appears", () => {
    expect(channelAccountConnectionState({ enabled: true }, undefined)).toEqual({
      label: "Waiting for connector",
      healthy: false,
    });
  });

  it("surfaces the connector's useful failure detail", () => {
    expect(channelAccountConnectionState(
      { enabled: true },
      { state: "failed", lastError: "Telegram rejected the bot token" },
    )).toEqual({
      label: "Failed",
      detail: "Telegram rejected the bot token",
      healthy: false,
    });
  });

  it("keeps an administratively disabled account distinct", () => {
    expect(channelAccountConnectionState(
      { enabled: false },
      { state: "ready" },
    )).toEqual({ label: "Disabled", healthy: false });
  });
});
