import { describe, expect, it } from "vitest";
import {
  PAIRING_ALPHABET,
  chatSpecPayload,
  inboxBotOptionIds,
  inboxSelectionSpec,
  mintPairingCode,
  sessionTtlMs,
} from "./triggers";

describe("bot inbox sender selection", () => {
  it("encodes any bot by omitting the from allowlist", () => {
    expect(inboxSelectionSpec("any", ["boss"])).toEqual({});
  });

  it("encodes selected bots by immutable id", () => {
    expect(inboxSelectionSpec("selected", ["boss", "reviewer"])).toEqual({
      from: ["boss", "reviewer"],
    });
  });

  it("excludes the current bot while preserving unavailable selected ids", () => {
    expect(
      inboxBotOptionIds(
        "testoor",
        [{ botId: "testoor" }, { botId: "boss" }],
        ["retired-bot"],
      ),
    ).toEqual(["boss", "retired-bot"]);
  });
});

const chatForm = {
  channelAccountId: "acct-1",
  scope: "any" as const,
  groupActivation: "mention" as const,
  prefixesText: "/ask, /ask,\n/lightspeed",
  mentionNamesText: "",
  accessTurn: "conversation" as const,
  accessControl: "admins" as const,
  requirePairing: true,
  priority: "",
};

describe("chat trigger spec payload", () => {
  it("lets the server mint the pairing code on create", () => {
    const spec = chatSpecPayload(chatForm, undefined);
    expect(spec).toEqual({
      channelAccountId: "acct-1",
      matchScope: null,
      activation: { group: "mention", triggerPrefixes: ["/ask", "/lightspeed"] },
      access: { turn: "conversation", control: "admins" },
    });
    expect("pairingCode" in spec).toBe(false);
  });

  it("keeps an existing code by omitting it and opens the connection with null", () => {
    expect("pairingCode" in chatSpecPayload(chatForm, "ExistingCode1")).toBe(false);
    expect(chatSpecPayload({ ...chatForm, requirePairing: false }, "ExistingCode1").pairingCode).toBeNull();
  });

  it("mints a fresh code when pairing is switched on for an open connection", () => {
    const spec = chatSpecPayload(chatForm, null);
    expect(spec.pairingCode).toMatch(new RegExp(`^[${PAIRING_ALPHABET}]{12}$`));
  });

  it("carries scope and priority when set", () => {
    expect(chatSpecPayload({ ...chatForm, scope: "group", priority: "5" }, "code")).toMatchObject({
      matchScope: "group",
      priority: 5,
    });
  });
});

describe("pairing codes", () => {
  it("are 12 unambiguous characters", () => {
    for (let i = 0; i < 20; i += 1) {
      const code = mintPairingCode();
      expect(code).toHaveLength(12);
      for (const char of code) expect(PAIRING_ALPHABET).toContain(char);
    }
  });
});

describe("session retention", () => {
  it("inherits with null, keeps forever with 0, and converts hours", () => {
    expect(sessionTtlMs({ ttlMode: "inherit", ttlHours: "" })).toBeNull();
    expect(sessionTtlMs({ ttlMode: "forever", ttlHours: "" })).toBe(0);
    expect(sessionTtlMs({ ttlMode: "hours", ttlHours: "24" })).toBe(86_400_000);
  });
});
