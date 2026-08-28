import { useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, CalendarClock, Check, ChevronRight, Copy, Inbox, MessageCircle, Pause, Pencil, Play, Plus, RefreshCw, RotateCw, Send, Terminal, Trash2, Webhook } from "lucide-react";
import {
  api,
  botLabel,
  type BotChatSpec,
  type BotListItem,
  type BotPollSpec,
  type BotRoute,
  type BotScheduleSpec,
  type BotTrigger,
  type BotInboxSpec,
  type BotWebhookSpec,
  type ChannelAccount,
} from "@/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import {
  Combobox,
  ComboboxChip,
  ComboboxChips,
  ComboboxChipsInput,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxItem,
  ComboboxList,
  ComboboxValue,
} from "@/components/ui/combobox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { CronBuilder } from "./cron-builder";
import { deliverySentence, deliveryShapeOf, triggerSummary } from "./trigger-summary";

export const NAME_PATTERN = /^[a-z0-9][a-z0-9-]*$/;

/// Temporal Schedules take classic 5-field crontab (minute hour day month
/// weekday) or an @-macro. Catch Quartz-style pastes (seconds field, `?`)
/// before they round-trip to a confusing server error.
export function cronProblem(value: string): string | null {
  const cron = value.trim();
  if (!cron || cron.startsWith("@")) return null;
  const fields = cron.split(/\s+/);
  if (cron.includes("?") || fields.length === 6 || fields.length === 7) {
    return "That looks like a Quartz cron. Use 5 fields (minute hour day month weekday) — every minute is * * * * *.";
  }
  if (fields.length !== 5) return "Expected 5 fields: minute hour day month weekday.";
  return null;
}

export type TriggerKind = BotTrigger["kind"];

export interface ScheduleFormState {
  once: boolean;
  at: string;
  cron: string;
  timezone: string;
  summary: string;
}

function localTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

export const defaultScheduleForm: ScheduleFormState = {
  once: false,
  at: "",
  cron: "0 9 * * 1-5",
  timezone: localTimezone(),
  summary: "",
};

export function scheduleFormProblem(form: ScheduleFormState): string | null {
  if (form.once) {
    const at = new Date(form.at).getTime();
    if (!form.at || Number.isNaN(at)) return "Pick when it fires.";
    if (at < Date.now() + 30_000) return "A one-shot fires at least 30 seconds from now.";
  } else {
    if (!form.cron.trim()) return "Set a schedule.";
    const issue = cronProblem(form.cron);
    if (issue) return issue;
  }
  if (!form.summary.trim()) return "Say what the bot should do when this fires.";
  return null;
}

export function scheduleSpecPayload(form: ScheduleFormState): BotScheduleSpec {
  return form.once
    ? { at: new Date(form.at).toISOString(), timezone: "UTC", summary: form.summary.trim() }
    : { cron: form.cron.trim(), timezone: form.timezone.trim() || "UTC", summary: form.summary.trim() };
}

/** Schedule essentials: when, and what the bot should do when it fires. */
export function ScheduleFields({
  form,
  setForm,
  lockedAt = null,
  idPrefix = "trigger",
}: {
  form: ScheduleFormState;
  setForm: (next: ScheduleFormState) => void;
  /** Editing a one-shot that already has its time: only the task is editable. */
  lockedAt?: string | null;
  idPrefix?: string;
}) {
  const cronIssue = form.once || lockedAt ? null : cronProblem(form.cron);
  return (
    <>
      {lockedAt ? (
        <p className="text-xs text-muted-foreground">
          Fires once at {new Date(lockedAt).toLocaleString()}; only the task is editable.
        </p>
      ) : (
        <Field>
          <FieldLabel>When</FieldLabel>
          <Select value={form.once ? "once" : "cron"} onValueChange={(value) => setForm({ ...form, once: value === "once" })}>
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="cron">On a recurring schedule</SelectItem>
              <SelectItem value="once">Once, at a specific time</SelectItem>
            </SelectContent>
          </Select>
        </Field>
      )}
      {!lockedAt && form.once && (
        <Field>
          <FieldLabel htmlFor={`${idPrefix}-at`}>Fire at</FieldLabel>
          <Input
            id={`${idPrefix}-at`}
            type="datetime-local"
            value={form.at}
            onChange={(event) => setForm({ ...form, at: event.target.value })}
          />
          <FieldDescription>Local time; the trigger pauses itself after firing once.</FieldDescription>
        </Field>
      )}
      {!lockedAt && !form.once && (
        <>
          <CronBuilder value={form.cron} onChange={(cron) => setForm({ ...form, cron })} />
          <div className="grid gap-3 sm:grid-cols-[1fr_10rem]">
            <Field>
              <FieldLabel htmlFor={`${idPrefix}-cron`}>Cron</FieldLabel>
              <Input
                id={`${idPrefix}-cron`}
                value={form.cron}
                onChange={(event) => setForm({ ...form, cron: event.target.value })}
                placeholder="0 9 * * 1-5"
                className="font-mono"
                aria-invalid={cronIssue !== null || undefined}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor={`${idPrefix}-timezone`}>Timezone</FieldLabel>
              <Input
                id={`${idPrefix}-timezone`}
                value={form.timezone}
                onChange={(event) => setForm({ ...form, timezone: event.target.value })}
                placeholder="UTC"
              />
            </Field>
          </div>
          {cronIssue ? (
            <p className="-mt-2 text-xs text-destructive">{cronIssue}</p>
          ) : (
            <p className="-mt-2 text-xs text-muted-foreground">
              5 fields: minute, hour, day, month, weekday — or a macro like @daily.
            </p>
          )}
        </>
      )}
      <Field>
        <FieldLabel htmlFor={`${idPrefix}-summary`}>Task</FieldLabel>
        <Textarea
          id={`${idPrefix}-summary`}
          value={form.summary}
          onChange={(event) => setForm({ ...form, summary: event.target.value })}
          rows={4}
          placeholder="What should the bot do each time this fires?"
        />
      </Field>
    </>
  );
}

export function TriggerKindIcon({
  kind,
  exec = false,
  className,
}: {
  kind: TriggerKind;
  exec?: boolean;
  className?: string;
}) {
  const classes = cn("size-3.5 shrink-0 text-muted-foreground", className);
  if (kind === "schedule") return <CalendarClock className={classes} />;
  if (kind === "bot") return <Inbox className={classes} />;
  if (kind === "chat") return <MessageCircle className={classes} />;
  if (kind === "poll") return exec ? <Terminal className={classes} /> : <RefreshCw className={classes} />;
  return <Webhook className={classes} />;
}

export function defaultTriggerName(kind: TriggerKind, pollSource?: "http" | "exec"): string {
  if (kind === "bot") return "inbox";
  if (kind === "poll") return pollSource === "exec" ? "command-poll" : "poll";
  return kind;
}

/** A sample delivery for a token-verified webhook, so a person sees the loop close without leaving the page. */
const GITHUB_SAMPLE = {
  action: "opened",
  number: 1,
  pull_request: {
    number: 1,
    title: "Sample pull request",
    html_url: "https://github.com/acme/api/pull/1",
    state: "open",
    draft: false,
    body: "A sample delivery sent from the bot page.",
    user: { login: "sample" },
    head: { ref: "feature/sample", sha: "0000000" },
    base: { ref: "main" },
  },
  repository: { full_name: "acme/api", html_url: "https://github.com/acme/api" },
  sender: { login: "sample" },
};
const PLAIN_SAMPLE = {
  kind: "sample",
  summary: "A sample event sent from the bot page.",
  data: { message: "Hello from the bot page." },
};

export async function sendSampleWebhook(trigger: BotTrigger): Promise<void> {
  const spec = trigger.spec as BotWebhookSpec;
  const github = spec.preset === "github";
  const response = await fetch(ingestUrl(trigger), {
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(github ? { "x-github-event": "pull_request", "x-github-delivery": crypto.randomUUID() } : {}),
    },
    body: JSON.stringify(github ? GITHUB_SAMPLE : PLAIN_SAMPLE),
  });
  if (!response.ok) throw new Error(`The webhook answered ${response.status}.`);
}

function ingestUrl(trigger: BotTrigger): string {
  return `${window.location.origin}${trigger.ingestPath ?? ""}`;
}

function routeLabel(route: BotRoute | null): string {
  if (route === null || route.policy === "bot") return "main session";
  if (route.policy === "perEvent") return "session per event";
  return route.key ? `session per key: ${route.key}` : "session per key";
}

/** Chat triggers never route to the main session: the default key is the conversation. */
function chatRouteLabel(route: BotRoute | null): string {
  if (route?.policy === "perEvent") return "session per message";
  return route?.policy === "perKey" && route.key ? `session per key: ${route.key}` : "session per conversation";
}

function sessionTtlLabel(ttlMs: number | null): string {
  if (ttlMs === null) return "inherits the bot's retention";
  if (ttlMs === 0) return "sessions kept forever";
  return `idle sessions close after ${Math.round(ttlMs / 3_600_000)}h`;
}

/** Unambiguous alphanumerics (no 0/O, 1/l/I), the same alphabet the server mints with. */
export const PAIRING_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";

export function mintPairingCode(length = 12): string {
  const bytes = crypto.getRandomValues(new Uint8Array(length));
  let code = "";
  for (const byte of bytes) {
    code += PAIRING_ALPHABET[byte % PAIRING_ALPHABET.length];
  }
  return code;
}

/** The bot profile's environment intent, as it affects exec pollers. */
export type BotEnvStatus =
  | { kind: "unknown" }
  | { kind: "none" }
  | { kind: "provision" }
  | { kind: "existing"; environmentId: string };

/** A paused trigger says why: the breaker, a failed poll, a one-shot that fired, or an operator. */
export function pausedLabel(reason: BotTrigger["disabledReason"]): string {
  switch (reason) {
    case "breaker":
      return "paused by breaker";
    case "poll_failed":
      return "paused: poll failed";
    case "one_shot":
      return "paused: one-shot fired";
    case "operator":
      return "paused by operator";
    case "bot_closed":
      return "bot closed";
    default:
      return "paused";
  }
}

export function pausedVariant(reason: BotTrigger["disabledReason"]): "destructive" | "outline" {
  return reason === "breaker" || reason === "poll_failed" ? "destructive" : "outline";
}

export function TriggersSection({
  universeId,
  botId,
  manage,
  env,
  id,
  headless = false,
  hideKinds = [],
}: {
  universeId: string;
  botId: string;
  manage: boolean;
  env: BotEnvStatus;
  id?: string;
  /** Rendered inside a section that already has a title; the add button moves under the list. */
  headless?: boolean;
  /** Kinds another section presents in its own words (the inbox under "Other bots"). */
  hideKinds?: TriggerKind[];
}) {
  const queryClient = useQueryClient();
  const [addOpen, setAddOpen] = useState(false);
  const [editing, setEditing] = useState<BotTrigger | null>(null);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const triggers = useQuery({
    queryKey: ["bot-triggers", universeId, botId],
    queryFn: () => api<{ triggers: BotTrigger[] }>("GET", `/api/v1/universes/${universeId}/bots/${botId}/triggers`),
    // The bot may add or change triggers itself (self-configuration); keep
    // the cards current while the page is open.
    refetchInterval: 5_000,
    refetchIntervalInBackground: false,
  });
  const bots = useQuery({
    queryKey: ["bots", universeId],
    queryFn: () => api<{ bots: BotListItem[] }>("GET", `/api/v1/universes/${universeId}/bots`),
    enabled: manage,
  });
  const invalidate = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: ["bot-triggers", universeId, botId] }),
    queryClient.invalidateQueries({ queryKey: ["bots"] }),
  ]);
  const toggle = useMutation({
    mutationFn: (trigger: BotTrigger) =>
      api("PATCH", `/api/v1/universes/${universeId}/bots/${botId}/triggers/${trigger.name}`, { enabled: !trigger.enabled }),
    onSuccess: invalidate,
  });
  const remove = useMutation({
    mutationFn: (triggerName: string) =>
      api("DELETE", `/api/v1/universes/${universeId}/bots/${botId}/triggers/${triggerName}`),
    onSuccess: () => {
      setPendingDelete(null);
      return invalidate();
    },
  });
  const visibleTriggers = (triggers.data?.triggers ?? []).filter((trigger) => !hideKinds.includes(trigger.kind));
  const sample = useMutation({
    mutationFn: sendSampleWebhook,
    onSuccess: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: ["bot-events", universeId, botId] }),
        queryClient.invalidateQueries({ queryKey: ["bot-state", universeId, botId] }),
      ]),
  });

  return (
    <section id={id} className="grid min-w-0 max-w-full scroll-mt-4 gap-2">
      {!headless && (
        <div className="flex min-w-0 items-end gap-3">
          <div className="grid min-w-0 gap-0.5">
            <h2 className="text-sm font-semibold">Triggers</h2>
            <p className="text-xs text-muted-foreground">When this bot wakes up: schedules, webhooks, polls, chats, other bots.</p>
          </div>
          {manage && (
            <Button variant="outline" size="xs" className="ml-auto" onClick={() => setAddOpen(true)}>
              <Plus data-icon="inline-start" /> Add trigger
            </Button>
          )}
        </div>
      )}
      {visibleTriggers.length === 0 && (
        <p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
          Nothing wakes this bot yet{manage ? " — add a schedule, a webhook, or a chat account. You can always message it from Chat." : "."}
        </p>
      )}
      {triggers.error && <p className="text-xs text-destructive">{triggers.error.message}</p>}
      {toggle.error && <p className="text-xs text-destructive">{toggle.error.message}</p>}
      {remove.error && <p className="text-xs text-destructive">{remove.error.message}</p>}
      {sample.error && <p className="text-xs text-destructive">{sample.error.message}</p>}
      {sample.isSuccess && (
        <p className="text-xs text-muted-foreground">Sample sent — it shows up under Activity in a moment.</p>
      )}
      {visibleTriggers.map((trigger) => {
        const exec = trigger.kind === "poll" && (trigger.spec as BotPollSpec).source.kind === "exec";
        const summary = triggerSummary(trigger);
        const delivery = deliverySentence(deliveryShapeOf(trigger), trigger.kind === "chat");
        const sampleable =
          trigger.kind === "webhook" && (trigger.spec as BotWebhookSpec).verification.scheme === "token";
        return (
          <div key={trigger.name} className="min-w-0 max-w-full overflow-hidden rounded-md border p-3 text-xs">
            <div className="flex min-w-0 items-start gap-2">
              <TriggerKindIcon kind={trigger.kind} exec={exec} className="mt-0.5" />
              <div className="min-w-0 flex-1">
                <div className="flex min-w-0 flex-wrap items-center gap-2">
                  <span className="truncate font-medium">{trigger.name}</span>
                  {!trigger.enabled && (
                    <Badge
                      variant={pausedVariant(trigger.disabledReason)}
                      title={trigger.disabledAt ? `since ${new Date(trigger.disabledAt).toLocaleString()}` : undefined}
                    >
                      {pausedLabel(trigger.disabledReason)}
                    </Badge>
                  )}
                </div>
                <p className="truncate text-muted-foreground" title={summary}>{summary}</p>
                <p className="truncate text-muted-foreground/80" title={delivery}>{delivery}</p>
              </div>
              {manage && (
                <span className="flex shrink-0 items-center">
                  {sampleable && (
                    <Button
                      variant="ghost"
                      size="xs"
                      onClick={() => sample.mutate(trigger)}
                      disabled={sample.isPending || !trigger.enabled}
                      title="Post a sample payload to this webhook"
                    >
                      <Send data-icon="inline-start" /> Send sample
                    </Button>
                  )}
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => setEditing(trigger)}
                    aria-label="Edit trigger"
                  >
                    <Pencil />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => toggle.mutate(trigger)}
                    disabled={toggle.isPending}
                    aria-label={trigger.enabled ? "Pause trigger" : "Resume trigger"}
                  >
                    {trigger.enabled ? <Pause /> : <Play />}
                  </Button>
                  {pendingDelete === trigger.name ? (
                    <Button
                      variant="destructive"
                      size="xs"
                      onClick={() => remove.mutate(trigger.name)}
                      disabled={remove.isPending}
                    >
                      Delete?
                    </Button>
                  ) : (
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      onClick={() => setPendingDelete(trigger.name)}
                      aria-label="Delete trigger"
                    >
                      <Trash2 />
                    </Button>
                  )}
                </span>
              )}
            </div>
            {trigger.lastFilterError && (
              <p
                className="mt-2 rounded-md bg-destructive/10 p-2 text-destructive wrap-anywhere"
                title={trigger.lastFilterErrorAt ? `at ${new Date(trigger.lastFilterErrorAt).toLocaleString()}` : undefined}
              >
                filter error: {trigger.lastFilterError}
              </p>
            )}
            <div className="mt-2 border-t pt-2">
              {trigger.kind === "schedule" ? (
                <ScheduleRowDetail spec={trigger.spec as BotScheduleSpec} />
              ) : trigger.kind === "poll" ? (
                <PollRowDetail trigger={trigger} />
              ) : trigger.kind === "bot" ? (
                <InboxRowDetail trigger={trigger} />
              ) : trigger.kind === "chat" ? (
                <ChatRowDetail universeId={universeId} botId={botId} trigger={trigger} manage={manage} />
              ) : (
                <WebhookRowDetail trigger={trigger} manage={manage} />
              )}
            </div>
          </div>
        );
      })}
      {manage && headless && (
        <Button variant="outline" size="sm" className="justify-self-start" onClick={() => setAddOpen(true)}>
          <Plus data-icon="inline-start" /> Add trigger
        </Button>
      )}
      {manage && (
        <AddTriggerDialog
          universeId={universeId}
          botId={botId}
          bots={bots.data?.bots ?? []}
          env={env}
          open={addOpen}
          onOpenChange={setAddOpen}
          excludeKinds={hideKinds}
        />
      )}
      {manage && editing && (
        <EditTriggerDialog
          universeId={universeId}
          botId={botId}
          bots={bots.data?.bots ?? []}
          trigger={editing}
          open
          onOpenChange={(open) => {
            if (!open) setEditing(null);
          }}
        />
      )}
    </section>
  );
}

function ScheduleRowDetail({ spec }: { spec: BotScheduleSpec }) {
  return (
    <>
      <p className="mt-1 text-muted-foreground wrap-anywhere">
        {spec.at ? (
          <>once at {new Date(spec.at).toLocaleString()}</>
        ) : (
          <>
            <code>{spec.cron}</code> · {spec.timezone}
          </>
        )}
      </p>
      <p className="mt-1 line-clamp-2 text-muted-foreground wrap-anywhere">{spec.summary}</p>
    </>
  );
}

function InboxRowDetail({ trigger }: { trigger: BotTrigger }) {
  const spec = trigger.spec as BotInboxSpec;
  return (
    <p className="mt-1 text-muted-foreground">
      Inbox: {spec.from && spec.from.length > 0 ? `accepts events from ${spec.from.join(", ")}` : "accepts events from any bot"} ·{" "}
      {routeLabel(trigger.route)}
      {trigger.filter ? ` · filter: ${trigger.filter}` : ""}
    </p>
  );
}

export interface InboxFormState extends DeliveryFormState {
  fromMode: "any" | "selected";
  fromBotIds: string[];
}

/** Lazy: `defaultDeliveryForm` is declared further down the module. */
export function defaultInboxForm(): InboxFormState {
  return { ...defaultDeliveryForm, fromMode: "any", fromBotIds: [] };
}

function inboxFormFromTrigger(trigger: BotTrigger): InboxFormState {
  const spec = trigger.spec as BotInboxSpec;
  return {
    ...deliveryFormFromTrigger(trigger),
    fromMode: spec.from === undefined ? "any" : "selected",
    fromBotIds: spec.from ?? [],
  };
}

export function inboxSelectionSpec(
  mode: InboxFormState["fromMode"],
  botIds: string[],
): BotInboxSpec {
  return mode === "any" ? {} : { from: botIds };
}

export function inboxPayload(form: InboxFormState) {
  return { spec: inboxSelectionSpec(form.fromMode, form.fromBotIds), ...deliveryPayload(form) };
}

export function inboxFormProblem(form: InboxFormState): string | null {
  if (form.fromMode === "selected" && form.fromBotIds.length === 0) {
    return "Choose at least one bot, or allow any bot.";
  }
  return deliveryFormProblem(form);
}

export function inboxBotOptionIds(
  currentBotId: string,
  bots: Pick<BotListItem, "botId">[],
  selected: string[],
): string[] {
  return [...new Set([
    ...bots.map((bot) => bot.botId).filter((id) => id !== currentBotId),
    ...selected,
  ])];
}

export function BotMultiSelect({
  currentBotId,
  bots,
  value,
  onChange,
}: {
  currentBotId: string;
  bots: BotListItem[];
  value: string[];
  onChange: (value: string[]) => void;
}) {
  const botMap = new Map(bots.map((bot) => [bot.botId, bot]));
  const label = (botId: string) => botLabel(botMap.get(botId) ?? { botId, displayName: null });
  const items = inboxBotOptionIds(currentBotId, bots, value).sort((left, right) =>
    label(left).localeCompare(label(right)),
  );

  return (
    <Combobox
      items={items}
      multiple
      value={value}
      onValueChange={onChange}
      itemToStringLabel={label}
      filter={(botId, query) => {
        const bot = botMap.get(botId);
        const search = `${label(botId)} ${botId} ${bot?.description ?? ""}`.toLocaleLowerCase();
        return search.includes(query.toLocaleLowerCase());
      }}
    >
      <ComboboxChips>
        <ComboboxValue>
          {value.map((botId) => (
            <ComboboxChip key={botId}>{label(botId)}</ComboboxChip>
          ))}
        </ComboboxValue>
        <ComboboxChipsInput id="inbox-from" placeholder={value.length ? "Add bot" : "Select bots"} />
      </ComboboxChips>
      <ComboboxContent>
        <ComboboxEmpty>No matching bots.</ComboboxEmpty>
        <ComboboxList>
          {(botId: string) => {
            const bot = botMap.get(botId);
            return (
              <ComboboxItem key={botId} value={botId}>
                <span className="min-w-0">
                  <span className="block truncate">{label(botId)}</span>
                  {(bot?.displayName || bot === undefined) && (
                    <span className="block truncate font-mono text-xs text-muted-foreground">
                      {botId}
                    </span>
                  )}
                </span>
              </ComboboxItem>
            );
          }}
        </ComboboxList>
      </ComboboxContent>
    </Combobox>
  );
}

export function InboxFields({
  currentBotId,
  bots,
  form,
  setForm,
}: {
  currentBotId: string;
  bots: BotListItem[];
  form: InboxFormState;
  setForm: (next: InboxFormState) => void;
}) {
  return (
    <>
      <Field>
        <FieldLabel>Allowed senders</FieldLabel>
        <Select
          value={form.fromMode}
          onValueChange={(value) =>
            value && setForm({ ...form, fromMode: value as InboxFormState["fromMode"] })
          }
        >
          <SelectTrigger><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value="any">Any bot in this universe</SelectItem>
            <SelectItem value="selected">Selected bots</SelectItem>
          </SelectContent>
        </Select>
        <FieldDescription>
          These bots can address this inbox with bot_emit. Narrow further with the filter (CEL sees
          event.sender).
        </FieldDescription>
      </Field>
      {form.fromMode === "selected" && (
        <Field>
          <FieldLabel htmlFor="inbox-from">Bots</FieldLabel>
          <BotMultiSelect
            currentBotId={currentBotId}
            bots={bots}
            value={form.fromBotIds}
            onChange={(fromBotIds) => setForm({ ...form, fromBotIds })}
          />
          <FieldDescription>
            Search by display name or immutable bot id. Only the selected ids are authorized.
          </FieldDescription>
        </Field>
      )}
      <DeliveryFields form={form} setForm={setForm} />
    </>
  );
}

function PollRowDetail({ trigger }: { trigger: BotTrigger }) {
  const spec = trigger.spec as BotPollSpec;
  const sourceLabel =
    spec.source.kind === "http" ? spec.source.url : `exec: ${spec.source.argv.join(" ")}`;
  return (
    <>
      <p className="mt-1 text-muted-foreground wrap-anywhere">
        <code title={sourceLabel}>{sourceLabel}</code> · every{" "}
        {Math.round(spec.intervalMs / 60_000)}m ·{" "}
        {spec.cursor.kind === "idSet" ? `dedupe by ${spec.cursor.id}` : `watermark ${spec.cursor.field}`}{" "}
        → {routeLabel(trigger.route)}
        {trigger.coalesce &&
          ` · batches ≤${trigger.coalesce.maxCount} over ${Math.round(trigger.coalesce.debounceMs / 1000)}s`}
        {trigger.deliver && trigger.deliver.whenBusy !== "queue" && ` · busy: ${trigger.deliver.whenBusy}`}
      </p>
      <p className="mt-1 text-muted-foreground">
        {trigger.cursor == null
          ? "Baselines on the first fire (existing items are not delivered)."
          : trigger.cursor.consecutiveFailures > 0
            ? `${trigger.cursor.consecutiveFailures} consecutive failure(s); last poll ${
                trigger.cursor.lastPolledAt ? new Date(trigger.cursor.lastPolledAt).toLocaleString() : "—"
              }`
            : `Last poll ${
                trigger.cursor.lastPolledAt ? new Date(trigger.cursor.lastPolledAt).toLocaleString() : "—"
              }`}
      </p>
      {trigger.filter && (
        <p className="mt-1 line-clamp-2 text-muted-foreground wrap-anywhere">
          filter: <code>{trigger.filter}</code>
        </p>
      )}
    </>
  );
}

function WebhookRowDetail({ trigger, manage }: { trigger: BotTrigger; manage: boolean }) {
  const spec = trigger.spec as BotWebhookSpec;
  const [copied, setCopied] = useState(false);
  return (
    <>
      {manage && (
        <div className="mt-1 flex min-w-0 max-w-full items-center gap-1 overflow-hidden">
          <code
            className="w-0 min-w-0 flex-1 truncate text-muted-foreground"
            title={ingestUrl(trigger)}
          >
            {ingestUrl(trigger)}
          </code>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label="Copy ingest URL"
            onClick={() => {
              void navigator.clipboard.writeText(ingestUrl(trigger)).then(() => {
                setCopied(true);
                setTimeout(() => setCopied(false), 1_500);
              });
            }}
          >
            {copied ? <Check /> : <Copy />}
          </Button>
        </div>
      )}
      <p className="mt-1 text-muted-foreground wrap-anywhere">
        {spec.preset === "github" ? "GitHub · " : ""}
        {spec.verification.scheme === "token" ? "URL token only" : "HMAC-SHA256 signed"} →{" "}
        {routeLabel(trigger.route)}
        {trigger.coalesce &&
          ` · batches ≤${trigger.coalesce.maxCount} over ${Math.round(trigger.coalesce.debounceMs / 1000)}s`}
        {trigger.deliver && trigger.deliver.whenBusy !== "queue" && ` · busy: ${trigger.deliver.whenBusy}`}
      </p>
      {trigger.filter && (
        <p className="mt-1 line-clamp-2 text-muted-foreground wrap-anywhere">
          filter: <code>{trigger.filter}</code>
        </p>
      )}
    </>
  );
}

export interface DeliveryFormState {
  routePolicy: "bot" | "perKey" | "perEvent";
  routeKey: string;
  filter: string;
  whenBusy: "queue" | "steer" | "append";
  debounceSeconds: string;
  maxWaitSeconds: string;
  maxCount: string;
  /** Routed-session retention: inherit the bot's setting, keep forever, or close after idle hours. */
  ttlMode: "inherit" | "forever" | "hours";
  ttlHours: string;
}

export interface ChatFormState extends DeliveryFormState {
  channelAccountId: string;
  scope: "any" | "direct" | "group";
  groupActivation: "mention" | "always";
  /** Comma- or newline-separated. */
  prefixesText: string;
  mentionNamesText: string;
  accessTurn: "conversation" | "members";
  accessControl: "none" | "members" | "admins" | "owners";
  requirePairing: boolean;
  priority: string;
}

function ChatRowDetail({
  universeId,
  botId,
  trigger,
  manage,
}: {
  universeId: string;
  botId: string;
  trigger: BotTrigger;
  manage: boolean;
}) {
  const spec = trigger.spec as BotChatSpec;
  const queryClient = useQueryClient();
  const [copied, setCopied] = useState(false);
  const rotate = useMutation({
    mutationFn: () =>
      api("PATCH", `/api/v1/universes/${universeId}/bots/${botId}/triggers/${trigger.name}`, {
        spec: { ...spec, pairingCode: mintPairingCode() },
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["bot-triggers", universeId, botId] }),
  });
  const account = trigger.channelAccount;
  const scope =
    spec.matchScope === "direct" ? "direct chats" : spec.matchScope === "group" ? "groups" : "direct chats and groups";
  return (
    <>
      <p className="mt-1 text-muted-foreground wrap-anywhere">
        {account ? (
          <>
            {account.provider} · {account.displayName}
          </>
        ) : (
          <span className="text-destructive">account missing</span>
        )}{" "}
        · {scope}
        {spec.matchScope !== "direct" &&
          ` · groups: ${spec.activation?.group === "always" ? "every message" : "on mention"}`}
        {" → "}
        {chatRouteLabel(trigger.route)} · {sessionTtlLabel(trigger.sessionTtlMs)}
        {trigger.coalesce &&
          ` · batches ≤${trigger.coalesce.maxCount} over ${trigger.coalesce.debounceMs / 1000}s`}
        {trigger.deliver && trigger.deliver.whenBusy !== "queue" && ` · busy: ${trigger.deliver.whenBusy}`}
      </p>
      {spec.pairingCode === null ? (
        <p className="mt-1 text-muted-foreground">Open: any conversation on the account connects without pairing.</p>
      ) : manage ? (
        <div className="mt-1 flex min-w-0 max-w-full items-center gap-1 overflow-hidden">
          <span className="shrink-0 text-muted-foreground">Pairing code</span>
          <code className="min-w-0 truncate font-medium" title={spec.pairingCode}>
            {spec.pairingCode}
          </code>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label="Copy pairing code"
            onClick={() => {
              void navigator.clipboard.writeText(spec.pairingCode ?? "").then(() => {
                setCopied(true);
                setTimeout(() => setCopied(false), 1_500);
              });
            }}
          >
            {copied ? <Check /> : <Copy />}
          </Button>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label="Rotate pairing code"
            title="Mint a fresh pairing code; conversations already paired stay connected"
            disabled={rotate.isPending}
            onClick={() => rotate.mutate()}
          >
            <RotateCw />
          </Button>
          {rotate.error && <span className="text-destructive">{rotate.error.message}</span>}
        </div>
      ) : (
        <p className="mt-1 text-muted-foreground">
          Pairing code required — shown to people who manage this bot.
        </p>
      )}
      {trigger.filter && (
        <p className="mt-1 line-clamp-2 text-muted-foreground wrap-anywhere">
          filter: <code>{trigger.filter}</code>
        </p>
      )}
    </>
  );
}

function splitList(text: string): string[] {
  return [...new Set(text.split(/[,\n]/).map((entry) => entry.trim()).filter((entry) => entry.length > 0))];
}

function chatFormFromTrigger(trigger: BotTrigger): ChatFormState {
  const spec = trigger.spec as BotChatSpec;
  return {
    ...deliveryFormFromTrigger(trigger),
    channelAccountId: spec.channelAccountId,
    scope: spec.matchScope ?? "any",
    groupActivation: spec.activation?.group ?? "mention",
    prefixesText: (spec.activation?.triggerPrefixes ?? []).join(", "),
    mentionNamesText: (spec.activation?.mentionNames ?? []).join(", "),
    accessTurn: spec.access?.turn ?? "conversation",
    accessControl: spec.access?.control ?? "admins",
    requirePairing: spec.pairingCode !== null,
    priority: String(spec.priority),
  };
}

/**
 * The chat spec for create or update. `pairingCode` is omitted to keep an
 * existing code (or let the server mint one on create), null to open the
 * connection, and freshly minted when pairing is switched on for a trigger
 * that was open.
 */
export function chatSpecPayload(
  form: Omit<ChatFormState, keyof DeliveryFormState>,
  existingCode: string | null | undefined,
): Omit<BotChatSpec, "pairingCode" | "priority"> & { pairingCode?: string | null; priority?: number } {
  const prefixes = splitList(form.prefixesText);
  const mentionNames = splitList(form.mentionNamesText);
  return {
    channelAccountId: form.channelAccountId.trim(),
    matchScope: form.scope === "any" ? null : form.scope,
    activation: {
      group: form.groupActivation,
      ...(prefixes.length > 0 ? { triggerPrefixes: prefixes } : {}),
      ...(mentionNames.length > 0 ? { mentionNames } : {}),
    },
    access: { turn: form.accessTurn, control: form.accessControl },
    ...(form.requirePairing
      ? existingCode === null
        ? { pairingCode: mintPairingCode() }
        : {}
      : { pairingCode: null }),
    ...(form.priority.trim() ? { priority: Math.round(Number(form.priority)) } : {}),
  };
}

export function chatPayload(form: ChatFormState, existingCode: string | null | undefined) {
  return { spec: chatSpecPayload(form, existingCode), ...deliveryPayload(form) };
}

export function chatFormProblem(form: ChatFormState): string | null {
  if (!form.channelAccountId.trim()) return "Choose the messaging account this bot answers on.";
  if (splitList(form.prefixesText).some((prefix) => prefix.length > 40)) {
    return "Trigger prefixes are at most 40 characters.";
  }
  if (splitList(form.mentionNamesText).some((name) => name.length > 60)) {
    return "Mention names are at most 60 characters.";
  }
  if (form.priority.trim()) {
    const priority = Number(form.priority);
    if (!Number.isInteger(priority) || priority < 0 || priority > 1000) {
      return "Priority is a whole number from 0 to 1000.";
    }
  }
  return deliveryFormProblem(form);
}

export function ChatFields({
  form,
  setForm,
}: {
  form: ChatFormState;
  setForm: (next: ChatFormState) => void;
}) {
  // Universe owners may list deployment accounts; the same picker Channels used.
  const accounts = useQuery({
    queryKey: ["channel-accounts"],
    queryFn: () => api<ChannelAccount[]>("GET", "/api/v1/channel-accounts"),
  });
  const accountList = accounts.data ?? [];
  const accountLabel = (id: string) => {
    const account = accountList.find((candidate) => candidate.id === id);
    return account ? `${account.provider} · ${account.displayName}` : id;
  };
  return (
    <>
      <Field>
        <FieldLabel htmlFor="chat-account">Messaging account</FieldLabel>
        <Select
          value={form.channelAccountId}
          onValueChange={(value) => setForm({ ...form, channelAccountId: (value as string | null) ?? "" })}
        >
          <SelectTrigger id="chat-account" className="w-full">
            <SelectValue placeholder="Select an account">
              {(value: string) => (value ? accountLabel(value) : "Select an account")}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            {accountList.map((account) => (
              <SelectItem key={account.id} value={account.id} disabled={!account.enabled}>
                {account.provider} · {account.displayName}
                {!account.enabled && " (disabled)"}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <FieldDescription>
          {accounts.error
            ? accounts.error.message
            : accounts.data && accountList.length === 0
              ? "No provider accounts are registered; a deployment admin adds them under Admin › Channels."
              : "A Telegram or WhatsApp account registered by a deployment admin."}
        </FieldDescription>
      </Field>
      <div className="grid grid-cols-2 gap-3">
        <Field>
          <FieldLabel>Conversations</FieldLabel>
          <Select
            value={form.scope}
            onValueChange={(value) => value && setForm({ ...form, scope: value as ChatFormState["scope"] })}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="any">Direct chats and groups</SelectItem>
              <SelectItem value="direct">Direct chats only</SelectItem>
              <SelectItem value="group">Groups only</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        {form.scope !== "direct" && (
          <Field>
            <FieldLabel>In groups, respond</FieldLabel>
            <Select
              value={form.groupActivation}
              onValueChange={(value) =>
                value && setForm({ ...form, groupActivation: value as ChatFormState["groupActivation"] })
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="mention">When mentioned or prefixed</SelectItem>
                <SelectItem value="always">To every message</SelectItem>
              </SelectContent>
            </Select>
          </Field>
        )}
      </div>
      {form.scope !== "direct" && (
        <div className="grid grid-cols-2 gap-3">
          <Field>
            <FieldLabel htmlFor="chat-prefixes">Trigger prefixes</FieldLabel>
            <Input
              id="chat-prefixes"
              value={form.prefixesText}
              onChange={(event) => setForm({ ...form, prefixesText: event.target.value })}
              placeholder="/ask, /lightspeed"
              className="font-mono"
            />
            <FieldDescription>Comma-separated; a message starting with one always activates the bot.</FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="chat-mention-names">Mention names</FieldLabel>
            <Input
              id="chat-mention-names"
              value={form.mentionNamesText}
              onChange={(event) => setForm({ ...form, mentionNamesText: event.target.value })}
              placeholder="@mybot"
            />
            <FieldDescription>Extra names stripped from a mention before the text reaches the bot.</FieldDescription>
          </Field>
        </div>
      )}
      <div className="grid grid-cols-2 gap-3">
        <Field>
          <FieldLabel>Who may talk to the bot</FieldLabel>
          <Select
            value={form.accessTurn}
            onValueChange={(value) => value && setForm({ ...form, accessTurn: value as ChatFormState["accessTurn"] })}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="conversation">Anyone in a paired conversation</SelectItem>
              <SelectItem value="members">Authorized members only</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <Field>
          <FieldLabel>Control commands</FieldLabel>
          <Select
            value={form.accessControl}
            onValueChange={(value) =>
              value && setForm({ ...form, accessControl: value as ChatFormState["accessControl"] })
            }
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="none">Nobody</SelectItem>
              <SelectItem value="members">Members</SelectItem>
              <SelectItem value="admins">Admins</SelectItem>
              <SelectItem value="owners">Owners</SelectItem>
            </SelectContent>
          </Select>
        </Field>
      </div>
      <div className="grid grid-cols-[1fr_8rem] gap-3">
        <Label className="gap-2 font-normal">
          <Checkbox
            checked={form.requirePairing}
            onCheckedChange={(checked) => setForm({ ...form, requirePairing: checked === true })}
          />
          <span>
            Require a pairing code
            <span className="block text-xs text-muted-foreground">
              A conversation connects once someone sends the code; the code is shown on the trigger
              to people who manage this bot. Off: any conversation on the account connects.
            </span>
          </span>
        </Label>
        <Field>
          <FieldLabel htmlFor="chat-priority">Priority</FieldLabel>
          <Input
            id="chat-priority"
            type="number"
            min={0}
            max={1000}
            value={form.priority}
            onChange={(event) => setForm({ ...form, priority: event.target.value })}
            placeholder="100"
          />
          <FieldDescription>Lower wins when several chat triggers match.</FieldDescription>
        </Field>
      </div>
      <DeliveryFields form={form} setForm={setForm} chat />
    </>
  );
}

export interface WebhookFormState extends DeliveryFormState {
  scheme: "token" | "hmac-sha256";
  grantId: string;
  header: string;
  prefix: string;
  preset: boolean;
}

export interface PollFormState extends DeliveryFormState {
  sourceKind: "http" | "exec";
  url: string;
  grantId: string;
  authHeader: string;
  authScheme: string;
  authAudience: string;
  environmentId: string;
  /** One argv entry per line. */
  argvText: string;
  cwd: string;
  intervalMinutes: string;
  items: string;
  dedupe: "idSet" | "watermark";
  dedupeField: string;
}

export const defaultDeliveryForm: DeliveryFormState = {
  routePolicy: "bot",
  routeKey: "",
  filter: "",
  whenBusy: "queue",
  debounceSeconds: "",
  maxWaitSeconds: "",
  maxCount: "",
  ttlMode: "inherit",
  ttlHours: "",
};

/// Chat defaults mirror the server's: a session per conversation kept
/// forever, batched over a short quiet period, pairing required.
export const defaultChatForm: ChatFormState = {
  ...defaultDeliveryForm,
  routePolicy: "perKey",
  debounceSeconds: "0.4",
  maxWaitSeconds: "1.5",
  maxCount: "8",
  ttlMode: "forever",
  channelAccountId: "",
  scope: "any",
  groupActivation: "mention",
  prefixesText: "",
  mentionNamesText: "",
  accessTurn: "conversation",
  accessControl: "admins",
  requirePairing: true,
  priority: "",
};

function deliveryFormFromTrigger(trigger: BotTrigger): DeliveryFormState {
  return {
    routePolicy: trigger.route?.policy ?? "bot",
    routeKey: trigger.route?.policy === "perKey" ? (trigger.route.key ?? "") : "",
    filter: trigger.filter ?? "",
    whenBusy: trigger.deliver?.whenBusy ?? "queue",
    debounceSeconds: trigger.coalesce ? String(trigger.coalesce.debounceMs / 1000) : "",
    maxWaitSeconds: trigger.coalesce ? String(trigger.coalesce.maxWaitMs / 1000) : "",
    maxCount: trigger.coalesce ? String(trigger.coalesce.maxCount) : "",
    ttlMode: trigger.sessionTtlMs === null ? "inherit" : trigger.sessionTtlMs === 0 ? "forever" : "hours",
    ttlHours: trigger.sessionTtlMs ? String(Math.round(trigger.sessionTtlMs / 3_600_000)) : "",
  };
}

export function sessionTtlMs(form: Pick<DeliveryFormState, "ttlMode" | "ttlHours">): number | null {
  if (form.ttlMode === "inherit") return null;
  if (form.ttlMode === "forever") return 0;
  return Math.round(Number(form.ttlHours) * 3_600_000);
}

export const defaultPollForm: PollFormState = {
  ...defaultDeliveryForm,
  sourceKind: "http",
  url: "",
  grantId: "",
  authHeader: "authorization",
  authScheme: "Bearer",
  authAudience: "",
  environmentId: "",
  argvText: "",
  cwd: "",
  intervalMinutes: "5",
  items: "",
  dedupe: "idSet",
  dedupeField: "id",
};

function pollFormFromTrigger(trigger: BotTrigger): PollFormState {
  const spec = trigger.spec as BotPollSpec;
  return {
    sourceKind: spec.source.kind,
    url: spec.source.kind === "http" ? spec.source.url : "",
    grantId: spec.source.kind === "http" ? (spec.source.auth?.grantId ?? "") : "",
    authHeader: spec.source.kind === "http" ? (spec.source.auth?.header ?? "authorization") : "authorization",
    authScheme: spec.source.kind === "http" ? (spec.source.auth?.scheme ?? "Bearer") : "Bearer",
    authAudience: spec.source.kind === "http" ? (spec.source.auth?.audience ?? "") : "",
    environmentId: spec.source.kind === "exec" ? (spec.source.environmentId ?? "") : "",
    argvText: spec.source.kind === "exec" ? spec.source.argv.join("\n") : "",
    cwd: spec.source.kind === "exec" ? (spec.source.cwd ?? "") : "",
    intervalMinutes: String(Math.round(spec.intervalMs / 60_000)),
    items: spec.items ?? "",
    dedupe: spec.cursor.kind,
    dedupeField: spec.cursor.kind === "idSet" ? spec.cursor.id : spec.cursor.field,
    ...deliveryFormFromTrigger(trigger),
  };
}

export function deliveryPayload(form: DeliveryFormState) {
  const debounce = Number(form.debounceSeconds);
  const coalesceOn = form.debounceSeconds.trim() !== "" && debounce > 0;
  return {
    route:
      form.routePolicy === "bot"
        ? null
        : form.routePolicy === "perEvent"
          ? { policy: "perEvent" as const }
          : { policy: "perKey" as const, key: form.routeKey.trim() || null },
    filter: form.filter.trim() || null,
    coalesce: coalesceOn
      ? {
          debounceMs: Math.round(debounce * 1000),
          maxWaitMs: Math.round(Number(form.maxWaitSeconds.trim() || form.debounceSeconds) * 1000),
          maxCount: Math.round(Number(form.maxCount.trim() || "50")),
        }
      : null,
    deliver: form.whenBusy === "queue" ? null : { whenBusy: form.whenBusy },
    sessionTtlMs: form.routePolicy === "bot" ? null : sessionTtlMs(form),
  };
}

function pollArgv(form: PollFormState): string[] {
  return form.argvText
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

export function pollPayload(form: PollFormState) {
  return {
    spec: {
      source:
        form.sourceKind === "http"
          ? {
              kind: "http" as const,
              url: form.url.trim(),
              ...(form.grantId.trim()
                ? {
                    auth: {
                      grantId: form.grantId.trim(),
                      ...(form.authHeader.trim() && form.authHeader.trim() !== "authorization"
                        ? { header: form.authHeader.trim() }
                        : {}),
                      ...(form.authScheme !== "Bearer" ? { scheme: form.authScheme } : {}),
                      ...(form.authAudience.trim() ? { audience: form.authAudience.trim() } : {}),
                    },
                  }
                : {}),
            }
          : {
              kind: "exec" as const,
              // Blank runs in the bot's own environment (the profile's existing one).
              ...(form.environmentId.trim() ? { environmentId: form.environmentId.trim() } : {}),
              argv: pollArgv(form),
              ...(form.cwd.trim() ? { cwd: form.cwd.trim() } : {}),
            },
      intervalMs: Math.round(Number(form.intervalMinutes) * 60_000),
      items: form.items.trim() || null,
      cursor:
        form.dedupe === "idSet"
          ? { kind: "idSet" as const, id: form.dedupeField.trim() }
          : { kind: "watermark" as const, field: form.dedupeField.trim() },
    },
    ...deliveryPayload(form),
  };
}

/// `env` is the bot profile's environment intent when known: a blank
/// environment is fine when the profile activates an existing one (the poll
/// runs there), and left to the server otherwise.
export function pollFormProblem(form: PollFormState, env?: BotEnvStatus): string | null {
  if (form.sourceKind === "http") {
    if (!/^https?:\/\//.test(form.url.trim())) return "The poll URL must be http(s).";
  } else {
    const ownEnvironment = env === undefined || env.kind === "existing";
    if (!form.environmentId.trim() && !ownEnvironment) {
      return "Name the environment the command runs in.";
    }
    if (pollArgv(form).length === 0) return "Give the command to run (one argv entry per line).";
  }
  const minutes = Number(form.intervalMinutes);
  if (!Number.isFinite(minutes) || minutes < 1) return "Poll at most once a minute.";
  if (!form.dedupeField.trim()) {
    return form.dedupe === "idSet"
      ? "Name the item field that identifies an item (e.g. id)."
      : "Name the item field that increases over time (e.g. updated_at).";
  }
  return deliveryFormProblem(form);
}

export function deliveryFormProblem(form: DeliveryFormState): string | null {
  if (form.debounceSeconds.trim() !== "") {
    const debounce = Number(form.debounceSeconds);
    if (!Number.isFinite(debounce) || debounce < 0.1) {
      return "Debounce must be at least 0.1 seconds.";
    }
    const maxWait = Number(form.maxWaitSeconds.trim() || form.debounceSeconds);
    if (!Number.isFinite(maxWait) || maxWait < debounce) {
      return "Max wait must be at least the debounce.";
    }
  }
  if (form.routePolicy !== "bot" && form.ttlMode === "hours") {
    const hours = Number(form.ttlHours);
    if (!Number.isFinite(hours) || hours < 1) return "Session retention must be at least 1 hour.";
  }
  return null;
}

export const defaultWebhookForm: WebhookFormState = {
  ...defaultDeliveryForm,
  scheme: "token",
  grantId: "",
  header: "",
  prefix: "",
  preset: false,
};

function webhookFormFromTrigger(trigger: BotTrigger): WebhookFormState {
  const spec = trigger.spec as BotWebhookSpec;
  return {
    scheme: spec.verification.scheme,
    grantId: spec.verification.scheme === "hmac-sha256" ? spec.verification.grantId : "",
    header: spec.verification.scheme === "hmac-sha256" ? spec.verification.header : "",
    prefix: spec.verification.scheme === "hmac-sha256" ? (spec.verification.prefix ?? "") : "",
    preset: spec.preset === "github",
    ...deliveryFormFromTrigger(trigger),
  };
}

export function webhookPayload(form: WebhookFormState) {
  return {
    spec: {
      verification:
        form.scheme === "token"
          ? { scheme: "token" as const }
          : {
              scheme: "hmac-sha256" as const,
              grantId: form.grantId.trim(),
              header: form.header.trim() || "x-signature-256",
              ...(form.prefix ? { prefix: form.prefix } : {}),
            },
      preset: form.preset ? ("github" as const) : null,
    },
    ...deliveryPayload(form),
  };
}

export function webhookFormProblem(form: WebhookFormState): string | null {
  if (form.scheme === "hmac-sha256" && !form.grantId.trim()) {
    return "Choose a retrievable credential grant for HMAC verification.";
  }
  return deliveryFormProblem(form);
}

export function WebhookFields({
  form,
  setForm,
}: {
  form: WebhookFormState;
  setForm: (next: WebhookFormState) => void;
}) {
  return (
    <>
      <Field>
        <FieldLabel>Verification</FieldLabel>
        <Select
          value={form.preset ? "github" : form.scheme}
          onValueChange={(value) => {
            if (!value) return;
            if (value === "github") {
              setForm({
                ...form,
                preset: true,
                scheme: "hmac-sha256",
                header: "x-hub-signature-256",
                prefix: "sha256=",
              });
            } else {
              setForm({ ...form, preset: false, scheme: value as "token" | "hmac-sha256" });
            }
          }}
        >
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="token">URL token only</SelectItem>
            <SelectItem value="hmac-sha256">HMAC-SHA256 signature</SelectItem>
            <SelectItem value="github">GitHub (signed)</SelectItem>
          </SelectContent>
        </Select>
        <FieldDescription>
          The ingest URL always carries a secret token; a signature additionally authenticates the
          payload.
        </FieldDescription>
      </Field>
      {form.scheme === "hmac-sha256" && (
        <div className="grid grid-cols-2 gap-3">
          <Field>
            <FieldLabel htmlFor="webhook-grant">Credential grant ID</FieldLabel>
            <Input
              id="webhook-grant"
              value={form.grantId}
              onChange={(event) => setForm({ ...form, grantId: event.target.value })}
              placeholder="grant_…"
              className="font-mono"
              autoComplete="off"
            />
            <FieldDescription>
              Active retrievable credential from Secrets; the signing value is leased only during verification.
            </FieldDescription>
          </Field>
          {!form.preset && (
            <Field>
              <FieldLabel htmlFor="webhook-header">Signature header</FieldLabel>
              <Input
                id="webhook-header"
                value={form.header}
                onChange={(event) => setForm({ ...form, header: event.target.value })}
                placeholder="x-signature-256"
              />
            </Field>
          )}
        </div>
      )}
      <DeliveryFields form={form} setForm={setForm} />
    </>
  );
}

/**
 * Routing, filtering, coalescing, busy handling, and retention live behind
 * one disclosure. Closed, it reads as a sentence, so the vocabulary is
 * learned before the fields are.
 */
export function DeliveryFields<T extends DeliveryFormState>({
  form,
  setForm,
  chat = false,
  defaultOpen = false,
}: {
  form: T;
  setForm: (next: T) => void;
  /** Chat triggers route per conversation; the main session is not an option. */
  chat?: boolean;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <Collapsible open={open} onOpenChange={setOpen} className="rounded-md border">
      <CollapsibleTrigger className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm">
        <ChevronRight
          className={cn("size-4 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")}
        />
        <span className="shrink-0 font-medium">Advanced</span>
        {!open && (
          <span className="min-w-0 truncate text-xs text-muted-foreground">{deliverySentence(form, chat)}</span>
        )}
      </CollapsibleTrigger>
      <CollapsibleContent className="grid gap-4 border-t p-3">
        <DeliveryFieldsBody form={form} setForm={setForm} chat={chat} />
      </CollapsibleContent>
    </Collapsible>
  );
}

function DeliveryFieldsBody<T extends DeliveryFormState>({
  form,
  setForm,
  chat = false,
}: {
  form: T;
  setForm: (next: T) => void;
  chat?: boolean;
}) {

  return (
    <>
      <Field>
        <FieldLabel>Sessions</FieldLabel>
        <Select
          value={form.routePolicy}
          onValueChange={(value) => value && setForm({ ...form, routePolicy: value as DeliveryFormState["routePolicy"] })}
        >
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {chat ? (
              <>
                <SelectItem value="perKey">One session per conversation</SelectItem>
                <SelectItem value="perEvent">One session per message</SelectItem>
              </>
            ) : (
              <>
                <SelectItem value="bot">Deliver to the main session</SelectItem>
                <SelectItem value="perKey">One session per key</SelectItem>
                <SelectItem value="perEvent">One session per event</SelectItem>
              </>
            )}
          </SelectContent>
        </Select>
      </Field>
      {form.routePolicy === "perKey" && (
        <Field>
          <FieldLabel htmlFor="webhook-route-key">Key expression (optional)</FieldLabel>
          <Input
            id="webhook-route-key"
            value={form.routeKey}
            onChange={(event) => setForm({ ...form, routeKey: event.target.value })}
            placeholder={chat ? "data.conversation.key" : "data.issue.number"}
            className="font-mono"
          />
          <FieldDescription>
            {chat
              ? "CEL over event, data, and headers. Defaults to the conversation (account, chat, and thread)."
              : "CEL over event, data, and headers. GitHub triggers default to the PR or issue number."}
          </FieldDescription>
        </Field>
      )}
      {form.routePolicy !== "bot" && (
        <div className="grid grid-cols-2 gap-3">
          <Field>
            <FieldLabel>Session retention</FieldLabel>
            <Select
              value={form.ttlMode}
              onValueChange={(value) =>
                value && setForm({ ...form, ttlMode: value as DeliveryFormState["ttlMode"] })
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="inherit">Inherit the bot's setting</SelectItem>
                <SelectItem value="forever">Keep sessions forever</SelectItem>
                <SelectItem value="hours">Close idle sessions after…</SelectItem>
              </SelectContent>
            </Select>
          </Field>
          {form.ttlMode === "hours" && (
            <Field>
              <FieldLabel htmlFor="trigger-ttl-hours">Idle hours</FieldLabel>
              <Input
                id="trigger-ttl-hours"
                type="number"
                min={1}
                value={form.ttlHours}
                onChange={(event) => setForm({ ...form, ttlHours: event.target.value })}
                placeholder="24"
              />
            </Field>
          )}
        </div>
      )}
      <Field>
        <FieldLabel htmlFor="webhook-filter">Filter (optional)</FieldLabel>
        <Input
          id="webhook-filter"
          value={form.filter}
          onChange={(event) => setForm({ ...form, filter: event.target.value })}
          placeholder='event.kind == "issues.opened"'
          className="font-mono"
        />
        <FieldDescription>
          CEL predicate; non-matching events are archived without waking the bot.
        </FieldDescription>
      </Field>
      <div className="grid grid-cols-3 gap-3">
        <Field>
          <FieldLabel htmlFor="webhook-debounce">Debounce (s)</FieldLabel>
          <Input
            id="webhook-debounce"
            type="number"
            min={0.1}
            step={0.1}
            value={form.debounceSeconds}
            onChange={(event) => setForm({ ...form, debounceSeconds: event.target.value })}
            placeholder="Off"
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="webhook-maxwait">Max wait (s)</FieldLabel>
          <Input
            id="webhook-maxwait"
            type="number"
            min={0.1}
            step={0.1}
            value={form.maxWaitSeconds}
            onChange={(event) => setForm({ ...form, maxWaitSeconds: event.target.value })}
            placeholder="= debounce"
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="webhook-maxcount">Max batch</FieldLabel>
          <Input
            id="webhook-maxcount"
            type="number"
            min={2}
            max={100}
            value={form.maxCount}
            onChange={(event) => setForm({ ...form, maxCount: event.target.value })}
            placeholder="50"
          />
        </Field>
      </div>
      <p className="-mt-2 text-xs text-muted-foreground">
        Coalescing: related events settle for the debounce window (bounded by max wait) and arrive
        as one batch. Leave debounce empty to deliver each event individually.
      </p>
      <Field>
        <FieldLabel>While the session is busy</FieldLabel>
        <Select
          value={form.whenBusy}
          onValueChange={(value) =>
            value && setForm({ ...form, whenBusy: value as DeliveryFormState["whenBusy"] })
          }
        >
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="queue">Queue a run for when it finishes</SelectItem>
            <SelectItem value="steer">Steer the active run</SelectItem>
            <SelectItem value="append">Append as context only (never runs)</SelectItem>
          </SelectContent>
        </Select>
      </Field>
    </>
  );
}

export function PollFields({
  form,
  setForm,
}: {
  form: PollFormState;
  setForm: (next: PollFormState) => void;
}) {
  return (
    <>
      {form.sourceKind === "http" ? (
        <>
          <Field>
            <FieldLabel htmlFor="poll-url">URL</FieldLabel>
            <Input
              id="poll-url"
              value={form.url}
              onChange={(event) => setForm({ ...form, url: event.target.value })}
              placeholder="https://api.example.com/issues?state=open"
              className="font-mono"
            />
            <FieldDescription>
              Fetched on the interval; the JSON response is diffed and only new items wake the bot.
              The first fire baselines without delivering.
            </FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="poll-grant">Credential grant ID (optional)</FieldLabel>
            <Input
              id="poll-grant"
              value={form.grantId}
              onChange={(event) => setForm({ ...form, grantId: event.target.value })}
              placeholder="grant_…"
              className="font-mono"
              autoComplete="off"
            />
            <FieldDescription>
              Active retrievable credential from Secrets. It is leased into worker memory only when the poll fires.
            </FieldDescription>
          </Field>
          {form.grantId.trim() && (
            <div className="grid grid-cols-2 gap-3">
              <Field>
                <FieldLabel htmlFor="poll-auth-header">Credential header</FieldLabel>
                <Input
                  id="poll-auth-header"
                  value={form.authHeader}
                  onChange={(event) => setForm({ ...form, authHeader: event.target.value })}
                  placeholder="authorization"
                  className="font-mono"
                />
              </Field>
              <Field>
                <FieldLabel htmlFor="poll-auth-scheme">Scheme</FieldLabel>
                <Input
                  id="poll-auth-scheme"
                  value={form.authScheme}
                  onChange={(event) => setForm({ ...form, authScheme: event.target.value })}
                  placeholder="Bearer (blank sends raw token)"
                  className="font-mono"
                />
              </Field>
            </div>
          )}
        </>
      ) : (
        <>
          <Field>
            <FieldLabel htmlFor="poll-environment">Environment</FieldLabel>
            <Input
              id="poll-environment"
              value={form.environmentId}
              onChange={(event) => setForm({ ...form, environmentId: event.target.value })}
              placeholder="Blank: the bot's own environment"
              className="font-mono"
            />
            <FieldDescription>
              The command runs as a one-shot job here with the environment's credentials; a
              sleeping environment wakes for the poll and idles back down after. Leave it blank
              to run in the environment the bot's profile activates.
            </FieldDescription>
          </Field>
          <div className="grid grid-cols-[2fr_1fr] gap-3">
            <Field>
              <FieldLabel htmlFor="poll-argv">Command</FieldLabel>
              <Textarea
                id="poll-argv"
                value={form.argvText}
                onChange={(event) => setForm({ ...form, argvText: event.target.value })}
                rows={3}
                placeholder={"./poll-orders.sh\n--json"}
                className="font-mono"
              />
              <FieldDescription>One argv entry per line; stdout must be JSON.</FieldDescription>
            </Field>
            <Field>
              <FieldLabel htmlFor="poll-cwd">Working dir (optional)</FieldLabel>
              <Input
                id="poll-cwd"
                value={form.cwd}
                onChange={(event) => setForm({ ...form, cwd: event.target.value })}
                placeholder="/srv/app"
                className="font-mono"
              />
            </Field>
          </div>
        </>
      )}
      <div className="grid grid-cols-2 gap-3">
        <Field>
          <FieldLabel htmlFor="poll-interval">Every (minutes)</FieldLabel>
          <Input
            id="poll-interval"
            type="number"
            min={1}
            value={form.intervalMinutes}
            onChange={(event) => setForm({ ...form, intervalMinutes: event.target.value })}
          />
        </Field>
        <Field>
          <FieldLabel htmlFor="poll-items">Items path (optional)</FieldLabel>
          <Input
            id="poll-items"
            value={form.items}
            onChange={(event) => setForm({ ...form, items: event.target.value })}
            placeholder="data.issues"
            className="font-mono"
          />
        </Field>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <Field>
          <FieldLabel>New-item detection</FieldLabel>
          <Select
            value={form.dedupe}
            onValueChange={(value) =>
              value &&
              setForm({
                ...form,
                dedupe: value as PollFormState["dedupe"],
                dedupeField: form.dedupeField || (value === "idSet" ? "id" : "updated_at"),
              })
            }
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="idSet">Unseen id</SelectItem>
              <SelectItem value="watermark">Increasing field</SelectItem>
            </SelectContent>
          </Select>
        </Field>
        <Field>
          <FieldLabel htmlFor="poll-dedupe-field">
            {form.dedupe === "idSet" ? "Id field" : "Watermark field"}
          </FieldLabel>
          <Input
            id="poll-dedupe-field"
            value={form.dedupeField}
            onChange={(event) => setForm({ ...form, dedupeField: event.target.value })}
            placeholder={form.dedupe === "idSet" ? "id" : "updated_at"}
            className="font-mono"
          />
        </Field>
      </div>
      <DeliveryFields form={form} setForm={setForm} />
    </>
  );
}

export interface TriggerForms {
  schedule: ScheduleFormState;
  webhook: WebhookFormState;
  poll: PollFormState;
  inbox: InboxFormState;
  chat: ChatFormState;
}

export function defaultTriggerForms(): TriggerForms {
  return {
    schedule: defaultScheduleForm,
    webhook: defaultWebhookForm,
    poll: defaultPollForm,
    inbox: defaultInboxForm(),
    chat: defaultChatForm,
  };
}

export function triggerFormProblem(kind: TriggerKind, forms: TriggerForms, env?: BotEnvStatus): string | null {
  switch (kind) {
    case "schedule":
      return scheduleFormProblem(forms.schedule);
    case "webhook":
      return webhookFormProblem(forms.webhook);
    case "poll":
      return pollFormProblem(forms.poll, env);
    case "bot":
      return inboxFormProblem(forms.inbox);
    case "chat":
      return chatFormProblem(forms.chat);
  }
}

/** The create body for one trigger, the shape `POST …/triggers` and the bot create's `triggers` take. */
export function triggerCreateBody(kind: TriggerKind, name: string, forms: TriggerForms): Record<string, unknown> {
  switch (kind) {
    case "schedule":
      return { name, kind, spec: scheduleSpecPayload(forms.schedule) };
    case "webhook":
      return { name, kind, ...webhookPayload(forms.webhook) };
    case "poll":
      return { name, kind, ...pollPayload(forms.poll) };
    case "bot":
      return { name, kind, ...inboxPayload(forms.inbox) };
    case "chat":
      return { name, kind, ...chatPayload(forms.chat, undefined) };
  }
}

/** The per-kind essentials; the Advanced disclosure is inside each kind's fields. */
export function TriggerKindFields({
  kind,
  forms,
  patch,
  botId,
  bots,
}: {
  kind: TriggerKind;
  forms: TriggerForms;
  patch: <K extends keyof TriggerForms>(key: K, next: TriggerForms[K]) => void;
  botId: string;
  bots: BotListItem[];
}) {
  switch (kind) {
    case "schedule":
      return <ScheduleFields form={forms.schedule} setForm={(next) => patch("schedule", next)} />;
    case "poll":
      return <PollFields form={forms.poll} setForm={(next) => patch("poll", next)} />;
    case "bot":
      return (
        <InboxFields currentBotId={botId} bots={bots} form={forms.inbox} setForm={(next) => patch("inbox", next)} />
      );
    case "chat":
      return <ChatFields form={forms.chat} setForm={(next) => patch("chat", next)} />;
    case "webhook":
      return <WebhookFields form={forms.webhook} setForm={(next) => patch("webhook", next)} />;
  }
}

/** The six ways a bot can be woken, as pickable cards. */
export function TriggerKindPicker({
  env,
  onPick,
  className,
  exclude = [],
}: {
  env: BotEnvStatus;
  onPick: (kind: TriggerKind, options?: { pollSource: "http" | "exec" }) => void;
  className?: string;
  exclude?: TriggerKind[];
}) {
  return (
    <div className={cn("grid content-start gap-3 sm:grid-cols-2", className)}>
      <TriggerKindChoice
        icon={<CalendarClock className="size-5" />}
        title="Schedule"
        description="At fixed times: every morning, weekdays at nine, once next Tuesday."
        onClick={() => onPick("schedule")}
      />
      <TriggerKindChoice
        icon={<Webhook className="size-5" />}
        title="Webhook"
        description="When something happens elsewhere: GitHub, an alerting tool, your own systems."
        onClick={() => onPick("webhook")}
      />
      <TriggerKindChoice
        icon={<MessageCircle className="size-5" />}
        title="Chat account"
        description="Messages from people on Telegram or WhatsApp; each conversation becomes a thread."
        onClick={() => onPick("chat")}
      />
      {!exclude.includes("bot") && (
        <TriggerKindChoice
          icon={<Inbox className="size-5" />}
          title="Other bots"
          description="Let bots in this universe message this one (at most one inbox)."
          onClick={() => onPick("bot")}
        />
      )}
      <TriggerKindChoice
        icon={<RefreshCw className="size-5" />}
        title="Check a URL"
        description="Fetch an HTTP endpoint on a timer and wake only for new items."
        onClick={() => onPick("poll", { pollSource: "http" })}
      />
      <TriggerKindChoice
        icon={<Terminal className="size-5" />}
        title="Run a command"
        description="Run a command in the bot's environment on a timer; its JSON output is diffed."
        disabled={env.kind !== "existing"}
        disabledReason={
          env.kind === "none"
            ? "The bot has no environment yet."
            : env.kind === "provision"
              ? "The bot gets a fresh environment per session; command polls need a lasting one."
              : env.kind === "unknown"
                ? "Checking the bot's environment…"
                : undefined
        }
        onClick={() => onPick("poll", { pollSource: "exec" })}
      />
    </div>
  );
}

function AddTriggerDialog({
  universeId,
  botId,
  bots,
  env,
  open,
  onOpenChange,
  excludeKinds = [],
}: {
  universeId: string;
  botId: string;
  bots: BotListItem[];
  env: BotEnvStatus;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  excludeKinds?: TriggerKind[];
}) {
  const queryClient = useQueryClient();
  const [kind, setKind] = useState<TriggerKind | null>(null);
  const [name, setName] = useState("");
  const [forms, setForms] = useState<TriggerForms>(defaultTriggerForms);
  const [error, setError] = useState<string | null>(null);
  const nameInvalid = name.trim().length > 0 && !NAME_PATTERN.test(name.trim());
  const formIssue = kind === null ? null : triggerFormProblem(kind, forms, env);
  const patch = <K extends keyof TriggerForms>(key: K, next: TriggerForms[K]) =>
    setForms((current) => ({ ...current, [key]: next }));
  const reset = () => {
    setKind(null);
    setName("");
    setForms(defaultTriggerForms());
    setError(null);
  };
  const changeOpen = (next: boolean) => {
    if (!next) reset();
    onOpenChange(next);
  };
  const pick = (next: TriggerKind, options?: { pollSource: "http" | "exec" }) => {
    if (next === "poll") {
      patch("poll", {
        ...defaultPollForm,
        sourceKind: options?.pollSource ?? "http",
        environmentId: options?.pollSource === "exec" && env.kind === "existing" ? env.environmentId : "",
      });
    }
    if (!name.trim()) setName(defaultTriggerName(next, options?.pollSource));
    setKind(next);
  };
  const create = useMutation({
    mutationFn: () => {
      if (!kind) throw new Error("Choose a trigger type.");
      return api("POST", `/api/v1/universes/${universeId}/bots/${botId}/triggers`, triggerCreateBody(kind, name.trim(), forms));
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["bot-triggers", universeId, botId] }),
        queryClient.invalidateQueries({ queryKey: ["bots"] }),
      ]);
      reset();
      onOpenChange(false);
    },
    onError: (err) => setError(err.message),
  });
  const incomplete = kind === null || !name.trim() || nameInvalid || formIssue !== null;

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogContent
        className={kind
          ? "h-[min(92dvh,900px)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 p-0 sm:max-w-xl"
          : "max-h-[92dvh] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 p-0 sm:max-w-xl"}
      >
        <DialogHeader className="border-b p-6 pr-14">
          <DialogTitle>
            {kind === "poll"
              ? forms.poll.sourceKind === "exec"
                ? "Run a command"
                : "Check a URL"
              : kind === "bot"
                ? "Other bots"
                : kind === "chat"
                  ? "Chat account"
                  : kind
                    ? `Add ${kind}`
                    : "Add trigger"}
          </DialogTitle>
          <DialogDescription>
            {kind === "schedule"
              ? "Wake the bot on a recurring schedule, or once at a specific time."
              : kind === "webhook"
                ? "Give the bot a protected URL that external systems post to."
                : kind === "poll"
                  ? "Fetch a source on an interval and wake the bot with new items."
                  : kind === "bot"
                    ? "Let other bots in this universe message this bot."
                    : kind === "chat"
                      ? "Answer Telegram or WhatsApp conversations on an account, one thread per conversation."
                      : "How should this bot wake up?"}
          </DialogDescription>
        </DialogHeader>
        {!kind ? (
          <>
            <div className="min-h-0 overflow-y-auto p-6">
              <TriggerKindPicker env={env} onPick={pick} exclude={excludeKinds} />
            </div>
            <DialogFooter className="border-t p-4">
              <Button type="button" variant="outline" onClick={() => changeOpen(false)}>
                Cancel
              </Button>
            </DialogFooter>
          </>
        ) : (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              setError(null);
              create.mutate();
            }}
            className="contents"
          >
            <div className="grid min-h-0 content-start gap-4 overflow-y-auto p-6">
              <Field>
                <FieldLabel htmlFor="trigger-name">Name</FieldLabel>
                <Input
                  id="trigger-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  aria-invalid={nameInvalid || undefined}
                  className="font-mono"
                  autoFocus
                />
                {nameInvalid ? (
                  <p className="text-xs text-destructive">
                    Use lowercase letters, numbers, and dashes, starting with a letter or number.
                  </p>
                ) : (
                  <FieldDescription>How the bot and the API refer to this trigger.</FieldDescription>
                )}
              </Field>
              <TriggerKindFields kind={kind} forms={forms} patch={patch} botId={botId} bots={bots} />
            </div>
            <div className="grid gap-2 border-t p-4">
              {formIssue && <p className="text-xs text-destructive">{formIssue}</p>}
              {error && <p className="text-sm text-destructive">{error}</p>}
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => setKind(null)}>
                  <ArrowLeft data-icon="inline-start" /> Back
                </Button>
                <Button type="submit" disabled={create.isPending || incomplete}>
                  {create.isPending ? "Adding…" : "Add trigger"}
                </Button>
              </DialogFooter>
            </div>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}

export function TriggerKindChoice({
  icon,
  title,
  description,
  onClick,
  disabled = false,
  disabledReason,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  onClick: () => void;
  disabled?: boolean;
  disabledReason?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="flex items-start gap-4 rounded-xl border p-4 text-left transition-colors hover:bg-muted/40 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:bg-transparent"
    >
      <span className="rounded-lg bg-muted p-2 text-foreground">{icon}</span>
      <span className="grid gap-1">
        <span className="font-medium">{title}</span>
        <span className="text-xs text-muted-foreground">{description}</span>
        {disabled && disabledReason && (
          <span className="text-xs text-amber-700 dark:text-amber-400">{disabledReason}</span>
        )}
      </span>
    </button>
  );
}

export function EditTriggerDialog({
  universeId,
  botId,
  bots,
  trigger,
  open,
  onOpenChange,
  deliveryOnly = false,
}: {
  universeId: string;
  botId: string;
  bots: BotListItem[];
  trigger: BotTrigger;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Only routing, batching, busy handling, and retention: the rest of the trigger is edited elsewhere (the inbox's sender list). */
  deliveryOnly?: boolean;
}) {
  const queryClient = useQueryClient();
  const scheduleSpec = trigger.kind === "schedule" ? (trigger.spec as BotScheduleSpec) : null;
  const oneShotAt = scheduleSpec?.at ?? null;
  const [forms, setForms] = useState<TriggerForms>(() => ({
    schedule: scheduleSpec
      ? {
          once: Boolean(scheduleSpec.at),
          at: scheduleSpec.at ?? "",
          cron: scheduleSpec.cron ?? "",
          timezone: scheduleSpec.timezone,
          summary: scheduleSpec.summary,
        }
      : defaultScheduleForm,
    webhook: trigger.kind === "webhook" ? webhookFormFromTrigger(trigger) : defaultWebhookForm,
    poll: trigger.kind === "poll" ? pollFormFromTrigger(trigger) : defaultPollForm,
    inbox: trigger.kind === "bot" ? inboxFormFromTrigger(trigger) : defaultInboxForm(),
    chat: trigger.kind === "chat" ? chatFormFromTrigger(trigger) : defaultChatForm,
  }));
  const [error, setError] = useState<string | null>(null);
  const patch = <K extends keyof TriggerForms>(key: K, next: TriggerForms[K]) =>
    setForms((current) => ({ ...current, [key]: next }));
  const formIssue =
    trigger.kind === "schedule" && oneShotAt
      ? forms.schedule.summary.trim()
        ? null
        : "Say what the bot should do when this fires."
      : triggerFormProblem(trigger.kind, forms);
  const save = useMutation({
    mutationFn: () =>
      api(
        "PATCH",
        `/api/v1/universes/${universeId}/bots/${botId}/triggers/${trigger.name}`,
        trigger.kind === "schedule"
          ? {
              spec: oneShotAt
                ? { at: oneShotAt, timezone: "UTC", summary: forms.schedule.summary.trim() }
                : scheduleSpecPayload({ ...forms.schedule, once: false }),
            }
          : trigger.kind === "poll"
            ? pollPayload(forms.poll)
            : trigger.kind === "bot"
              ? inboxPayload(forms.inbox)
              : trigger.kind === "chat"
                ? chatPayload(forms.chat, (trigger.spec as BotChatSpec).pairingCode)
                : webhookPayload(forms.webhook),
      ),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["bot-triggers", universeId, botId] });
      setError(null);
      onOpenChange(false);
    },
    onError: (err) => setError(err.message),
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="h-[min(92dvh,900px)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 p-0 sm:max-w-xl">
        <DialogHeader className="border-b p-6 pr-14">
          <DialogTitle>{deliveryOnly ? "Routing & batching" : `Edit ${trigger.name}`}</DialogTitle>
          <DialogDescription>
            {deliveryOnly
              ? "How messages from other bots reach this bot: which conversation, whether they batch, and what happens while it is busy."
              : trigger.kind === "schedule"
              ? "Changes apply to the next fire."
              : trigger.kind === "poll"
                ? "Source changes reset the cursor: the next check re-baselines against the source."
                : trigger.kind === "bot"
                  ? "Sender and routing changes apply to the next message another bot sends here."
                  : trigger.kind === "chat"
                    ? "Account, activation, and access changes apply to the next message; paired conversations keep their threads."
                    : "The URL keeps its token; verification and routing changes apply to the next delivery."}
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            setError(null);
            save.mutate();
          }}
          className="contents"
        >
          <div className="grid min-h-0 content-start gap-4 overflow-y-auto p-6">
            {deliveryOnly ? (
              <DeliveryFieldsBody
                form={forms.inbox}
                setForm={(next) => patch("inbox", next)}
                chat={trigger.kind === "chat"}
              />
            ) : trigger.kind === "schedule" ? (
              <ScheduleFields
                form={forms.schedule}
                setForm={(next) => patch("schedule", next)}
                lockedAt={oneShotAt}
                idPrefix="edit-trigger"
              />
            ) : (
              <TriggerKindFields kind={trigger.kind} forms={forms} patch={patch} botId={botId} bots={bots} />
            )}
          </div>
          <div className="grid gap-2 border-t p-4">
            {formIssue && <p className="text-xs text-destructive">{formIssue}</p>}
            {error && <p className="text-sm text-destructive">{error}</p>}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={save.isPending || formIssue !== null}>
                {save.isPending ? "Saving…" : "Save"}
              </Button>
            </DialogFooter>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
