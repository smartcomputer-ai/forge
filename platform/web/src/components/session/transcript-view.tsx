import { Loader2, TriangleAlert, X } from "lucide-react";
import { Bubble, BubbleContent } from "@/components/ui/bubble";
import { Button } from "@/components/ui/button";
import { Marker, MarkerContent, MarkerIcon } from "@/components/ui/marker";
import { Message, MessageContent } from "@/components/ui/message";
import { MarkdownContent } from "@/components/session/markdown-content";
import { ReasoningTrace, ToolGroupTrace } from "@/components/session/tool-trace";
import {
  type ActiveRun,
  type TranscriptEntry,
} from "@/lib/sessions/transcript";
import { cn } from "@/lib/utils";

/// Coding-bot transcript idiom: no avatars, full-width rows. User input
/// is a tinted band, assistant output plain rendered text, tool activity
/// and lifecycle notes are compact marker rows.

export function TranscriptEntryView({ entry }: { entry: TranscriptEntry }) {
  switch (entry.kind) {
    case "message":
      return entry.role === "user" ? (
        <UserBand text={entry.text} steering={entry.steering === true} />
      ) : (
        <Message>
          <MessageContent>
            <Bubble variant="ghost" className="max-w-full">
              <BubbleContent>
                <MarkdownContent>{entry.text}</MarkdownContent>
              </BubbleContent>
            </Bubble>
          </MessageContent>
        </Message>
      );
    case "system":
      return (
        <Marker variant="separator">
          <MarkerContent>{entry.text}</MarkerContent>
        </Marker>
      );
    case "reasoning":
      return <ReasoningTrace text={entry.text} />;
    case "tool-group":
      return <ToolGroupTrace group={entry} />;
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

export function UserBand({
  text,
  pending = false,
  steering = false,
}: {
  text: string;
  pending?: boolean;
  /// A message injected into a running run rather than the input that
  /// started it; rendered with a small tag so the reader can tell them
  /// apart.
  steering?: boolean;
}) {
  return (
    <Message>
      <MessageContent>
        <Bubble variant="muted" className={cn("w-full max-w-full", pending && "opacity-60")}>
          <BubbleContent className="w-full whitespace-pre-wrap">
            {steering && (
              <span
                className="mr-2 rounded-sm border px-1 py-px align-middle text-[10px] uppercase tracking-wide text-muted-foreground"
                title="Sent into the running run; the agent saw it at its next turn"
              >
                steer
              </span>
            )}
            {text}
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
    <div className="shrink-0 border-t bg-muted/40 px-4 py-2 md:px-8" aria-label="Queued messages">
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
