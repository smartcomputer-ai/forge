import { describe, expect, it } from "vitest";
import { decodeHistoryCursor, encodeHistoryCursor, historyLimit } from "./bots.js";

describe("bot history pagination", () => {
  it("round-trips a stable timestamp and id cursor", () => {
    const at = new Date("2026-08-23T20:00:00.123Z");
    const id = "019ec16e-4b0c-7527-8909-c39441bad5a1";

    expect(decodeHistoryCursor(encodeHistoryCursor(at, id))).toEqual({ at, id });
  });

  it("rejects malformed cursors", () => {
    expect(decodeHistoryCursor("not-a-cursor")).toBeUndefined();
    expect(decodeHistoryCursor(Buffer.from(JSON.stringify({ at: "nope", id: "nope" })).toString("base64url")))
      .toBeUndefined();
  });

  it("bounds page sizes", () => {
    expect(historyLimit(undefined)).toBe(50);
    expect(historyLimit("12.9")).toBe(12);
    expect(historyLimit("0")).toBe(1);
    expect(historyLimit("1000")).toBe(100);
    expect(historyLimit("invalid")).toBe(50);
  });
});
