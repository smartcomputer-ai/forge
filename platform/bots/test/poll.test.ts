import { describe, expect, it } from "vitest";
import {
  MAX_POLL_CURSOR_IDS,
  diffPollItems,
  extractPollItems,
  parsePollPayload,
  pollItemKey,
  pollItemSummary,
} from "../src/poll.js";

const NOW = "2026-08-24T12:00:00.000Z";

describe("extractPollItems", () => {
  it("resolves the item array by path, or treats the payload as the list", () => {
    expect(extractPollItems({ data: { issues: [1, 2] } }, "data.issues")).toEqual([1, 2]);
    expect(extractPollItems([1, 2], null)).toEqual([1, 2]);
    expect(extractPollItems({ one: 1 }, null)).toEqual([{ one: 1 }]);
    expect(() => extractPollItems({ data: {} }, "data.issues")).toThrow(/not found/);
    expect(() => extractPollItems({ data: { issues: 3 } }, "data.issues")).toThrow(/not an array/);
  });
});

describe("id-set cursor", () => {
  const cursor = { kind: "idSet", id: "id" } as const;

  it("baselines on first contact without delivering", () => {
    const diff = diffPollItems(null, [{ id: 1 }, { id: 2 }], cursor, NOW);
    expect(diff.baselined).toBe(true);
    expect(diff.newItems).toEqual([]);
    expect(diff.nextState.ids).toEqual(["1", "2"]);
    expect(diff.nextState.baselinedAt).toBe(NOW);
  });

  it("delivers only unseen ids and advances the cursor", () => {
    const state = { ids: ["1", "2"], consecutiveFailures: 3 };
    const diff = diffPollItems(state, [{ id: 2 }, { id: 3 }, { id: 3 }], cursor, NOW);
    expect(diff.baselined).toBe(false);
    expect(diff.newItems.map((entry) => entry.key)).toEqual(["3"]);
    expect(diff.nextState.ids).toEqual(["1", "2", "3"]);
    // A successful poll clears the failure streak.
    expect(diff.nextState.consecutiveFailures).toBe(0);
  });

  it("caps the id set, aging out the oldest", () => {
    const state = {
      ids: Array.from({ length: MAX_POLL_CURSOR_IDS }, (_, index) => `old-${index}`),
      consecutiveFailures: 0,
    };
    const diff = diffPollItems(state, [{ id: "fresh" }], cursor, NOW);
    expect(diff.nextState.ids).toHaveLength(MAX_POLL_CURSOR_IDS);
    expect(diff.nextState.ids?.at(-1)).toBe("fresh");
    expect(diff.nextState.ids?.includes("old-0")).toBe(false);
  });

  it("skips items without a usable id", () => {
    const diff = diffPollItems({ ids: [], consecutiveFailures: 0 }, [{ id: null }, { x: 1 }], cursor, NOW);
    expect(diff.newItems).toEqual([]);
  });
});

describe("watermark cursor", () => {
  const cursor = { kind: "watermark", field: "updatedAt" } as const;

  it("baselines to the highest mark and then delivers only newer items", () => {
    const first = diffPollItems(
      null,
      [{ updatedAt: "2026-01-01" }, { updatedAt: "2026-03-01" }],
      cursor,
      NOW,
    );
    expect(first.baselined).toBe(true);
    expect(first.nextState.watermark).toBe("2026-03-01");

    const second = diffPollItems(
      first.nextState,
      [{ updatedAt: "2026-02-01" }, { updatedAt: "2026-04-01" }],
      cursor,
      NOW,
    );
    expect(second.newItems.map((entry) => entry.key)).toEqual(["2026-04-01"]);
    expect(second.nextState.watermark).toBe("2026-04-01");
  });

  it("compares numeric watermarks numerically", () => {
    const state = { watermark: 90, consecutiveFailures: 0 };
    const diff = diffPollItems(state, [{ updatedAt: 100 }, { updatedAt: 9 }], cursor, NOW);
    expect(diff.newItems.map((entry) => entry.key)).toEqual(["100"]);
    expect(diff.nextState.watermark).toBe(100);
  });
});

describe("pollItemKey and summaries", () => {
  it("derives keys from nested paths", () => {
    expect(pollItemKey({ issue: { number: 7 } }, { kind: "idSet", id: "issue.number" })).toBe("7");
    expect(pollItemKey({ x: {} }, { kind: "idSet", id: "x" })).toBeNull();
  });

  it("prefers human fields in summaries", () => {
    expect(pollItemSummary("feed", { title: "Broken build" }, "77")).toBe("feed: Broken build");
    expect(pollItemSummary("feed", { id: 77 }, "77")).toBe("feed: new item 77");
  });
});

describe("parsePollPayload", () => {
  it("parses JSON and shows the offending text otherwise", () => {
    expect(parsePollPayload('{"a":1}', "stdout")).toEqual({ a: 1 });
    expect(() => parsePollPayload("Warning: deprecated\n[1,2]", "stdout")).toThrow(
      /stdout starts: Warning: deprecated \[1,2\]/,
    );
    expect(() => parsePollPayload("<html>login</html>", "response body")).toThrow(
      /response body starts: <html>login<\/html>/,
    );
    expect(() => parsePollPayload("", "stdout")).toThrow(/\(empty\)/);
  });
});
