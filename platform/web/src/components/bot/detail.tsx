import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowUpRight, RotateCcw, Settings2, Webhook } from "lucide-react";
import { Link } from "react-router-dom";
import { api, type Bot, type BotActivityEntry, type BotState } from "@/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { BotSettingsDialog } from "./settings-dialog";
import { SendEventDialog } from "./send-event-dialog";
import { BotStatusBadge, KeyValue, PanelHeading } from "./status";
import { TriggersSection } from "./triggers";

/**
 * Bot-centric detail: the bot is a router over sessions, so this view shows
 * its decisions and lists the sessions it manages (one today, per-key soon)
 * rather than embedding a single transcript.
 */
export function BotDetail({
  slug,
  bot,
  state,
  stateError,
  manage,
}: {
  slug: string;
  bot: Bot;
  state?: BotState;
  stateError?: string;
  manage: boolean;
}) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [eventOpen, setEventOpen] = useState(false);
  const queryClient = useQueryClient();
  const activity = useQuery({
    queryKey: ["bot-activity", bot.id],
    queryFn: () => api<{ activity: BotActivityEntry[] }>("GET", `/api/v1/bots/${bot.id}/activity`),
    refetchInterval: 10_000,
  });
  const replay = useMutation({
    mutationFn: (eventId: string) =>
      api("POST", `/api/v1/bots/${bot.id}/events/replay`, { eventId }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["bot-state", bot.id] });
      await queryClient.invalidateQueries({ queryKey: ["bot-activity", bot.id] });
    },
  });

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex h-12 shrink-0 items-center gap-2 border-b px-4">
        <span className="min-w-0 truncate text-sm font-semibold">{bot.name}</span>
        <BotStatusBadge status={state?.controllerStatus} />
        {manage && (
          <div className="ml-auto flex items-center gap-1">
            <Button variant="outline" size="xs" onClick={() => setEventOpen(true)}>
              <Webhook data-icon="inline-start" /> Send event
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => setSettingsOpen(true)}
              aria-label="Bot settings"
            >
              <Settings2 />
            </Button>
          </div>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto grid w-full max-w-2xl gap-8 p-6 text-sm">
          <section className="grid gap-2">
            <PanelHeading title="Bot" />
            <KeyValue label="Profile" value={bot.profileId} />
            <KeyValue
              label="Budget"
              value={
                bot.runsPerDay === null
                  ? "Unlimited"
                  : `${state?.runsToday ?? 0} / ${bot.runsPerDay} runs today`
              }
            />
            <KeyValue label="Processed" value={String(state?.eventsProcessed ?? 0)} />
            {bot.brief && <p className="mt-1 text-xs text-muted-foreground">{bot.brief}</p>}
            {!bot.enabled && (
              <p className="rounded-md bg-muted p-2 text-xs text-muted-foreground">
                Disabled: schedules are paused and pending events wait.
              </p>
            )}
            {stateError && (
              <p className="rounded-md bg-destructive/10 p-2 text-xs text-destructive">
                Controller unavailable: {stateError}
              </p>
            )}
            {state?.lastError && (
              <p className="rounded-md bg-destructive/10 p-2 text-xs text-destructive">{state.lastError}</p>
            )}
          </section>

          <section className="grid gap-2">
            <PanelHeading title="Sessions" />
            {state ? (
              state.sessions.map((session) => {
                const isMain = session.kind === "main";
                const ready = !isMain || state.sessionReady;
                return (
                  <div key={session.sessionId} className="flex items-center gap-2 rounded-md border p-2 text-xs">
                    <span className="min-w-0 flex-1">
                      <code className="block truncate">{session.sessionId}</code>
                      <span className="text-muted-foreground">
                        {isMain
                          ? "Main session"
                          : session.kind === "keyed"
                            ? `Key: ${session.label}`
                            : session.label}
                      </span>
                    </span>
                    <Badge variant={ready ? "secondary" : "outline"}>
                      {ready ? "ready" : "starting"}
                    </Badge>
                    {ready && (
                      <Button
                        variant="outline"
                        size="xs"
                        render={<Link to={`/u/${slug}/sessions/${session.sessionId}`} />}
                      >
                        Open <ArrowUpRight data-icon="inline-end" />
                      </Button>
                    )}
                  </div>
                );
              })
            ) : (
              <p className="text-xs text-muted-foreground">Waiting for the controller…</p>
            )}
          </section>

          <TriggersSection botId={bot.id} manage={manage} />

          <section className="grid gap-2">
            <PanelHeading title="Event inbox" />
            <KeyValue label="Pending" value={String(state?.pendingEventCount ?? 0)} />
            {state?.buffers.map((buffer) => (
              <p key={buffer.key} className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
                Coalescing {buffer.count} event(s) · flushes {flushLabel(buffer.flushAtMs)}
              </p>
            ))}
            {state?.activeDeliveries.map((active) => (
              <EventRow
                key={active.id}
                id={active.id}
                status="active"
                eventCount={active.eventCount}
                summary={active.sessionId === state.sessionId ? undefined : `→ ${active.sessionId}`}
              />
            ))}
            {state?.recentEvents
              .slice()
              .reverse()
              .slice(0, 8)
              .map((event) => (
                <EventRow
                  key={event.id}
                  id={event.id}
                  status={event.status}
                  eventCount={event.eventCount}
                  summary={event.summary ?? event.failure}
                  onReplay={
                    manage && !event.id.startsWith("batch-")
                      ? () => replay.mutate(event.id)
                      : undefined
                  }
                />
              ))}
            {state && state.activeDeliveries.length === 0 && state.recentEvents.length === 0 && (
              <p className="text-xs text-muted-foreground">No events delivered yet.</p>
            )}
          </section>

          <section className="grid gap-2">
            <PanelHeading title="Activity" />
            {activity.data?.activity.slice(0, 20).map((entry) => (
              <div key={entry.id} className="rounded-md border p-2 text-xs">
                <div className="flex items-center gap-2">
                  <Badge variant={activityVariant(entry.kind)}>{entry.kind.replaceAll("_", " ")}</Badge>
                  <span className="ml-auto shrink-0 text-muted-foreground">{timeLabel(entry.createdAt)}</span>
                </div>
                {(entry.detail ?? entry.eventId) && (
                  <p className="mt-1 line-clamp-2 text-muted-foreground wrap-anywhere">
                    {entry.detail ?? entry.eventId}
                  </p>
                )}
              </div>
            ))}
            {activity.data?.activity.length === 0 && (
              <p className="text-xs text-muted-foreground">No activity recorded yet.</p>
            )}
          </section>
        </div>
      </div>
      {manage && (
        <>
          <BotSettingsDialog universeId={bot.universeId} bot={bot} open={settingsOpen} onOpenChange={setSettingsOpen} />
          <SendEventDialog botId={bot.id} open={eventOpen} onOpenChange={setEventOpen} />
        </>
      )}
    </div>
  );
}

function EventRow({
  id,
  status,
  eventCount,
  summary,
  onReplay,
}: {
  id: string;
  status: string;
  eventCount?: number;
  summary?: string;
  onReplay?: () => void;
}) {
  return (
    <div className="rounded-md border p-2 text-xs">
      <div className="flex items-center gap-2">
        <code className="min-w-0 flex-1 truncate">{id}</code>
        {eventCount !== undefined && eventCount > 1 && (
          <Badge variant="outline">{eventCount} events</Badge>
        )}
        <Badge variant={status === "run_failed" || status === "blocked" ? "destructive" : "outline"}>
          {status.replaceAll("_", " ")}
        </Badge>
        {onReplay && (
          <Button variant="ghost" size="icon-xs" onClick={onReplay} aria-label="Replay event">
            <RotateCcw />
          </Button>
        )}
      </div>
      {summary && <p className="mt-1 line-clamp-2 text-muted-foreground wrap-anywhere">{summary}</p>}
    </div>
  );
}

function flushLabel(flushAtMs: number): string {
  const deltaSeconds = Math.round((flushAtMs - Date.now()) / 1000);
  if (deltaSeconds <= 0) return "now";
  if (deltaSeconds < 120) return `in ${deltaSeconds}s`;
  return `in ${Math.round(deltaSeconds / 60)}m`;
}

function activityVariant(kind: string): "destructive" | "secondary" | "outline" {
  if (kind === "run_failed" || kind === "degraded" || kind === "budget_exhausted") return "destructive";
  if (kind === "run_completed") return "secondary";
  return "outline";
}

function timeLabel(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}
