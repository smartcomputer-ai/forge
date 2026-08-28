/// Bots: durable event routers over managed sessions, with a small in-page
/// controller standing in for the Temporal one. Every event — manual,
/// webhook, replay, fixture — goes through `admitBotEvent`: numbered,
/// routed by its trigger's policy to one of the bot's sessions, and
/// delivered as a run whose end writes the event's outcome. The record's
/// event list is the read model the roster and the activity feed page.
import { Hono } from "hono";
import type { Context } from "hono";
import type {
  Bot,
  BotChatSpec,
  BotCoalesce,
  BotEventEnvelope,
  BotEventOutcome,
  BotLastEvent,
  BotListItem,
  BotManagedSession,
  BotPollSpec,
  BotRecentEvent,
  BotRoute,
  BotState,
  BotTrigger,
  BotWebhookSpec,
  ProfileDocument,
} from "@/api";
import type { RunView } from "@lightspeed/agent-client";
import {
  DEFAULT_MODEL,
  activeRun,
  applyEntries,
  closeSession,
  contextMessage,
  newSession,
  startRun,
  steerRun,
} from "../engine";
import type { BotRecord, DemoStore, SessionRecord, UniverseState } from "../store";
import { badRequest, conflict, intQuery, notFound, nowIso, readBody, universeFor } from "./common";

const NAME_PATTERN = /^[a-z0-9][a-z0-9-]*$/;
const TRIGGER_KINDS: ReadonlySet<string> = new Set(["schedule", "webhook", "poll", "bot", "chat"]);
const CHAT_COALESCE_DEFAULT: BotCoalesce = { debounceMs: 400, maxWaitMs: 1_500, maxCount: 8 };
const PAIRING_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
/// The controller's "picking it up" pause before a delivery becomes a run.
const DELIVERY_DELAY_MS = 800;
const ROTATE_DELAY_MS = 600;
const MAX_RECENT_EVENTS = 20;
const MAX_HISTORY_LIMIT = 100;
const MAX_BODY_BYTES = 1024 * 1024;
const MAX_PROMPT_PAYLOAD_CHARS = 2_000;
const COLLATOR = new Intl.Collator("en", { sensitivity: "base", numeric: true });

type TriggerKind = BotTrigger["kind"];
type WhenBusy = NonNullable<BotTrigger["deliver"]>["whenBusy"];
type PerKeyRoute = Extract<BotRoute, { policy: "perKey" }>;

/// Typed refusal so one handler maps config and admission failures to the
/// status the server would answer with.
class BotConfigError extends Error {
  constructor(
    message: string,
    readonly status: 400 | 404 | 409 | 410,
  ) {
    super(message);
    this.name = "BotConfigError";
  }
}

function configErrorResponse(c: Context, error: unknown): Response {
  if (error instanceof BotConfigError) return c.json({ error: error.message }, error.status);
  throw error;
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return isRecord(value) ? value : undefined;
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function optionalNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function randomHex(bytes: number): string {
  const buffer = crypto.getRandomValues(new Uint8Array(bytes));
  return [...buffer].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function mintPairingCode(length = 12): string {
  const bytes = crypto.getRandomValues(new Uint8Array(length));
  let code = "";
  for (const byte of bytes) code += PAIRING_ALPHABET[byte % PAIRING_ALPHABET.length];
  return code;
}

interface Found {
  universe: UniverseState;
  record: BotRecord;
}

function botFor(store: DemoStore, c: Context): Found | null {
  const universe = universeFor(store, c);
  const record = universe?.bots.get(c.req.param("botId") ?? "");
  return universe && record ? { universe, record } : null;
}

function lastEventOf(record: BotRecord): BotLastEvent | null {
  const event = record.events.at(-1);
  if (!event) return null;
  return {
    seq: event.seq,
    kind: event.kind,
    source: event.source,
    outcome: event.outcome,
    outcomeDetail: event.outcomeDetail,
    receivedAt: event.receivedAt,
    resolvedAt: event.resolvedAt,
    session: event.session,
  };
}

// ---------------------------------------------------------------------------
// Managed sessions
// ---------------------------------------------------------------------------

function profileInstructions(profile: ProfileDocument | undefined): string | null {
  const instructions = asRecord(profile?.instructions);
  const text = instructions?.text;
  return instructions?.type === "text" && typeof text === "string" ? text : null;
}

/// A session the bot's controller owns: managed, lifecycle-bound to the
/// controller, configured from the bot's profile.
function botSession(store: DemoStore, universe: UniverseState, bot: Bot, label: string): SessionRecord {
  const profile = universe.profiles.get(bot.profileId);
  const config = asRecord(profile?.config);
  return newSession(store, universe, {
    displayName: `${bot.botId} · ${label}`,
    managed: true,
    management: {
      version: 1,
      lifecycleController: { workflowId: `bot:v1:${bot.botId}`, workflowKind: "bot_controller_v1" },
      tools: [],
    },
    config: { model: { ...DEFAULT_MODEL }, ...config },
    instructions: profileInstructions(profile),
  });
}

function initialState(bot: Bot, profile: ProfileDocument | undefined, mainSessionId: string): BotState {
  return {
    botName: bot.botId,
    displayName: bot.displayName,
    profileId: bot.profileId,
    sessionId: mainSessionId,
    sessions: [{ sessionId: mainSessionId, label: "main", kind: "main" }],
    controllerStatus: "idle",
    activeDeliveries: [],
    sessionReady: true,
    pendingEventCount: 0,
    pendingDeliveryCount: 0,
    buffers: [],
    recentEvents: [],
    eventsProcessed: 0,
    duplicateEventCount: 0,
    duplicateEmissionCount: 0,
    appliedProfileRevision: profile?.revision ?? null,
    runsPerDay: bot.runsPerDay,
    runsToday: 0,
    descendantsToday: 0,
    lastError: null,
  };
}

interface Target {
  session: SessionRecord;
  managed: BotManagedSession;
}

/// Routing key → session id per bot; a keyed session survives rotation
/// under the same key.
const keyedSessions = new WeakMap<BotRecord, Map<string, string>>();

function keyedMap(record: BotRecord): Map<string, string> {
  let map = keyedSessions.get(record);
  if (!map) {
    map = new Map();
    keyedSessions.set(record, map);
  }
  return map;
}

function openSession(universe: UniverseState, sessionId: string | null | undefined): SessionRecord | null {
  const session = sessionId ? universe.sessions.get(sessionId) : undefined;
  return session && session.view.status !== "closed" ? session : null;
}

function addManaged(
  store: DemoStore,
  universe: UniverseState,
  record: BotRecord,
  label: string,
  kind: BotManagedSession["kind"],
): Target {
  const session = botSession(store, universe, record.bot, label);
  const managed: BotManagedSession = { sessionId: session.view.id, label, kind };
  record.state.sessions.push(managed);
  return { session, managed };
}

/// The main session, re-created when it is gone (a closed fixture session).
function mainSession(store: DemoStore, universe: UniverseState, record: BotRecord): Target {
  const state = record.state;
  const existing = openSession(universe, state.sessionId);
  const managed = state.sessions.find((entry) => entry.kind === "main");
  if (existing && managed) return { session: existing, managed };
  const session = botSession(store, universe, record.bot, "main");
  const entry: BotManagedSession = { sessionId: session.view.id, label: "main", kind: "main" };
  state.sessionId = session.view.id;
  state.sessions = [entry, ...state.sessions.filter((item) => item.kind !== "main")];
  return { session, managed: entry };
}

function routeKey(trigger: BotTrigger | null, route: PerKeyRoute, payload: unknown): { key: string; label: string } {
  const body = asRecord(payload);
  const keyed = (key: string) => ({ key, label: key });
  if (route.key) {
    const value = body?.[route.key];
    if ((typeof value === "string" && value) || typeof value === "number") return keyed(String(value).slice(0, 200));
  }
  if (trigger?.kind === "chat") {
    const conversation = asRecord(body?.conversation);
    const key = conversation?.key;
    const label = conversation?.label;
    if (typeof key === "string" && key) {
      return { key, label: typeof label === "string" && label ? label.slice(0, 200) : key };
    }
  }
  if (trigger?.kind === "webhook" && (trigger.spec as BotWebhookSpec).preset === "github") {
    const pullRequest = asRecord(body?.pull_request)?.number;
    if (typeof pullRequest === "number") return keyed(`pr-${pullRequest}`);
    const issue = asRecord(body?.issue)?.number;
    if (typeof issue === "number") return keyed(`issue-${issue}`);
    const repository = asRecord(body?.repository)?.full_name;
    if (typeof repository === "string") return keyed(repository);
  }
  return keyed(trigger?.name ?? "default");
}

/// Where an event lands: the main session, one session per key, or a fresh
/// session per event — decided at admission, where the payload is at hand.
function resolveTarget(
  store: DemoStore,
  universe: UniverseState,
  record: BotRecord,
  input: {
    trigger: BotTrigger | null;
    payload: unknown;
    eventId: string;
    session: { sessionId: string; label: string } | null;
  },
): Target {
  const state = record.state;
  const reused = openSession(universe, input.session?.sessionId);
  if (reused && input.session) {
    let managed = state.sessions.find((entry) => entry.sessionId === reused.view.id);
    if (!managed) {
      managed = { sessionId: reused.view.id, label: input.session.label, kind: "keyed" };
      state.sessions.push(managed);
    }
    return { session: reused, managed };
  }
  const trigger = input.trigger;
  let route: BotRoute | null = trigger?.route ?? null;
  // A chat conversation always gets its own session.
  if (trigger?.kind === "chat" && (route === null || route.policy === "bot")) route = { policy: "perKey", key: null };
  if (route === null || route.policy === "bot") return mainSession(store, universe, record);
  if (route.policy === "perEvent") {
    return addManaged(store, universe, record, `event ${input.eventId.slice(0, 24)}`, "event");
  }
  const { key, label } = routeKey(trigger, route, input.payload);
  const map = keyedMap(record);
  const existing = openSession(universe, map.get(key));
  const managed = existing ? state.sessions.find((entry) => entry.sessionId === existing.view.id) : undefined;
  if (existing && managed) return { session: existing, managed };
  const target = addManaged(store, universe, record, label, "keyed");
  map.set(key, target.session.view.id);
  return target;
}

// ---------------------------------------------------------------------------
// Admission and delivery
// ---------------------------------------------------------------------------

export interface BotEventDocument {
  version: 1;
  kind: string;
  source: string;
  occurredAt: string;
  summary: string;
  data?: unknown;
  correlationId?: string;
  links?: string[];
}

export interface BotEventInput {
  kind: string;
  source: string;
  /// One line about the event; derived from kind and source when absent.
  summary?: string;
  payload?: unknown;
  /// Rendered in place of `payload` (a preset's projection); the stored
  /// document keeps the full payload.
  promptData?: unknown;
  /// Caller-supplied id, deduped per bot; a fresh uuid when absent.
  eventId?: string;
  occurredAt?: string;
  trigger?: BotTrigger | null;
  /// Routed target of an earlier admission (replays reuse it).
  session?: { sessionId: string; label: string } | null;
  sender?: string | null;
  hops?: number;
  inReplyTo?: { bot: string; seq: number } | null;
  /// An already-stored document ref (replays).
  ref?: string;
  correlationId?: string | null;
  links?: string[];
}

function admissionRefusal(record: BotRecord, trigger: BotTrigger | null): BotConfigError | null {
  if (record.bot.closedAt !== null) return new BotConfigError("bot is closed", 410);
  if (!record.bot.enabled) return new BotConfigError("bot is disabled", 409);
  if (trigger && !trigger.enabled) return new BotConfigError("trigger is disabled", 409);
  return null;
}

function nextSeq(record: BotRecord): number {
  let max = 0;
  for (const event of record.events) if (event.seq !== null && event.seq > max) max = event.seq;
  return max + 1;
}

/// The model-facing text of one event: a header the model can quote back
/// by #N, the summary, and the payload (cut when large).
function renderEvent(
  event: Pick<BotEventEnvelope, "seq" | "kind" | "source" | "occurredAt" | "inReplyTo">,
  summary: string,
  payload: unknown,
): string {
  const handle = event.seq === null ? "event" : `event #${event.seq}`;
  const time = `${event.occurredAt.slice(0, 16).replace("T", " ")} UTC`;
  const parts = [`── ${handle} · ${event.kind} · ${event.source} · ${time}`, summary];
  if (payload !== undefined && payload !== null) {
    const json = JSON.stringify(payload, null, 2);
    parts.push(
      json.length > MAX_PROMPT_PAYLOAD_CHARS
        ? `${json.slice(0, MAX_PROMPT_PAYLOAD_CHARS)}\n(… truncated — full payload: bot_event_read ${handle.replace("event ", "")})`
        : json,
    );
  }
  if (event.inReplyTo) parts.push(`reply to your #${event.inReplyTo.seq} at ${event.inReplyTo.bot}`);
  return parts.join("\n");
}

/// Store, then deliver: numbers the event, routes it, and hands it to the
/// controller simulation. Throws a `BotConfigError` when the bot or trigger
/// refuses (closed, disabled); routes check first and answer 410/409.
export function admitBotEvent(
  store: DemoStore,
  universe: UniverseState,
  record: BotRecord,
  input: BotEventInput,
): { event: BotEventEnvelope; document: BotEventDocument; duplicate: boolean } {
  const trigger = input.trigger ?? null;
  const refusal = admissionRefusal(record, trigger);
  if (refusal) throw refusal;
  const eventId = input.eventId ?? crypto.randomUUID();
  const occurredAt = input.occurredAt ?? nowIso();
  const summary = input.summary ?? `${input.kind} from ${input.source}`;
  const document: BotEventDocument = {
    version: 1,
    kind: input.kind,
    source: input.source,
    occurredAt,
    summary,
    ...(input.payload === undefined ? {} : { data: input.payload }),
    ...(input.correlationId ? { correlationId: input.correlationId } : {}),
    ...(input.links === undefined ? {} : { links: input.links }),
  };
  const existing = record.events.find((event) => event.eventId === eventId);
  if (existing) {
    record.state.duplicateEventCount += 1;
    return { event: existing, document, duplicate: true };
  }
  const target = resolveTarget(store, universe, record, {
    trigger,
    payload: input.payload,
    eventId,
    session: input.session ?? null,
  });
  const event: BotEventEnvelope = {
    id: crypto.randomUUID(),
    eventId,
    seq: nextSeq(record),
    promptRef: null,
    kind: input.kind,
    source: input.source,
    occurredAt,
    ref: input.ref ?? store.putText(JSON.stringify(document, null, 2)),
    session:
      target.managed.kind === "main"
        ? null
        : { sessionId: target.session.view.id, label: target.managed.label },
    sender: input.sender ?? null,
    hops: input.hops ?? 0,
    inReplyTo: input.inReplyTo ?? null,
    receivedAt: nowIso(),
    outcome: null,
    outcomeDetail: null,
    deliveryId: null,
    runId: null,
    resolvedAt: null,
  };
  const prompt = renderEvent(event, summary, input.promptData ?? input.payload);
  event.promptRef = store.putText(prompt);
  record.events.push(event);
  deliver(store, universe, record, event, target, trigger?.deliver?.whenBusy ?? "queue", prompt);
  return { event, document, duplicate: false };
}

function removeDelivery(state: BotState, deliveryId: string): void {
  state.activeDeliveries = state.activeDeliveries.filter((delivery) => delivery.id !== deliveryId);
}

/// Write-once outcome plus the controller's bookkeeping.
function resolveEvent(
  record: BotRecord,
  event: BotEventEnvelope,
  outcome: BotEventOutcome,
  detail: string | null,
  runId: string | null,
  usage?: BotRecentEvent["usage"],
): void {
  if (event.outcome !== null) return;
  const state = record.state;
  event.outcome = outcome;
  event.outcomeDetail = detail;
  event.runId = runId;
  event.resolvedAt = nowIso();
  state.pendingEventCount = Math.max(0, state.pendingEventCount - 1);
  state.eventsProcessed += 1;
  if (runId !== null) state.runsToday += 1;
  const recent: BotRecentEvent = {
    id: event.eventId,
    ref: event.ref,
    ...(event.seq === null ? {} : { seqs: [event.seq] }),
    outcome,
    eventCount: 1,
    ...(runId === null ? {} : { runId }),
    ...(detail === null ? {} : outcome === "run_failed" ? { failure: detail } : { summary: detail }),
    ...(usage === undefined ? {} : { usage }),
  };
  state.recentEvents.unshift(recent);
  if (state.recentEvents.length > MAX_RECENT_EVENTS) state.recentEvents.length = MAX_RECENT_EVENTS;
  if (
    state.activeDeliveries.length === 0 &&
    state.controllerStatus !== "closed" &&
    state.controllerStatus !== "closing"
  ) {
    state.controllerStatus = "idle";
  }
}

/// Settles once the run is terminal — including cancellation, which the
/// engine never reports through `onFinished`.
function watchRun(session: SessionRecord, run: RunView, settle: (run: RunView) => void): void {
  const check = () => {
    if (run.status !== "completed" && run.status !== "cancelled" && run.status !== "failed") return;
    session.waiters.delete(check);
    settle(run);
  };
  session.waiters.add(check);
  check();
}

function lastAssistantLine(session: SessionRecord): string | null {
  const entries = session.activeContext.entries;
  for (let index = entries.length - 1; index >= 0; index--) {
    const entry = entries[index];
    if (!entry || entry.kind.type !== "message") continue;
    if (!("role" in entry.kind) || entry.kind.role !== "assistant") continue;
    if (!("text" in entry) || typeof entry.text !== "string") continue;
    const line = entry.text.split("\n").find((candidate) => candidate.trim()) ?? "";
    return line.trim().slice(0, 200) || null;
  }
  return null;
}

/// Plausible prompt accounting: the prefix grows per turn and is served
/// from cache once the session has one.
function usageFor(session: SessionRecord, prompt: string): NonNullable<BotRecentEvent["usage"]> {
  const inputTokens = 1_400 + Math.round(prompt.length / 4) + Math.max(0, session.turns - 1) * 850;
  const cachedInputTokens = session.turns > 1 ? Math.round(inputTokens * 0.82) : 0;
  return { inputTokens, cachedInputTokens };
}

/// The controller's delivery policy for one event: steer or append into a
/// busy session when the trigger says so, otherwise a run (queued behind an
/// active one by the engine).
function deliver(
  store: DemoStore,
  universe: UniverseState,
  record: BotRecord,
  event: BotEventEnvelope,
  target: Target,
  whenBusy: WhenBusy,
  prompt: string,
): void {
  const state = record.state;
  const session = target.session;
  const deliveryId = store.nextId("delivery");
  event.deliveryId = deliveryId;
  state.pendingEventCount += 1;
  target.managed.lastActiveAtMs = Date.now();
  const active = activeRun(session);
  if (active && whenBusy === "steer") {
    const steered = steerRun(store, session, active.id, prompt);
    if (steered) {
      resolveEvent(record, event, "steered", `steered run ${active.id}`, null);
      return;
    }
  }
  if (active && whenBusy === "append") {
    applyEntries(session, [contextMessage(store.nextId("entry"), "user", prompt)], { runId: active.id });
    resolveEvent(record, event, "appended", `appended to run ${active.id}`, null);
    return;
  }
  const delivery = { id: deliveryId, eventCount: 1, sessionId: session.view.id, runId: null as string | null };
  state.activeDeliveries.push(delivery);
  state.pendingDeliveryCount += 1;
  state.controllerStatus = active ? "session_busy" : "delivering_event";
  setTimeout(() => {
    state.pendingDeliveryCount = Math.max(0, state.pendingDeliveryCount - 1);
    // Archived while waiting (bot closed, session rotated).
    if (event.outcome !== null) return;
    if (session.view.status === "closed") {
      removeDelivery(state, deliveryId);
      resolveEvent(record, event, "archived", "session closed before delivery", null);
      return;
    }
    const run = startRun(store, universe, session, { text: prompt });
    delivery.runId = run.id;
    state.controllerStatus = "delivering_event";
    watchRun(session, run, (finished) => {
      removeDelivery(state, deliveryId);
      if (finished.status === "completed") {
        resolveEvent(
          record,
          event,
          "handled",
          lastAssistantLine(session) ?? "handled",
          finished.id,
          usageFor(session, prompt),
        );
      } else {
        resolveEvent(record, event, "run_failed", `run ${finished.status}`, finished.id);
      }
    });
  }, DELIVERY_DELAY_MS);
}

function abandonDeliveries(record: BotRecord, sessionId: string, detail: string): void {
  const state = record.state;
  for (const delivery of state.activeDeliveries.filter((entry) => entry.sessionId === sessionId)) {
    const event = record.events.find((entry) => entry.deliveryId === delivery.id);
    if (event) resolveEvent(record, event, "archived", detail, null);
    removeDelivery(state, delivery.id);
  }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Terminal close: the row first (admission refuses from here on), every
/// trigger paused as `bot_closed`, pending events archived, every session
/// force-closed and recorded.
function closeBot(universe: UniverseState, record: BotRecord): void {
  const { bot, state } = record;
  const at = nowIso();
  if (bot.closedAt === null) {
    bot.closedAt = at;
    bot.enabled = false;
    bot.updatedAt = at;
  }
  for (const trigger of record.triggers.values()) {
    if (!trigger.enabled) continue;
    trigger.enabled = false;
    trigger.disabledReason = "bot_closed";
    trigger.disabledAt = at;
    trigger.updatedAt = at;
  }
  state.controllerStatus = "closing";
  for (const event of record.events) {
    if (event.outcome === null) resolveEvent(record, event, "archived", "bot closed", null);
  }
  state.activeDeliveries = [];
  state.pendingDeliveryCount = 0;
  state.pendingEventCount = 0;
  state.buffers = [];
  const closed: string[] = [];
  for (const managed of state.sessions) {
    const session = universe.sessions.get(managed.sessionId);
    if (!session) continue;
    closeSession(session, true);
    closed.push(session.view.id);
  }
  bot.closedSessions = closed;
  state.sessionReady = false;
  state.controllerStatus = "closed";
}

/// Operator reset of one managed session: the old one closes, a fresh one
/// takes its place under the same label (and routing key).
function rotateSession(store: DemoStore, universe: UniverseState, record: BotRecord, managed: BotManagedSession): void {
  const state = record.state;
  const old = universe.sessions.get(managed.sessionId);
  if (old) {
    abandonDeliveries(record, old.view.id, "session rotated");
    closeSession(old, true);
  }
  const fresh = botSession(store, universe, record.bot, managed.label);
  const entry: BotManagedSession = { sessionId: fresh.view.id, label: managed.label, kind: managed.kind };
  const index = state.sessions.indexOf(managed);
  if (index >= 0) state.sessions[index] = entry;
  else state.sessions.push(entry);
  if (state.sessionId === managed.sessionId) state.sessionId = fresh.view.id;
  const map = keyedMap(record);
  for (const [key, sessionId] of map) if (sessionId === managed.sessionId) map.set(key, fresh.view.id);
  const remaining = (state.rotatingSessionIds ?? []).filter((id) => id !== managed.sessionId);
  if (remaining.length > 0) state.rotatingSessionIds = remaining;
  else delete state.rotatingSessionIds;
}

// ---------------------------------------------------------------------------
// Triggers
// ---------------------------------------------------------------------------

function isTriggerKind(value: unknown): value is TriggerKind {
  return typeof value === "string" && TRIGGER_KINDS.has(value);
}

function normalizeRoute(route: unknown): BotRoute | null {
  if (route === null || route === undefined) return null;
  const policy = asRecord(route)?.policy;
  const key = asRecord(route)?.key;
  if (policy === "perKey") return { policy: "perKey", key: typeof key === "string" && key ? key : null };
  if (policy === "perEvent") return { policy: "perEvent" };
  if (policy === "bot") return { policy: "bot" };
  throw new BotConfigError("validation failed", 400);
}

/// Chat triggers route per conversation; the main session cannot carry a
/// conversation's reply tools.
function chatRoute(route: unknown): BotRoute {
  const normalized = normalizeRoute(route);
  if (normalized === null) return { policy: "perKey", key: null };
  if (normalized.policy === "bot") {
    throw new BotConfigError(
      "chat triggers route per conversation (perKey or perEvent); the main session cannot take a chat",
      400,
    );
  }
  return normalized;
}

function normalizeCoalesce(value: unknown): BotCoalesce | null {
  const input = asRecord(value);
  if (!input) return null;
  const { debounceMs, maxWaitMs, maxCount } = input;
  if (typeof debounceMs !== "number" || typeof maxWaitMs !== "number" || typeof maxCount !== "number") {
    throw new BotConfigError("validation failed", 400);
  }
  return { debounceMs, maxWaitMs, maxCount };
}

function normalizeDeliver(value: unknown): BotTrigger["deliver"] {
  if (value === null || value === undefined) return null;
  const whenBusy = asRecord(value)?.whenBusy;
  if (whenBusy === "queue" || whenBusy === "steer" || whenBusy === "append") return { whenBusy };
  throw new BotConfigError("validation failed", 400);
}

function normalizeSessionTtl(value: unknown): number | null {
  if (value === null || value === undefined) return null;
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) {
    throw new BotConfigError("validation failed", 400);
  }
  return value;
}

function normalizeSpec(
  store: DemoStore,
  kind: TriggerKind,
  spec: unknown,
  existing?: BotTrigger["spec"],
): BotTrigger["spec"] {
  const input = asRecord(spec) ?? {};
  switch (kind) {
    case "schedule": {
      const { cron, at, timezone, summary } = input;
      if (typeof cron !== "string" && typeof at !== "string") {
        throw new BotConfigError("a schedule needs a cron expression or a one-shot time", 400);
      }
      return {
        cron: typeof cron === "string" && cron ? cron : null,
        at: typeof at === "string" && at ? at : null,
        timezone: typeof timezone === "string" && timezone ? timezone : "UTC",
        summary: typeof summary === "string" ? summary : "",
      };
    }
    case "webhook": {
      const previous = existing as BotWebhookSpec | undefined;
      const verification = asRecord(input.verification);
      const hmac =
        verification?.scheme === "hmac-sha256" &&
        typeof verification.grantId === "string" &&
        typeof verification.header === "string";
      return {
        // The URL token survives spec edits; rotation means a new trigger.
        token: previous?.token ?? randomHex(24),
        verification: hmac ? (verification as unknown as BotWebhookSpec["verification"]) : { scheme: "token" },
        preset: input.preset === "github" ? "github" : null,
      };
    }
    case "poll": {
      const { source, intervalMs, items, cursor } = input;
      const sourceKind = asRecord(source)?.kind;
      if ((sourceKind !== "http" && sourceKind !== "exec") || typeof intervalMs !== "number") {
        throw new BotConfigError("validation failed", 400);
      }
      const cursorKind = asRecord(cursor)?.kind;
      return {
        source: source as BotPollSpec["source"],
        intervalMs,
        items: typeof items === "string" && items ? items : null,
        cursor:
          cursorKind === "idSet" || cursorKind === "watermark"
            ? (cursor as BotPollSpec["cursor"])
            : { kind: "idSet", id: "id" },
      };
    }
    case "bot": {
      const from = Array.isArray(input.from)
        ? [...new Set(input.from.filter((value): value is string => typeof value === "string"))]
        : [];
      return from.length === 0 ? {} : { from };
    }
    case "chat": {
      const previous = existing as BotChatSpec | undefined;
      const { channelAccountId, matchScope, activation, access, pairingCode, priority } = input;
      if (typeof channelAccountId !== "string") throw new BotConfigError("validation failed", 400);
      if (!store.channelAccounts.has(channelAccountId)) throw new BotConfigError("unknown channel account", 400);
      return {
        channelAccountId,
        matchScope: matchScope === "direct" || matchScope === "group" ? matchScope : null,
        activation: isRecord(activation) ? (activation as BotChatSpec["activation"]) : null,
        access: isRecord(access) ? (access as BotChatSpec["access"]) : null,
        // Omitted keeps the existing code (or mints one on create); null opens the connection.
        pairingCode:
          pairingCode === undefined
            ? previous === undefined
              ? mintPairingCode()
              : previous.pairingCode
            : typeof pairingCode === "string" && pairingCode
              ? pairingCode
              : null,
        priority: typeof priority === "number" ? priority : (previous?.priority ?? 100),
      };
    }
  }
}

function nextTriggerName(record: BotRecord, kind: TriggerKind): string {
  if (!record.triggers.has(kind)) return kind;
  for (let n = 2; ; n++) {
    const candidate = `${kind}-${n}`;
    if (!record.triggers.has(candidate)) return candidate;
  }
}

function createTrigger(store: DemoStore, record: BotRecord, body: Record<string, unknown>): BotTrigger {
  const kind = body.kind;
  if (!isTriggerKind(kind)) throw new BotConfigError("validation failed", 400);
  const name = typeof body.name === "string" && body.name ? body.name : nextTriggerName(record, kind);
  if (!NAME_PATTERN.test(name) || name.length > 64) {
    throw new BotConfigError("validation failed: trigger names are lowercase alphanumerics and dashes", 400);
  }
  if (record.triggers.has(name)) throw new BotConfigError("a trigger with that name already exists", 409);
  if (kind === "bot") {
    // One inbox per bot: `to: "b"` must mean exactly one event in B.
    const inbox = [...record.triggers.values()].find((trigger) => trigger.kind === "bot");
    if (inbox) {
      throw new BotConfigError(
        `this bot already has an inbox (trigger ${inbox.name}); a bot has at most one trigger of kind bot`,
        409,
      );
    }
  }
  const spec = normalizeSpec(store, kind, body.spec);
  const routed = kind !== "schedule";
  const at = nowIso();
  const trigger: BotTrigger = {
    name,
    kind,
    spec,
    filter: routed ? optionalString(body.filter) : null,
    route: kind === "chat" ? chatRoute(body.route) : routed ? normalizeRoute(body.route) : null,
    coalesce:
      kind === "chat" && body.coalesce === undefined
        ? { ...CHAT_COALESCE_DEFAULT }
        : routed
          ? normalizeCoalesce(body.coalesce)
          : null,
    deliver: routed ? normalizeDeliver(body.deliver) : null,
    // Conversations keep their session: 0 = never close.
    sessionTtlMs: routed ? (normalizeSessionTtl(body.sessionTtlMs) ?? (kind === "chat" ? 0 : null)) : null,
    ...(kind === "poll" ? { cursor: null } : {}),
    enabled: body.enabled !== false,
    disabledReason: null,
    disabledAt: null,
    lastFilterError: null,
    lastFilterErrorAt: null,
    createdAt: at,
    updatedAt: at,
  };
  record.triggers.set(name, trigger);
  return trigger;
}

function updateTrigger(store: DemoStore, trigger: BotTrigger, body: Record<string, unknown>): BotTrigger {
  if (Object.keys(body).length === 0) throw new BotConfigError("at least one field is required", 400);
  if (
    trigger.kind === "schedule" &&
    ["filter", "route", "coalesce", "deliver", "sessionTtlMs"].some((key) => body[key] !== undefined)
  ) {
    throw new BotConfigError(
      "filters, routes, coalescing, delivery policy, and retention apply to webhook, poll, bot, and chat triggers",
      400,
    );
  }
  if (body.enabled !== undefined) {
    // Re-enabling clears whatever switched the trigger off; disabling by hand says so.
    trigger.enabled = body.enabled === true;
    trigger.disabledReason = trigger.enabled ? null : "operator";
    trigger.disabledAt = trigger.enabled ? null : nowIso();
  }
  if (body.filter !== undefined) trigger.filter = optionalString(body.filter);
  if (body.route !== undefined) {
    trigger.route = trigger.kind === "chat" ? chatRoute(body.route) : normalizeRoute(body.route);
  }
  if (body.coalesce !== undefined) trigger.coalesce = normalizeCoalesce(body.coalesce);
  if (body.deliver !== undefined) trigger.deliver = normalizeDeliver(body.deliver);
  if (body.sessionTtlMs !== undefined) trigger.sessionTtlMs = normalizeSessionTtl(body.sessionTtlMs);
  if (body.spec !== undefined) {
    trigger.spec = normalizeSpec(store, trigger.kind, body.spec, trigger.spec);
    // A spec edit re-baselines the poll against the (possibly different) source.
    if (trigger.kind === "poll") trigger.cursor = null;
  }
  trigger.updatedAt = nowIso();
  return trigger;
}

/// Wire view: the webhook ingest path and the chat account are derived.
function triggerView(store: DemoStore, record: BotRecord, trigger: BotTrigger): BotTrigger {
  if (trigger.kind === "webhook") {
    const { token } = trigger.spec as BotWebhookSpec;
    return { ...trigger, ingestPath: `/api/v1/hooks/bots/${record.bot.botId}--${trigger.name}/${token}` };
  }
  if (trigger.kind === "chat") {
    const account = store.channelAccounts.get((trigger.spec as BotChatSpec).channelAccountId);
    return {
      ...trigger,
      channelAccount: account
        ? { id: account.id, provider: account.provider, accountId: account.accountId, displayName: account.displayName }
        : null,
    };
  }
  return trigger;
}

function triggerFromSource(record: BotRecord, source: string): BotTrigger | null {
  const name = /^(?:webhook|poll|schedule|chat|bot):(.+)$/.exec(source)?.[1];
  return name ? (record.triggers.get(name) ?? null) : null;
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

export function botRoutes(store: DemoStore): Hono {
  const app = new Hono();

  app.get("/:id/bots", (c) => {
    const universe = universeFor(store, c);
    if (!universe) return notFound(c);
    const items: BotListItem[] = [...universe.bots.values()]
      .sort((left, right) => {
        const byLabel = COLLATOR.compare(
          left.bot.displayName ?? left.bot.botId,
          right.bot.displayName ?? right.bot.botId,
        );
        return byLabel !== 0 ? byLabel : COLLATOR.compare(left.bot.botId, right.bot.botId);
      })
      .map((record) => ({
        ...record.bot,
        triggerCount: record.triggers.size,
        pendingCount: record.events.filter((event) => event.outcome === null).length,
        lastEvent: lastEventOf(record),
      }));
    return c.json({ bots: items });
  });

  app.post("/:id/bots", async (c) => {
    const universe = universeFor(store, c);
    if (!universe) return notFound(c);
    const body = await readBody(c);
    const botId = optionalString(body.botId) ?? "";
    if (!NAME_PATTERN.test(botId) || botId.length > 64) {
      return badRequest(c, "validation failed: bot ids are lowercase alphanumerics and dashes");
    }
    const profileId = optionalString(body.profileId);
    if (!profileId) return badRequest(c, "validation failed: profileId is required");
    if (!universe.profiles.has(profileId)) return badRequest(c, `profile ${profileId} does not exist`);
    if (universe.bots.has(botId)) return conflict(c, "a bot with that id already exists");
    const breaker = asRecord(body.breaker);
    const fires = breaker?.fires;
    const windowMs = breaker?.windowMs;
    const at = nowIso();
    const bot: Bot = {
      botId,
      universeId: universe.universe.id,
      displayName: optionalString(body.displayName),
      description: optionalString(body.description),
      profileId,
      brief: optionalString(body.brief),
      runsPerDay: optionalNumber(body.runsPerDay),
      breaker: typeof fires === "number" && typeof windowMs === "number" ? { fires, windowMs } : null,
      routedSessionTtlMs: optionalNumber(body.routedSessionTtlMs),
      selfConfig: body.selfConfig === true,
      emit: body.emit === true,
      enabled: true,
      closedAt: null,
      closedSessions: null,
      createdAt: at,
      updatedAt: at,
    };
    const main = botSession(store, universe, bot, "main");
    const record: BotRecord = {
      bot,
      triggers: new Map(),
      events: [],
      state: initialState(bot, universe.profiles.get(profileId), main.view.id),
      lineage: {},
    };
    universe.bots.set(botId, record);
    // One refused trigger fails the whole create and leaves no half-made bot.
    try {
      if (body.acceptsBotEvents === true) createTrigger(store, record, { name: "inbox", kind: "bot" });
      const triggers = Array.isArray(body.triggers) ? body.triggers.filter(isRecord) : [];
      for (const input of triggers) createTrigger(store, record, input);
    } catch (error) {
      universe.bots.delete(botId);
      universe.sessions.delete(main.view.id);
      return configErrorResponse(c, error);
    }
    return c.json({ bot }, 201);
  });

  /// Bots applying the profile re-read it at their next idle moment.
  app.post("/:id/bots/reconcile", async (c) => {
    const universe = universeFor(store, c);
    if (!universe) return notFound(c);
    const body = await readBody(c);
    const profileId = optionalString(body.profileId);
    if (!profileId) return badRequest(c, "validation failed: profileId is required");
    const revision = universe.profiles.get(profileId)?.revision;
    const signalled: string[] = [];
    for (const record of universe.bots.values()) {
      if (record.bot.profileId !== profileId || record.bot.closedAt !== null) continue;
      if (revision !== undefined) record.state.appliedProfileRevision = revision;
      signalled.push(record.bot.botId);
    }
    return c.json({ signalled });
  });

  app.get("/:id/bots/:botId", (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    return c.json({ bot: found.record.bot });
  });

  app.post("/:id/bots/:botId/close", (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    closeBot(found.universe, found.record);
    return c.json({ bot: found.record.bot, completed: true });
  });

  app.delete("/:id/bots/:botId", (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    const { universe, record } = found;
    closeBot(universe, record);
    let sessionsDeleted = 0;
    for (const sessionId of record.bot.closedSessions ?? []) {
      if (universe.sessions.delete(sessionId)) sessionsDeleted += 1;
    }
    universe.bots.delete(record.bot.botId);
    return c.json({ deleted: true, sessionsDeleted });
  });

  app.patch("/:id/bots/:botId", async (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    const { universe, record } = found;
    const bot = record.bot;
    const body = await readBody(c);
    const keys = Object.keys(body);
    if (keys.length === 0) return badRequest(c, "at least one field is required");
    // A closed bot is history: labels may change, nothing that would bring it back.
    if (bot.closedAt !== null && !keys.every((key) => key === "displayName" || key === "description")) {
      return conflict(c, "bot is closed");
    }
    if (body.profileId !== undefined) {
      const profileId = optionalString(body.profileId);
      if (!profileId || !universe.profiles.has(profileId)) {
        return badRequest(c, `profile ${String(body.profileId)} does not exist`);
      }
      bot.profileId = profileId;
      record.state.profileId = profileId;
      record.state.appliedProfileRevision = universe.profiles.get(profileId)?.revision ?? null;
    }
    if (body.displayName !== undefined) {
      bot.displayName = optionalString(body.displayName);
      record.state.displayName = bot.displayName;
    }
    if (body.description !== undefined) bot.description = optionalString(body.description);
    if (body.brief !== undefined) bot.brief = optionalString(body.brief);
    if (body.runsPerDay !== undefined) {
      bot.runsPerDay = optionalNumber(body.runsPerDay);
      record.state.runsPerDay = bot.runsPerDay;
    }
    if (body.breaker !== undefined) {
      const breaker = asRecord(body.breaker);
      const fires = breaker?.fires;
      const windowMs = breaker?.windowMs;
      bot.breaker = typeof fires === "number" && typeof windowMs === "number" ? { fires, windowMs } : null;
    }
    if (body.routedSessionTtlMs !== undefined) bot.routedSessionTtlMs = optionalNumber(body.routedSessionTtlMs);
    if (body.selfConfig !== undefined) bot.selfConfig = body.selfConfig === true;
    if (body.emit !== undefined) bot.emit = body.emit === true;
    if (body.enabled !== undefined) bot.enabled = body.enabled === true;
    bot.updatedAt = nowIso();
    return c.json({ bot });
  });

  app.get("/:id/bots/:botId/triggers", (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    const triggers = [...found.record.triggers.values()]
      .sort((left, right) => left.name.localeCompare(right.name))
      .map((trigger) => triggerView(store, found.record, trigger));
    return c.json({ triggers });
  });

  app.post("/:id/bots/:botId/triggers", async (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    const body = await readBody(c);
    try {
      const trigger = createTrigger(store, found.record, body);
      return c.json({ trigger: triggerView(store, found.record, trigger) }, 201);
    } catch (error) {
      return configErrorResponse(c, error);
    }
  });

  app.patch("/:id/bots/:botId/triggers/:triggerName", async (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    const existing = found.record.triggers.get(c.req.param("triggerName") ?? "");
    if (!existing) return notFound(c);
    const body = await readBody(c);
    try {
      const trigger = updateTrigger(store, existing, body);
      return c.json({ trigger: triggerView(store, found.record, trigger) });
    } catch (error) {
      return configErrorResponse(c, error);
    }
  });

  app.delete("/:id/bots/:botId/triggers/:triggerName", (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    const name = c.req.param("triggerName") ?? "";
    if (!found.record.triggers.delete(name)) return notFound(c);
    return c.json({ deleted: true });
  });

  app.get("/:id/bots/:botId/state", (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    return c.json({ state: found.record.state, lineage: found.record.lineage });
  });

  app.post("/:id/bots/:botId/sessions/:sessionId/rotate", (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    const { universe, record } = found;
    const sessionId = c.req.param("sessionId") ?? "";
    const managed = record.state.sessions.find((entry) => entry.sessionId === sessionId);
    if (!managed) return c.json({ error: "session is not managed by this bot" }, 404);
    if (record.bot.closedAt !== null) return conflict(c, "bot is closed");
    record.state.rotatingSessionIds = [...new Set([...(record.state.rotatingSessionIds ?? []), sessionId])];
    setTimeout(() => rotateSession(store, universe, record, managed), ROTATE_DELAY_MS);
    return c.json({ accepted: true, sessionId }, 202);
  });

  app.post("/:id/bots/:botId/events", async (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    const { universe, record } = found;
    if (record.bot.closedAt !== null) return c.json({ error: "bot is closed" }, 410);
    if (!record.bot.enabled) return conflict(c, "bot is disabled");
    const body = await readBody(c);
    const kind = optionalString(body.kind);
    const summary = optionalString(body.summary);
    if (!kind || !summary) return badRequest(c, "validation failed: kind and summary are required");
    const links = Array.isArray(body.links)
      ? body.links.filter((link): link is string => typeof link === "string")
      : undefined;
    const { event, document, duplicate } = admitBotEvent(store, universe, record, {
      kind,
      source: optionalString(body.source) ?? "manual",
      summary,
      payload: body.data,
      ...(typeof body.id === "string" && body.id ? { eventId: body.id } : {}),
      ...(typeof body.occurredAt === "string" ? { occurredAt: body.occurredAt } : {}),
      correlationId: optionalString(body.correlationId),
      ...(links === undefined ? {} : { links }),
    });
    return c.json({ event, document, duplicate }, 202);
  });

  /// A replay is a fresh envelope reusing the stored document and routing.
  app.post("/:id/bots/:botId/events/replay", async (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    const { universe, record } = found;
    if (!record.bot.enabled) return conflict(c, "bot is disabled");
    const body = await readBody(c);
    const eventId = optionalString(body.eventId);
    if (!eventId) return badRequest(c, "validation failed: eventId is required");
    const stored = record.events.find((event) => event.eventId === eventId);
    if (!stored) return notFound(c);
    let summary = `replay of ${stored.eventId}`;
    let payload: unknown;
    const raw = store.readText(stored.ref);
    if (raw) {
      try {
        const document = asRecord(JSON.parse(raw));
        if (typeof document?.summary === "string") summary = document.summary;
        payload = document?.data;
      } catch {
        // Unreadable document: the envelope stub above stands in.
      }
    }
    try {
      const { event } = admitBotEvent(store, universe, record, {
        eventId: `replay-${crypto.randomUUID()}`,
        kind: stored.kind,
        source: stored.source,
        summary,
        payload,
        ref: stored.ref,
        trigger: triggerFromSource(record, stored.source),
        session: stored.session,
      });
      return c.json({ event, original: stored.eventId }, 202);
    } catch (error) {
      return configErrorResponse(c, error);
    }
  });

  app.get("/:id/bots/:botId/events", (c) => {
    const found = botFor(store, c);
    if (!found) return notFound(c);
    const limit = Math.min(intQuery(c, "limit", 50), MAX_HISTORY_LIMIT);
    const cursor = c.req.query("cursor");
    const newestFirst = [...found.record.events].reverse();
    let start = 0;
    if (cursor) {
      const index = newestFirst.findIndex((event) => event.id === cursor);
      if (index < 0) return badRequest(c, "invalid cursor");
      start = index + 1;
    }
    const events = newestFirst.slice(start, start + limit);
    const last = events.at(-1);
    return c.json({
      events,
      nextCursor: last && start + limit < newestFirst.length ? last.id : null,
    });
  });

  return app;
}

// ---------------------------------------------------------------------------
// Webhook ingest
// ---------------------------------------------------------------------------

interface WebhookExtraction {
  eventId: string;
  kind: string;
  summary: string;
  promptData?: unknown;
}

/// The GitHub preset projects the envelope (action, repository, sender, the
/// subject named after the event); plain webhooks take `kind` from the body.
function extractWebhookEvent(
  triggerName: string,
  spec: BotWebhookSpec,
  data: unknown,
  header: (name: string) => string | undefined,
): WebhookExtraction {
  const body = asRecord(data);
  if (spec.preset === "github") {
    const ghEvent = header("x-github-event") ?? "unknown";
    const action = typeof body?.action === "string" ? body.action : null;
    const kind = action ? `${ghEvent}.${action}` : ghEvent;
    const repoName = asRecord(body?.repository)?.full_name;
    const repository = typeof repoName === "string" ? repoName : null;
    const subject = asRecord(body?.[ghEvent]);
    const promptData =
      subject === undefined
        ? data
        : {
            ...(action === null ? {} : { action }),
            ...(repository === null ? {} : { repository }),
            ...(body?.sender === undefined ? {} : { sender: body.sender }),
            [ghEvent]: subject,
          };
    return {
      eventId: header("x-github-delivery") ?? crypto.randomUUID(),
      kind,
      summary: `GitHub ${kind}${repository ? ` in ${repository}` : ""}`,
      promptData,
    };
  }
  const kind = typeof body?.kind === "string" && body.kind ? body.kind.slice(0, 200) : "webhook";
  return {
    // A digest id would dedupe the page's identical sample bodies; a uuid
    // lets every test-fire show up.
    eventId: crypto.randomUUID(),
    kind,
    summary: `Webhook ${kind} received on trigger ${triggerName}`,
  };
}

function findWebhook(
  store: DemoStore,
  triggerId: string,
): { universe: UniverseState; record: BotRecord; trigger: BotTrigger } | null {
  for (const universe of store.universes.values()) {
    for (const record of universe.bots.values()) {
      for (const trigger of record.triggers.values()) {
        if (trigger.kind === "webhook" && `${record.bot.botId}--${trigger.name}` === triggerId) {
          return { universe, record, trigger };
        }
      }
    }
  }
  return null;
}

/// Public ingress, authenticated by the per-trigger URL token alone.
export function hookRoutes(store: DemoStore): Hono {
  const app = new Hono();

  app.post("/bots/:triggerId/:token", async (c) => {
    const found = findWebhook(store, c.req.param("triggerId") ?? "");
    if (!found) return notFound(c);
    const spec = found.trigger.spec as BotWebhookSpec;
    // Token mismatch is indistinguishable from an unknown endpoint.
    if (spec.token !== c.req.param("token")) return notFound(c);
    const raw = await c.req.text();
    if (raw.length > MAX_BODY_BYTES) return c.json({ error: "payload too large" }, 413);
    // A closed bot is gone for good: tell the sender to stop, not to retry.
    if (found.record.bot.closedAt !== null) return c.json({ error: "bot is closed" }, 410);
    if (!found.record.bot.enabled || !found.trigger.enabled) return conflict(c, "trigger is disabled");
    let data: unknown;
    try {
      data = raw ? JSON.parse(raw) : undefined;
    } catch {
      data = undefined;
    }
    const extraction = extractWebhookEvent(found.trigger.name, spec, data, (name) => c.req.header(name));
    const { event, duplicate } = admitBotEvent(store, found.universe, found.record, {
      eventId: extraction.eventId,
      kind: extraction.kind,
      source: `webhook:${found.trigger.name}`,
      summary: extraction.summary,
      payload: data,
      ...(extraction.promptData === undefined ? {} : { promptData: extraction.promptData }),
      trigger: found.trigger,
    });
    return c.json({ eventId: event.eventId, duplicate }, 202);
  });

  return app;
}
