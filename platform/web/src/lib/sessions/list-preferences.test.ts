import { describe, expect, it } from "vitest";
import {
  DEFAULT_SESSION_LIST_PREFERENCES,
  metadataFilterFromSearchParams,
  parseMetadataPair,
  readSessionMetadataFilter,
  readSessionListPreferences,
  searchParamsWithMetadataFilter,
  writeSessionMetadataFilter,
  writeSessionListPreferences,
} from "./list-preferences";

describe("session list preferences", () => {
  it("reads valid values and falls back field by field", () => {
    expect(readSessionListPreferences({
      getItem: () => JSON.stringify({ showClosed: false, showSubagents: "yes" }),
    })).toEqual({ showClosed: false, showSubagents: true });
    expect(readSessionListPreferences({ getItem: () => "not-json" }))
      .toEqual(DEFAULT_SESSION_LIST_PREFERENCES);
  });

  it("writes preferences as one browser-local document", () => {
    let saved = "";
    writeSessionListPreferences(
      { showClosed: false, showSubagents: true },
      { setItem: (_key, value) => { saved = value; } },
    );
    expect(JSON.parse(saved)).toEqual({ showClosed: false, showSubagents: true });
  });

  it("stores metadata filters per universe", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => { values.set(key, value); },
    };
    writeSessionMetadataFilter("universe-a", { campaign: "nightly" }, storage);
    writeSessionMetadataFilter("universe-b", { source: "bot" }, storage);
    expect(readSessionMetadataFilter("universe-a", storage)).toEqual({ campaign: "nightly" });
    expect(readSessionMetadataFilter("universe-b", storage)).toEqual({ source: "bot" });
  });
});

describe("session metadata filter query", () => {
  it("parses values containing equals signs", () => {
    expect(parseMetadataPair(" campaign = terminal=bench "))
      .toEqual({ key: "campaign", value: "terminal=bench" });
    expect(parseMetadataPair("missing-value=")).toBeNull();
  });

  it("round-trips repeated metadata parameters while preserving other query state", () => {
    const params = searchParamsWithMetadataFilter(
      new URLSearchParams("view=tree&metadata=old=value"),
      { source: "harbor", campaign: "nightly" },
    );
    expect(params.get("view")).toBe("tree");
    expect(params.getAll("metadata")).toEqual(["source=harbor", "campaign=nightly"]);
    expect(metadataFilterFromSearchParams(params))
      .toEqual({ source: "harbor", campaign: "nightly" });
  });
});
