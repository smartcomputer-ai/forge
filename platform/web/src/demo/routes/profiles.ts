/// Profile registry: whole-document puts guarded by the revision the editor
/// loaded, like the engine's `profiles/put`. Validation stays loose (id
/// present and path-consistent); the engine is the validator of record.
import { Hono } from "hono";
import type { ProfileDocument, ProfileSummary } from "@/api";
import type { DemoStore } from "../store";
import { badRequest, conflict, notFound, readBody, universeFor } from "./common";

export function profileRoutes(store: DemoStore): Hono {
  const app = new Hono();

  app.get("/:id/profiles", (c) => {
    const universe = universeFor(store, c);
    if (!universe) return notFound(c);
    const summaries: ProfileSummary[] = [...universe.profiles.values()].map(summaryOf);
    return c.json(summaries);
  });

  app.get("/:id/profiles/:profileId", (c) => {
    const universe = universeFor(store, c);
    if (!universe) return notFound(c);
    const profile = universe.profiles.get(c.req.param("profileId"));
    return profile ? c.json(profile) : notFound(c, "not found in engine");
  });

  /// Create-or-replace. A `revision` in the body (as loaded from GET) is the
  /// expected one — a stale editor gets a 409 instead of clobbering a
  /// concurrent edit; a new profile (the create dialog) carries none.
  app.put("/:id/profiles/:profileId", async (c) => {
    const universe = universeFor(store, c);
    if (!universe) return notFound(c);
    const body = await readBody<Partial<ProfileDocument>>(c);
    const profileId = body.profileId;
    if (typeof profileId !== "string" || !profileId) return badRequest(c, "profileId is required");
    if (profileId !== c.req.param("profileId")) {
      return badRequest(c, "profileId in document does not match URL");
    }
    const { revision: expectedRevision, ...document } = body;
    const existing = universe.profiles.get(profileId);
    if (
      existing &&
      typeof expectedRevision === "number" &&
      expectedRevision !== (existing.revision ?? 0)
    ) {
      return conflict(
        c,
        `engine conflict: expected profile revision ${expectedRevision}, got ${existing.revision ?? 0}`,
      );
    }
    const now = Date.now();
    const profile: ProfileDocument = {
      ...document,
      profileId,
      revision: existing ? (existing.revision ?? 0) + 1 : 1,
      createdAtMs: existing?.createdAtMs ?? now,
      updatedAtMs: now,
    };
    universe.profiles.set(profileId, profile);
    return c.json({ profile });
  });

  app.delete("/:id/profiles/:profileId", (c) => {
    const universe = universeFor(store, c);
    if (!universe) return notFound(c);
    if (!universe.profiles.delete(c.req.param("profileId"))) {
      return notFound(c, "not found in engine");
    }
    return c.json({ ok: true });
  });

  return app;
}

function summaryOf(profile: ProfileDocument): ProfileSummary {
  return {
    profileId: profile.profileId,
    displayName: typeof profile.displayName === "string" ? profile.displayName : null,
    description: typeof profile.description === "string" ? profile.description : null,
    revision: profile.revision ?? 0,
    updatedAtMs: profile.updatedAtMs ?? 0,
  };
}
