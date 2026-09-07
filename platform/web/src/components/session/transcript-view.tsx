import { useId, useLayoutEffect, useRef, useState } from "react";
import { ChevronDown, ChevronUp, Check, Loader2, ShieldQuestion, TriangleAlert, X } from "lucide-react";
import type { PendingApprovalView } from "@lightspeed-ai/agent-client";
import { Bubble, BubbleContent } from "@/components/ui/bubble";
import { Button } from "@/components/ui/button";
import { Marker, MarkerContent, MarkerIcon } from "@/components/ui/marker";
import { Message, MessageContent } from "@/components/ui/message";
import { MarkdownContent } from "@/components/session/markdown-content";
import type { FullTextLoader } from "@/components/session/expandable-content";
import { ReasoningTrace, ToolGroupTrace } from "@/components/session/tool-trace";
import {
  type ActiveRun,
  type TranscriptEntry,
} from "@/lib/sessions/transcript";
import { cn } from "@/lib/utils";

/// Full-width transcript rows without avatars. User inputs use muted bands;
/// assistant output is plain rendered text, with compact tool and lifecycle markers.

export function TranscriptEntryView({
  entry,
  loadFullText,
}: {
  entry: TranscriptEntry;
  loadFullText?: FullTextLoader;
}) {
  switch (entry.kind) {
    case "message":
      return entry.role === "user" ? (
        <UserBand
          text={entry.text}
          steering={entry.steering === true}
        />
      ) : (
        <Message>
          <MessageContent>
            <Bubble variant="ghost" className="max-w-full">
              <BubbleContent>
                <MarkdownContent>{entry.text}</MarkdownContent>
                {entry.citations?.length ? (
                  <div className="mt-3 flex flex-wrap gap-2 border-t pt-3 pb-1 text-xs text-muted-foreground">
                    <span className="font-medium">Sources</span>
                    {entry.citations.map((citation, index) => (
                      <a
                        key={`${citation.url}:${index}`}
                        href={citation.url}
                        target="_blank"
                        rel="noreferrer"
                        title={citation.citedText ?? undefined}
                        className="text-primary underline underline-offset-4"
                      >
                        {citation.title || citationHost(citation.url)}
                      </a>
                    ))}
                  </div>
                ) : null}
              </BubbleContent>
            </Bubble>
          </MessageContent>
        </Message>
      );
    case "system":
      return (
        <Marker
          variant="separator"
          className={entry.superseded ? "opacity-50" : undefined}
          title={entry.superseded ? "Superseded by a newer version" : undefined}
        >
          <MarkerContent>{entry.text}</MarkerContent>
        </Marker>
      );
    case "reasoning":
      return <ReasoningTrace text={entry.text} />;
    case "tool-group":
      return <ToolGroupTrace group={entry} loadFullText={loadFullText} />;
    case "marker":
      return entry.tone === "error" ? (
        <Marker className="text-destructive">
          <MarkerIcon>
            <TriangleAlert />
          </MarkerIcon>
          <MarkerContent>{entry.text}</MarkerContent>
        </Marker>
      ) : (
        <Marker variant="separator">
          <MarkerContent>{entry.text}</MarkerContent>
        </Marker>
      );
  }
}

function citationHost(url: string): string {
  try {
    return new URL(url).hostname || url;
  } catch {
    return url;
  }
}

// 160px of text plus padding and the expansion control keeps a collapsed band
// around 200px tall. Measure natural content so wrapping and font changes count.
const COLLAPSED_TEXT_HEIGHT = 160;

export function UserBand({
  text,
  pending = false,
  steering = false,
}: {
  text: string;
  pending?: boolean;
  /// A message injected into a running run rather than its initial input.
  steering?: boolean;
}) {
  const contentId = useId();
  const contentRef = useRef<HTMLDivElement>(null);
  const [overflowing, setOverflowing] = useState(false);
  const [expanded, setExpanded] = useState(false);

  useLayoutEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    const measure = () => setOverflowing(content.scrollHeight > COLLAPSED_TEXT_HEIGHT);
    measure();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", measure);
      return () => window.removeEventListener("resize", measure);
    }
    const observer = new ResizeObserver(measure);
    observer.observe(content);
    return () => observer.disconnect();
  }, [text, steering]);

  return (
    <Message>
      <MessageContent>
        <Bubble
          variant="muted"
          className={cn(
            "w-full max-w-full",
            pending && "opacity-60",
          )}
        >
          <BubbleContent className="w-full">
            <div
              id={contentId}
              className="overflow-hidden"
              style={{
                maxHeight: expanded ? undefined : COLLAPSED_TEXT_HEIGHT,
                maskImage: overflowing && !expanded
                  ? "linear-gradient(to bottom, black calc(100% - 48px), transparent)"
                  : undefined,
              }}
            >
              <div ref={contentRef} className="whitespace-pre-wrap [overflow-wrap:anywhere]">
                {steering && (
                  <span
                    className="mr-2 rounded-sm border border-current/25 px-1 py-px align-middle text-[10px] uppercase tracking-wide opacity-70"
                    title="Sent into the running run; the agent saw it at its next turn"
                  >
                    steer
                  </span>
                )}
                {text}
              </div>
            </div>
            {overflowing && (
              <div className="mt-1 flex justify-center">
                <button
                  type="button"
                  data-message-expansion
                  aria-expanded={expanded}
                  aria-controls={contentId}
                  onClick={() => setExpanded((value) => !value)}
                  className={cn(
                    "inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium opacity-80 transition-colors hover:opacity-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-current",
                    "hover:bg-foreground/5",
                  )}
                >
                  {expanded ? "Show less" : "Show more"}
                  {expanded ? <ChevronUp className="size-3.5" /> : <ChevronDown className="size-3.5" />}
                </button>
              </div>
            )}
          </BubbleContent>
        </Bubble>
      </MessageContent>
    </Message>
  );
}

export interface QueuedRunItem {
  /// Stable identity across the optimistic → confirmed transition (the
  /// client submission id when known), so the row is updated, not remounted.
  key: string;
  runId: string | null;
  text: string;
  /// Still being submitted or awaiting the engine's acknowledgement.
  pending?: boolean;
  /// A cancel is in flight for this queued run.
  cancelling?: boolean;
}

/// Messages queued behind the active run, in start order, each with a
/// cancel control. Sits between the transcript and the composer.
export function QueuedRunsBar({
  items,
  onCancel,
}: {
  items: QueuedRunItem[];
  onCancel: (runId: string) => void;
}) {
  if (items.length === 0) {
    return null;
  }
  return (
    <div className="shrink-0 border-t bg-muted/40" aria-label="Queued messages">
      <div className="mx-auto w-full max-w-5xl px-4 py-2 md:px-8">
        <p className="pb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
          Queued — starts after the current run
        </p>
        <ul className="flex flex-col gap-1">
        {items.map((item, index) => (
          <li
            key={item.key}
            className={cn(
              "flex items-center gap-2 text-sm",
              (item.pending || item.cancelling) && "opacity-60",
            )}
          >
            <span className="w-5 shrink-0 text-right text-xs text-muted-foreground">
              {index + 1}.
            </span>
            <span className="min-w-0 flex-1 truncate whitespace-pre-wrap" title={item.text}>
              {item.text}
            </span>
            {item.cancelling ? (
              <Loader2 className="size-3.5 shrink-0 animate-spin text-muted-foreground" />
            ) : (
              <Button
                variant="ghost"
                size="icon-xs"
                disabled={!item.runId}
                aria-label="Cancel queued message"
                title={item.runId ? "Remove from the queue" : "Waiting for the engine to accept this message"}
                onClick={() => item.runId && onCancel(item.runId)}
              >
                <X />
              </Button>
            )}
          </li>
        ))}
        </ul>
      </div>
    </div>
  );
}

/// Live spinner row while a run is in flight, with the TUI's status
/// vocabulary (queued / running / thinking / running tools / …).
export function ActiveRunMarker({ run }: { run: ActiveRun }) {
  return (
    <Marker role="status">
      <MarkerIcon>
        <Loader2 className="animate-spin" />
      </MarkerIcon>
      <MarkerContent>{run.label}…</MarkerContent>
    </Marker>
  );
}

export function ApprovalCards({
  approvals,
  deciding,
  error,
  onDecide,
}: {
  approvals: PendingApprovalView[];
  deciding: { approvalId: string; decision: "approve" | "reject" } | null;
  error: { approvalId: string; message: string } | null;
  onDecide: (approvalId: string, decision: "approve" | "reject") => void;
}) {
  if (approvals.length === 0) return null;
  return (
    <div className="flex flex-col gap-3" aria-label="Pending approvals">
      {approvals.map((approval) => {
        const subject = approval.subject;
        const busy = deciding?.approvalId === approval.approvalId;
        return (
          <section
            key={approval.approvalId}
            className="rounded-lg border border-amber-500/35 bg-amber-500/5 p-4"
          >
            <div className="flex items-start gap-3">
              <ShieldQuestion className="mt-0.5 size-4 shrink-0 text-amber-600" />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium">Approve MCP tool call?</p>
                <p className="mt-1 text-xs text-muted-foreground">
                  <span className="font-medium text-foreground">{subject.toolName}</span>
                  {` on ${subject.serverLabel}`}
                </p>
                <pre className="mt-3 max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-md bg-muted/70 p-3 font-mono text-xs">
                  {subject.argumentsPreview}
                </pre>
                <div className="mt-3 flex items-center gap-2">
                  <Button
                    size="sm"
                    disabled={deciding !== null}
                    onClick={() => onDecide(approval.approvalId, "approve")}
                  >
                    {busy && deciding?.decision === "approve" ? (
                      <Loader2 className="animate-spin" />
                    ) : (
                      <Check />
                    )}
                    Approve
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={deciding !== null}
                    onClick={() => onDecide(approval.approvalId, "reject")}
                  >
                    {busy && deciding?.decision === "reject" ? (
                      <Loader2 className="animate-spin" />
                    ) : (
                      <X />
                    )}
                    Reject
                  </Button>
                  <span className="ml-auto font-mono text-[10px] text-muted-foreground">
                    {approval.approvalId}
                  </span>
                </div>
                {error?.approvalId === approval.approvalId && (
                  <p className="mt-2 text-xs text-destructive">{error.message}</p>
                )}
              </div>
            </div>
          </section>
        );
      })}
    </div>
  );
}
