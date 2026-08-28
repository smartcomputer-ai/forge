/// Session routes over the engine simulation: the sessions browser, the
/// transcript's long-poll tail, run control, and the settings sheet. Shapes
/// and status codes follow the platform server's gateway so the UI cannot
/// tell the difference.
import { Hono, type Context } from "hono";
import type { Environment, ProfileSource, SessionView } from "@/api";
import type { ProfileEnvironment, ProfileInstructions } from "@lightspeed/agent-client";
import {
  DEFAULT_MODEL,
  PROFILE_INSTRUCTIONS_KEY,
  cancelRun,
  closeSession,
  findRun,
  modelOf,
  newSession,
  pushEvent,
  setInstructions,
  startRun,
  steerRun,
  waitForEvents,
} from "../engine";
import { sessionSummary, type DemoStore, type SessionRecord, type UniverseState } from "../store";
import { badRequest, conflict, intQuery, notFound, readBody, universeFor } from "./common";
import { closeEnvironment, provisionEnvironment } from "./environments";

/// What a session start consumes from a profile, whichever source it came
/// from. `profileId` is null for inline profiles.
interface ResolvedProfile {
  profileId: string | null;
  config: Record<string, unknown>;
  instructions: ProfileInstructions | null;
  environment: ProfileEnvironment | null;
}

export function sessionRoutes(store: DemoStore): Hono {
  const app = new Hono();

  const lookup = (c: Context): { universe: UniverseState; session: SessionRecord } | null => {
    const universe = universeFor(store, c);
    const session = universe?.sessions.get(c.req.param("sessionId") ?? "");
    return universe && session ? { universe, session } : null;
  };

  /// Newest activity first; the cursor is a plain offset because the demo
  /// list is small and never changes underneath a page.
  app.get("/:id/sessions", (c) => {
    const universe = universeFor(store, c);
    if (!universe) return notFound(c);
    const limit = Math.min(intQuery(c, "limit", 50), 200);
    const offset = intQuery(c, "cursor", 0);
    const rootSessionId = c.req.query("rootSessionId") || null;
    const parentSessionId = c.req.query("parentSessionId") || null;
    const all = [...universe.sessions.values()]
      .filter((record) => !rootSessionId || record.view.origin?.rootSessionId === rootSessionId)
      .filter(
        (record) => !parentSessionId || record.view.origin?.parentSessionId === parentSessionId,
      )
      .sort((a, b) => b.view.updatedAtMs - a.view.updatedAtMs);
    const page = all.slice(offset, offset + limit);
    return c.json({
      sessions: page.map(sessionSummary),
      nextCursor: offset + limit < all.length ? String(offset + limit) : null,
    });
  });

  /// The environment intent is resolved before the session exists so a
  /// refused profile leaves nothing behind; the id is minted early because
  /// a provisioned environment is keyed by it.
  app.post("/:id/sessions", async (c) => {
    const universe = universeFor(store, c);
    if (!universe) return notFound(c);
    const body = await readBody<{ displayName?: string; profile?: ProfileSource }>(c);
    if (!body.profile) return badRequest(c, "profile is required");
    const profile = resolveProfile(universe, body.profile);
    if (!profile) return notFound(c, "not found in engine");
    const config = sessionConfig(profile.config);
    const sessionId = store.nextId("session");
    const resolved = resolveEnvironment(store, universe, profile, sessionId, config);
    if ("error" in resolved) return conflict(c, `engine conflict: ${resolved.error}`);
    const session = newSession(store, universe, {
      id: sessionId,
      displayName: body.displayName?.trim() || null,
      config,
      activeEnvironmentId: resolved.environmentId,
      instructions: instructionText(store, profile.instructions),
    });
    return c.json(session.view);
  });

  app.get("/:id/sessions/:sessionId", (c) => {
    const found = lookup(c);
    return found ? c.json(found.session.view) : notFound(c, "not found in engine");
  });

  /// Closing keeps history; `force` cancels active and queued work first.
  /// Environments a profile provisioned for this session go with it.
  app.post("/:id/sessions/:sessionId/close", async (c) => {
    const found = lookup(c);
    if (!found) return notFound(c, "not found in engine");
    const { universe, session } = found;
    const body = await readBody<{ force?: boolean }>(c);
    const wasClosed = session.view.status === "closed";
    if (!closeSession(session, body.force === true)) {
      return conflict(c, "engine conflict: session has active work; close with force to cancel it");
    }
    if (!wasClosed) closeOriginEnvironments(universe, session.view.id);
    return c.json(session.view);
  });

  app.delete("/:id/sessions/:sessionId", (c) => {
    const found = lookup(c);
    if (!found) return notFound(c, "not found in engine");
    const { universe, session } = found;
    if (session.view.status !== "closed") {
      return conflict(c, "engine conflict: only closed sessions can be deleted");
    }
    for (const timer of session.timers) clearTimeout(timer);
    session.timers.clear();
    // A parked tail returns now instead of waiting out its poll.
    for (const wake of [...session.waiters]) wake();
    universe.sessions.delete(session.view.id);
    return c.json(sessionSummary(session));
  });

  /// The transcript tail: pages forward with `waitMs=0`, then parks here
  /// until an event lands or the poll times out.
  app.get("/:id/sessions/:sessionId/events", async (c) => {
    const found = lookup(c);
    if (!found) return notFound(c, "not found in engine");
    const afterRaw = c.req.query("after");
    const after = afterRaw !== undefined && Number.isFinite(Number(afterRaw)) ? Number(afterRaw) : null;
    const limit = Math.min(intQuery(c, "limit", 200), 500);
    const waitMs = Math.min(intQuery(c, "waitMs", 0), 30_000);
    return c.json(await waitForEvents(found.session, after, limit, waitMs, c.req.raw.signal));
  });

  /// Whole-document replace with optimistic concurrency, applied at once:
  /// the demo has no turn boundary to wait for.
  app.put("/:id/sessions/:sessionId/config", async (c) => {
    const found = lookup(c);
    if (!found) return notFound(c, "not found in engine");
    const { session } = found;
    const body = await readBody<{ config?: unknown; expectedConfigRevision?: unknown }>(c);
    if (!isRecord(body.config)) return badRequest(c, "config must be an object");
    if (typeof body.expectedConfigRevision !== "number") {
      return badRequest(c, "expectedConfigRevision is required");
    }
    if (body.expectedConfigRevision !== session.view.configRevision) {
      return conflict(
        c,
        `engine conflict: expected config revision ${body.expectedConfigRevision}, got ${session.view.configRevision}`,
      );
    }
    const config = sessionConfig(body.config);
    session.view.config = config;
    session.view.configRevision += 1;
    pushEvent(session, {
      type: "sessionConfigChanged",
      revision: session.view.configRevision,
      model: modelOf(config),
    });
    return c.json(session.view);
  });

  app.get("/:id/sessions/:sessionId/instructions", (c) => {
    const found = lookup(c);
    if (!found) return notFound(c, "not found in engine");
    const { session } = found;
    const active = session.activeContext.entries
      .filter((entry) => entry.kind.type === "instructions")
      .map((entry) => ({
        key: entry.key ?? null,
        contentRef: entry.contentRef,
        preview: entry.preview ?? null,
      }));
    const custom = active.find((entry) => entry.key === PROFILE_INSTRUCTIONS_KEY);
    return c.json({
      text: custom ? store.readText(custom.contentRef) : null,
      contextRevision: session.activeContext.revision,
      active,
    });
  });

  /// Blank text clears the custom entry; the default one always stays.
  app.put("/:id/sessions/:sessionId/instructions", async (c) => {
    const found = lookup(c);
    if (!found) return notFound(c, "not found in engine");
    const { session } = found;
    const body = await readBody<{ text?: unknown }>(c);
    const text = typeof body.text === "string" && body.text.trim() ? body.text : null;
    if (text !== session.instructions) {
      setInstructions(store, session, text);
      session.activeContext.revision += 1;
      session.view.updatedAtMs = Date.now();
    }
    return c.json(session.view);
  });

  app.post("/:id/sessions/:sessionId/environments/:environmentId/activate", (c) => {
    const found = lookup(c);
    if (!found) return notFound(c, "not found in engine");
    const { universe, session } = found;
    if (session.view.status !== "idle") {
      return conflict(c, "engine conflict: environment activation requires an idle session");
    }
    if (!grantsEnvironments(session.view.config ?? {})) {
      return conflict(c, "engine conflict: environment activation requires the environments feature");
    }
    const environment = universe.environments.get(c.req.param("environmentId"));
    if (!environment) return notFound(c, "not found in engine");
    if (!usable(environment)) {
      return conflict(
        c,
        `engine conflict: environment is ${environment.status}: ${environment.environmentId}`,
      );
    }
    session.view.activeEnvironmentId = environment.environmentId;
    session.view.updatedAtMs = Date.now();
    return c.json(session.view);
  });

  app.post("/:id/sessions/:sessionId/environments/deactivate", (c) => {
    const found = lookup(c);
    if (!found) return notFound(c, "not found in engine");
    const { session } = found;
    if (session.view.status !== "idle") {
      return conflict(c, "engine conflict: environment deactivation requires an idle session");
    }
    session.view.activeEnvironmentId = null;
    session.view.updatedAtMs = Date.now();
    return c.json(session.view);
  });

  /// Acceptance boundary: the run is `running` or `queued` on return and
  /// the reply arrives on the tail. `submissionId` dedupes retries.
  app.post("/:id/sessions/:sessionId/messages", async (c) => {
    const found = lookup(c);
    if (!found) return notFound(c, "not found in engine");
    const { universe, session } = found;
    const body = await readBody<{ text?: unknown; submissionId?: unknown }>(c);
    if (typeof body.text !== "string" || !body.text.trim()) return badRequest(c, "text is required");
    if (session.view.status === "closed") return conflict(c, "engine conflict: session is closed");
    const run = startRun(store, universe, session, {
      text: body.text,
      submissionId: typeof body.submissionId === "string" ? body.submissionId : null,
    });
    return c.json({ run: { id: run.id, status: run.status } });
  });

  app.post("/:id/sessions/:sessionId/runs/:runId/cancel", (c) => {
    const found = lookup(c);
    if (!found) return notFound(c, "not found in engine");
    const run = cancelRun(found.session, c.req.param("runId"));
    if (!run) return notFound(c, "not found in engine");
    return c.json({ run: { id: run.id, status: run.status } });
  });

  /// Only a running run takes steering; queued, cancelling, and finished
  /// runs refuse it the way the engine does.
  app.post("/:id/sessions/:sessionId/runs/:runId/steer", async (c) => {
    const found = lookup(c);
    if (!found) return notFound(c, "not found in engine");
    const { session } = found;
    const runId = c.req.param("runId");
    const body = await readBody<{ text?: unknown }>(c);
    if (typeof body.text !== "string" || !body.text.trim()) return badRequest(c, "text is required");
    const run = findRun(session, runId);
    if (!run) return notFound(c, "not found in engine");
    const steered = steerRun(store, session, runId, body.text);
    if (!steered) {
      return conflict(c, `engine conflict: run ${runId} is ${run.status}; only a running run accepts steering`);
    }
    return c.json({
      steeringId: steered.steeringId,
      run: { id: steered.run.id, status: steered.run.status },
    });
  });

  return app;
}

function resolveProfile(universe: UniverseState, source: ProfileSource): ResolvedProfile | null {
  if (source.kind === "inline") {
    const profile = source.profile ?? {};
    return {
      profileId: null,
      config: isRecord(profile.config) ? profile.config : {},
      instructions: profile.instructions ?? null,
      environment: profile.environment ?? null,
    };
  }
  const document = universe.profiles.get(source.profileId);
  if (!document) return null;
  const instructions = document.instructions;
  return {
    profileId: document.profileId,
    config: isRecord(document.config) ? document.config : {},
    instructions: isRecord(instructions) ? (instructions as unknown as ProfileInstructions) : null,
    environment: document.environment ?? null,
  };
}

/// The session's own copy of a profile config, with the model the demo
/// answers as when the profile leaves it open.
function sessionConfig(config: Record<string, unknown>): Record<string, unknown> {
  const copy = structuredClone(config);
  return { ...copy, model: modelOf(copy) ?? { ...DEFAULT_MODEL } };
}

function instructionText(store: DemoStore, instructions: ProfileInstructions | null): string | null {
  if (!instructions) return null;
  return instructions.type === "text" ? instructions.text : store.readText(instructions.blobRef);
}

/// `existing` activates a universe environment; `provision` creates one
/// keyed by the session id so a retried start finds it again. Provisioning
/// needs the feature grant and an enabled binding for the provider, as the
/// engine checks before it touches a provider.
function resolveEnvironment(
  store: DemoStore,
  universe: UniverseState,
  profile: ResolvedProfile,
  sessionId: string,
  config: Record<string, unknown>,
): { environmentId: string | null } | { error: string } {
  const intent = profile.environment;
  if (!intent || intent.type === "inherit") return { environmentId: null };
  if (intent.type === "existing") {
    const environment = universe.environments.get(intent.environmentId);
    if (!environment) return { error: `environment not found: ${intent.environmentId}` };
    if (!usable(environment)) {
      return { error: `environment is ${environment.status}: ${intent.environmentId}` };
    }
    return { environmentId: intent.environmentId };
  }
  if (!grantsEnvironments(config)) {
    return {
      error:
        "profile provisions an environment but the effective session config does not grant features.environments",
    };
  }
  const binding = universe.providerBindings.find(
    (candidate) => candidate.providerId === intent.providerId,
  );
  if (!binding) {
    return {
      error: `profile provisions from environment provider ${intent.providerId}, but this universe has no binding for it`,
    };
  }
  if (binding.status !== "enabled") {
    return {
      error: `profile provisions from environment provider ${intent.providerId}, but binding ${binding.bindingId} is disabled`,
    };
  }
  const result = provisionEnvironment(store, universe, {
    requestId: `session:${sessionId}`,
    bindingId: binding.bindingId,
    templateId: intent.templateId,
    displayName:
      intent.displayName ??
      (profile.profileId ? `${profile.profileId} · ${sessionId}` : `session ${sessionId}`),
    idlePolicy: intent.idlePolicy ?? null,
    metadata: intent.metadata ?? {},
    originSession: {
      sessionId,
      ...(profile.profileId ? { profileId: profile.profileId } : {}),
      closeWithSession: (intent.retention ?? "closeWithSession") === "closeWithSession",
    },
  });
  if ("error" in result) return { error: result.error };
  return { environmentId: result.environment.environmentId };
}

/// The reconciler's sweep, done eagerly: environments a profile provisioned
/// for this session with `closeWithSession` close when it does.
function closeOriginEnvironments(universe: UniverseState, sessionId: string): void {
  for (const environment of universe.environments.values()) {
    if (
      environment.originSession?.sessionId === sessionId &&
      environment.originSession.closeWithSession
    ) {
      closeEnvironment(universe, environment.environmentId);
    }
  }
}

/// Provisioning and booting are valid activation targets; a terminal or
/// terminating environment is not.
function usable(environment: Environment): boolean {
  return (
    environment.status !== "closed" &&
    environment.status !== "closing" &&
    environment.status !== "failed"
  );
}

function grantsEnvironments(config: NonNullable<SessionView["config"]>): boolean {
  const features = config.features;
  return isRecord(features) && Boolean(features.environments);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}
