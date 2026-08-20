import {
  condition,
  continueAsNew,
  defineQuery,
  defineSignal,
  proxyActivities,
  setHandler,
  sleep,
  workflowInfo,
} from "@temporalio/workflow";
import type { BotActivities } from "../activities/index.js";
import { DELIVER_EMISSION_SIGNAL, parseEmissionEnvelope } from "../contracts/emissions.js";
import {
  BOT_CONFIG_SIGNAL,
  BOT_CONTROLLER_WORKFLOW,
  BOT_EVENT_RESOLVE_TOOL_ID,
  BOT_EVENT_SIGNAL,
  BOT_STATE_QUERY,
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

const EVENT_TERMINAL_TIMEOUT = "24 hours";
const BUSY_RETRY_DELAY = "5 seconds";
const CONTINUE_AS_NEW_AFTER_RUNS = 100;
const SEEN_EVENT_CAP = 2_000;
const SEEN_EMISSION_CAP = 2_000;
const RECENT_EVENT_CAP = 50;

export interface BotRecentEventSnapshot {
  id: string;
  ref: string;
  status: BotEventOutcome | "unresolved" | "run_failed" | "appended" | "steered";
  eventCount?: number;
  runId?: string;
  summary?: string;
  failure?: string;
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
}

export interface BotSnapshot {
  botName: string;
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
  activeDelivery: { id: string; eventCount: number; sessionId: string } | null;
  activeRunId: string | null;
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
  extraSessions?: BotManagedSession[];
  sessionCursors?: Record<string, number>;
}

const EXTRA_SESSION_CAP = 200;

const activities = proxyActivities<BotActivities>({
  taskQueue: BOTS_ACTIVITY_TASK_QUEUE,
  startToCloseTimeout: "60 seconds",
  retry: { maximumAttempts: 5 },
});

export const botEventSignal = defineSignal<[BotEvent]>(BOT_EVENT_SIGNAL);
export const botConfigSignal = defineSignal<[BotStartV1]>(BOT_CONFIG_SIGNAL);
export const deliverEmissionSignal = defineSignal<[unknown]>(DELIVER_EMISSION_SIGNAL);
export const botStateQuery = defineQuery<BotSnapshot>(BOT_STATE_QUERY);

/** One durable inbox, router, and session lifecycle controller per bot. */
export async function botControllerWorkflowV1(
  initial: BotStartV1,
  carry?: BotCarryV1,
): Promise<never> {
  validateConfig(initial);
  let config = carry?.config ?? initial;
  const sessionId = botSessionId(config.botName);
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
  let eventCursorSeq = carry?.eventCursorSeq ?? 0;
  let appliedProfileId = carry?.appliedProfileId ?? null;
  let appliedProfileRevision = carry?.appliedProfileRevision ?? null;
  let sessionReady = carry?.sessionReady ?? false;
  let eventsProcessed = carry?.eventsProcessed ?? 0;
  let duplicateEventCount = carry?.duplicateEventCount ?? 0;
  let duplicateEmissionCount = carry?.duplicateEmissionCount ?? 0;
  let runDay = carry?.runDay ?? utcDay();
  let runsToday = carry?.runsToday ?? 0;
  let controllerStatus: BotSnapshot["controllerStatus"] = "initializing";
  let activeDelivery: BotDelivery | null = null;
  let activeRunId: string | null = null;
  let activeResolution: { outcome: BotEventOutcome; summary: string | null } | null = null;
  let activeTerminal:
    | { token: string; status: string; runId: string; failureRef: string | null }
    | null = null;
  let configDirty = true;
  let lastError: string | null = null;

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
    config = next;
    configDirty = true;
  });
  setHandler(deliverEmissionSignal, (emission) => {
    emissionInbox.push(emission);
  });
  setHandler(botStateQuery, () => ({
    botName: config.botName,
    profileId: config.profileId,
    sessionId,
    sessions: [
      { sessionId, label: "main", kind: "main" as const },
      ...extraSessions,
    ],
    controllerStatus,
    activeDelivery:
      activeDelivery === null
        ? null
        : {
            id: activeDelivery.id,
            eventCount: activeDelivery.events.length,
            sessionId: activeDelivery.session?.sessionId ?? sessionId,
          },
    activeRunId,
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
    lastError,
  }));

  function utcDay(): string {
    return new Date(Date.now()).toISOString().slice(0, 10);
  }

  function rollBudgetDay(): void {
    const today = utcDay();
    if (today !== runDay) {
      runDay = today;
      runsToday = 0;
    }
  }

  function budgetExhausted(): boolean {
    rollBudgetDay();
    return config.runsPerDay !== null && runsToday >= config.runsPerDay;
  }

  function msUntilNextUtcDay(): number {
    const now = Date.now();
    const next = new Date(now);
    next.setUTCHours(24, 0, 0, 0);
    return Math.max(1_000, next.getTime() - now);
  }

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

  async function reconcileSession(): Promise<boolean> {
    try {
      const ensured = await activities.ensureBotSession({
        universeId: config.universeId,
        sessionId,
        displayName: `bot ${config.botName}`,
        profileId: config.profileId,
        botName: config.botName,
        brief: config.brief,
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
      configDirty = false;
      lastError = null;
      controllerStatus = "idle";
      return true;
    } catch (error) {
      lastError = errorMessage(error);
      controllerStatus = "degraded";
      configDirty = false;
      sessionReady = false;
      await record("degraded", { detail: lastError });
      return false;
    }
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
    for (const invocation of pulled.invocations) {
      if (invocation.runId !== runId) continue;
      if (invocation.toolId !== BOT_EVENT_RESOLVE_TOOL_ID) continue;
      const args = await activities.readJsonBlob({
        universeId: config.universeId,
        blobRef: invocation.argumentsRef,
      });
      const resolution = parseEventResolveArgs(args);
      if (activeDelivery !== null && resolution.eventId === activeDelivery.id) {
        activeResolution = { outcome: resolution.outcome, summary: resolution.summary };
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
          (producer.session_id !== sessionId && !producer.session_id.startsWith(`${sessionId}:`))
        ) {
          throw new TypeError("emission does not belong to this bot's sessions");
        }
        producerSessionId = producer.session_id;
      }
      if (envelope.body.kind !== "run_terminal") continue;
      const terminalRunId = `run_${envelope.body.run_id}`;
      await reconcileRun(terminalRunId, producerSessionId);
      if (
        activeDelivery !== null &&
        envelope.body.token === botEventTerminalToken(activeDelivery.id)
      ) {
        activeTerminal = {
          token: envelope.body.token,
          status: envelope.body.status,
          runId: terminalRunId,
          failureRef: envelope.body.failure_message_ref,
        };
      }
    }
  }

  async function waitUntilSessionIdle(target: string): Promise<void> {
    for (;;) {
      await processEmissions();
      const state = await activities.readBotSessionStatus({
        universeId: config.universeId,
        sessionId: target,
      });
      if (state.status === "idle") {
        controllerStatus = "idle";
        return;
      }
      controllerStatus = "session_busy";
      await sleep(BUSY_RETRY_DELAY);
    }
  }

  /** Create a routed (perKey / perEvent) session on first use. */
  async function ensureRoutedSession(target: {
    sessionId: string;
    label: string;
  }): Promise<boolean> {
    if (ensuredExtra.has(target.sessionId)) return true;
    try {
      await activities.ensureBotSession({
        universeId: config.universeId,
        sessionId: target.sessionId,
        displayName: `bot ${config.botName} · ${target.label}`,
        profileId: config.profileId,
        botName: config.botName,
        brief: config.brief,
        // Routed sessions take the profile at creation; only the main
        // session tracks profile revisions across its lifetime.
        appliedProfileRevision: null,
        controller: {
          workflowId: workflowInfo().workflowId,
          workflowKind: BOT_CONTROLLER_WORKFLOW,
        },
      });
    } catch (error) {
      lastError = errorMessage(error);
      return false;
    }
    ensuredExtra.add(target.sessionId);
    extraSessions.push({
      sessionId: target.sessionId,
      label: target.label,
      kind: target.sessionId.includes(":e-") ? "event" : "keyed",
    });
    if (extraSessions.length > EXTRA_SESSION_CAP) {
      const evicted = extraSessions.splice(0, extraSessions.length - EXTRA_SESSION_CAP);
      for (const session of evicted) {
        ensuredExtra.delete(session.sessionId);
        sessionCursors.delete(session.sessionId);
      }
    }
    return true;
  }

  function finishDelivery(recent: BotRecentEventSnapshot): void {
    recentEvents.push(recent);
    if (recentEvents.length > RECENT_EVENT_CAP) {
      recentEvents.splice(0, recentEvents.length - RECENT_EVENT_CAP);
    }
    eventsProcessed += 1;
    activeDelivery = null;
    activeRunId = null;
    activeResolution = null;
    activeTerminal = null;
    controllerStatus = "idle";
  }

  async function deliverDelivery(delivery: BotDelivery): Promise<void> {
    const firstEvent = delivery.events[0];
    if (firstEvent === undefined) return;
    activeDelivery = delivery;
    activeResolution = null;
    activeTerminal = null;
    controllerStatus = "delivering_event";
    const targetSessionId = delivery.session?.sessionId ?? sessionId;
    if (delivery.session !== undefined) {
      const ensured = await ensureRoutedSession(delivery.session);
      if (!ensured) {
        await record("run_failed", {
          eventId: delivery.id,
          detail: `failed to create session ${delivery.session.sessionId}`,
        });
        finishDelivery({
          id: delivery.id,
          ref: firstEvent.ref,
          status: "run_failed",
          eventCount: delivery.events.length,
          failure: `failed to create session ${delivery.session.sessionId}: ${lastError ?? "unknown"}`,
        });
        return;
      }
    }

    if (delivery.whenBusy === "append") {
      try {
        await activities.appendBotContext({
          universeId: config.universeId,
          sessionId: targetSessionId,
          deliveryId: delivery.id,
          events: delivery.events,
        });
      } catch (error) {
        lastError = errorMessage(error);
        await record("run_failed", { eventId: delivery.id, detail: lastError });
        finishDelivery({
          id: delivery.id,
          ref: firstEvent.ref,
          status: "run_failed",
          eventCount: delivery.events.length,
          failure: `context append failed: ${lastError}`,
        });
        return;
      }
      await record("appended", {
        eventId: delivery.id,
        detail: `${delivery.events.length} event(s) appended as context`,
      });
      lastError = null;
      finishDelivery({
        id: delivery.id,
        ref: firstEvent.ref,
        status: "appended",
        eventCount: delivery.events.length,
      });
      return;
    }

    if (delivery.whenBusy === "steer") {
      const state = await activities.readBotSessionStatus({
        universeId: config.universeId,
        sessionId: targetSessionId,
      });
      if (state.status !== "idle") {
        const steered = await activities.steerBotRun({
          universeId: config.universeId,
          sessionId: targetSessionId,
          deliveryId: delivery.id,
          events: delivery.events,
        });
        if (steered.steered) {
          await record("steered", {
            eventId: delivery.id,
            ...(steered.runId === undefined ? {} : { runId: steered.runId }),
            detail: `${delivery.events.length} event(s) folded into the active run`,
          });
          lastError = null;
          finishDelivery({
            id: delivery.id,
            ref: firstEvent.ref,
            status: "steered",
            eventCount: delivery.events.length,
            ...(steered.runId === undefined ? {} : { runId: steered.runId }),
          });
          return;
        }
        // The run finished under us; fall through to an ordinary run.
      }
    }

    await waitUntilSessionIdle(targetSessionId);
    try {
      const run = await activities.startBotRun({
        universeId: config.universeId,
        sessionId: targetSessionId,
        deliveryId: delivery.id,
        events: delivery.events,
        submissionId: botEventSubmissionId(delivery.id),
        terminalToken: botEventTerminalToken(delivery.id),
      });
      activeRunId = run.runId;
      rollBudgetDay();
      runsToday += 1;
      await record("run_started", { eventId: delivery.id, runId: run.runId });
    } catch (error) {
      // A direct run can win the narrow read/start race. Preserve the
      // delivery and retry after the session becomes idle again.
      lastError = errorMessage(error);
      pendingDeliveries.unshift(delivery);
      activeDelivery = null;
      activeRunId = null;
      controllerStatus = "session_busy";
      await sleep(BUSY_RETRY_DELAY);
      return;
    }

    const signaled = await condition(() => emissionInbox.length > 0, EVENT_TERMINAL_TIMEOUT);
    while (signaled && activeTerminal === null) {
      await processEmissions();
      if (activeTerminal === null) {
        const more = await condition(() => emissionInbox.length > 0, EVENT_TERMINAL_TIMEOUT);
        if (!more) break;
      }
    }

    const terminal = activeTerminal as {
      token: string;
      status: string;
      runId: string;
      failureRef: string | null;
    } | null;
    const resolution = activeResolution as {
      outcome: BotEventOutcome;
      summary: string | null;
    } | null;
    const eventCount = delivery.events.length;
    let recent: BotRecentEventSnapshot;
    if (terminal === null) {
      recent = {
        id: delivery.id,
        ref: firstEvent.ref,
        status: "run_failed",
        eventCount,
        ...(activeRunId === null ? {} : { runId: activeRunId }),
        failure: "timed out waiting for the run terminal",
      };
    } else if (terminal.status !== "completed") {
      recent = {
        id: delivery.id,
        ref: firstEvent.ref,
        status: "run_failed",
        eventCount,
        runId: activeRunId ?? terminal.runId,
        failure: `run ended ${terminal.status}`,
      };
    } else if (resolution !== null) {
      recent = {
        id: delivery.id,
        ref: firstEvent.ref,
        status: resolution.outcome,
        eventCount,
        runId: activeRunId ?? terminal.runId,
        ...(resolution.summary === null ? {} : { summary: resolution.summary }),
      };
    } else {
      recent = {
        id: delivery.id,
        ref: firstEvent.ref,
        status: "unresolved",
        eventCount,
        runId: activeRunId ?? terminal.runId,
      };
    }
    await record(recent.status === "run_failed" ? "run_failed" : "run_completed", {
      eventId: delivery.id,
      ...(recent.runId === undefined ? {} : { runId: recent.runId }),
      detail: recent.failure ?? recent.summary ?? recent.status,
    });
    lastError = null;
    finishDelivery(recent);
  }

  await reconcileSession();

  for (;;) {
    await processEmissions();
    flushRipeBuffers();
    if (configDirty && activeDelivery === null) {
      await waitUntilSessionIdle(sessionId).catch(() => undefined);
      await reconcileSession();
    }
    if (config.enabled && sessionReady && !configDirty && pendingDeliveries.length > 0) {
      if (budgetExhausted()) {
        if (controllerStatus !== "budget_exhausted") {
          controllerStatus = "budget_exhausted";
          await record("budget_exhausted", {
            detail: `${runsToday}/${config.runsPerDay} runs used for ${runDay}`,
          });
        }
        await condition(() => emissionInbox.length > 0 || configDirty, msUntilNextUtcDay());
        continue;
      }
      const delivery = pendingDeliveries.shift();
      if (delivery !== undefined) await deliverDelivery(delivery);
      continue;
    }
    if (
      emissionInbox.length === 0 &&
      activeDelivery === null &&
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
        extraSessions: [...extraSessions],
        sessionCursors: Object.fromEntries(sessionCursors),
      } satisfies BotCarryV1);
    }
    const wake = () =>
      emissionInbox.length > 0 ||
      configDirty ||
      (config.enabled && sessionReady && pendingDeliveries.length > 0);
    const deadline = nextBufferDeadline();
    if (deadline === null) {
      await condition(wake);
    } else {
      // Wake at the earliest buffer flush time; flushRipeBuffers at the top
      // of the loop turns ripe buffers into deliveries.
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
}

function errorMessage(error: unknown): string {
  if (!(error instanceof Error)) return String(error);
  let current: Error = error;
  while (current.cause instanceof Error) current = current.cause;
  return current.message;
}
