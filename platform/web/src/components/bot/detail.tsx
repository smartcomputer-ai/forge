import { useState, type ReactNode } from "react";
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowUpRight,
  ChevronRight,
  Inbox,
  LayoutDashboard,
  LoaderCircle,
  RotateCcw,
  Settings2,
  Webhook,
} from "lucide-react";
import { Link, NavLink, useNavigate } from "react-router-dom";
import {
  api,
  botLabel,
  type Bot,
  type BotEventEnvelope,
  type BotEventPage,
  type BotRecentEvent,
  type BotLineage,
  type BotState,
  type ProfileDocument,
} from "@/api";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { BotEnvironmentCard } from "./environment-card";
import { BotSettingsDialog } from "./settings-dialog";
import { SendEventDialog } from "./send-event-dialog";
import { BotStatusBadge, DetailSection, KeyValue } from "./status";
import { TriggersSection, type BotEnvStatus } from "./triggers";

type BotView = "overview" | "events";

/** Bot workspace: live routing state plus paginated event history with outcomes. */
export function BotDetail({
  slug,
  bot,
  state,
  lineage,
  stateError,
  manage,
}: {
  slug: string;
  bot: Bot;
  state?: BotState;
  lineage?: BotLineage;
  stateError?: string;
  manage: boolean;
}) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [eventOpen, setEventOpen] = useState(false);
  const [view, setView] = useState<BotView>("overview");
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const eventPages = useInfiniteQuery({
    queryKey: ["bot-events", bot.universeId, bot.botId],
    queryFn: ({ pageParam }) =>
      api<BotEventPage>(
        "GET",
        `/api/v1/universes/${bot.universeId}/bots/${bot.botId}/events?limit=50${
          pageParam ? `&cursor=${encodeURIComponent(pageParam)}` : ""
        }`,
      ),
    initialPageParam: "",
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    enabled: view === "events",
  });
  const events = eventPages.data?.pages.flatMap((page) => page.events) ?? [];
  const replay = useMutation({
    mutationFn: (eventId: string) =>
      api("POST", `/api/v1/universes/${bot.universeId}/bots/${bot.botId}/events/replay`, { eventId }),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["bot-state", bot.universeId, bot.botId] }),
        queryClient.invalidateQueries({ queryKey: ["bot-events", bot.universeId, bot.botId] }),
      ]);
    },
  });

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex h-12 shrink-0 items-center gap-2 border-b px-4">
        <NavLink to={`/u/${slug}/bots`} className="md:hidden" aria-label="Back to bots">
          <ChevronRight className="size-4 rotate-180" />
        </NavLink>
        <span className="min-w-0 truncate text-sm font-semibold">{botLabel(bot)}</span>
        {bot.displayName && (
          <code className="hidden truncate text-xs text-muted-foreground sm:inline">{bot.botId}</code>
        )}
        <BotStatusBadge status={state?.controllerStatus} />
        {manage && (
          <div className="ml-auto flex items-center gap-1">
            <Button variant="outline" size="xs" onClick={() => setEventOpen(true)}>
              <Webhook data-icon="inline-start" /> Send event
            </Button>
            <Button variant="ghost" size="icon-sm" onClick={() => setSettingsOpen(true)} aria-label="Bot settings">
              <Settings2 />
            </Button>
          </div>
        )}
      </div>
      <div className="shrink-0 border-b px-4">
        <Tabs value={view} onValueChange={(next) => setView(next as BotView)} className="gap-0">
          <TabsList variant="line" className="h-10">
            <TabsTrigger value="overview"><LayoutDashboard /> Overview</TabsTrigger>
            <TabsTrigger value="events">
              <Inbox /> Events
              {state && state.pendingEventCount > 0 && <Badge variant="outline">{state.pendingEventCount}</Badge>}
            </TabsTrigger>
          </TabsList>
        </Tabs>
      </div>
      <div className="min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto">
        <div className="mx-auto w-full min-w-0 max-w-5xl px-4 py-6 text-sm md:px-8">
          {view === "overview" ? (
            <BotOverview
              slug={slug}
              lineage={lineage}
              bot={bot}
              state={state}
              stateError={stateError}
              manage={manage}
            />
          ) : (
            <EventHistory
              events={events}
              state={state}
              loading={eventPages.isLoading}
              error={eventPages.error?.message}
              hasMore={eventPages.hasNextPage}
              loadingMore={eventPages.isFetchingNextPage}
              onLoadMore={() => void eventPages.fetchNextPage()}
              onReplay={manage ? (eventId) => replay.mutate(eventId) : undefined}
            />
          )}
        </div>
      </div>
      {manage && (
        <>
          <BotSettingsDialog
            universeId={bot.universeId}
            bot={bot}
            open={settingsOpen}
            onOpenChange={setSettingsOpen}
            onDeleted={() => navigate(`/u/${slug}/bots`)}
          />
          <SendEventDialog universeId={bot.universeId} botId={bot.botId} open={eventOpen} onOpenChange={setEventOpen} />
        </>
      )}
    </div>
  );
}

function BotOverview({
  slug,
  bot,
  state,
  lineage,
  stateError,
  manage,
}: {
  slug: string;
  bot: Bot;
  state?: BotState;
  lineage?: BotLineage;
  stateError?: string;
  manage: boolean;
}) {
  // The profile's environment intent decides whether exec pollers (and
  // environment tools) have a stable machine to run on; surface it here so
  // an operator learns about the gap before a trigger fails.
  const profile = useQuery({
    queryKey: ["profile", bot.universeId, bot.profileId],
    // The route returns the profile document directly (not wrapped).
    queryFn: () =>
      api<ProfileDocument>(
        "GET",
        `/api/v1/universes/${bot.universeId}/profiles/${encodeURIComponent(bot.profileId)}`,
      ),
    staleTime: 60_000,
    retry: false,
  });
  // Unreadable profile (e.g. a viewer without write access) stays
  // "unknown": no warning banner, and the exec-poll card explains itself.
  const env: BotEnvStatus =
    profile.isLoading || profile.isError || profile.data === undefined
      ? { kind: "unknown" }
      : profile.data.environment == null
        ? { kind: "none" }
        : profile.data.environment.type === "existing"
          ? { kind: "existing", environmentId: profile.data.environment.environmentId }
          : { kind: "provision" };
  return (
    <div className="grid gap-10">
      <div className="grid min-w-0 gap-10 lg:grid-cols-2">
        <DetailSection title="Bot" description="Configuration and current controller health.">
          <KeyValue label="Profile" value={bot.profileId} />
          <KeyValue label="Budget" value={budgetLabel(bot.runsPerDay, state)} />
          <KeyValue label="Processed" value={String(state?.eventsProcessed ?? 0)} />
          {bot.closedAt ? (
            <p className="rounded-md bg-muted p-2 text-xs text-muted-foreground">
              Closed {new Date(bot.closedAt).toLocaleString()}: sessions and schedules were
              released and events are refused. The record and its history stay until the bot is
              deleted.
            </p>
          ) : (
            !bot.enabled && (
              <p className="rounded-md bg-muted p-2 text-xs text-muted-foreground">
                Disabled: schedules are paused and pending events wait.
              </p>
            )
          )}
          {stateError && (
            <p className="rounded-md bg-destructive/10 p-2 text-xs text-destructive">
              Controller unavailable: {stateError}
            </p>
          )}
          {state?.lastError && (
            <p className="rounded-md bg-destructive/10 p-2 text-xs text-destructive">{state.lastError}</p>
          )}
          {env.kind === "none" && (
            <p className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
              Profile <code>{bot.profileId}</code> has no environment: environment tools and
              command (exec) pollers are unavailable to this bot.
            </p>
          )}
          {env.kind === "provision" && (
            <p className="rounded-md bg-amber-500/10 p-2 text-xs text-amber-700 dark:text-amber-400">
              Profile <code>{bot.profileId}</code> provisions a fresh environment per session
              (a sandbox per event). Command (exec) pollers need a stable environment — a
              per-session machine closes with its session and would strand the trigger. Point
              the profile at an existing environment to author pollers.
            </p>
          )}
        </DetailSection>

        {env.kind === "existing" && (
          <BotEnvironmentCard
            slug={slug}
            universeId={bot.universeId}
            environmentId={env.environmentId}
            manage={manage}
          />
        )}

        <DetailSection title="Inbox now" description="Events waiting, coalescing, or actively delivering.">
          {state ? (
            <>
              <KeyValue label="Pending" value={String(state.pendingEventCount)} />
              <KeyValue label="Deliveries" value={String(state.pendingDeliveryCount)} />
              {state.buffers.map((buffer) => (
                <p key={buffer.key} className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
                  Coalescing {buffer.count} event(s) · flushes {flushLabel(buffer.flushAtMs)}
                </p>
              ))}
              {state.activeDeliveries.map((active) => (
                <EventRow
                  key={active.id}
                  id={active.id}
                  status="active"
                  eventCount={active.eventCount}
                  summary={active.sessionId === state.sessionId ? undefined : `→ ${active.sessionId}`}
                />
              ))}
              {state.pendingEventCount === 0 && state.pendingDeliveryCount === 0 &&
                state.buffers.length === 0 && state.activeDeliveries.length === 0 && (
                  <p className="text-xs text-muted-foreground">Inbox is clear.</p>
                )}
            </>
          ) : (
            <p className="text-xs text-muted-foreground">Waiting for the controller…</p>
          )}
        </DetailSection>
      </div>

      {bot.brief && (
        <DetailSection title="Standing brief" description="The persistent instruction applied to every event this bot handles.">
          <div className="rounded-lg border bg-muted/30 p-4">
            <p className="whitespace-pre-wrap text-sm leading-relaxed text-foreground">{bot.brief}</p>
          </div>
        </DetailSection>
      )}

      <DetailSection
        title="Sessions"
        description="Runtime sessions currently managed by this bot, with the sub-agents they delegated to."
      >
        {state ? state.sessions.map((session) => {
          const isMain = session.kind === "main";
          const ready = !isMain || state.sessionReady;
          const rotating = state.rotatingSessionIds?.includes(session.sessionId) ?? false;
          const descendants = lineage?.[session.sessionId];
          return (
            <div
              key={session.sessionId}
              className="min-w-0 max-w-full overflow-hidden rounded-md border p-2 text-xs"
            >
              <div className="flex min-w-0 items-center gap-2">
                <span className="min-w-0 flex-1">
                  <code className="block truncate">{session.sessionId}</code>
                  <span className="text-muted-foreground">
                    {isMain ? "Main session" : session.kind === "keyed" ? `Key: ${session.label}` : session.label}
                  </span>
                </span>
                {descendants && descendants.total > 0 && (
                  <Badge variant="outline" title="Sub-agent sessions under this session: open / lifetime">
                    {descendants.open}/{descendants.total} sub-agents
                  </Badge>
                )}
                <Badge variant={ready && !rotating ? "secondary" : "outline"}>
                  {rotating ? "resetting" : ready ? "ready" : "starting"}
                </Badge>
                {ready && (
                  <Button variant="outline" size="xs" render={<Link to={`/u/${slug}/sessions/${session.sessionId}`} />}>
                    Open <ArrowUpRight data-icon="inline-end" />
                  </Button>
                )}
                {manage && (
                  <SessionResetButton
                    universeId={bot.universeId}
                    botId={bot.botId}
                    sessionId={session.sessionId}
                    label={isMain ? "main session" : session.label}
                    rotating={rotating}
                  />
                )}
              </div>
              {descendants && descendants.children.length > 0 && (
                <ul className="mt-2 flex flex-wrap gap-1 border-t pt-2">
                  {descendants.children.map((child) => (
                    <li key={child.id}>
                      <Link
                        to={`/u/${slug}/sessions/${child.id}`}
                        className={cn(
                          "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 font-mono hover:bg-muted",
                          child.lifecycleStatus === "closed" && "text-muted-foreground",
                        )}
                        title={`${child.id} · ${child.profileId ?? "sub-agent"} · depth ${child.depth} · ${child.lifecycleStatus}`}
                      >
                        {"↳ ".repeat(Math.max(0, child.depth - 1))}
                        {child.displayName ?? child.id.slice(0, 14)}
                        {child.lifecycleStatus !== "closed" ? " ●" : ""}
                      </Link>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          );
        }) : <p className="text-xs text-muted-foreground">Waiting for the controller…</p>}
      </DetailSection>
      <TriggersSection universeId={bot.universeId} botId={bot.botId} manage={manage} env={env} />
    </div>
  );
}

function SessionResetButton({
  universeId,
  botId,
  sessionId,
  label,
  rotating,
}: {
  universeId: string;
  botId: string;
  sessionId: string;
  label: string;
  rotating: boolean;
}) {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const reset = useMutation({
    mutationFn: () =>
      api(
        "POST",
        `/api/v1/universes/${universeId}/bots/${botId}/sessions/${encodeURIComponent(sessionId)}/rotate`,
      ),
    onSuccess: () => {
      setOpen(false);
      return queryClient.invalidateQueries({ queryKey: ["bot-state", universeId, botId] });
    },
  });
  const pending = rotating || reset.isPending;
  return (
    <div className="flex shrink-0 flex-col items-end gap-1">
      <AlertDialog
        open={open}
        onOpenChange={(next) => {
          setOpen(next);
          if (next) reset.reset();
        }}
      >
        <AlertDialogTrigger
          render={
            <Button
              variant="ghost"
              size="icon-xs"
              disabled={pending}
              aria-label={`Reset ${label}`}
              title="Close this session and continue with a fresh generation"
            />
          }
        >
          {pending ? <LoaderCircle className="animate-spin" /> : <RotateCcw />}
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Reset {label}?</AlertDialogTitle>
            <AlertDialogDescription>
              The current session and its open sub-agents will close, and the bot will continue
              in a fresh session with no prior conversation history. Active work finishes first;
              already admitted events remain queued.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep session</AlertDialogCancel>
            <AlertDialogAction disabled={reset.isPending} onClick={() => reset.mutate()}>
              {reset.isPending ? "Resetting…" : "Reset session"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      {reset.error && (
        <span className="max-w-48 text-right text-xs text-destructive">{reset.error.message}</span>
      )}
    </div>
  );
}

function EventHistory({
  events,
  state,
  loading,
  error,
  hasMore,
  loadingMore,
  onLoadMore,
  onReplay,
}: {
  events: BotEventEnvelope[];
  state?: BotState;
  loading: boolean;
  error?: string;
  hasMore: boolean;
  loadingMore: boolean;
  onLoadMore: () => void;
  onReplay?: (eventId: string) => void;
}) {
  // Live controller state fills in rows whose stored outcome is still null
  // (a delivery in flight); the stored outcome wins once written.
  const decisions = new Map(state?.recentEvents.map((event) => [event.id, event]) ?? []);
  const batchSizes = new Map<string, number>();
  for (const event of events) {
    if (event.deliveryId) batchSizes.set(event.deliveryId, (batchSizes.get(event.deliveryId) ?? 0) + 1);
  }
  return (
    <DetailSection
      title="Event history"
      description="Every stored event envelope, newest first, with its outcome: what came of each event — the bot's decision, or the system's. Replay creates a new delivery from the same payload."
    >
      {loading && <p className="text-xs text-muted-foreground">Loading events…</p>}
      {error && <p className="text-xs text-destructive">{error}</p>}
      {events.map((event) => (
        <StoredEventRow
          key={event.id}
          event={event}
          decision={decisions.get(event.eventId)}
          batchSize={event.deliveryId ? batchSizes.get(event.deliveryId) ?? 1 : 1}
          onReplay={onReplay ? () => onReplay(event.eventId) : undefined}
        />
      ))}
      {!loading && !error && events.length === 0 && <p className="text-xs text-muted-foreground">No events received yet.</p>}
      {hasMore && <LoadMoreButton loading={loadingMore} onClick={onLoadMore} />}
    </DetailSection>
  );
}

function LoadMoreButton({ loading, onClick }: { loading: boolean; onClick: () => void }) {
  return (
    <Button variant="outline" size="sm" className="w-full" disabled={loading} onClick={onClick}>
      {loading ? "Loading…" : "Load more"}
    </Button>
  );
}

function StoredEventRow({
  event,
  decision,
  batchSize,
  onReplay,
}: {
  event: BotEventEnvelope;
  decision?: BotRecentEvent;
  /** Visible events sharing this event's delivery; > 1 marks a coalesced batch. */
  batchSize: number;
  onReplay?: () => void;
}) {
  const outcome = event.outcome ?? decision?.outcome ?? null;
  const detail = event.outcomeDetail ?? decision?.summary ?? decision?.failure;
  const runId = event.runId ?? decision?.runId;
  return (
    <div className="rounded-md border p-3 text-xs">
      <div className="flex min-w-0 items-center gap-2">
        <code className="min-w-0 flex-1 truncate" title={event.eventId}>
          {event.seq != null ? `#${event.seq}` : event.eventId}
        </code>
        {batchSize > 1 && (
          <Badge variant="outline" title={event.deliveryId ?? undefined}>batch of {batchSize}</Badge>
        )}
        {runId && <code className="shrink-0 text-muted-foreground">{runId}</code>}
        <Badge
          variant={eventStatusVariant(outcome ?? undefined)}
          title={event.resolvedAt ? `resolved ${timeLabel(event.resolvedAt)}` : undefined}
        >
          {outcome ? outcome.replaceAll("_", " ") : "pending"}
        </Badge>
        {onReplay && (
          <Button variant="ghost" size="icon-xs" onClick={onReplay} aria-label="Replay event"><RotateCcw /></Button>
        )}
      </div>
      <p className="mt-1 text-muted-foreground wrap-anywhere">
        {event.kind} · {event.source} · received {timeLabel(event.receivedAt)}
      </p>
      {(event.sender || event.inReplyTo) && (
        <p className="mt-1 flex flex-wrap gap-1">
          {event.sender && <Badge variant="outline">from {event.sender}</Badge>}
          {event.inReplyTo && (
            <Badge variant="outline">
              reply to #{event.inReplyTo.seq} at {event.inReplyTo.bot}
            </Badge>
          )}
          {event.hops > 0 && <Badge variant="outline">{event.hops} hop{event.hops === 1 ? "" : "s"}</Badge>}
        </p>
      )}
      {detail && <p className="mt-1 line-clamp-2 text-muted-foreground wrap-anywhere">{detail}</p>}
      {decision?.usage && (
        <p className="mt-1 text-muted-foreground">
          {Math.round((decision.usage.cachedInputTokens / decision.usage.inputTokens) * 100)}% of{" "}
          {decision.usage.inputTokens.toLocaleString()} prompt tokens cached
        </p>
      )}
      {event.session && <p className="mt-1 truncate text-muted-foreground">Session: {event.session.label}</p>}
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
      <div className="flex min-w-0 items-center gap-2">
        <code className="min-w-0 flex-1 truncate">{id}</code>
        {eventCount !== undefined && eventCount > 1 && <Badge variant="outline">{eventCount} events</Badge>}
        <Badge variant={eventStatusVariant(status)}>{status.replaceAll("_", " ")}</Badge>
        {onReplay && <Button variant="ghost" size="icon-xs" onClick={onReplay} aria-label="Replay event"><RotateCcw /></Button>}
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

function eventStatusVariant(status: string | undefined): "destructive" | "secondary" | "outline" {
  if (status === "run_failed" || status === "blocked") return "destructive";
  if (status === "handled") return "secondary";
  return "outline";
}

function timeLabel(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}


/** `runsPerDay` is spent by bot runs and by sub-agent sessions the runs delegate. */
function budgetLabel(runsPerDay: number | null, state: BotState | null | undefined): string {
  if (runsPerDay === null) return "Unlimited";
  const runs = state?.runsToday ?? 0;
  const descendants = state?.descendantsToday ?? 0;
  const used = `${runs + descendants} / ${runsPerDay} today`;
  return descendants > 0 ? `${used} (${runs} runs, ${descendants} sub-agents)` : `${used}`;
}
