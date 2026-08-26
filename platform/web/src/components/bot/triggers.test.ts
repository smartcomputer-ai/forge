import { describe, expect, it } from "vitest";
import { inboxBotOptionIds, inboxSelectionSpec } from "./triggers";

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
