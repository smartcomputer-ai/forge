import { describe, expect, it } from "vitest";
import type { ProfileDocument } from "@/api";
import { copiedProfileDocument } from "./ProfilesPage";

describe("copiedProfileDocument", () => {
  it("copies setup fields without carrying registry identity", () => {
    const source: ProfileDocument = {
      profileId: "base-agent",
      displayName: "Base agent",
      description: "Shared agent setup",
      revision: 7,
      createdAtMs: 100,
      updatedAtMs: 200,
      metadata: { team: "platform" },
      config: {
        generation: { reasoningEffort: "high" },
        features: { timers: {} },
      },
      instructions: { type: "text", text: "Be useful." },
    };

    expect(copiedProfileDocument(source, "researcher", "Researcher")).toEqual({
      profileId: "researcher",
      displayName: "Researcher",
      description: "Shared agent setup",
      revision: 0,
      metadata: { team: "platform" },
      config: {
        generation: { reasoningEffort: "high" },
        features: { timers: {} },
      },
      instructions: { type: "text", text: "Be useful." },
    });
    expect(source.profileId).toBe("base-agent");
    expect(source.revision).toBe(7);
  });

  it("does not reuse the source display name when the new profile has none", () => {
    const source: ProfileDocument = {
      profileId: "base-agent",
      displayName: "Base agent",
      config: { features: { web: { fetch: {} } } },
    };

    expect(copiedProfileDocument(source, "copy", "")).toEqual({
      profileId: "copy",
      revision: 0,
      config: { features: { web: { fetch: {} } } },
    });
  });
});
