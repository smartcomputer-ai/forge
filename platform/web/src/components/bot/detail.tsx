import { useState, type ReactNode } from "react";
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Activity,
  ArrowUpRight,
  ChevronRight,
  Inbox,
  LayoutDashboard,
  RotateCcw,
  Settings2,
  Webhook,
} from "lucide-react";
import { Link, NavLink } from "react-router-dom";
import {
  api,
  type Bot,
  type BotActivityEntry,
  type BotActivityPage,
  type BotEventEnvelope,
  type BotEventPage,
  type BotRecentEvent,
  type BotLineage,
  type BotState,
  type ProfileDocument,
} from "@/api";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { BotSettingsDialog } from "./settings-dialog";
import { SendEventDialog } from "./send-event-dialog";
import { BotStatusBadge, KeyValue } from "./status";
import { TriggersSection, type BotEnvStatus } from "./triggers";

type BotView = "overview" | "events" | "activity";

/** Bot workspace: live routing state plus paginated event and activity history. */
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
  const activityPages = useInfiniteQuery({
    queryKey: ["bot-activity", bot.id],
    queryFn: ({ pageParam }) =>
      api<BotActivityPage>(
        "GET",
        `/api/v1/bots/${bot.id}/activity?limit=50${
          pageParam ? `&cursor=${encodeURIComponent(pageParam)}` : ""
        }`,
      ),
    initialPageParam: "",
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    enabled: view === "activity",
  });
  const eventPages = useInfiniteQuery({
    queryKey: ["bot-events", bot.id],
    queryFn: ({ pageParam }) =>
      api<BotEventPage>(
        "GET",
        `/api/v1/bots/${bot.id}/events?limit=50${
          pageParam ? `&cursor=${encodeURIComponent(pageParam)}` : ""
        }`,
      ),
    initialPageParam: "",
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    enabled: view === "events",
  });
  const activity = activityPages.data?.pages.flatMap((page) => page.activity) ?? [];
  const events = eventPages.data?.pages.flatMap((page) => page.events) ?? [];
  const replay = useMutation({
    mutationFn: (eventId: string) =>
      api("POST", `/api/v1/bots/${bot.id}/events/replay`, { eventId }),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["bot-state", bot.id] }),
        queryClient.invalidateQueries({ queryKey: ["bot-activity", bot.id] }),
        queryClient.invalidateQueries({ queryKey: ["bot-events", bot.id] }),
      ]);
    },
  });

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex h-12 shrink-0 items-center gap-2 border-b px-4">
        <NavLink to={`/u/${slug}/bots`} className="md:hidden" aria-label="Back to bots">
          <ChevronRight className="size-4 rotate-180" />
        </NavLink>
        <span className="min-w-0 truncate text-sm font-semibold">{bot.name}</span>
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
            <TabsTrigger value="activity"><Activity /> Activity</TabsTrigger>
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
          ) : view === "events" ? (
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
          ) : (
            <ActivityHistory
              activity={activity}
              loading={activityPages.isLoading}
              error={activityPages.error?.message}
              hasMore={activityPages.hasNextPage}
              loadingMore={activityPages.isFetchingNextPage}
              onLoadMore={() => void activityPages.fetchNextPage()}
            />
          )}
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
          {env.kind === "none" && (
            <p className="rounded-md border border-dashed p-2 text-xs text-muted-foreground">
              Profile <code>{bot.profileId}</code> has no environment: environment tools and
              command (exec) pollers are unavailable to this bot.
            </p>
          )}
          {env.kind === "provision" && (
            <p className="rounded-md bg-amber-500/10 p-2 text-xs text-amber-700 dark:text-amber-400">
              Profile <code>{bot.profileId}</code> provisions a fresh environment per session.
              Command (exec) pollers need a stable environment id — a per-session machine closes
              with its session and would strand the trigger. Point the profile at an existing
              environment to author pollers.
            </p>
          )}
        </DetailSection>

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
                <Badge variant={ready ? "secondary" : "outline"}>{ready ? "ready" : "starting"}</Badge>
                {ready && (
                  <Button variant="outline" size="xs" render={<Link to={`/u/${slug}/sessions/${session.sessionId}`} />}>
                    Open <ArrowUpRight data-icon="inline-end" />
                  </Button>
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
      <TriggersSection botId={bot.id} manage={manage} env={env} />
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
  const decisions = new Map(state?.recentEvents.map((event) => [event.id, event]) ?? []);
  return (
    <DetailSection
      title="Event history"
      description="Every stored event envelope, newest first. Replay creates a new delivery from the same payload."
    >
      {loading && <p className="text-xs text-muted-foreground">Loading events…</p>}
      {error && <p className="text-xs text-destructive">{error}</p>}
      {events.map((event) => (
        <StoredEventRow
          key={event.id}
          event={event}
          decision={decisions.get(event.eventId)}
          onReplay={onReplay ? () => onReplay(event.eventId) : undefined}
        />
      ))}
      {!loading && !error && events.length === 0 && <p className="text-xs text-muted-foreground">No events received yet.</p>}
      {hasMore && <LoadMoreButton loading={loadingMore} onClick={onLoadMore} />}
    </DetailSection>
  );
}

function ActivityHistory({
  activity,
  loading,
  error,
  hasMore,
  loadingMore,
  onLoadMore,
}: {
  activity: BotActivityEntry[];
  loading: boolean;
  error?: string;
  hasMore: boolean;
  loadingMore: boolean;
  onLoadMore: () => void;
}) {
  return (
    <DetailSection title="Activity history" description="The durable audit trail of routing decisions, runs, errors, and self-configuration.">
      {loading && <p className="text-xs text-muted-foreground">Loading activity…</p>}
      {error && <p className="text-xs text-destructive">{error}</p>}
      {activity.map((entry) => <ActivityRow key={entry.id} entry={entry} />)}
      {!loading && !error && activity.length === 0 && <p className="text-xs text-muted-foreground">No activity recorded yet.</p>}
      {hasMore && <LoadMoreButton loading={loadingMore} onClick={onLoadMore} />}
    </DetailSection>
  );
}

function DetailSection({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="grid min-w-0 content-start gap-3">
      <div className="flex min-w-0 items-start justify-between gap-3">
        <div className="grid min-w-0 flex-1 gap-0.5">
          <h2 className="text-sm font-semibold">{title}</h2>
          {description && <p className="text-xs text-muted-foreground">{description}</p>}
        </div>
      </div>
      {children}
    </section>
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
  onReplay,
}: {
  event: BotEventEnvelope;
  decision?: BotRecentEvent;
  onReplay?: () => void;
}) {
  return (
    <div className="rounded-md border p-3 text-xs">
      <div className="flex min-w-0 items-center gap-2">
        <code className="min-w-0 flex-1 truncate" title={event.eventId}>
          {event.seq != null ? `#${event.seq}` : event.eventId}
        </code>
        <Badge variant={eventStatusVariant(decision?.status)}>{decision?.status ?? "received"}</Badge>
        {onReplay && (
          <Button variant="ghost" size="icon-xs" onClick={onReplay} aria-label="Replay event"><RotateCcw /></Button>
        )}
      </div>
      <p className="mt-1 text-muted-foreground wrap-anywhere">
        {event.kind} · {event.source} · received {timeLabel(event.receivedAt)}
      </p>
      {(decision?.summary ?? decision?.failure) && (
        <p className="mt-1 line-clamp-2 text-muted-foreground wrap-anywhere">{decision?.summary ?? decision?.failure}</p>
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

function ActivityRow({ entry }: { entry: BotActivityEntry }) {
  return (
    <div className="rounded-md border p-3 text-xs">
      <div className="flex items-center gap-2">
        <Badge variant={activityVariant(entry.kind)}>{entry.kind.replaceAll("_", " ")}</Badge>
        <span className="ml-auto shrink-0 text-muted-foreground">{timeLabel(entry.createdAt)}</span>
      </div>
      {entry.detail && <p className="mt-1 text-muted-foreground wrap-anywhere">{entry.detail}</p>}
      {(entry.eventId || entry.runId) && (
        <div className="mt-1 grid min-w-0 gap-1 text-muted-foreground">
          {entry.eventId && <code className="block min-w-0 truncate">event: {entry.eventId}</code>}
          {entry.runId && <code className="block min-w-0 truncate">run: {entry.runId}</code>}
        </div>
      )}
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

function activityVariant(kind: string): "destructive" | "secondary" | "outline" {
  if (kind === "run_failed" || kind === "degraded" || kind === "budget_exhausted") return "destructive";
  if (kind === "run_completed") return "secondary";
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
