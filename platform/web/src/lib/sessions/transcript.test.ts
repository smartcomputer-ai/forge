import { describe, expect, it } from "vitest";
import type { SessionEvent, SessionItem } from "@/api";
import {
  applyEvents,
  emptyTranscript,
  isFailedToolCall,
  reconcileRuns,
  runInProgress,
} from "./transcript";
import type { SessionRunView } from "@/api";

type EventKind = SessionEvent["kind"];
type EventInput<T extends EventKind["type"]> =
  & { type: T }
  & Partial<Omit<Extract<EventKind, { type: T }>, "type">>;

const event = <T extends EventKind["type"]>(
  seq: number,
  kind: EventInput<T>,
): SessionEvent => ({
  cursor: { seq },
  observedAtMs: seq,
  joins: {},
  sessionId: "session-test",
  kind: {
    runId: "run-test",
    turnId: "turn-test",
    batchId: "batch-1",
    source: { type: "input", entries: [] },
    baseRevision: seq - 1,
    revision: seq,
    ...kind,
  } as unknown as Extract<EventKind, { type: T }>,
});

const item = (
  id: string,
  kind: SessionItem["kind"],
  extra: Omit<Partial<SessionItem>, "id" | "kind" | "contentRef"> = {},
): SessionItem => ({ id, kind, ...extra, contentRef: `sha256:${id}` });

describe("session transcript traces", () => {
  it("keeps readable reasoning summaries and hides opaque continuation state", () => {
    const state = applyEvents(emptyTranscript(), [
      event(1, {
        type: "contextEntriesApplied",
        entries: [
          item("reasoning-visible", { type: "reasoningState" }, {
            preview: "**Inspecting the code**\n\nI need to read the reducer first.",
          }),
          item("reasoning-opaque", { type: "reasoningState" }, {
            preview: "reasoning state rs_123",
          }),
        ],
      }),
    ]);

    expect(state.entries).toEqual([
      {
        kind: "reasoning",
        key: "reasoning-visible",
        text: "**Inspecting the code**\n\nI need to read the reducer first.",
      },
    ]);
  });

  it("merges context calls, batch metadata, results, and status into one group", () => {
    const state = applyEvents(emptyTranscript(), [
      event(1, {
        type: "contextEntriesApplied",
        entries: [
          item("tool-a", { type: "toolCall", callId: "call-a", name: "read_file" }),
          item("tool-b", { type: "toolCall", callId: "call-b", name: "exec_command" }),
        ],
      }),
      event(2, {
        type: "toolBatchStarted",
        batchId: "batch-1",
        calls: [
          {
            callId: "call-a",
            toolName: "read_file",
            argumentsRef: "sha256:args-a",
            arguments: "{\"path\":\"README.md\"}",
            display: { group: "explore", verb: "Read", target: "README.md" },
          },
          {
            callId: "call-b",
            toolName: "exec_command",
            argumentsRef: "sha256:args-b",
            arguments: "{\"argv\":[\"git\",\"status\"]}",
            display: { group: "execute", verb: "Run", target: "git status" },
          },
        ],
      }),
      event(3, { type: "toolCallCompleted", callId: "call-a", status: "succeeded" }),
      event(4, { type: "toolCallCompleted", callId: "call-b", status: "succeeded" }),
      event(5, {
        type: "contextEntriesApplied",
        entries: [
          item("result-a", { type: "toolResult", callId: "call-a", isError: false }, {
            text: "# Project",
          }),
          item("result-b", { type: "toolResult", callId: "call-b", isError: false }, {
            text: "clean",
          }),
        ],
      }),
      event(6, { type: "toolBatchCompleted", batchId: "batch-1" }),
    ]);

    expect(state.entries).toHaveLength(1);
    expect(state.entries[0]).toMatchObject({
      kind: "tool-group",
      batchId: "batch-1",
      status: "succeeded",
      calls: [
        {
          callId: "call-a",
          argumentsJson: "{\"path\":\"README.md\"}",
          output: "# Project",
          display: { group: "explore", verb: "Read", target: "README.md" },
        },
        {
          callId: "call-b",
          argumentsJson: "{\"argv\":[\"git\",\"status\"]}",
          output: "clean",
          display: { group: "execute", verb: "Run", target: "git status" },
        },
      ],
    });
  });

  it("retains failed output without treating the completed batch as a run failure", () => {
    const state = applyEvents(emptyTranscript(), [
      event(1, {
        type: "contextEntriesApplied",
        entries: [item("tool", { type: "toolCall", callId: "call", name: "web_fetch" })],
      }),
      event(2, {
        type: "contextEntriesApplied",
        entries: [
          item("result", { type: "toolResult", callId: "call", isError: true }, {
            text: "request failed",
          }),
        ],
      }),
    ]);

    expect(state.entries[0]).toMatchObject({
      kind: "tool-group",
      status: "completedWithErrors",
      calls: [{ status: "failed", output: "request failed", error: "request failed" }],
    });
  });

  it("keeps the batch and run active while sibling calls are still in progress", () => {
    const partial = applyEvents(emptyTranscript(), [
      event(1, { type: "runAccepted", runId: "5" }),
      event(2, { type: "runStarted", runId: "5" }),
      event(3, {
        type: "contextEntriesApplied",
        entries: [
          item("tool-a", { type: "toolCall", callId: "call-a", name: "read_file" }),
          item("tool-b", { type: "toolCall", callId: "call-b", name: "grep" }),
          item("tool-c", { type: "toolCall", callId: "call-c", name: "grep" }),
        ],
      }),
      event(4, {
        type: "toolBatchStarted",
        batchId: "batch-1",
        calls: [
          { callId: "call-a", toolName: "read_file", argumentsRef: "sha256:args-a" },
          { callId: "call-b", toolName: "grep", argumentsRef: "sha256:args-b" },
          { callId: "call-c", toolName: "grep", argumentsRef: "sha256:args-c" },
        ],
      }),
      event(5, { type: "toolCallStarted", callId: "call-a" }),
      event(6, { type: "toolCallStarted", callId: "call-b" }),
      event(7, { type: "toolCallStarted", callId: "call-c" }),
      event(8, { type: "toolCallCompleted", callId: "call-a", status: "succeeded" }),
      event(9, {
        type: "contextEntriesApplied",
        entries: [
          item("result-a", { type: "toolResult", callId: "call-a", isError: false }, {
            text: "contents",
          }),
        ],
      }),
      event(10, { type: "toolCallCompleted", callId: "call-b", status: "failed" }),
      event(11, {
        type: "contextEntriesApplied",
        entries: [
          item("result-b", { type: "toolResult", callId: "call-b", isError: true }, {
            text: "search timed out",
          }),
        ],
      }),
    ]);

    expect(partial.activeRun).toEqual({
      runId: "5",
      label: "running tools",
      cancelling: false,
    });
    expect(partial.entries[0]).toMatchObject({
      kind: "tool-group",
      status: "running",
      calls: [
        { callId: "call-a", status: "succeeded" },
        { callId: "call-b", status: "failed" },
        { callId: "call-c", status: "running" },
      ],
    });

    const complete = applyEvents(partial, [
      event(12, { type: "toolCallCompleted", callId: "call-c", status: "succeeded" }),
      event(13, {
        type: "contextEntriesApplied",
        entries: [
          item("result-c", { type: "toolResult", callId: "call-c", isError: false }, {
            text: "matches",
          }),
        ],
      }),
      event(14, { type: "toolBatchCompleted", batchId: "batch-1" }),
    ]);

    expect(complete.activeRun).toEqual({ runId: "5", label: "working", cancelling: false });
    expect(complete.entries[0]).toMatchObject({
      kind: "tool-group",
      status: "completedWithErrors",
    });
  });

  it("keeps a cancelled tool status neutral", () => {
    const state = applyEvents(emptyTranscript(), [
      event(1, {
        type: "contextEntriesApplied",
        entries: [item("tool", { type: "toolCall", callId: "call", name: "grep" })],
      }),
      event(2, {
        type: "toolBatchStarted",
        batchId: "batch-1",
        calls: [{ callId: "call", toolName: "grep", argumentsRef: "sha256:args" }],
      }),
      event(3, { type: "toolCallStarted", callId: "call" }),
      event(4, { type: "toolCallCompleted", callId: "call", status: "cancelled" }),
      event(5, {
        type: "contextEntriesApplied",
        entries: [
          item("result", { type: "toolResult", callId: "call", isError: true }, {
            text: "cancelled",
            display: {
              status: "cancelled",
              toolName: "grep",
              summary: { group: "explore", verb: "Search" },
            },
          }),
        ],
      }),
      event(6, { type: "toolBatchCompleted", batchId: "batch-1" }),
    ]);

    expect(state.entries[0]).toMatchObject({
      kind: "tool-group",
      status: "cancelled",
      calls: [{ status: "cancelled", isError: true }],
    });
    const group = state.entries[0];
    expect(group?.kind === "tool-group" && isFailedToolCall(group.calls[0]!)).toBe(false);
  });

  it("restores neutral cancellation from projected context alone", () => {
    const cancelledDisplay = {
      status: "cancelled" as const,
      toolName: "grep",
      summary: { group: "explore" as const, verb: "Search", target: "src" },
    };
    const state = applyEvents(emptyTranscript(), [
      event(1, {
        type: "contextStateReplaced",
        entries: [
          item("tool", { type: "toolCall", callId: "call", name: "grep" }, {
            display: {
              ...cancelledDisplay,
              arguments: "{\"pattern\":\"needle\",\"path\":\"src\"}",
            },
          }),
          item("result", { type: "toolResult", callId: "call", isError: true }, {
            text: "cancelled",
            display: cancelledDisplay,
          }),
        ],
      }),
    ]);

    expect(state.entries[0]).toMatchObject({
      kind: "tool-group",
      status: "cancelled",
      calls: [{ status: "cancelled", isError: true }],
    });
    const group = state.entries[0];
    expect(group?.kind === "tool-group" && isFailedToolCall(group.calls[0]!)).toBe(false);
  });

  it("completes a context-only tool group without batch lifecycle events", () => {
    const state = applyEvents(emptyTranscript(), [
      event(1, {
        type: "contextEntriesApplied",
        entries: [
          item("tool", { type: "toolCall", callId: "call", name: "read_file" }, {
            display: {
              toolName: "read_file",
              status: "succeeded",
              arguments: "{\"path\":\"README.md\"}",
              summary: { group: "explore", verb: "Read", target: "README.md" },
            },
          }),
        ],
      }),
    ]);

    expect(state.entries[0]).toMatchObject({
      kind: "tool-group",
      status: "succeeded",
      calls: [{ status: "succeeded" }],
    });
  });
});

describe("session transcript run control", () => {
  const runView = (id: string, status: SessionRunView["status"], text = ""): SessionRunView => ({
    id,
    status,
    source: { type: "input", items: text ? [{ type: "text", text }] : [] },
    entries: [],
  });

  it("queues runs accepted behind the active one and starts them in order", () => {
    let state = applyEvents(emptyTranscript(), [
      event(1, { type: "runAccepted", runId: "run_1" }),
      event(2, { type: "runStarted", runId: "run_1" }),
      event(3, { type: "runAccepted", runId: "run_2" }),
      event(4, { type: "runAccepted", runId: "run_3" }),
    ]);
    expect(state.activeRun).toEqual({ runId: "run_1", label: "running", cancelling: false });
    expect(state.queuedRuns).toEqual([{ runId: "run_2" }, { runId: "run_3" }]);
    expect(runInProgress(state)).toBe(true);

    state = applyEvents(state, [
      event(5, { type: "runCancelled", runId: "run_2" }),
      event(6, { type: "runCompleted", runId: "run_1" }),
      event(7, { type: "runStarted", runId: "run_3" }),
    ]);
    expect(state.activeRun).toEqual({ runId: "run_3", label: "running", cancelling: false });
    expect(state.queuedRuns).toEqual([]);
    expect(state.entries).toEqual([
      { kind: "marker", key: "evt-5", text: "queued message cancelled", tone: "muted" },
    ]);

    state = applyEvents(state, [event(8, { type: "runCompleted", runId: "run_3" })]);
    expect(state.activeRun).toBeNull();
    expect(runInProgress(state)).toBe(false);
  });

  it("maps client submission ids to run ids on acceptance", () => {
    const state = applyEvents(emptyTranscript(), [
      event(1, { type: "runAccepted", runId: "run_1", submissionId: "sub-a" }),
      event(2, { type: "runStarted", runId: "run_1" }),
      event(3, { type: "runAccepted", runId: "run_2", submissionId: "sub-b" }),
    ]);
    expect(state.runBySubmission.get("sub-a")).toBe("run_1");
    expect(state.runBySubmission.get("sub-b")).toBe("run_2");
    expect(state.runPhases.get("run_2")).toBe("queued");
  });

  it("marks the active run cancelling until the terminal event lands", () => {
    let state = applyEvents(emptyTranscript(), [
      event(1, { type: "runAccepted", runId: "run_1" }),
      event(2, { type: "runStarted", runId: "run_1" }),
      event(3, { type: "turnStarted", runId: "run_1" }),
      event(4, { type: "runCancellationRequested", runId: "run_1" }),
      // Lifecycle labels no longer override "cancelling".
      event(5, { type: "turnCancelled", runId: "run_1" }),
    ]);
    expect(state.activeRun).toEqual({ runId: "run_1", label: "cancelling", cancelling: true });

    state = applyEvents(state, [event(6, { type: "runCancelled", runId: "run_1" })]);
    expect(state.activeRun).toBeNull();
    expect(state.entries.at(-1)).toEqual({
      kind: "marker",
      key: "evt-6",
      text: "run cancelled",
      tone: "muted",
    });
  });

  it("folds steering messages as tagged user entries on their run", () => {
    const state = applyEvents(emptyTranscript(), [
      event(1, {
        type: "contextEntriesApplied",
        entries: [
          item("input", { type: "message", role: "user" }, {
            text: "do the task",
            source: { type: "runInput", runId: "run_1", inputIndex: 0 },
          }),
          item("steer", { type: "message", role: "user" }, {
            text: "also mention the moon",
            source: { type: "steering", runId: "run_1", steeringId: "steering_1", inputIndex: 0 },
          }),
        ],
      }),
    ]);
    expect(state.entries).toEqual([
      { kind: "message", key: "input", role: "user", text: "do the task", runId: "run_1" },
      {
        kind: "message",
        key: "steer",
        role: "user",
        text: "also mention the moon",
        runId: "run_1",
        steering: true,
      },
    ]);
  });

  it("reconciles the authoritative session view forward only", () => {
    // The tail missed the start (truncated catch-up): the snapshot seeds it.
    let state = reconcileRuns(emptyTranscript(), [
      runView("run_1", "completed"),
      runView("run_2", "running"),
      runView("run_3", "queued", "later"),
    ]);
    expect(state.activeRun).toEqual({ runId: "run_2", label: "running", cancelling: false });
    expect(state.queuedRuns).toEqual([{ runId: "run_3" }]);

    // A stale snapshot never regresses what the tail already knows.
    state = applyEvents(state, [event(9, { type: "runCompleted", runId: "run_2" })]);
    const stale = reconcileRuns(state, [runView("run_2", "running")]);
    expect(stale).toBe(state);
    expect(stale.activeRun).toBeNull();

    // A terminal status in the snapshot heals a missed terminal event.
    const healed = reconcileRuns(state, [runView("run_3", "cancelled")]);
    expect(healed.queuedRuns).toEqual([]);
    expect(runInProgress(healed)).toBe(false);

    // Identity is preserved when nothing changes.
    expect(reconcileRuns(healed, [runView("run_3", "cancelled")])).toBe(healed);
  });
});

describe("run usage", () => {
  it("summarizes prompt tokens and the cached share when the run finishes", () => {
    let state = applyEvents(emptyTranscript(), [
      event(1, { type: "runAccepted", runId: "run_1" }),
      event(2, { type: "runStarted", runId: "run_1" }),
      event(3, {
        type: "turnGenerationCompleted",
        runId: "run_1",
        turnId: "turn_1",
        status: "succeeded",
        usage: { inputTokens: 9000, cachedInputTokens: 8000 },
      }),
      event(4, {
        type: "turnGenerationCompleted",
        runId: "run_1",
        turnId: "turn_2",
        status: "succeeded",
        usage: { inputTokens: 11000, cachedInputTokens: 10000 },
      }),
    ]);
    expect(state.entries.some((entry) => entry.kind === "marker")).toBe(false);

    state = applyEvents(state, [event(5, { type: "runCompleted", runId: "run_1" })]);
    const marker = state.entries.find((entry) => entry.kind === "marker");
    expect(marker?.kind === "marker" && marker.text).toBe("20k tokens in · 90% cached");
    expect(marker?.kind === "marker" && marker.tone).toBe("muted");
  });

  it("adds no marker when the provider reported no usage", () => {
    const state = applyEvents(emptyTranscript(), [
      event(1, { type: "runAccepted", runId: "run_1" }),
      event(2, { type: "runStarted", runId: "run_1" }),
      event(3, { type: "runCompleted", runId: "run_1" }),
    ]);
    expect(state.entries.some((entry) => entry.kind === "marker")).toBe(false);
  });
});
