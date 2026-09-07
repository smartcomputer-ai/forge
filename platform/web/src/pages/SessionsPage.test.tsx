// @vitest-environment jsdom
import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BotView } from "@/api";
import { BotChat } from "@/components/bot/chat";
import { sessionDraftKey } from "@/lib/sessions/draft";
import { emptyTranscript } from "@/lib/sessions/transcript";
import { SessionDetail } from "./SessionsPage";

const mocks = vi.hoisted(() => ({
  api: vi.fn(),
  tail: vi.fn(),
  scrollable: { start: true, end: true },
  scrollToEnd: vi.fn(),
}));

vi.mock("@/api", async (original) => ({
  ...await original<typeof import("@/api")>(),
  api: mocks.api,
}));
vi.mock("@/lib/sessions/tail", () => ({ useSessionTail: mocks.tail }));
// Keep queries and unrelated settings out of these composer/transcript tests.
vi.mock("@tanstack/react-query", () => ({
  useQueryClient: () => ({}),
  useQuery: () => ({ data: undefined, refetch: vi.fn() }),
  useInfiniteQuery: () => ({ data: undefined }),
  useMutation: () => ({ isPending: false }),
}));
vi.mock("@/components/session/session-settings-sheet", () => ({ SessionSettingsDialog: () => null }));
vi.mock("@/components/ui/message-scroller", () => {
  const Container = ({ children }: { children?: ReactNode }) => <div>{children}</div>;
  return {
    MessageScrollerProvider: ({ children, autoScroll }: { children?: ReactNode; autoScroll?: boolean }) => (
      <div data-auto-scroll={autoScroll}>{children}</div>
    ),
    MessageScroller: Container,
    MessageScrollerContent: Container,
    MessageScrollerViewport: Container,
    MessageScrollerItem: Container,
    MessageScrollerButton: () => null,
    useMessageScroller: () => ({ scrollToEnd: mocks.scrollToEnd }),
    useMessageScrollerScrollable: () => mocks.scrollable,
  };
});

const bot: BotView = {
  botId: "bot", profileId: "profile", revision: 1, eventSeq: 0,
  createdAtMs: 0, updatedAtMs: 0,
};
type Pathway = "session" | "bot-main" | "bot-other";
let container: HTMLDivElement;
let root: Root;
let transcript = emptyTranscript();

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  window.localStorage.clear();
  transcript = emptyTranscript();
  mocks.tail.mockReturnValue({
    transcript, phase: "live", error: null, reconcileRuns: vi.fn(),
    hasOlder: false, loadingOlder: false, historyError: null, historyRevision: 0, loadOlder: vi.fn(),
  });
  mocks.api.mockReset();
  mocks.scrollable.end = true;
  mocks.scrollToEnd.mockReset().mockImplementation(() => {
    // The primitive's scrollToEnd restores following-bottom when autoScroll is on.
    mocks.scrollable.end = false;
    return true;
  });
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function show(pathway: Pathway, universeId = "universe", sessionId = "session") {
  await act(async () => root.render(
    <MemoryRouter>
      {pathway === "session" ? (
        <SessionDetail universeId={universeId} slug={universeId} sessionId={sessionId} />
      ) : (
        <BotChat
          universeId={universeId}
          slug={universeId}
          bot={bot}
          state={{ controller: {
            mainSessionId: pathway === "bot-main" ? sessionId : "main",
            setupStatus: "ready", controllerStatus: "idle", enabled: true, closed: false,
          } }}
          sessionId={pathway === "bot-main" ? undefined : sessionId}
        />
      )}
    </MemoryRouter>,
  ));
}

function input() {
  return container.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message"]')!;
}

async function type(text: string) {
  await act(async () => {
    // Bypass React's value tracker to simulate a browser edit.
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input(), text);
    input().dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function enter(options: KeyboardEventInit = {}) {
  await act(async () => {
    input().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, ...options }));
  });
}

function pauseFollowing() {
  mocks.scrollable.end = true;
  mocks.scrollToEnd.mockClear();
}

async function navigateAway() {
  await act(async () => root.render(null));
}

describe.each<Pathway>(["session", "bot-main", "bot-other"])("%s composer", (pathway) => {
  it("persists exact drafts across navigation and isolates sessions and universes", async () => {
    await show(pathway);
    await type("  unfinished\nmessage 🦀  ");
    expect(window.localStorage.getItem(sessionDraftKey("universe", "session"))).toBe("  unfinished\nmessage 🦀  ");

    await show(pathway, "universe", "other");
    expect(input().value).toBe("");
    await type("other session draft");
    await show(pathway, "another-universe", "session");
    expect(input().value).toBe("");
    await type("other universe draft");

    await navigateAway();
    await show(pathway);
    expect(input().value).toBe("  unfinished\nmessage 🦀  ");
    await show(pathway, "universe", "other");
    expect(input().value).toBe("other session draft");
    await show(pathway, "another-universe", "session");
    expect(input().value).toBe("other universe draft");
  });

  it.each(["message", "queue", "steer"] as const)("clears a submitted %s and resumes following only after acceptance", async (mode) => {
    if (mode !== "message") {
      transcript.activeRun = { runId: "active-run", label: "running", cancelling: false };
    }
    let accept!: (value: unknown) => void;
    mocks.api.mockReturnValue(new Promise((resolve) => { accept = resolve; }));
    await show(pathway);
    expect(container.querySelector('[data-auto-scroll="true"]')).not.toBeNull();
    pauseFollowing();
    await type("  send this  ");
    await enter(mode === "steer" ? { ctrlKey: true } : {});
    expect(mocks.api).toHaveBeenCalledWith(
      "POST",
      `/api/v1/universes/universe/sessions/session/${mode === "steer" ? "runs/active-run/steer" : "messages"}`,
      mode === "steer" ? { text: "send this" } : { text: "send this", submissionId: expect.any(String) },
    );
    expect(mocks.scrollToEnd).not.toHaveBeenCalled();
    await act(async () => accept({ run: { id: "accepted-run", status: mode === "queue" ? "queued" : "running" } }));
    expect(mocks.scrollToEnd).toHaveBeenCalledWith({ behavior: "auto" });
    expect(input().value).toBe("");
    expect(window.localStorage.getItem(sessionDraftKey("universe", "session"))).toBeNull();

    // Streaming content continues to follow after a successful submission.
    mocks.scrollToEnd.mockClear();
    transcript.entries = [{ kind: "message", key: "reply", role: "assistant", text: "Newest reply" }];
    await show(pathway);
    expect(mocks.scrollToEnd).toHaveBeenCalledWith({ behavior: "auto" });

    // Manual reading remains respected until the next successful submission.
    pauseFollowing();
    transcript.entries = [...transcript.entries, { kind: "message", key: "later", role: "assistant", text: "Later reply" }];
    await show(pathway);
    expect(mocks.scrollToEnd).not.toHaveBeenCalled();
    await navigateAway();
    await show(pathway);
    expect(input().value).toBe("");
  });

  it("does not resume following when submission fails", async () => {
    mocks.api.mockRejectedValue(new Error("Message rejected"));
    await show(pathway);
    pauseFollowing();
    await type("message");
    await enter();
    expect(mocks.scrollToEnd).not.toHaveBeenCalled();
    expect(container.textContent).toContain("Message rejected");
  });

  it("lets the scroller preserve a history prepend without requesting the end", async () => {
    await show(pathway);
    mocks.scrollToEnd.mockClear();
    mocks.tail.mock.results.at(-1)!.value.historyRevision = 1;
    transcript.entries = [{ kind: "message", key: "older", role: "user", text: "Older input" }];
    await show(pathway);
    expect(mocks.scrollToEnd).not.toHaveBeenCalled();
  });

  it("retains an unavailable steering draft and does not resume following", async () => {
    transcript.queuedRuns = [{ runId: "queued-run" }];
    await show(pathway);
    pauseFollowing();
    await type("steer later");
    await enter({ ctrlKey: true });
    expect(mocks.api).not.toHaveBeenCalled();
    expect(mocks.scrollToEnd).not.toHaveBeenCalled();
    await navigateAway();
    await show(pathway);
    expect(input().value).toBe("steer later");
  });
});

it("shares the draft between a normal session and bot views of that same session", async () => {
  await show("session");
  await type("shared draft");
  await navigateAway();
  await show("bot-main");
  expect(input().value).toBe("shared draft");
  await navigateAway();
  await show("bot-other");
  expect(input().value).toBe("shared draft");
});

it("keeps editing and submitting usable when local storage access is blocked", async () => {
  vi.spyOn(window, "localStorage", "get").mockImplementation(() => { throw new Error("Storage blocked"); });
  mocks.api.mockResolvedValue({ run: { id: "run", status: "running" } });
  await show("session");
  await type("in-memory draft");
  expect(input().value).toBe("in-memory draft");
  await enter();
  expect(input().value).toBe("");
  expect(mocks.api).toHaveBeenCalledOnce();
});
