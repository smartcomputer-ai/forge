import { describe, expect, it } from "vitest";
import { environmentFeatureSnapshot, withEnvironmentFeature } from "./profile-config-scope";

describe("bot profile config save scopes", () => {
  it("preserves the latest environment capability when saving other capabilities", () => {
    const draft = { features: { web: { search: {} }, environments: { jobs: true } } };
    const latest = { features: { environments: { providers: ["incus"] } } };

    expect(withEnvironmentFeature(draft, latest)).toEqual({
      features: {
        web: { search: {} },
        environments: { providers: ["incus"] },
      },
    });
  });

  it("changes only the environment capability when saving Environment", () => {
    const latest = { features: { web: { fetch: {} }, environments: { jobs: true } } };
    const draft = { features: { environments: { selectionTools: true } } };

    expect(withEnvironmentFeature(latest, draft)).toEqual({
      features: {
        web: { fetch: {} },
        environments: { selectionTools: true },
      },
    });
    expect(environmentFeatureSnapshot(draft)).toEqual({ selectionTools: true });
  });

  it("removes only the environment capability when it is turned off", () => {
    expect(withEnvironmentFeature(
      { features: { web: { search: {} }, environments: {} } },
      undefined,
    )).toEqual({ features: { web: { search: {} } } });
  });
});
