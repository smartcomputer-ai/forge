import { useMemo, useState } from "react";
import { useInfiniteQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, ChevronRight, RotateCcw, Webhook } from "lucide-react";
import { Link } from "react-router-dom";
import {
  api,
  type Bot,
  type BotEventEnvelope,
  type BotEventOutcome,
  type BotEventPage,
  type BotRecentEvent,
  type BotState,
} from "@/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { SendEventDialog } from "./send-event-dialog";

type OutcomeFilter = "all" | "pending" | "handled" | "ignored" | "deferred" | "failed" | "system";

const OUTCOME_FILTERS: Array<{ value: OutcomeFilter; label: string }> = [
  { value: "all", label: "All outcomes" },
  { value: "pending", label: "Pending" },
  { value: "handled", label: "Handled" },
  { value: "ignored", label: "Ignored" },
  { value: "deferred", label: "Deferred" },
  { value: "failed", label: "Failed or blocked" },
  { value: "system", label: "Steered, appended, archived" },
];

export function matchesOutcomeFilter(outcome: BotEventOutcome | null, filter: OutcomeFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "pending":
      return outcome === null || outcome === "unresolved";
    case "failed":
      return outcome === "run_failed" || outcome === "blocked";
    case "system":
      return outcome === "steered" || outcome === "appended" || outcome === "archived";
    default:
      return outcome === filter;
  }
}

/**
 * What the bot did: one live timeline of numbered events with their outcome
 * and the bot's own one-line summary, under a strip that answers "now" and
 * "today". Replay and the test event live here, next to the history they
 * add to.
 */
export function BotActivity({
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
  const queryClient = useQueryClient();
  const [eventOpen, setEventOpen] = useState(false);
  const [outcomeFilter, setOutcomeFilter] = useState<OutcomeFilter>("all");
  const [search, setSearch] = useState("");
  const base = `/u/${slug}/bots/${bot.botId}`;
  const pages = useInfiniteQuery({
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
    refetchInterval: 5_000,
    refetchIntervalInBackground: false,
  });
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
  const events = pages.data?.pages.flatMap((page) => page.events) ?? [];
  const needle = search.trim().toLowerCase();
  const visible = useMemo(
    () =>
      events.filter(
        (event) =>
          matchesOutcomeFilter(event.outcome, outcomeFilter) &&
          (needle === "" ||
            `${event.kind} ${event.source} ${event.outcomeDetail ?? ""} ${event.sender ?? ""} ${event.session?.label ?? ""}`
              .toLowerCase()
              .includes(needle)),
      ),
    [events, outcomeFilter, needle],
  );
  // Live controller state fills in rows whose stored outcome is still null
  // (a delivery in flight); the stored outcome wins once written.
  const decisions = new Map(state?.recentEvents.map((event) => [event.id, event]) ?? []);
  const activeDeliveryIds = new Set(state?.activeDeliveries.map((delivery) => delivery.id) ?? []);
  const batchSizes = new Map<string, number>();
  for (const event of events) {
    if (event.deliveryId) batchSizes.set(event.deliveryId, (batchSizes.get(event.deliveryId) ?? 0) + 1);
  }
  const sessionHref = (sessionId: string) =>
    sessionId === state?.sessionId ? base : `${base}/chat/${encodeURIComponent(sessionId)}`;

  return (
    <div className="min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto">
      <div className="mx-auto grid w-full min-w-0 max-w-5xl gap-5 px-4 py-5 text-sm md:px-8">
        <NowStrip bot={bot} state={state} stateError={stateError} sessionHref={sessionHref} />
        <div className="flex flex-wrap items-center gap-2">
          <Select value={outcomeFilter} onValueChange={(value) => value && setOutcomeFilter(value as OutcomeFilter)}>
            <SelectTrigger size="sm" className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {OUTCOME_FILTERS.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Filter by kind, source, thread, or summary"
            className="h-8 w-64 text-xs"
          />
          <span className="text-xs text-muted-foreground">
            {visible.length}
            {pages.hasNextPage ? "+" : ""} of {events.length}
            {pages.hasNextPage ? "+" : ""} loaded
          </span>
          {manage && !bot.closedAt && (
            <Button variant="outline" size="xs" className="ml-auto" onClick={() => setEventOpen(true)}>
              <Webhook data-icon="inline-start" /> Send a test event
            </Button>
          )}
        </div>
        <div className="grid gap-1.5">
          {pages.isLoading && <p className="text-xs text-muted-foreground">Loading events…</p>}
          {pages.error && <p className="text-xs text-destructive">{pages.error.message}</p>}
          {replay.error && <p className="text-xs text-destructive">{replay.error.message}</p>}
          {visible.map((event) => (
            <EventRow
              key={event.id}
              event={event}
              decision={decisions.get(event.eventId)}
              working={event.deliveryId !== null && activeDeliveryIds.has(event.deliveryId)}
              batchSize={event.deliveryId ? (batchSizes.get(event.deliveryId) ?? 1) : 1}
              sessionHref={sessionHref}
              onReplay={manage && !bot.closedAt ? () => replay.mutate(event.eventId) : undefined}
            />
          ))}
          {!pages.isLoading && !pages.error && events.length === 0 && (
            <p className="rounded-md border border-dashed p-4 text-center text-xs text-muted-foreground">
              Nothing has happened yet. Events arrive from the bot's triggers
              {manage ? " — or send a test event to see it work." : "."}
            </p>
          )}
          {!pages.isLoading && events.length > 0 && visible.length === 0 && (
            <p className="text-xs text-muted-foreground">No loaded events match the filter.</p>
          )}
          {pages.hasNextPage && (
            <Button
              variant="outline"
              size="sm"
              className="w-full"
              disabled={pages.isFetchingNextPage}
              onClick={() => void pages.fetchNextPage()}
            >
              {pages.isFetchingNextPage ? "Loading…" : "Load older events"}
            </Button>
          )}
        </div>
      </div>
      {manage && (
        <SendEventDialog universeId={bot.universeId} botId={bot.botId} open={eventOpen} onOpenChange={setEventOpen} />
      )}
    </div>
  );
}

function NowStrip({
  bot,
  state,
  stateError,
  sessionHref,
}: {
  bot: Bot;
  state?: BotState;
  stateError?: string;
  sessionHref: (sessionId: string) => string;
}) {
  if (stateError) {
    return (
      <p className="rounded-md bg-destructive/10 p-3 text-xs text-destructive">
        The controller is unavailable: {stateError}
      </p>
    );
  }
  if (!state) return <p className="text-xs text-muted-foreground">Waiting for the controller…</p>;
  const quiet =
    state.activeDeliveries.length === 0 && state.buffers.length === 0 && state.pendingDeliveryCount === 0;
  return (
    <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
      <Stat label="Now">
        {quiet ? (
          <span className="text-muted-foreground">Nothing in flight</span>
        ) : (
          <span className="grid gap-0.5">
            {state.activeDeliveries.map((delivery) => (
              <span key={delivery.id} className="truncate">
                Working on {delivery.eventCount > 1 ? `${delivery.eventCount} events` : "an event"}
                {" → "}
                <Link to={sessionHref(delivery.sessionId)} className="underline-offset-2 hover:underline">
                  {delivery.sessionId === state.sessionId
                    ? "Main"
                    : (state.sessions.find((session) => session.sessionId === delivery.sessionId)?.label ?? "thread")}
                </Link>
              </span>
            ))}
            {state.pendingDeliveryCount > 0 && (
              <span className="text-muted-foreground">
                {state.pendingDeliveryCount} {state.pendingDeliveryCount === 1 ? "delivery" : "deliveries"} queued
              </span>
            )}
          </span>
        )}
      </Stat>
      <Stat label="Coalescing">
        {state.buffers.length === 0 ? (
          <span className="text-muted-foreground">No batches forming</span>
        ) : (
          <span className="grid gap-0.5">
            {state.buffers.map((buffer) => (
              <span key={buffer.key} className="truncate">
                {buffer.count} {buffer.count === 1 ? "event" : "events"} · flushes {flushLabel(buffer.flushAtMs)}
              </span>
            ))}
          </span>
        )}
      </Stat>
      <Stat label="Today">
        <span>
          {budgetLabel(bot.runsPerDay, state)}
          <span className="block text-muted-foreground">{state.eventsProcessed} events processed in all</span>
        </span>
      </Stat>
      <Stat label="Errors" tone={state.lastError ? "destructive" : undefined}>
        {state.lastError ? (
          <span className="line-clamp-2 wrap-anywhere" title={state.lastError}>
            {state.lastError}
          </span>
        ) : (
          <span className="text-muted-foreground">None</span>
        )}
      </Stat>
    </div>
  );
}

function Stat({
  label,
  tone,
  children,
}: {
  label: string;
  tone?: "destructive";
  children: React.ReactNode;
}) {
  return (
    <div
      className={cn(
        "grid min-w-0 content-start gap-1 rounded-md border p-3 text-xs",
        tone === "destructive" && "border-destructive/40 text-destructive",
      )}
    >
      <span className="text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">{label}</span>
      {children}
    </div>
  );
}

function EventRow({
  event,
  decision,
  working,
  batchSize,
  sessionHref,
  onReplay,
}: {
  event: BotEventEnvelope;
  decision?: BotRecentEvent;
  working: boolean;
  /** Visible events sharing this event's delivery; > 1 marks a coalesced batch. */
  batchSize: number;
  sessionHref: (sessionId: string) => string;
  onReplay?: () => void;
}) {
  const [open, setOpen] = useState(false);
  const outcome = event.outcome ?? decision?.outcome ?? null;
  const detail = event.outcomeDetail ?? decision?.summary ?? decision?.failure;
  const runId = event.runId ?? decision?.runId;
  const chip = outcome ? outcome.replaceAll("_", " ") : working ? "working" : "pending";
  return (
    <div className={cn("rounded-md border text-xs", open && "bg-muted/30")}>
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-baseline gap-x-2 px-3 py-2 text-left"
        aria-expanded={open}
      >
        <code className="w-12 shrink-0 text-muted-foreground" title={event.eventId}>
          {event.seq != null ? `#${event.seq}` : "—"}
        </code>
        <span className="min-w-0">
          <span className="block truncate">
            <span className="font-medium">{event.kind}</span>
            <span className="text-muted-foreground"> · {event.source}</span>
            {event.sender && <span className="text-muted-foreground"> · from {event.sender}</span>}
            {event.session && (
              <span className="text-muted-foreground">
                {" → "}
                <Link
                  to={sessionHref(event.session.sessionId)}
                  onClick={(click) => click.stopPropagation()}
                  className="underline-offset-2 hover:underline"
                >
                  {event.session.label}
                </Link>
              </span>
            )}
          </span>
          {detail && (
            <span className={cn("block truncate text-muted-foreground", outcome === "run_failed" && "text-destructive")}>
              {detail}
            </span>
          )}
        </span>
        <span className="flex shrink-0 items-center gap-1.5">
          {batchSize > 1 && (
            <Badge variant="outline" title={event.deliveryId ?? undefined}>
              batch of {batchSize}
            </Badge>
          )}
          {event.hops > 0 && <Badge variant="outline">{event.hops} hop{event.hops === 1 ? "" : "s"}</Badge>}
          <Badge
            variant={outcomeVariant(outcome, working)}
            title={event.resolvedAt ? `resolved ${timeLabel(event.resolvedAt)}` : undefined}
          >
            {chip}
          </Badge>
          <span className="w-16 text-right text-muted-foreground">{timeLabel(event.receivedAt)}</span>
          {open ? <ChevronDown className="size-3.5 text-muted-foreground" /> : <ChevronRight className="size-3.5 text-muted-foreground" />}
        </span>
      </button>
      {open && (
        <div className="grid gap-1.5 border-t px-3 py-2 text-muted-foreground">
          <div className="grid gap-x-4 gap-y-1 sm:grid-cols-2">
            <span>
              Event id <code className="wrap-anywhere">{event.eventId}</code>
            </span>
            <span>Received {new Date(event.receivedAt).toLocaleString()}</span>
            {event.resolvedAt && <span>Resolved {new Date(event.resolvedAt).toLocaleString()}</span>}
            {runId && (
              <span>
                Run <code>{runId}</code>
              </span>
            )}
            {event.deliveryId && (
              <span>
                Delivery <code className="wrap-anywhere">{event.deliveryId}</code>
              </span>
            )}
            {event.inReplyTo && (
              <span>
                Reply to #{event.inReplyTo.seq} at {event.inReplyTo.bot}
              </span>
            )}
            {decision?.usage && decision.usage.inputTokens > 0 && (
              <span>
                {Math.round((decision.usage.cachedInputTokens / decision.usage.inputTokens) * 100)}% of{" "}
                {decision.usage.inputTokens.toLocaleString()} prompt tokens served from cache
              </span>
            )}
          </div>
          {detail && <p className="wrap-anywhere text-foreground">{detail}</p>}
          {onReplay && (
            <div>
              <Button variant="outline" size="xs" onClick={onReplay}>
                <RotateCcw data-icon="inline-start" /> Replay this event
              </Button>
              <span className="ml-2">Creates a new delivery from the same payload.</span>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function outcomeVariant(
  outcome: BotEventOutcome | null,
  working: boolean,
): "destructive" | "secondary" | "outline" | "default" {
  if (outcome === "run_failed" || outcome === "blocked") return "destructive";
  if (outcome === "handled") return "secondary";
  if (outcome === null && working) return "default";
  return "outline";
}

function flushLabel(flushAtMs: number): string {
  const deltaSeconds = Math.round((flushAtMs - Date.now()) / 1000);
  if (deltaSeconds <= 0) return "now";
  if (deltaSeconds < 120) return `in ${deltaSeconds}s`;
  return `in ${Math.round(deltaSeconds / 60)}m`;
}

function timeLabel(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const today = new Date().toDateString() === date.toDateString();
  return today
    ? date.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
    : date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** `runsPerDay` is spent by bot runs and by the sub-agent sessions those runs delegate. */
export function budgetLabel(runsPerDay: number | null, state: BotState | null | undefined): string {
  const runs = state?.runsToday ?? 0;
  const descendants = state?.descendantsToday ?? 0;
  const used = runs + descendants;
  const limit = runsPerDay === null ? `${used} runs, no daily limit` : `${used} / ${runsPerDay} runs`;
  return descendants > 0 ? `${limit} (${runs} runs, ${descendants} sub-agents)` : limit;
}
