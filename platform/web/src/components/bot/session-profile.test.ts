import { describe, expect, it } from "vitest";
import { mergeSessionProfileFields } from "./session-profile";

describe("bot session profile saves", () => {
  it("overlays edited fields onto the latest document", () => {
    expect(mergeSessionProfileFields({
      profileId: "triage",
      revision: 4,
      createdAtMs: 10,
      updatedAtMs: 20,
      description: "shared profile",
      config: { features: { web: { search: {} } } },
      environment: { type: "existing", environmentId: "old-box" },
      metadata: { owner: "ops" },
    }, {
      environment: { type: "existing", environmentId: "new-box" },
      retention: { deleteAfterCloseMs: 86_400_000 },
    })).toEqual({
      profileId: "triage",
      revision: 4,
      description: "shared profile",
      config: { features: { web: { search: {} } } },
      environment: { type: "existing", environmentId: "new-box" },
      metadata: { owner: "ops" },
      retention: { deleteAfterCloseMs: 86_400_000 },
    });
  });

  it("clears optional fields without disturbing unrelated profile data", () => {
    expect(mergeSessionProfileFields({
      profileId: "triage",
      revision: 2,
      instructions: { type: "text", text: "old" },
      environment: { type: "existing", environmentId: "ops-box" },
      metadata: { team: "ops" },
    }, {
      instructions: undefined,
      environment: undefined,
    })).toEqual({
      profileId: "triage",
      revision: 2,
      metadata: { team: "ops" },
    });
  });
});
