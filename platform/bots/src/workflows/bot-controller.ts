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
  botEventSubmissionId,
  botEventTerminalToken,
  botSessionId,
  parseEventResolveArgs,
  validateBotEvent,
  type BotEvent,
  type BotEventOutcome,
  type BotStartV1,
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
  status: BotEventOutcome | "unresolved" | "run_failed";
  runId?: string;
  summary?: string;
  failure?: string;
}

export interface BotSnapshot {
  botName: string;
  profileId: string;
  sessionId: string;
  controllerStatus:
    | "initializing"
    | "idle"
    | "session_busy"
    | "delivering_event"
    | "budget_exhausted"
    | "degraded";
  activeEvent: BotEvent | null;
  activeRunId: string | null;
  sessionReady: boolean;
  pendingEvents: BotEvent[];
  pendingEventCount: number;
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
  pendingEvents: BotEvent[];
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

/** One durable inbox, router, and session lifecycle controller per bot. */
export async function botControllerWorkflowV1(
  initial: BotStartV1,
  carry?: BotCarryV1,
): Promise<never> {
  validateConfig(initial);
  let config = carry?.config ?? initial;
  const sessionId = botSessionId(config.botName);
  const pendingEvents = [...(carry?.pendingEvents ?? [])];
  const recentEvents = [...(carry?.recentEvents ?? [])];
  const seenEventIds = new Set(carry?.seenEventIds ?? pendingEvents.map((event) => event.id));
  const seenEmissionIds = new Set(carry?.seenEmissionIds ?? []);
  const emissionInbox: unknown[] = [];
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
  let activeEvent: BotEvent | null = null;
  let activeRunId: string | null = null;
  let activeResolution: { outcome: BotEventOutcome; summary: string | null } | null = null;
  let activeTerminal:
    | { token: string; status: string; runId: string; failureRef: string | null }
    | null = null;
  let configDirty = true;
  let lastError: string | null = null;

  setHandler(botEventSignal, (event) => {
    validateBotEvent(event);
    if (seenEventIds.has(event.id)) {
      duplicateEventCount += 1;
      return;
    }
    seenEventIds.add(event.id);
    pendingEvents.push(event);
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
    controllerStatus,
    activeEvent,
    activeRunId,
    sessionReady,
    pendingEvents: pendingEvents.slice(0, 100),
    pendingEventCount: pendingEvents.length,
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

  async function reconcileRun(runId: string): Promise<void> {
    const pulled = await activities.readWorkflowToolInvocations({
      universeId: config.universeId,
      sessionId,
      afterSeq: eventCursorSeq,
    });
    eventCursorSeq = pulled.nextSeq;
    for (const invocation of pulled.invocations) {
      if (invocation.runId !== runId) continue;
      if (invocation.toolId !== BOT_EVENT_RESOLVE_TOOL_ID) continue;
      const args = await activities.readJsonBlob({
        universeId: config.universeId,
        blobRef: invocation.argumentsRef,
      });
      const resolution = parseEventResolveArgs(args);
      if (activeEvent !== null && resolution.eventId === activeEvent.id) {
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
      if (
        envelope.producer.kind === "session" &&
        (envelope.producer.universe_id !== config.universeId ||
          envelope.producer.session_id !== sessionId)
      ) {
        throw new TypeError("emission does not belong to this bot's session");
      }
      if (envelope.body.kind !== "run_terminal") continue;
      const terminalRunId = `run_${envelope.body.run_id}`;
      await reconcileRun(terminalRunId);
      if (activeEvent !== null && envelope.body.token === botEventTerminalToken(activeEvent.id)) {
        activeTerminal = {
          token: envelope.body.token,
          status: envelope.body.status,
          runId: terminalRunId,
          failureRef: envelope.body.failure_message_ref,
        };
      }
    }
  }

  async function waitUntilSessionIdle(): Promise<void> {
    for (;;) {
      await processEmissions();
      const state = await activities.readBotSessionStatus({
        universeId: config.universeId,
        sessionId,
      });
      if (state.status === "idle") {
        controllerStatus = "idle";
        return;
      }
      controllerStatus = "session_busy";
      await sleep(BUSY_RETRY_DELAY);
    }
  }

  async function deliverEvent(event: BotEvent): Promise<void> {
    activeEvent = event;
    activeResolution = null;
    activeTerminal = null;
    controllerStatus = "delivering_event";
    await waitUntilSessionIdle();
    try {
      const run = await activities.startBotRun({
        universeId: config.universeId,
        sessionId,
        event,
        submissionId: botEventSubmissionId(event.id),
        terminalToken: botEventTerminalToken(event.id),
      });
      activeRunId = run.runId;
      rollBudgetDay();
      runsToday += 1;
      await record("run_started", { eventId: event.id, runId: run.runId });
    } catch (error) {
      // A direct run can win the narrow read/start race. Preserve the event
      // and retry after the session becomes idle again.
      lastError = errorMessage(error);
      pendingEvents.unshift(event);
      activeEvent = null;
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
    let recent: BotRecentEventSnapshot;
    if (terminal === null) {
      recent = {
        id: event.id,
        ref: event.ref,
        status: "run_failed",
        ...(activeRunId === null ? {} : { runId: activeRunId }),
        failure: "timed out waiting for the run terminal",
      };
    } else if (terminal.status !== "completed") {
      recent = {
        id: event.id,
        ref: event.ref,
        status: "run_failed",
        runId: activeRunId ?? terminal.runId,
        failure: `run ended ${terminal.status}`,
      };
    } else if (resolution !== null) {
      recent = {
        id: event.id,
        ref: event.ref,
        status: resolution.outcome,
        runId: activeRunId ?? terminal.runId,
        ...(resolution.summary === null ? {} : { summary: resolution.summary }),
      };
    } else {
      recent = {
        id: event.id,
        ref: event.ref,
        status: "unresolved",
        runId: activeRunId ?? terminal.runId,
      };
    }
    recentEvents.push(recent);
    if (recentEvents.length > RECENT_EVENT_CAP) {
      recentEvents.splice(0, recentEvents.length - RECENT_EVENT_CAP);
    }
    await record(recent.status === "run_failed" ? "run_failed" : "run_completed", {
      eventId: event.id,
      ...(recent.runId === undefined ? {} : { runId: recent.runId }),
      detail: recent.failure ?? recent.summary ?? recent.status,
    });
    eventsProcessed += 1;
    activeEvent = null;
    activeRunId = null;
    activeResolution = null;
    activeTerminal = null;
    lastError = null;
    controllerStatus = "idle";
  }

  await reconcileSession();

  for (;;) {
    await processEmissions();
    if (configDirty && activeEvent === null) {
      await waitUntilSessionIdle().catch(() => undefined);
      await reconcileSession();
    }
    if (config.enabled && sessionReady && !configDirty && pendingEvents.length > 0) {
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
      const event = pendingEvents.shift();
      if (event !== undefined) await deliverEvent(event);
      continue;
    }
    if (
      emissionInbox.length === 0 &&
      activeEvent === null &&
      eventsProcessed >= CONTINUE_AS_NEW_AFTER_RUNS
    ) {
      await continueAsNew<typeof botControllerWorkflowV1>(config, {
        version: 1,
        config,
        pendingEvents,
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
      } satisfies BotCarryV1);
    }
    await condition(
      () =>
        emissionInbox.length > 0 ||
        configDirty ||
        (config.enabled && sessionReady && pendingEvents.length > 0),
    );
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
