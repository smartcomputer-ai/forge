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
  it("encodes any bot with an explicit null, clearing an earlier allowlist", () => {
    expect(inboxSelectionSpec("any", ["boss"])).toEqual({ from: null });
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
  accessTurn: "anyone" as const,
  allowedText: "",
  controllersText: "@lukas",
  requirePairing: true,
  priority: "",
};

describe("chat trigger payload", () => {
  it("lets the server mint the pairing code on create", () => {
    const spec = chatSpecPayload(chatForm, undefined);
    expect(spec).toEqual({
      accountId: "acct-1",
      matchScope: null,
      activation: { group: "mention", triggerPrefixes: ["/ask", "/lightspeed"] },
      access: { turn: "anyone", controllers: ["@lukas"] },
      pairing: "code",
    });
    expect("pairingCode" in spec).toBe(false);
  });

  it("keeps an existing code and opens the connection without one", () => {
    expect(chatSpecPayload(chatForm, "ExistingCode1").pairingCode).toBe("ExistingCode1");
    const open = chatSpecPayload({ ...chatForm, requirePairing: false }, "ExistingCode1");
    expect(open.pairing).toBe("open");
    expect("pairingCode" in open).toBe(false);
  });

  it("lists allowed handles when turns are restricted", () => {
    expect(chatSpecPayload({ ...chatForm, accessTurn: "listed", allowedText: "@a,@b" }, undefined).access).toEqual({
      turn: "listed",
      allowed: ["@a", "@b"],
      controllers: ["@lukas"],
    });
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
