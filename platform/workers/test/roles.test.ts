import { describe, expect, it } from "vitest";
import { resolvePlatformWorkerRoles } from "../src/roles.js";

describe("Platform worker roles", () => {
  it("starts every core worker and no connector by default", () => {
    expect(resolvePlatformWorkerRoles(undefined, undefined)).toEqual([
      "channels-workflows",
      "channels-activities",
      "bots-workflows",
      "bots-activities",
    ]);
  });

  it("adds only explicitly configured connectors to all", () => {
    expect(resolvePlatformWorkerRoles("all", "telegram, whatsapp, telegram")).toEqual([
      "channels-workflows",
      "channels-activities",
      "bots-workflows",
      "bots-activities",
      "telegram",
      "whatsapp",
    ]);
  });

  it("supports the Channels and Bots composites", () => {
    expect(resolvePlatformWorkerRoles("channels", "telegram")).toEqual([
      "channels-workflows",
      "channels-activities",
    ]);
    expect(resolvePlatformWorkerRoles("bots", "telegram")).toEqual([
      "bots-workflows",
      "bots-activities",
    ]);
  });

  it.each([
    "channels-workflows",
    "channels-activities",
    "bots-workflows",
    "bots-activities",
    "telegram",
    "whatsapp",
  ])("can start the %s role independently", (role) => {
    expect(resolvePlatformWorkerRoles(role, "telegram,whatsapp")).toEqual([role]);
  });

  it("rejects unknown roles and connectors", () => {
    expect(() => resolvePlatformWorkerRoles("gateway", undefined)).toThrow(
      "unknown Platform workers role",
    );
    expect(() => resolvePlatformWorkerRoles("all", "signal")).toThrow(
      "invalid LIGHTSPEED_CHANNELS_CONNECTORS entry",
    );
  });
});
