import {
  ActivityFailure,
  ApplicationFailure,
  condition,
  continueAsNew,
  defineQuery,
  defineSignal,
  getExternalWorkflowHandle,
  proxyActivities,
  setHandler,
  sleep,
  workflowInfo,
} from "@temporalio/workflow";
import type { BotActivities } from "../activities/index.js";
import {
  DELIVER_EMISSION_SIGNAL,
  parseEmissionEnvelope,
  replyPromiseId,
  sourceResolutionEnvelope,
  type WorkflowToolInvocation,
} from "@lightspeed/agent-client/workflow";
import {
  BOT_CONFIG_SIGNAL,
  BOT_CONTROLLER_WORKFLOW,
  BOT_EVENT_RESOLVE_TOOL_ID,
  BOT_EVENT_SIGNAL,
  BOT_PUSHED_TOOL_IDS,
  BOT_SESSION_DECLARATION_MISMATCH,
  BOT_STATE_QUERY,
  BOT_TOOLS_REVISION,
  BOTS_ACTIVITY_TASK_QUEUE,
  botDeliveryId,
  botEventSubmissionId,
  botEventTerminalToken,
  botSessionId,
  parseEventResolveArgs,
  validateBotEvent,
  type BotCoalesceParamsV1,
  type BotEvent,
  type BotEventOutcome,
  type BotEventSession,
  type BotStartV1,
  type BotWhenBusyV1,
} from "../contracts/bots.js";

const EVENT_TERMINAL_TIMEOUT_MS = 24 * 60 * 60 * 1000;
const BUSY_RETRY_DELAY = "5 seconds";
const CONTINUE_AS_NEW_AFTER_RUNS = 100;
/** How often a busy bot re-counts its sub-agent descendants against the budget. */
const DESCENDANT_REFRESH_INTERVAL_MS = 60_000;
const SEEN_EVENT_CAP = 2_000;
const SEEN_EMISSION_CAP = 2_000;
const RECENT_EVENT_CAP = 50;
const EXTRA_SESSION_CAP = 200;
const HANDLED_INVOCATION_CAP = 2_000;

export interface BotRecentEventSnapshot {
  id: string;
  ref: string;
  /** Event sequence numbers (#N) in this delivery, when known. */
  seqs?: number[];
  status: BotEventOutcome | "unresolved" | "run_failed" | "appended" | "steered";
  eventCount?: number;
  runId?: string;
  summary?: string;
  failure?: string;
  /** Prompt tokens the run consumed and how many came from the provider's cache. */
  usage?: { inputTokens: number; cachedInputTokens: number };
}

/** One unit of work for a session: a single event or a coalesced batch. */
export interface BotDelivery {
  id: string;
  events: BotEvent[];
  session?: BotEventSession;
  whenBusy: BotWhenBusyV1;
}

interface CoalesceBuffer {
  params: BotCoalesceParamsV1;
  session?: BotEventSession;
  whenBusy: BotWhenBusyV1;
  events: BotEvent[];
  firstAtMs: number;
  lastAtMs: number;
}

export interface BotBufferSnapshot {
  key: string;
  count: number;
  flushAtMs: number;
}

export interface BotManagedSession {
  sessionId: string;
  label: string;
  kind: "main" | "keyed" | "event";
  lastActiveAtMs?: number;
}

export interface BotActiveDeliverySnapshot {
  id: string;
  eventCount: number;
  sessionId: string;
  runId: string | null;
}

export interface BotSnapshot {
  botName: string;
  displayName: string | null;
  profileId: string;
  sessionId: string;
  sessions: BotManagedSession[];
  controllerStatus:
    | "initializing"
    | "idle"
    | "session_busy"
    | "delivering_event"
    | "budget_exhausted"
    | "degraded";
  activeDeliveries: BotActiveDeliverySnapshot[];
  sessionReady: boolean;
  pendingEventCount: number;
  pendingDeliveryCount: number;
  buffers: BotBufferSnapshot[];
  recentEvents: BotRecentEventSnapshot[];
  eventsProcessed: number;
  duplicateEventCount: number;
  duplicateEmissionCount: number;
  appliedProfileRevision: number | null;
  runsPerDay: number | null;
  runsToday: number;
  /** Sub-agent sessions delegated under the bot's sessions today; they count against `runsPerDay`. */
  descendantsToday: number;
  mainGeneration: number;
  toolsRevision: number | null;
  lastError: string | null;
}

export interface BotCarryV1 {
  version: 1;
  config: BotStartV1;
  /** Legacy single-event queue; converted to deliveries on start. */
  pendingEvents?: BotEvent[];
  pendingDeliveries?: BotDelivery[];
  buffers?: [string, CoalesceBuffer][];
  recentEvents: BotRecentEventSnapshot[];
  seenEventIds: string[];
  seenEmissionIds: string[];
  eventCursorSeq: number;
  appliedProfileId: string | null;
  appliedProfileRevision: number | null;
  sessionReady: boolean;
  eventsProcessed: number;
  duplicateEventCount: number;
  duplicateEmissionCount: number;
  runDay: string;
  runsToday: number;
  descendantsToday?: number;
  /** Bot sessions whose sub-agent trees are counted for `runDay`. */
  budgetRoots?: string[];
  extraSessions?: BotManagedSession[];
  sessionCursors?: Record<string, number>;
  /** Routed session id → generation, bumped each time that session closes. */
  sessionGenerations?: Record<string, number>;
  /** Main session generation; bumped when the tool declaration rotates it. */
  mainGeneration?: number;
  /** Tool declaration revision the main session was created under. */
  toolsRevision?: number | null;
  handledInvocationIds?: string[];
}

interface ActiveDelivery {
  delivery: BotDelivery;
  sessionId: string;
  runId: string | null;
  terminal: { token: string; status: string; runId: string; failureRef: string | null } | null;
  resolution: { outcome: BotEventOutcome; summary: string | null } | null;
}

const activities = proxyActivities<BotActivities>({
  taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
  startToCloseTimeout: "60 seconds",
  retry: { maximumAttempts: 5 },
});

export const botEventSignal = defineSignal<[BotEvent]>(BOT_EVENT_SIGNAL);
export const botConfigSignal = defineSignal<[BotStartV1]>(BOT_CONFIG_SIGNAL);
export const deliverEmissionSignal = defineSignal<[unknown]>(DELIVER_EMISSION_SIGNAL);
export const botStateQuery = defineQuery<BotSnapshot>(BOT_STATE_QUERY);

/**
 * One durable inbox, router, and session lifecycle controller per bot.
 * Deliveries run as one lane per target session: a stalled run blocks only
 * its own session while other sessions keep receiving work.
 */
export async function botControllerWorkflowV1(
  initial: BotStartV1,
  carry?: BotCarryV1,
): Promise<never> {
  validateConfig(initial);
  let config = carry?.config ?? initial;
  const baseSessionId = botSessionId(config.botName);
  let mainGeneration = carry?.mainGeneration ?? 1;
  let toolsRevision: number | null = carry?.toolsRevision ?? null;
  const mainSessionIdFor = (generation: number): string =>
    generation === 1 ? baseSessionId : `${baseSessionId}-g${generation}`;
  let sessionId = mainSessionIdFor(mainGeneration);
  const handledInvocationIds = new Set(carry?.handledInvocationIds ?? []);
  const pendingDeliveries: BotDelivery[] = [
    ...(carry?.pendingEvents ?? []).map((event) => ({
      id: event.id,
      events: [event],
      ...(event.session === undefined ? {} : { session: event.session }),
      whenBusy: (event.deliver?.whenBusy ?? "queue") as BotWhenBusyV1,
    })),
    ...(carry?.pendingDeliveries ?? []),
  ];
  const buffers = new Map<string, CoalesceBuffer>(carry?.buffers ?? []);
  const recentEvents = [...(carry?.recentEvents ?? [])];
  const seenEventIds = new Set(
    carry?.seenEventIds ??
      pendingDeliveries.flatMap((delivery) => delivery.events.map((event) => event.id)),
  );
  const seenEmissionIds = new Set(carry?.seenEmissionIds ?? []);
  const emissionInbox: unknown[] = [];
  // Sessions this controller has created beyond the main one (perKey /
  // perEvent routing). The list is capped for carry; an evicted session's
  // cursor restarts at zero, which only re-reads already-consumed facts.
  const extraSessions: BotManagedSession[] = [...(carry?.extraSessions ?? [])];
  const ensuredExtra = new Set(extraSessions.map((session) => session.sessionId));
  const sessionCursors = new Map<string, number>(Object.entries(carry?.sessionCursors ?? {}));
  const sessionGenerations = new Map<string, number>(
    Object.entries(carry?.sessionGenerations ?? {}),
  );
  const activeBySession = new Map<string, ActiveDelivery>();
  // A steer/append delivery may act on a session already occupied by the
  // terminal-tracked delivery. Keep one ordered sidecar per session without
  // replacing that delivery in activeBySession.
  const sidecarBySession = new Set<string>();
  let eventCursorSeq = carry?.eventCursorSeq ?? 0;
  let appliedProfileId = carry?.appliedProfileId ?? null;
  let appliedProfileRevision = carry?.appliedProfileRevision ?? null;
  let sessionReady = carry?.sessionReady ?? false;
  let eventsProcessed = carry?.eventsProcessed ?? 0;
  let duplicateEventCount = carry?.duplicateEventCount ?? 0;
  let duplicateEmissionCount = carry?.duplicateEmissionCount ?? 0;
  let runDay = carry?.runDay ?? utcDay();
  let runsToday = carry?.runsToday ?? 0;
  // Sub-agent sessions under the bot's sessions (P134 lineage) count like
  // runs. The count is read from core: the controller never sees the
  // delegations themselves, only their sessions by root.
  let descendantsToday = carry?.descendantsToday ?? 0;
  const budgetRoots = new Set<string>(carry?.budgetRoots ?? []);
  let descendantsRefreshedAtMs = 0;
  let descendantsRefreshedProcessed = -1;
  let budgetNotified = false;
  // Bumped whenever a lane changes shared state outside the main loop, so a
  // parked loop re-evaluates deadlines (retention, budget) and dispatch.
  let laneTick = 0;
  let setupStatus: "initializing" | "degraded" | "ready" = "initializing";
  let configDirty = true;
  // A display-name change is label-only; it renames sessions, never rotates them.
  let renameDirty = false;
  let lastError: string | null = null;

  function utcDay(): string {
    return new Date(Date.now()).toISOString().slice(0, 10);
  }

  function rollBudgetDay(): void {
    const today = utcDay();
    if (today !== runDay) {
      runDay = today;
      runsToday = 0;
      descendantsToday = 0;
      budgetRoots.clear();
      descendantsRefreshedAtMs = 0;
      descendantsRefreshedProcessed = -1;
      budgetNotified = false;
    }
  }

  function descendantsRefreshDue(): boolean {
    if (config.runsPerDay === null) return false;
    if (eventsProcessed !== descendantsRefreshedProcessed) return true;
    return (
      activeBySession.size > 0 &&
      Date.now() - descendantsRefreshedAtMs >= DESCENDANT_REFRESH_INTERVAL_MS
    );
  }

  /**
   * Re-count today's sub-agent descendants: after every finished delivery
   * and, while a run is in flight, once a minute. Best effort — a core
   * outage keeps the last count rather than blocking dispatch.
   */
  async function refreshDescendantsToday(): Promise<void> {
    rollBudgetDay();
    if (!descendantsRefreshDue()) return;
    budgetRoots.add(sessionId);
    for (const session of extraSessions) budgetRoots.add(session.sessionId);
    descendantsRefreshedAtMs = Date.now();
    descendantsRefreshedProcessed = eventsProcessed;
    try {
      const counted = await activities.countBotDescendantSessions({
        universeId: config.universeId,
        sessionIds: [...budgetRoots],
        sinceMs: Date.parse(`${runDay}T00:00:00.000Z`),
      });
      descendantsToday = counted.count;
    } catch (error) {
      lastError = errorMessage(error);
    }
  }

  /** Runs already started today, sub-agent sessions delegated today, and lanes about to start a run. */
  function reservedRuns(): number {
    let reserved = runsToday + descendantsToday;
    for (const active of activeBySession.values()) {
      if (active.runId === null && active.delivery.whenBusy !== "append") reserved += 1;
    }
    return reserved;
  }

  function budgetExhausted(): boolean {
    rollBudgetDay();
    return config.runsPerDay !== null && reservedRuns() >= config.runsPerDay;
  }

  /** Pure variant for the query handler, which must not mutate state. */
  function budgetExhaustedView(): boolean {
    if (config.runsPerDay === null) return false;
    if (utcDay() !== runDay) return false;
    return reservedRuns() >= config.runsPerDay;
  }

  function msUntilNextUtcDay(): number {
    const now = Date.now();
    const next = new Date(now);
    next.setUTCHours(24, 0, 0, 0);
    return Math.max(1_000, next.getTime() - now);
  }

  /** Session display names carry the label, never the id. */
  function botLabel(): string {
    return `bot ${config.displayName ?? config.botName}`;
  }

  function routedBase(target: string): string {
    return target.replace(/-g\d+$/, "");
  }

  /** Current session id for a routed target, accounting for closed generations. */
  function resolveRoutedSessionId(base: string): string {
    const generation = sessionGenerations.get(base);
    return generation === undefined ? base : `${base}-g${generation}`;
  }

  function targetOf(delivery: BotDelivery): string {
    return delivery.session === undefined
      ? sessionId
      : resolveRoutedSessionId(delivery.session.sessionId);
  }

  function flushBuffer(key: string): void {
    const buffer = buffers.get(key);
    if (buffer === undefined || buffer.events.length === 0) return;
    buffers.delete(key);
    pendingDeliveries.push({
      id: botDeliveryId(buffer.events.map((event) => event.id)),
      events: buffer.events,
      ...(buffer.session === undefined ? {} : { session: buffer.session }),
      whenBusy: buffer.whenBusy,
    });
  }

  function flushRipeBuffers(): void {
    const now = Date.now();
    for (const [key, buffer] of [...buffers]) {
      if (
        now >= buffer.lastAtMs + buffer.params.debounceMs ||
        now >= buffer.firstAtMs + buffer.params.maxWaitMs
      ) {
        flushBuffer(key);
      }
    }
  }

  function nextBufferDeadline(): number | null {
    let earliest: number | null = null;
    for (const buffer of buffers.values()) {
      const deadline = Math.min(
        buffer.lastAtMs + buffer.params.debounceMs,
        buffer.firstAtMs + buffer.params.maxWaitMs,
      );
      if (earliest === null || deadline < earliest) earliest = deadline;
    }
    return earliest;
  }

  function nextRetentionDeadline(): number | null {
    const ttl = config.routedSessionTtlMs ?? null;
    if (ttl === null) return null;
    let earliest: number | null = null;
    for (const session of extraSessions) {
      if (activeBySession.has(session.sessionId) || sidecarBySession.has(session.sessionId)) continue;
      const expiry = (session.lastActiveAtMs ?? 0) + ttl;
      if (earliest === null || expiry < earliest) earliest = expiry;
    }
    return earliest;
  }

  function touchSession(target: string): void {
    const session = extraSessions.find((entry) => entry.sessionId === target);
    if (session !== undefined) session.lastActiveAtMs = Date.now();
  }

  function dispatchable(): boolean {
    if (!config.enabled || !sessionReady || configDirty) return false;
    if (pendingDeliveries.length === 0) return false;
    if (config.runsPerDay !== null && utcDay() === runDay && reservedRuns() >= config.runsPerDay) {
      return false;
    }
    return pendingDeliveries.some((delivery) => {
      const target = targetOf(delivery);
      const active = activeBySession.get(target);
      if (active === undefined) return true;
      return (
        active.runId !== null &&
        delivery.whenBusy !== "queue" &&
        !sidecarBySession.has(target)
      );
    });
  }

  setHandler(botEventSignal, (event) => {
    validateBotEvent(event);
    if (seenEventIds.has(event.id)) {
      duplicateEventCount += 1;
      return;
    }
    seenEventIds.add(event.id);
    const whenBusy = event.deliver?.whenBusy ?? "queue";
    if (event.coalesce === undefined) {
      pendingDeliveries.push({
        id: event.id,
        events: [event],
        ...(event.session === undefined ? {} : { session: event.session }),
        whenBusy,
      });
      return;
    }
    const params = event.coalesce;
    const now = Date.now();
    let buffer = buffers.get(params.key);
    if (buffer === undefined) {
      buffer = {
        params,
        ...(event.session === undefined ? {} : { session: event.session }),
        whenBusy,
        events: [],
        firstAtMs: now,
        lastAtMs: now,
      };
      buffers.set(params.key, buffer);
    }
    buffer.events.push(event);
    buffer.lastAtMs = now;
    buffer.params = params;
    if (buffer.events.length >= params.maxCount) flushBuffer(params.key);
  });
  setHandler(botConfigSignal, (next) => {
    validateConfig(next);
    if (
      next.universeId !== initial.universeId ||
      next.botId !== initial.botId ||
      next.botName !== initial.botName
    ) {
      throw new TypeError("bot identity cannot change");
    }
    if (next.displayName !== config.displayName) renameDirty = true;
    config = next;
    configDirty = true;
  });
  setHandler(deliverEmissionSignal, (emission) => {
    emissionInbox.push(emission);
  });
  setHandler(botStateQuery, () => ({
    botName: config.botName,
    displayName: config.displayName,
    profileId: config.profileId,
    sessionId,
    sessions: [{ sessionId, label: "main", kind: "main" as const }, ...extraSessions],
    controllerStatus:
      setupStatus !== "ready"
        ? setupStatus
        : activeBySession.size > 0
          ? "delivering_event"
          : pendingDeliveries.length > 0 && budgetExhaustedView()
            ? "budget_exhausted"
            : "idle",
    activeDeliveries: [...activeBySession.values()].map((active) => ({
      id: active.delivery.id,
      eventCount: active.delivery.events.length,
      sessionId: active.sessionId,
      runId: active.runId,
    })),
    sessionReady,
    pendingEventCount:
      pendingDeliveries.reduce((sum, delivery) => sum + delivery.events.length, 0) +
      [...buffers.values()].reduce((sum, buffer) => sum + buffer.events.length, 0),
    pendingDeliveryCount: pendingDeliveries.length,
    buffers: [...buffers.entries()].map(([key, buffer]) => ({
      key,
      count: buffer.events.length,
      flushAtMs: Math.min(
        buffer.lastAtMs + buffer.params.debounceMs,
        buffer.firstAtMs + buffer.params.maxWaitMs,
      ),
    })),
    recentEvents: [...recentEvents],
    eventsProcessed,
    duplicateEventCount,
    duplicateEmissionCount,
    appliedProfileRevision,
    runsPerDay: config.runsPerDay,
    runsToday,
    descendantsToday,
    mainGeneration,
    toolsRevision,
    lastError,
  }));

  async function record(
    kind: string,
    fields?: { eventId?: string; runId?: string; detail?: string },
  ): Promise<void> {
    await activities
      .recordBotActivity({
        botId: config.botId,
        entries: [{ kind, ...fields }],
      })
      .catch(() => undefined);
  }

  function isDeclarationMismatch(error: unknown): boolean {
    return (
      error instanceof ActivityFailure &&
      error.cause instanceof ApplicationFailure &&
      error.cause.type === BOT_SESSION_DECLARATION_MISMATCH
    );
  }

  async function reconcileSession(): Promise<boolean> {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      try {
        const ensured = await activities.ensureBotSession({
          universeId: config.universeId,
          sessionId,
          displayName: botLabel(),
          profileId: config.profileId,
          botName: config.botName,
          brief: config.brief,
          selfConfig: config.selfConfig === true,
          emit: config.emit === true,
          appliedProfileRevision:
            appliedProfileId === config.profileId ? appliedProfileRevision : null,
          controller: {
            workflowId: workflowInfo().workflowId,
            workflowKind: BOT_CONTROLLER_WORKFLOW,
          },
        });
        sessionReady = true;
        appliedProfileId = config.profileId;
        appliedProfileRevision = ensured.profileRevision;
        toolsRevision = BOT_TOOLS_REVISION;
        configDirty = false;
        lastError = null;
        setupStatus = "ready";
        return true;
      } catch (error) {
        if (isDeclarationMismatch(error) && attempt === 0) {
          // Tool declarations are immutable per session: rotate to a
          // successor main session rather than editing the live one.
          const previous = sessionId;
          mainGeneration += 1;
          sessionId = mainSessionIdFor(mainGeneration);
          eventCursorSeq = 0;
          appliedProfileId = null;
          appliedProfileRevision = null;
          await record("session_rotated", {
            detail: `main session ${previous} rotated to ${sessionId} for tool revision ${BOT_TOOLS_REVISION}`,
          });
          continue;
        }
        lastError = errorMessage(error);
        setupStatus = "degraded";
        configDirty = false;
        sessionReady = false;
        await record("degraded", { detail: lastError });
        return false;
      }
    }
    return false;
  }

  function cursorFor(target: string): number {
    return target === sessionId ? eventCursorSeq : (sessionCursors.get(target) ?? 0);
  }

  function setCursor(target: string, seq: number): void {
    if (target === sessionId) eventCursorSeq = seq;
    else sessionCursors.set(target, seq);
  }

  async function reconcileRun(runId: string, target: string): Promise<void> {
    const pulled = await activities.readWorkflowToolInvocations({
      universeId: config.universeId,
      sessionId: target,
      afterSeq: cursorFor(target),
    });
    setCursor(target, pulled.nextSeq);
    const active = activeBySession.get(target);
    for (const invocation of pulled.invocations) {
      if (invocation.runId !== runId) continue;
      if (invocation.toolId !== BOT_EVENT_RESOLVE_TOOL_ID) continue;
      const args = await activities.readJsonBlob({
        universeId: config.universeId,
        blobRef: invocation.argumentsRef,
      });
      const resolution = parseEventResolveArgs(args);
      if (active !== undefined) {
        // Run-scoped correlation: this lane runs exactly one delivery per
        // run, so any resolve from the run decides this delivery — the model
        // never echoes a delivery id. A repeated call's last decision wins.
        active.resolution = { outcome: resolution.outcome, summary: resolution.summary };
      }
    }
  }

  async function processEmissions(): Promise<void> {
    while (emissionInbox.length > 0) {
      const raw = emissionInbox.shift();
      if (raw === undefined) continue;
      const envelope = parseEmissionEnvelope(raw);
      if (seenEmissionIds.has(envelope.emission_id)) {
        duplicateEmissionCount += 1;
        continue;
      }
      seenEmissionIds.add(envelope.emission_id);
      let producerSessionId = sessionId;
      if (envelope.producer.kind === "session") {
        const producer = envelope.producer;
        if (
          producer.universe_id !== config.universeId ||
          !producer.session_id.startsWith(baseSessionId)
        ) {
          throw new TypeError("emission does not belong to this bot's sessions");
        }
        producerSessionId = producer.session_id;
      }
      if (envelope.body.kind === "tool_invocation") {
        const invocation = envelope.body.invocation;
        if (!BOT_PUSHED_TOOL_IDS.has(invocation.tool_id)) continue;
        if (handledInvocationIds.has(invocation.invocation_id)) continue;
        handledInvocationIds.add(invocation.invocation_id);
        if (handledInvocationIds.size > HANDLED_INVOCATION_CAP) {
          const first = handledInvocationIds.values().next().value;
          if (first !== undefined) handledInvocationIds.delete(first);
        }
        void handleInvocation(invocation, envelope.body.holder_workflow_id);
        continue;
      }
      if (envelope.body.kind !== "run_terminal") continue;
      const terminalRunId = `run_${envelope.body.run_id}`;
      await reconcileRun(terminalRunId, producerSessionId);
      const token = envelope.body.token;
      for (const active of activeBySession.values()) {
        if (token === botEventTerminalToken(active.delivery.id)) {
          active.terminal = {
            token,
            status: envelope.body.status,
            runId: terminalRunId,
            failureRef: envelope.body.failure_message_ref ?? null,
          };
        }
      }
    }
  }

  /** The highest hop count among a delivery's events (0 when none carry one). */
  function deliveryHops(delivery: BotDelivery): number {
    return delivery.events.reduce((max, event) => Math.max(max, event.hops ?? 0), 0);
  }

  /**
   * What the tool activity may show the model: labels and `#N`s only.
   * Session ids, delivery ids, and buffer keys stay here. `invocation` is
   * the federation context of the invoking session — private to `bot_emit`.
   */
  function controllerSummary(invocationSessionId: string) {
    const active = activeBySession.get(invocationSessionId);
    const routed = extraSessions.find((session) => session.sessionId === invocationSessionId);
    return {
      invocation: {
        hops: active === undefined ? 0 : deliveryHops(active.delivery),
        // A logical route: the base id, never a generation, so a receipt
        // finds the session after a rotation.
        ...(routed === undefined
          ? {}
          : { session: { sessionId: routedBase(routed.sessionId), label: routed.label } }),
      },
      sessions: [{ label: "main", kind: "main" }, ...extraSessions].map((session) => ({
        label: session.label,
        kind: session.kind,
      })),
      activeDeliveries: [...activeBySession.values()].map((active) => ({
        events: active.delivery.events.flatMap((event) => (event.seq === undefined ? [] : [event.seq])),
        session: active.delivery.session?.label ?? "main",
      })),
      buffers: [...buffers.values()].map((buffer) => ({
        session: buffer.session?.label ?? "main",
        count: buffer.events.length,
        flushAtMs: Math.min(
          buffer.lastAtMs + buffer.params.debounceMs,
          buffer.firstAtMs + buffer.params.maxWaitMs,
        ),
      })),
      runsToday,
      eventsProcessed,
    };
  }

  /**
   * Apply a display-name change to every managed session. Label-only and
   * best effort: the bot id, workflow id, and session ids never move, so a
   * failed rename costs a stale label, never a delivery.
   */
  async function applyDisplayName(): Promise<void> {
    renameDirty = false;
    const targets = [{ sessionId, label: "main", kind: "main" as const }, ...extraSessions];
    for (const target of targets) {
      const displayName =
        target.kind === "main" ? botLabel() : `${botLabel()} · ${target.label}`;
      try {
        await activities.renameBotSession({
          universeId: config.universeId,
          sessionId: target.sessionId,
          displayName,
        });
      } catch (error) {
        lastError = errorMessage(error);
      }
    }
    await record("renamed", {
      detail: `display name is now "${config.displayName ?? config.botName}"`,
    });
  }

  /**
   * Answer a pushed bot_* invocation from the controller's own state and
   * activities, then resolve the session's parked call by signalling the
   * session workflow directly. Runs as its own lane, independent of delivery
   * and terminal handling. Every pushed tool is joined — including
   * `bot_emit`, whose refusals (the rate cap) the model must read.
   */
  async function handleInvocation(
    invocation: WorkflowToolInvocation,
    holderWorkflowId: string,
  ): Promise<void> {
    let resolution:
      | { kind: "resolved"; payload_ref: string | null }
      | { kind: "failed"; error_ref: string | null };
    try {
      const args = await activities.readJsonBlob({
        universeId: config.universeId,
        blobRef: invocation.arguments_ref,
      });
      const result = await activities.executeBotTool({
        universeId: config.universeId,
        botId: config.botId,
        botName: config.botName,
        sessionId: invocation.session_id,
        invocationId: invocation.invocation_id,
        toolId: invocation.tool_id,
        args,
        controller: controllerSummary(invocation.session_id),
      });
      if (result.ok) {
        resolution = { kind: "resolved", payload_ref: result.payloadRef };
      } else {
        resolution = { kind: "failed", error_ref: result.errorRef };
        await record("tool_failed", { detail: `${invocation.tool_id}: ${result.message}` });
      }
    } catch (error) {
      lastError = errorMessage(error);
      await record("tool_failed", { detail: `${invocation.tool_id}: ${lastError}` });
      resolution = { kind: "failed", error_ref: null };
    }
    try {
      const holder = getExternalWorkflowHandle(holderWorkflowId);
      await holder.signal(
        DELIVER_EMISSION_SIGNAL,
        sourceResolutionEnvelope({
          universeId: config.universeId,
          producerWorkflowId: workflowInfo().workflowId,
          holderWorkflowId,
          promiseId: replyPromiseId(invocation),
          resolution,
        }),
      );
    } catch (error) {
      lastError = errorMessage(error);
      await record("tool_failed", {
        detail: `${invocation.tool_id}: reply delivery failed: ${lastError}`,
      });
    }
    laneTick += 1;
  }

  async function waitUntilSessionIdle(target: string): Promise<void> {
    for (;;) {
      await processEmissions();
      const state = await activities.readBotSessionStatus({
        universeId: config.universeId,
        sessionId: target,
      });
      if (state.status === "idle") return;
      await sleep(BUSY_RETRY_DELAY);
    }
  }

  /**
   * Create a routed (perKey / perEvent) session on first use, returning the
   * session id actually ensured. A declaration mismatch — the session
   * pre-exists under an older toolset, e.g. after a controller restart
   * without carry — rotates the key to its next generation and retries
   * once, mirroring the main session's rotation, instead of wedging the
   * delivery.
   */
  async function ensureRoutedSession(
    target: BotEventSession,
    resolvedId: string,
  ): Promise<string | null> {
    let sessionIdToEnsure = resolvedId;
    for (let attempt = 0; attempt < 2; attempt += 1) {
      if (ensuredExtra.has(sessionIdToEnsure)) return sessionIdToEnsure;
      try {
        await activities.ensureBotSession({
          universeId: config.universeId,
          sessionId: sessionIdToEnsure,
          displayName: `${botLabel()} · ${target.label}`,
          profileId: config.profileId,
          botName: config.botName,
          brief: config.brief,
          selfConfig: config.selfConfig === true,
          emit: config.emit === true,
          // Routed sessions take the profile at creation; only the main
          // session tracks profile revisions across its lifetime.
          appliedProfileRevision: null,
          controller: {
            workflowId: workflowInfo().workflowId,
            workflowKind: BOT_CONTROLLER_WORKFLOW,
          },
        });
      } catch (error) {
        if (isDeclarationMismatch(error) && attempt === 0) {
          const base = routedBase(sessionIdToEnsure);
          sessionGenerations.set(base, (sessionGenerations.get(base) ?? 1) + 1);
          const previous = sessionIdToEnsure;
          sessionIdToEnsure = resolveRoutedSessionId(base);
          await record("session_rotated", {
            detail: `routed session ${previous} rotated to ${sessionIdToEnsure} after a declaration mismatch`,
          });
          continue;
        }
        lastError = errorMessage(error);
        return null;
      }
      ensuredExtra.add(sessionIdToEnsure);
      extraSessions.push({
        sessionId: sessionIdToEnsure,
        label: target.label,
        kind: sessionIdToEnsure.includes(":e-") ? "event" : "keyed",
        lastActiveAtMs: Date.now(),
      });
      if (extraSessions.length > EXTRA_SESSION_CAP) {
        const evicted = extraSessions.splice(0, extraSessions.length - EXTRA_SESSION_CAP);
        for (const session of evicted) {
          ensuredExtra.delete(session.sessionId);
          sessionCursors.delete(session.sessionId);
        }
      }
      return sessionIdToEnsure;
    }
    return null;
  }

  /** Close routed sessions idle past the retention window. */
  async function sweepRoutedSessions(): Promise<void> {
    const ttl = config.routedSessionTtlMs ?? null;
    if (ttl === null) return;
    const now = Date.now();
    for (const session of [...extraSessions]) {
      if (activeBySession.has(session.sessionId) || sidecarBySession.has(session.sessionId)) continue;
      if ((session.lastActiveAtMs ?? 0) + ttl > now) continue;
      let closed = false;
      let descendantsClosed = 0;
      try {
        const result = await activities.closeBotSession({
          universeId: config.universeId,
          sessionId: session.sessionId,
        });
        closed = result.closed;
        descendantsClosed = result.descendantsClosed ?? 0;
      } catch (error) {
        lastError = errorMessage(error);
      }
      if (!closed) {
        // Busy or unreachable: push the expiry out and retry next sweep.
        session.lastActiveAtMs = now;
        continue;
      }
      const index = extraSessions.indexOf(session);
      if (index >= 0) extraSessions.splice(index, 1);
      ensuredExtra.delete(session.sessionId);
      sessionCursors.delete(session.sessionId);
      const base = routedBase(session.sessionId);
      sessionGenerations.set(base, (sessionGenerations.get(base) ?? 1) + 1);
      await record("session_closed", {
        detail:
          `closed idle routed session ${session.sessionId} (${session.label})` +
          (descendantsClosed > 0 ? ` and ${descendantsClosed} sub-agent session(s)` : ""),
      });
    }
  }

  function rememberDelivery(delivery: BotDelivery, recent: BotRecentEventSnapshot): void {
    recentEvents.push(recent);
    if (recentEvents.length > RECENT_EVENT_CAP) {
      recentEvents.splice(0, recentEvents.length - RECENT_EVENT_CAP);
    }
    eventsProcessed += 1;
    void settleReceipts(delivery, recent);
  }

  /**
   * Receipts for events that asked for one ride on the delivery's finish:
   * the receiver's outcome, deterministic, never a reply the model authors.
   * Best effort — a failed receipt is an activity row, never a stuck lane.
   */
  async function settleReceipts(delivery: BotDelivery, recent: BotRecentEventSnapshot): Promise<void> {
    const asked = delivery.events.filter((event) => event.reply === true);
    if (asked.length === 0) return;
    try {
      await activities.sendBotReceipts({
        universeId: config.universeId,
        botId: config.botId,
        deliveryId: delivery.id,
        eventIds: asked.map((event) => event.id),
        status: recent.status,
        summary: recent.summary ?? null,
        hops: deliveryHops(delivery),
      });
    } catch (error) {
      lastError = errorMessage(error);
      await record("reply_failed", { eventId: delivery.id, detail: lastError });
    }
    laneTick += 1;
  }

  function finishDelivery(active: ActiveDelivery, recent: BotRecentEventSnapshot): void {
    const seqs = active.delivery.events.flatMap((event) =>
      event.seq === undefined ? [] : [event.seq],
    );
    if (seqs.length > 0) recent.seqs = seqs;
    rememberDelivery(active.delivery, recent);
    activeBySession.delete(active.sessionId);
    touchSession(active.sessionId);
    laneTick += 1;
  }

  async function runBusySidecar(delivery: BotDelivery, target: string): Promise<void> {
    const firstEvent = delivery.events[0];
    if (firstEvent === undefined) return;
    const eventCount = delivery.events.length;
    try {
      if (delivery.whenBusy === "append") {
        await activities.appendBotContext({
          universeId: config.universeId,
          sessionId: target,
          deliveryId: delivery.id,
          events: delivery.events,
        });
        await record("appended", {
          eventId: delivery.id,
          detail: `${eventCount} event(s) appended as context`,
        });
        rememberDelivery(delivery, {
          id: delivery.id,
          ref: firstEvent.ref,
          status: "appended",
          eventCount,
        });
        return;
      }

      const state = await activities.readBotSessionStatus({
        universeId: config.universeId,
        sessionId: target,
      });
      if (state.status !== "idle") {
        const steered = await activities.steerBotRun({
          universeId: config.universeId,
          sessionId: target,
          deliveryId: delivery.id,
          events: delivery.events,
        });
        if (steered.steered) {
          await record("steered", {
            eventId: delivery.id,
            ...(steered.runId === undefined ? {} : { runId: steered.runId }),
            detail: `${eventCount} event(s) folded into the active run`,
          });
          rememberDelivery(delivery, {
            id: delivery.id,
            ref: firstEvent.ref,
            status: "steered",
            eventCount,
            ...(steered.runId === undefined ? {} : { runId: steered.runId }),
          });
          return;
        }
      }

      // The tracked run finished (or has not started) under us. Preserve the
      // delivery for an ordinary lane attempt once the session is available.
      pendingDeliveries.unshift(delivery);
    } catch (error) {
      lastError = errorMessage(error);
      await record("run_failed", { eventId: delivery.id, detail: lastError });
      rememberDelivery(delivery, {
        id: delivery.id,
        ref: firstEvent.ref,
        status: "run_failed",
        eventCount,
        failure: lastError,
      });
    } finally {
      sidecarBySession.delete(target);
      touchSession(target);
      laneTick += 1;
    }
  }

  async function runDelivery(active: ActiveDelivery): Promise<void> {
    const { delivery } = active;
    const firstEvent = delivery.events[0];
    if (firstEvent === undefined) {
      activeBySession.delete(active.sessionId);
      return;
    }
    const eventCount = delivery.events.length;
    let target = active.sessionId;
    try {
      if (delivery.session !== undefined) {
        const ensured = await ensureRoutedSession(delivery.session, target);
        if (ensured === null) {
          await record("run_failed", {
            eventId: delivery.id,
            detail: `failed to create session ${target}`,
          });
          finishDelivery(active, {
            id: delivery.id,
            ref: firstEvent.ref,
            status: "run_failed",
            eventCount,
            failure: `failed to create session ${target}: ${lastError ?? "unknown"}`,
          });
          return;
        }
        if (ensured !== target) {
          // The routed session rotated during ensure; move this lane to the
          // successor id so terminals and busy checks find it.
          activeBySession.delete(target);
          active.sessionId = ensured;
          activeBySession.set(ensured, active);
          target = ensured;
        }
      }

      if (config.emit === true) {
        // An emitting bot reads the directory before it decides. A failed
        // put costs a stale directory, never the delivery.
        try {
          await activities.publishBotDirectory({
            universeId: config.universeId,
            botId: config.botId,
            sessionId: target,
          });
        } catch (error) {
          lastError = errorMessage(error);
        }
      }

      if (delivery.whenBusy === "append") {
        await activities.appendBotContext({
          universeId: config.universeId,
          sessionId: target,
          deliveryId: delivery.id,
          events: delivery.events,
        });
        await record("appended", {
          eventId: delivery.id,
          detail: `${eventCount} event(s) appended as context`,
        });
        finishDelivery(active, {
          id: delivery.id,
          ref: firstEvent.ref,
          status: "appended",
          eventCount,
        });
        return;
      }

      if (delivery.whenBusy === "steer") {
        const state = await activities.readBotSessionStatus({
          universeId: config.universeId,
          sessionId: target,
        });
        if (state.status !== "idle") {
          const steered = await activities.steerBotRun({
            universeId: config.universeId,
            sessionId: target,
            deliveryId: delivery.id,
            events: delivery.events,
          });
          if (steered.steered) {
            await record("steered", {
              eventId: delivery.id,
              ...(steered.runId === undefined ? {} : { runId: steered.runId }),
              detail: `${eventCount} event(s) folded into the active run`,
            });
            finishDelivery(active, {
              id: delivery.id,
              ref: firstEvent.ref,
              status: "steered",
              eventCount,
              ...(steered.runId === undefined ? {} : { runId: steered.runId }),
            });
            return;
          }
          // The run finished under us; fall through to an ordinary run.
        }
      }

      await waitUntilSessionIdle(target);
      try {
        const run = await activities.startBotRun({
          universeId: config.universeId,
          sessionId: target,
          deliveryId: delivery.id,
          events: delivery.events,
          submissionId: botEventSubmissionId(delivery.id),
          terminalToken: botEventTerminalToken(delivery.id),
        });
        active.runId = run.runId;
        rollBudgetDay();
        runsToday += 1;
        await record("run_started", { eventId: delivery.id, runId: run.runId });
        laneTick += 1;
      } catch (error) {
        // A direct run can win the narrow read/start race. Hold the lane
        // through a short delay, then requeue at the front for this session.
        lastError = errorMessage(error);
        await sleep(BUSY_RETRY_DELAY);
        activeBySession.delete(target);
        pendingDeliveries.unshift(delivery);
        laneTick += 1;
        return;
      }

      await condition(() => active.terminal !== null, EVENT_TERMINAL_TIMEOUT_MS);

      const terminal = active.terminal;
      const resolution = active.resolution;
      let recent: BotRecentEventSnapshot;
      if (terminal === null) {
        recent = {
          id: delivery.id,
          ref: firstEvent.ref,
          status: "run_failed",
          eventCount,
          ...(active.runId === null ? {} : { runId: active.runId }),
          failure: "timed out waiting for the run terminal",
        };
      } else if (terminal.status !== "completed") {
        recent = {
          id: delivery.id,
          ref: firstEvent.ref,
          status: "run_failed",
          eventCount,
          runId: active.runId ?? terminal.runId,
          failure: `run ended ${terminal.status}`,
        };
      } else if (resolution !== null) {
        recent = {
          id: delivery.id,
          ref: firstEvent.ref,
          status: resolution.outcome,
          eventCount,
          runId: active.runId ?? terminal.runId,
          ...(resolution.summary === null ? {} : { summary: resolution.summary }),
        };
      } else {
        recent = {
          id: delivery.id,
          ref: firstEvent.ref,
          status: "unresolved",
          eventCount,
          runId: active.runId ?? terminal.runId,
        };
      }
      if (recent.runId !== undefined && recent.status !== "run_failed") {
        // Best effort: the cached share is observability, never a reason to
        // fail a delivery that already finished.
        try {
          const usage = await activities.readBotRunUsage({
            universeId: config.universeId,
            sessionId: target,
            runId: recent.runId,
          });
          if (usage !== null) recent.usage = usage;
        } catch {
          // The read is retried by Temporal; a final failure leaves usage unset.
        }
      }
      const cachedShare =
        recent.usage === undefined
          ? ""
          : ` · ${Math.round((recent.usage.cachedInputTokens / recent.usage.inputTokens) * 100)}% of ${recent.usage.inputTokens} prompt tokens cached`;
      await record(recent.status === "run_failed" ? "run_failed" : "run_completed", {
        eventId: delivery.id,
        ...(recent.runId === undefined ? {} : { runId: recent.runId }),
        detail: `${recent.failure ?? recent.summary ?? recent.status}${cachedShare}`,
      });
      finishDelivery(active, recent);
    } catch (error) {
      lastError = errorMessage(error);
      await record("run_failed", { eventId: delivery.id, detail: lastError });
      finishDelivery(active, {
        id: delivery.id,
        ref: firstEvent.ref,
        status: "run_failed",
        eventCount,
        failure: lastError,
      });
    }
  }

  function dispatch(): void {
    let index = 0;
    while (index < pendingDeliveries.length) {
      if (budgetExhausted()) return;
      const delivery = pendingDeliveries[index];
      if (delivery === undefined) break;
      const target = targetOf(delivery);
      const occupied = activeBySession.get(target);
      if (occupied !== undefined) {
        if (
          occupied.runId !== null &&
          delivery.whenBusy !== "queue" &&
          !sidecarBySession.has(target)
        ) {
          pendingDeliveries.splice(index, 1);
          sidecarBySession.add(target);
          void runBusySidecar(delivery, target);
          continue;
        }
        index += 1;
        continue;
      }
      pendingDeliveries.splice(index, 1);
      const active: ActiveDelivery = {
        delivery,
        sessionId: target,
        runId: null,
        terminal: null,
        resolution: null,
      };
      activeBySession.set(target, active);
      void runDelivery(active);
    }
  }

  await reconcileSession();

  for (;;) {
    await processEmissions();
    flushRipeBuffers();
    await sweepRoutedSessions();
    await refreshDescendantsToday();
    if (renameDirty) await applyDisplayName();
    if (configDirty && !activeBySession.has(sessionId) && !sidecarBySession.has(sessionId)) {
      await waitUntilSessionIdle(sessionId).catch(() => undefined);
      await reconcileSession();
    }
    if (config.enabled && sessionReady && !configDirty) {
      dispatch();
      if (pendingDeliveries.length > 0 && budgetExhausted() && !budgetNotified) {
        budgetNotified = true;
        await record("budget_exhausted", {
          detail:
            `${runsToday + descendantsToday}/${config.runsPerDay} runs used for ${runDay}` +
            (descendantsToday > 0
              ? ` (${runsToday} bot runs, ${descendantsToday} sub-agent sessions)`
              : ""),
        });
      }
    }
    if (
      emissionInbox.length === 0 &&
      activeBySession.size === 0 &&
      sidecarBySession.size === 0 &&
      eventsProcessed >= CONTINUE_AS_NEW_AFTER_RUNS
    ) {
      await continueAsNew<typeof botControllerWorkflowV1>(config, {
        version: 1,
        config,
        pendingDeliveries,
        buffers: [...buffers.entries()],
        recentEvents,
        seenEventIds: [...seenEventIds].slice(-SEEN_EVENT_CAP),
        seenEmissionIds: [...seenEmissionIds].slice(-SEEN_EMISSION_CAP),
        eventCursorSeq,
        appliedProfileId,
        appliedProfileRevision,
        sessionReady,
        eventsProcessed: 0,
        duplicateEventCount,
        duplicateEmissionCount,
        runDay,
        runsToday,
        descendantsToday,
        budgetRoots: [...budgetRoots],
        extraSessions: [...extraSessions],
        sessionCursors: Object.fromEntries(sessionCursors),
        sessionGenerations: Object.fromEntries(sessionGenerations),
        mainGeneration,
        toolsRevision,
        handledInvocationIds: [...handledInvocationIds].slice(-HANDLED_INVOCATION_CAP),
      } satisfies BotCarryV1);
    }
    const tick = laneTick;
    const wake = () =>
      emissionInbox.length > 0 ||
      (configDirty && !activeBySession.has(sessionId) && !sidecarBySession.has(sessionId)) ||
      laneTick !== tick ||
      dispatchable();
    const deadlines = [nextBufferDeadline(), nextRetentionDeadline()];
    if (pendingDeliveries.length > 0 && config.runsPerDay !== null && budgetExhaustedView()) {
      deadlines.push(Date.now() + msUntilNextUtcDay());
    }
    if (config.runsPerDay !== null && activeBySession.size > 0) {
      // A run in flight may be delegating; re-count against the budget.
      deadlines.push(descendantsRefreshedAtMs + DESCENDANT_REFRESH_INTERVAL_MS);
    }
    const deadline = deadlines.reduce<number | null>(
      (earliest, value) => (value === null ? earliest : earliest === null ? value : Math.min(earliest, value)),
      null,
    );
    if (deadline === null) {
      await condition(wake);
    } else {
      // Wake at the earliest buffer flush, retention expiry, or budget reset;
      // the top of the loop turns ripe state into work.
      await condition(wake, Math.max(1, deadline - Date.now()));
    }
  }
}

function validateConfig(config: BotStartV1): void {
  if (config.version !== 1) throw new TypeError("unsupported bot config version");
  if (!config.profileId) throw new TypeError("profileId is required");
  if (config.runsPerDay !== null && (!Number.isSafeInteger(config.runsPerDay) || config.runsPerDay < 1)) {
    throw new TypeError("runsPerDay must be a positive integer or null");
  }
  const ttl = config.routedSessionTtlMs ?? null;
  if (ttl !== null && (!Number.isSafeInteger(ttl) || ttl < 1_000)) {
    throw new TypeError("routedSessionTtlMs must be at least 1000 or null");
  }
}

function errorMessage(error: unknown): string {
  if (!(error instanceof Error)) return String(error);
  let current: Error = error;
  while (current.cause instanceof Error) current = current.cause;
  return current.message;
}
