import type { ToolItemStatus } from "@lightspeed-ai/agent-client";
import type { SessionEvent, SessionItem, SessionRunView, ToolCallDisplay } from "@/api";

/// Folded chat model for a session. The event log is the source of truth;
/// this module reduces it into renderable entries plus live-run state,
/// incrementally — the tail hook applies event pages as they arrive.
/// Patterns lifted from the Lightspeed chat TUI: entries dedupe by id
/// (replaced context states repeat them), a tool call and its result merge
/// into one entry, run-lifecycle events drive a human status vocabulary,
/// and opaque engine reasoning markers are filtered out.

export type TranscriptEntry =
  | {
      kind: "message";
      key: string;
      role: "user" | "assistant";
      text: string;
      contentRef: string;
      textTruncated: boolean;
      citations?: Array<{ url: string; title?: string | null; citedText?: string | null }>;
      /// The run this entry belongs to, when the engine recorded one.
      runId?: string;
      /// A user message injected into a running run (steering) rather than
      /// the input that started it.
      steering?: boolean;
    }
  | {
      kind: "system";
      key: string;
      text: string;
      /// A catalog version a newer version has updated; kept in context so
      /// the rendered prefix stays cacheable, shown dimmed.
      superseded?: boolean;
    }
  | { kind: "reasoning"; key: string; text: string }
  | TranscriptToolGroup
  | { kind: "marker"; key: string; text: string; tone: "muted" | "error" };

export interface TranscriptToolCall {
  callId: string;
  toolId?: string | null;
  /// The original name returned by the model, preserved across model changes.
  toolName: string;
  status: ToolItemStatus;
  argumentsJson?: string | null;
  output?: string | null;
  outputContentRef?: string;
  outputTruncated?: boolean;
  error?: string | null;
  isError: boolean;
  display?: ToolCallDisplay | null;
  effects?: Array<{ kind?: string; data?: Record<string, string> }>;
}

export interface TranscriptToolGroup {
  kind: "tool-group";
  key: string;
  batchId?: string;
  status: TranscriptToolGroupStatus;
  calls: TranscriptToolCall[];
}

export type TranscriptToolGroupStatus =
  | ToolItemStatus
  | "waiting"
  | "completedWithErrors";

type ToolBatchStartedEvent = Extract<
  SessionEvent["kind"],
  { type: "toolBatchStarted" }
>;

interface ToolCallLocation {
  entryIndex: number;
  callIndex: number;
}

export interface ActiveRun {
  runId: string;
  /// TUI status vocabulary: running / planning / thinking / running tools /
  /// working / cancelling.
  label: string;
  /// A cancel was admitted; the run is draining to `cancelled`.
  cancelling: boolean;
}

export interface QueuedRun {
  runId: string;
}

/// Monotonic per-run lifecycle knowledge: a run only ever moves forward
/// (queued → running → terminal), which is what lets a session snapshot be
/// reconciled into the event-derived state without going backwards.
export type RunPhase = "queued" | "running" | "terminal";

export interface TranscriptState {
  entries: TranscriptEntry[];
  /// The run the engine is executing (running or cancelling), if any.
  activeRun: ActiveRun | null;
  /// Runs accepted behind the active run, in start order.
  queuedRuns: QueuedRun[];
  /// Bumped on every run lifecycle change so the page can refresh the
  /// authoritative session view (queued-run text, terminal statuses).
  runRevision: number;
  closed: boolean;
  /// Entry ids already folded (context events repeat entries on replace).
  seenItems: Set<string>;
  runPhases: Map<string, RunPhase>;
  /// Client submission id → engine run id, from `runAccepted`. Lets an
  /// optimistic send resolve its run before its own POST returns (the tail
  /// is usually faster than the gateway's acceptance poll).
  runBySubmission: Map<string, string>;
  /// Tool event metadata, context items, and results arrive separately.
  /// These indexes merge them into one stable group in the transcript.
  toolCallByCallId: Map<string, ToolCallLocation>;
  toolGroupByBatchId: Map<string, number>;
  /// Provider-reported tokens per run, summed over its generations, with the
  /// share served from prompt cache — surfaced when the run finishes.
  runUsage: Map<string, RunUsage>;
  /// Committed wall-clock start times used to calculate terminal duration.
  runStartedAtMs: Map<string, number>;
}

export interface RunUsage {
  inputTokens: number;
  cachedInputTokens?: number;
  outputTokens: number;
}

export function emptyTranscript(): TranscriptState {
  return {
    entries: [],
    activeRun: null,
    queuedRuns: [],
    runRevision: 0,
    closed: false,
    seenItems: new Set(),
    runPhases: new Map(),
    runBySubmission: new Map(),
    toolCallByCallId: new Map(),
    toolGroupByBatchId: new Map(),
    runUsage: new Map(),
    runStartedAtMs: new Map(),
  };
}

/// "8.2s · 12.3k tokens in (94% cached) · 750 tokens out" for a finished
/// run. Duration is still useful when the provider reports no usage.
export function describeRunSummary(
  usage: RunUsage | undefined,
  durationMs: number | undefined,
): string | null {
  const parts: string[] = [];
  if (durationMs !== undefined) {
    parts.push(formatDuration(durationMs));
  }
  if (usage && usage.inputTokens > 0) {
    const cached = usage.cachedInputTokens;
    const cacheLabel = cached !== undefined && cached > 0
      ? ` (${Math.round((cached / usage.inputTokens) * 100)}% cached)`
      : "";
    parts.push(`${formatTokens(usage.inputTokens)} tokens in${cacheLabel}`);
  }
  if (usage && usage.outputTokens > 0) {
    parts.push(`${formatTokens(usage.outputTokens)} tokens out`);
  }
  if (parts.length === 1 && durationMs !== undefined) return `run completed in ${parts[0]}`;
  return parts.length ? parts.join(" · ") : null;
}

function formatTokens(count: number): string {
  return count >= 10_000
    ? `${Math.round(count / 1000)}k`
    : count >= 1000
      ? `${(count / 1000).toFixed(1)}k`
      : String(count);
}

function formatDuration(durationMs: number): string {
  if (durationMs < 1_000) return `${durationMs}ms`;
  if (durationMs < 10_000) return `${(durationMs / 1_000).toFixed(1)}s`;
  const totalSeconds = Math.round(durationMs / 1_000);
  if (totalSeconds < 60) return `${totalSeconds}s`;
  if (totalSeconds < 3_600) {
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return seconds ? `${minutes}m ${seconds}s` : `${minutes}m`;
  }
  const hours = Math.floor(totalSeconds / 3_600);
  const minutes = Math.floor((totalSeconds % 3_600) / 60);
  return minutes ? `${hours}h ${minutes}m` : `${hours}h`;
}

/// A run is live when the engine is executing or has queued it; clients use
/// this to offer stop/steer/queue instead of a plain send.
export function runInProgress(state: TranscriptState): boolean {
  return state.activeRun !== null || state.queuedRuns.length > 0;
}

/// Returns a new state object (fresh `entries` array identity for React);
/// the dedup set/map are monotonic and carried over.
export function applyEvents(
  state: TranscriptState,
  events: SessionEvent[],
): TranscriptState {
  const next: TranscriptState = {
    entries: state.entries.slice(),
    activeRun: state.activeRun,
    queuedRuns: state.queuedRuns,
    runRevision: state.runRevision,
    closed: state.closed,
    seenItems: state.seenItems,
    runPhases: state.runPhases,
    runBySubmission: state.runBySubmission,
    toolCallByCallId: state.toolCallByCallId,
    toolGroupByBatchId: state.toolGroupByBatchId,
    runUsage: state.runUsage,
    runStartedAtMs: state.runStartedAtMs,
  };
  for (const event of events) {
    const kind = event.kind;
    switch (kind.type) {
      case "contextEntriesApplied":
      case "contextKeyPrefixReplaced":
      case "contextStateReplaced":
        applyItems(next, kind.entries);
        break;
      case "runAccepted":
        if (kind.submissionId) {
          next.runBySubmission.set(kind.submissionId, String(kind.runId));
        }
        enqueueRun(next, String(kind.runId));
        break;
      case "runStarted":
        startRun(next, String(kind.runId), event.observedAtMs);
        break;
      case "runCancellationRequested":
        if (next.activeRun?.runId === String(kind.runId)) {
          next.activeRun = { ...next.activeRun, label: "cancelling", cancelling: true };
          next.runRevision += 1;
        }
        break;
      case "runSteeringAccepted":
        // The steering text lands as a context entry once it materializes;
        // nothing to fold yet beyond the revision bump for the page.
        next.runRevision += 1;
        break;
      case "approvalRequested":
        next.runRevision += 1;
        break;
      case "approvalRunParked":
        setRunLabel(next, "approval required");
        next.runRevision += 1;
        break;
      case "approvalDecided":
      case "approvalCancelled":
        next.runRevision += 1;
        break;
      case "turnStarted":
        setRunLabel(next, "planning");
        break;
      case "turnPlanned":
      case "turnGenerationRequested":
        setRunLabel(next, "thinking");
        break;
      case "turnGenerationCompleted":
        if (kind.usage && (kind.usage.inputTokens != null || kind.usage.outputTokens != null)) {
          const runId = String(kind.runId);
          const current = next.runUsage.get(runId) ?? {
            inputTokens: 0,
            outputTokens: 0,
          };
          const usage: RunUsage = {
            inputTokens: current.inputTokens + (kind.usage.inputTokens ?? 0),
            outputTokens: current.outputTokens + (kind.usage.outputTokens ?? 0),
          };
          if (kind.usage.cachedInputTokens != null) {
            usage.cachedInputTokens = (current.cachedInputTokens ?? 0)
              + kind.usage.cachedInputTokens;
          } else if (current.cachedInputTokens !== undefined) {
            usage.cachedInputTokens = current.cachedInputTokens;
          }
          next.runUsage.set(runId, usage);
        }
        break;
      case "toolBatchStarted":
        applyToolBatchStarted(next, kind);
        setRunLabel(next, "running tools");
        break;
      case "toolCallStarted":
        updateToolCall(next, String(kind.callId), (call) => ({
          ...call,
          status: "running",
        }));
        setRunLabel(next, "running tools");
        break;
      case "toolCallCompleted": {
        updateToolCall(next, String(kind.callId), (call) => ({
          ...call,
          status: kind.status,
          ...(kind.effects?.length ? { effects: kind.effects } : {}),
        }));
        syncToolGroupStatusForCall(next, String(kind.callId));
        break;
      }
      case "toolBatchDeferred":
        updateToolGroupStatus(next, String(kind.batchId), "waiting");
        setRunLabel(next, "waiting for tools");
        break;
      case "toolBatchResumed":
        updateToolGroupStatus(next, String(kind.batchId), "running");
        setRunLabel(next, "running tools");
        break;
      case "toolBatchCompleted":
        completeToolGroup(next, String(kind.batchId));
        setRunLabel(next, "working");
        break;
      case "runCompleted": {
        const runId = String(kind.runId);
        const alreadyFinished = next.runPhases.get(runId) === "terminal";
        finishRun(next, runId);
        const summary = describeRunSummary(
          next.runUsage.get(runId),
          runDurationMs(next, runId, event.observedAtMs),
        );
        if (summary && !alreadyFinished) {
          next.entries.push({
            kind: "marker",
            key: `evt-${event.cursor.seq}`,
            text: summary,
            tone: "muted",
          });
        }
        break;
      }
      case "runFailed": {
        const runId = String(kind.runId);
        const durationMs = runDurationMs(next, runId, event.observedAtMs);
        finishRun(next, runId);
        next.entries.push({
          kind: "marker",
          key: `evt-${event.cursor.seq}`,
          text: `run failed${durationMs === undefined ? "" : ` after ${formatDuration(durationMs)}`}: ${String(kind.message ?? "unknown error")}`,
          tone: "error",
        });
        break;
      }
      case "runCancelled": {
        const wasQueued = next.queuedRuns.some((run) => run.runId === String(kind.runId));
        const runId = String(kind.runId);
        const durationMs = runDurationMs(next, runId, event.observedAtMs);
        finishRun(next, runId);
        next.entries.push({
          kind: "marker",
          key: `evt-${event.cursor.seq}`,
          text: wasQueued
            ? "queued message cancelled"
            : `run cancelled${durationMs === undefined ? "" : ` after ${formatDuration(durationMs)}`}`,
          tone: "muted",
        });
        break;
      }
      case "contextCompactionFinished":
        next.entries.push({
          kind: "marker",
          key: `evt-${event.cursor.seq}`,
          text: "context compacted",
          tone: "muted",
        });
        break;
      case "sessionClosed":
        next.activeRun = null;
        next.queuedRuns = [];
        next.runRevision += 1;
        next.closed = true;
        next.entries.push({
          kind: "marker",
          key: `evt-${event.cursor.seq}`,
          text: "session closed",
          tone: "muted",
        });
        break;
      default:
        break;
    }
  }
  return next;
}

function setRunLabel(state: TranscriptState, label: string) {
  if (state.activeRun && !state.activeRun.cancelling) {
    state.activeRun = { ...state.activeRun, label };
  }
}

function enqueueRun(state: TranscriptState, runId: string) {
  if (state.runPhases.has(runId)) {
    return;
  }
  state.runPhases.set(runId, "queued");
  state.queuedRuns = [...state.queuedRuns, { runId }];
  state.runRevision += 1;
}

function startRun(state: TranscriptState, runId: string, observedAtMs?: number) {
  const phase = state.runPhases.get(runId);
  if (phase === "running" || phase === "terminal") {
    return;
  }
  state.runPhases.set(runId, "running");
  if (observedAtMs !== undefined && !state.runStartedAtMs.has(runId)) {
    state.runStartedAtMs.set(runId, observedAtMs);
  }
  state.queuedRuns = state.queuedRuns.filter((run) => run.runId !== runId);
  state.activeRun = { runId, label: "running", cancelling: false };
  state.runRevision += 1;
}

function runDurationMs(
  state: TranscriptState,
  runId: string,
  completedAtMs: number,
): number | undefined {
  const startedAtMs = state.runStartedAtMs.get(runId);
  return startedAtMs === undefined ? undefined : Math.max(0, completedAtMs - startedAtMs);
}

function finishRun(state: TranscriptState, runId: string) {
  if (state.runPhases.get(runId) === "terminal") {
    return;
  }
  state.runPhases.set(runId, "terminal");
  state.queuedRuns = state.queuedRuns.filter((run) => run.runId !== runId);
  if (state.activeRun?.runId === runId) {
    state.activeRun = null;
  }
  state.runRevision += 1;
}

/// Fold the authoritative session view into the event-derived state. Only
/// forward moves are applied (a snapshot can be older than the tail): a run
/// the snapshot shows terminal is cleared, a running run the tail has not
/// seen yet becomes the active run, a queued run the tail has not seen is
/// enqueued. This is what heals a truncated catch-up or a missed page.
export function reconcileRuns(
  state: TranscriptState,
  runs: SessionRunView[],
): TranscriptState {
  const next: TranscriptState = { ...state, entries: state.entries };
  let changed = false;
  for (const run of runs) {
    const runId = String(run.id);
    if (run.startedAtMs != null && !next.runStartedAtMs.has(runId)) {
      next.runStartedAtMs.set(runId, run.startedAtMs);
    }
    const phase = next.runPhases.get(runId);
    switch (run.status) {
      case "completed":
      case "failed":
      case "cancelled":
        if (phase !== "terminal") {
          finishRun(next, runId);
          changed = true;
        }
        break;
      case "running":
      case "parked":
      case "cancelling":
        if (phase === undefined || phase === "queued") {
          startRun(next, runId);
          if (run.status === "cancelling" && next.activeRun?.runId === runId) {
            next.activeRun = { ...next.activeRun, label: "cancelling", cancelling: true };
          } else if (run.status === "parked" && next.activeRun?.runId === runId) {
            next.activeRun = { ...next.activeRun, label: "approval required" };
          }
          changed = true;
        } else if (
          run.status === "cancelling" &&
          next.activeRun?.runId === runId &&
          !next.activeRun.cancelling
        ) {
          next.activeRun = { ...next.activeRun, label: "cancelling", cancelling: true };
          changed = true;
        } else if (
          run.status === "parked" &&
          next.activeRun?.runId === runId &&
          next.activeRun.label !== "approval required"
        ) {
          next.activeRun = { ...next.activeRun, label: "approval required" };
          changed = true;
        }
        break;
      case "queued":
        if (phase === undefined) {
          enqueueRun(next, runId);
          changed = true;
        }
        break;
      default:
        break;
    }
  }
  return changed ? next : state;
}

function applyItems(state: TranscriptState, items: SessionItem[]) {
  let generatedToolGroup: number | null = null;

  for (const item of items) {
    if (state.seenItems.has(item.id)) {
      continue;
    }
    state.seenItems.add(item.id);
    const kind = item.kind;

    if (kind.type === "toolCall") {
      const existing = state.toolCallByCallId.get(kind.callId);
      if (existing) {
        updateToolCall(state, kind.callId, (call) => ({
          ...call,
          toolName: item.display?.toolName ?? call.toolName,
          argumentsJson: item.display?.arguments ?? call.argumentsJson,
          display: item.display?.summary ?? call.display,
        }));
        continue;
      }
      if (generatedToolGroup === null) {
        generatedToolGroup = createToolGroup(state, item.id, "requested");
      }
      appendToolCall(state, generatedToolGroup, {
        callId: kind.callId,
        toolName: item.display?.toolName ?? kind.name,
        status: item.display?.status ?? "requested",
        argumentsJson: item.display?.arguments,
        display: item.display?.summary,
        isError: false,
      });
      syncToolGroupStatusForCall(state, kind.callId);
      continue;
    }

    applyNonToolCallItem(state, item, kind);
  }
}

function applyNonToolCallItem(
  state: TranscriptState,
  item: SessionItem,
  kind: SessionItem["kind"],
) {
  switch (kind.type) {
    case "message":
      if (item.text && (kind.role === "user" || kind.role === "assistant")) {
        const source = item.source ?? null;
        const runId = source && "runId" in source ? String(source.runId) : undefined;
        state.entries.push({
          kind: "message",
          key: item.id,
          role: kind.role,
          text: item.text,
          contentRef: item.contentRef,
          textTruncated: item.textTruncated === true,
          ...(kind.role === "assistant" && item.citations?.length
            ? { citations: item.citations }
            : {}),
          ...(runId ? { runId } : {}),
          ...(source?.type === "steering" ? { steering: true } : {}),
        });
      }
      break;
    case "reasoningState": {
      const text = (item.preview ?? item.text ?? "").trim();
      if (displayableReasoningText(text)) {
        state.entries.push({ kind: "reasoning", key: item.id, text });
      }
      break;
    }
    case "instructions":
    case "vfsCatalog":
    case "skillCatalog":
    case "subagentCatalog":
    case "skillActivation":
      if (displayableSystemText(item.preview ?? "")) {
        state.entries.push({
          kind: "system",
          key: item.id,
          text: item.preview ?? "",
          ...(item.supersededBy ? { superseded: true } : {}),
        });
      }
      break;
    case "catalog": {
      // A client-owned catalog renders its title; a superseded version stays
      // in context for prefix stability and is shown dimmed.
      const text = kind.title || item.preview || "";
      if (displayableSystemText(text)) {
        state.entries.push({
          kind: "system",
          key: item.id,
          text: item.supersedes ? `${text} (updated)` : text,
          ...(item.supersededBy ? { superseded: true } : {}),
        });
      }
      break;
    }
    case "providerOpaque":
      if (item.display?.toolName) {
        const groupIndex = createToolGroup(
          state,
          item.id,
          item.display.status,
        );
        appendToolCall(state, groupIndex, {
          callId: item.id,
          toolName: item.display.toolName,
          status: item.display.status,
          argumentsJson: item.display.arguments,
          output: item.display.output,
          error: item.display.error,
          isError: item.display.isError === true,
          display: item.display.summary,
        });
      }
      break;
    case "toolResult": {
      const isError = kind.isError;
      updateToolCall(state, kind.callId, (call) => ({
        ...call,
        status: item.display?.status ?? (isError ? "failed" : "succeeded"),
        output: item.display?.output ?? item.text,
        outputContentRef: item.contentRef,
        outputTruncated: item.textTruncated === true,
        error: item.display?.error ?? (isError ? item.display?.output ?? item.text : null),
        isError,
      }));
      syncToolGroupStatusForCall(state, kind.callId);
      break;
    }
    default:
      break;
  }
}

function createToolGroup(
  state: TranscriptState,
  key: string,
  status: TranscriptToolGroupStatus,
): number {
  const index = state.entries.length;
  state.entries.push({ kind: "tool-group", key, status, calls: [] });
  return index;
}

function appendToolCall(
  state: TranscriptState,
  entryIndex: number,
  call: TranscriptToolCall,
) {
  const entry = state.entries[entryIndex];
  if (!entry || entry.kind !== "tool-group") {
    return;
  }
  const callIndex = entry.calls.length;
  state.entries[entryIndex] = { ...entry, calls: [...entry.calls, call] };
  state.toolCallByCallId.set(call.callId, { entryIndex, callIndex });
}

function updateToolCall(
  state: TranscriptState,
  callId: string,
  update: (call: TranscriptToolCall) => TranscriptToolCall,
) {
  const location = state.toolCallByCallId.get(callId);
  if (!location) {
    return;
  }
  const entry = state.entries[location.entryIndex];
  const call = entry?.kind === "tool-group" ? entry.calls[location.callIndex] : undefined;
  if (!entry || entry.kind !== "tool-group" || !call) {
    return;
  }
  const calls = entry.calls.slice();
  calls[location.callIndex] = update(call);
  state.entries[location.entryIndex] = { ...entry, calls };
}

function applyToolBatchStarted(
  state: TranscriptState,
  kind: ToolBatchStartedEvent,
) {
  const batchId = kind.batchId;
  const calls = kind.calls;
  const existingGroup = calls
    .map((call) => state.toolCallByCallId.get(call.callId)?.entryIndex)
    .find((index) => index !== undefined);
  const entryIndex = existingGroup ?? createToolGroup(state, `tool-${batchId}`, "running");
  const entry = state.entries[entryIndex];
  if (entry?.kind === "tool-group") {
    state.entries[entryIndex] = { ...entry, batchId, status: "running" };
    state.toolGroupByBatchId.set(batchId, entryIndex);
  }

  for (const raw of calls) {
    const callId = raw.callId;
    const display = raw.display ?? undefined;
    if (state.toolCallByCallId.has(callId)) {
      updateToolCall(state, callId, (call) => ({
        ...call,
        toolId: raw.toolId,
        toolName: raw.toolName,
        status: isTerminalToolStatus(call.status) ? call.status : "requested",
        argumentsJson: raw.arguments ?? call.argumentsJson,
        display: display ?? call.display,
      }));
    } else {
      appendToolCall(state, entryIndex, {
        callId,
        toolId: raw.toolId,
        toolName: raw.toolName,
        status: "requested",
        argumentsJson: raw.arguments,
        display,
        isError: false,
      });
    }
  }
}

function updateToolGroupStatus(
  state: TranscriptState,
  batchId: string,
  status: TranscriptToolGroupStatus,
) {
  const entryIndex = state.toolGroupByBatchId.get(batchId);
  const entry = entryIndex !== undefined ? state.entries[entryIndex] : undefined;
  if (entry && entry.kind === "tool-group") {
    state.entries[entryIndex!] = { ...entry, status };
  }
}

function completeToolGroup(state: TranscriptState, batchId: string) {
  const entryIndex = state.toolGroupByBatchId.get(batchId);
  const entry = entryIndex !== undefined ? state.entries[entryIndex] : undefined;
  if (!entry || entry.kind !== "tool-group") {
    return;
  }
  state.entries[entryIndex!] = {
    ...entry,
    status: completedToolGroupStatus(entry.calls),
  };
}

function syncToolGroupStatusForCall(state: TranscriptState, callId: string) {
  const location = state.toolCallByCallId.get(callId);
  const entry = location ? state.entries[location.entryIndex] : undefined;
  if (!entry || entry.kind !== "tool-group") {
    return;
  }
  const terminal = entry.calls.length > 0 && entry.calls.every((call) =>
    isTerminalToolStatus(call.status),
  );
  if (terminal) {
    state.entries[location!.entryIndex] = {
      ...entry,
      status: completedToolGroupStatus(entry.calls),
    };
  }
}

function completedToolGroupStatus(
  calls: TranscriptToolCall[],
): TranscriptToolGroupStatus {
  if (calls.some(isFailedToolCall)) {
    return "completedWithErrors";
  }
  if (calls.some((call) => call.status === "cancelled")) {
    return "cancelled";
  }
  return "succeeded";
}

export function isFailedToolCall(call: TranscriptToolCall): boolean {
  if (call.status === "cancelled") {
    return false;
  }
  return call.isError || call.status === "failed" || call.status === "unavailable";
}

export function isTerminalToolStatus(
  status: ToolItemStatus | TranscriptToolGroupStatus,
): boolean {
  return [
    "succeeded",
    "completedWithErrors",
    "failed",
    "cancelled",
    "unavailable",
  ].includes(status);
}

/// The engine emits opaque state markers as system events; the TUI hides
/// them and so do we.
function displayableSystemText(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) {
    return false;
  }
  return !["context item", "reasoning state", "compaction state"].some((prefix) =>
    trimmed.startsWith(prefix),
  );
}

/// Reasoning state entries can either carry a human-readable provider
/// summary or an opaque continuation token. Only the summary is useful UI.
function displayableReasoningText(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) {
    return false;
  }
  const lower = trimmed.toLowerCase();
  return !(
    lower === "context item" ||
    lower === "reasoning state" ||
    lower.startsWith("reasoning state rs_") ||
    lower === "compaction state" ||
    lower.startsWith("compaction state ")
  );
}

/// Friendly fallback when an older event or provider item has no semantic
/// display metadata: pick the most recognizable argument.
export function toolTarget(argumentsJson: string | null | undefined): string | null {
  if (!argumentsJson) {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(argumentsJson);
  } catch {
    return null;
  }
  if (!parsed || typeof parsed !== "object") {
    return null;
  }
  const args = parsed as Record<string, unknown>;
  if (Array.isArray(args.argv)) {
    const command = args.argv.filter((part): part is string => typeof part === "string").join(" ");
    if (command) {
      return command.length > 80 ? `${command.slice(0, 77)}…` : command;
    }
  }
  for (const key of ["path", "file", "filePath", "file_path", "cwd", "command", "cmd", "url", "workspaceId", "query"]) {
    const value = args[key];
    if (typeof value === "string" && value.trim()) {
      return value.length > 80 ? `${value.slice(0, 77)}…` : value;
    }
  }
  return null;
}
