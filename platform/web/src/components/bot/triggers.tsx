import { useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, CalendarClock, Check, Copy, Pause, Pencil, Play, Plus, Trash2, Webhook } from "lucide-react";
import {
  api,
  type BotRoute,
  type BotScheduleSpec,
  type BotTrigger,
  type BotWebhookSpec,
} from "@/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { CronBuilder } from "./cron-builder";

const NAME_PATTERN = /^[a-z0-9][a-z0-9-]*$/;

/// Temporal Schedules take classic 5-field crontab (minute hour day month
/// weekday) or an @-macro. Catch Quartz-style pastes (seconds field, `?`)
/// before they round-trip to a confusing server error.
function cronProblem(value: string): string | null {
  const cron = value.trim();
  if (!cron || cron.startsWith("@")) return null;
  const fields = cron.split(/\s+/);
  if (cron.includes("?") || fields.length === 6 || fields.length === 7) {
    return "That looks like a Quartz cron. Use 5 fields (minute hour day month weekday) — every minute is * * * * *.";
  }
  if (fields.length !== 5) return "Expected 5 fields: minute hour day month weekday.";
  return null;
}

function ingestUrl(trigger: BotTrigger): string {
  const spec = trigger.spec as BotWebhookSpec;
  return `${window.location.origin}/api/v1/hooks/bots/${trigger.id}/${spec.token}`;
}

function routeLabel(route: BotRoute | null): string {
  if (route === null || route.policy === "bot") return "main session";
  if (route.policy === "perEvent") return "session per event";
  return route.key ? `session per key: ${route.key}` : "session per key";
}

export function TriggersSection({ botId, manage }: { botId: string; manage: boolean }) {
  const queryClient = useQueryClient();
  const [addOpen, setAddOpen] = useState(false);
  const [editing, setEditing] = useState<BotTrigger | null>(null);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const triggers = useQuery({
    queryKey: ["bot-triggers", botId],
    queryFn: () => api<{ triggers: BotTrigger[] }>("GET", `/api/v1/bots/${botId}/triggers`),
  });
  const invalidate = () => queryClient.invalidateQueries({ queryKey: ["bot-triggers", botId] });
  const toggle = useMutation({
    mutationFn: (trigger: BotTrigger) =>
      api("PATCH", `/api/v1/bots/${botId}/triggers/${trigger.id}`, { enabled: !trigger.enabled }),
    onSuccess: invalidate,
  });
  const remove = useMutation({
    mutationFn: (triggerId: string) => api("DELETE", `/api/v1/bots/${botId}/triggers/${triggerId}`),
    onSuccess: () => {
      setPendingDelete(null);
      return invalidate();
    },
  });

  return (
    <section className="grid min-w-0 max-w-full gap-2">
      <div className="flex min-w-0 items-end gap-3">
        <div className="grid min-w-0 gap-0.5">
          <h2 className="text-sm font-semibold">Triggers</h2>
          <p className="text-xs text-muted-foreground">Schedules and webhooks that wake this bot.</p>
        </div>
        {manage && (
          <Button variant="outline" size="xs" className="ml-auto" onClick={() => setAddOpen(true)}>
            <Plus data-icon="inline-start" /> Trigger
          </Button>
        )}
      </div>
      {triggers.data?.triggers.length === 0 && (
        <p className="text-xs text-muted-foreground">
          No triggers yet{manage ? " — add a schedule or webhook to make this bot proactive." : "."}
        </p>
      )}
      {triggers.error && <p className="text-xs text-destructive">{triggers.error.message}</p>}
      {triggers.data?.triggers.map((trigger) => (
        <div key={trigger.id} className="min-w-0 max-w-full overflow-hidden rounded-md border p-2 text-xs">
          <div className="flex min-w-0 items-center gap-2">
            {trigger.kind === "schedule" ? (
              <CalendarClock className="size-3.5 shrink-0 text-muted-foreground" />
            ) : (
              <Webhook className="size-3.5 shrink-0 text-muted-foreground" />
            )}
            <span className="min-w-0 flex-1 truncate font-medium">{trigger.name}</span>
            {!trigger.enabled && <Badge variant="outline">paused</Badge>}
            {manage && (
              <span className="flex shrink-0 items-center">
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
                {pendingDelete === trigger.id ? (
                  <Button
                    variant="destructive"
                    size="xs"
                    onClick={() => remove.mutate(trigger.id)}
                    disabled={remove.isPending}
                  >
                    Delete?
                  </Button>
                ) : (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => setPendingDelete(trigger.id)}
                    aria-label="Delete trigger"
                  >
                    <Trash2 />
                  </Button>
                )}
              </span>
            )}
          </div>
          {trigger.kind === "schedule" ? (
            <ScheduleRowDetail spec={trigger.spec as BotScheduleSpec} />
          ) : (
            <WebhookRowDetail trigger={trigger} manage={manage} />
          )}
        </div>
      ))}
      {manage && <AddTriggerDialog botId={botId} open={addOpen} onOpenChange={setAddOpen} />}
      {manage && editing && (
        <EditTriggerDialog
          botId={botId}
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

interface WebhookFormState {
  scheme: "token" | "hmac-sha256";
  secret: string;
  header: string;
  prefix: string;
  preset: boolean;
  routePolicy: "bot" | "perKey" | "perEvent";
  routeKey: string;
  filter: string;
  whenBusy: "queue" | "steer" | "append";
  debounceSeconds: string;
  maxWaitSeconds: string;
  maxCount: string;
}

const defaultWebhookForm: WebhookFormState = {
  scheme: "token",
  secret: "",
  header: "",
  prefix: "",
  preset: false,
  routePolicy: "bot",
  routeKey: "",
  filter: "",
  whenBusy: "queue",
  debounceSeconds: "",
  maxWaitSeconds: "",
  maxCount: "",
};

function webhookFormFromTrigger(trigger: BotTrigger): WebhookFormState {
  const spec = trigger.spec as BotWebhookSpec;
  return {
    scheme: spec.verification.scheme,
    secret: spec.verification.scheme === "hmac-sha256" ? spec.verification.secret : "",
    header: spec.verification.scheme === "hmac-sha256" ? spec.verification.header : "",
    prefix: spec.verification.scheme === "hmac-sha256" ? (spec.verification.prefix ?? "") : "",
    preset: spec.preset === "github",
    routePolicy: trigger.route?.policy ?? "bot",
    routeKey: trigger.route?.policy === "perKey" ? (trigger.route.key ?? "") : "",
    filter: trigger.filter ?? "",
    whenBusy: trigger.deliver?.whenBusy ?? "queue",
    debounceSeconds: trigger.coalesce ? String(trigger.coalesce.debounceMs / 1000) : "",
    maxWaitSeconds: trigger.coalesce ? String(trigger.coalesce.maxWaitMs / 1000) : "",
    maxCount: trigger.coalesce ? String(trigger.coalesce.maxCount) : "",
  };
}

function webhookPayload(form: WebhookFormState) {
  const debounce = Number(form.debounceSeconds);
  const coalesceOn = form.debounceSeconds.trim() !== "" && debounce > 0;
  return {
    spec: {
      verification:
        form.scheme === "token"
          ? { scheme: "token" as const }
          : {
              scheme: "hmac-sha256" as const,
              secret: form.secret,
              header: form.header.trim() || "x-signature-256",
              ...(form.prefix ? { prefix: form.prefix } : {}),
            },
      preset: form.preset ? ("github" as const) : null,
    },
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
  };
}

function webhookFormProblem(form: WebhookFormState): string | null {
  if (form.scheme === "hmac-sha256" && form.secret.length < 8) {
    return "The HMAC secret needs at least 8 characters.";
  }
  if (form.debounceSeconds.trim() !== "") {
    const debounce = Number(form.debounceSeconds);
    if (!Number.isFinite(debounce) || debounce < 1) {
      return "Debounce must be at least 1 second.";
    }
    const maxWait = Number(form.maxWaitSeconds.trim() || form.debounceSeconds);
    if (!Number.isFinite(maxWait) || maxWait < debounce) {
      return "Max wait must be at least the debounce.";
    }
  }
  return null;
}

function WebhookFields({
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
            <FieldLabel htmlFor="webhook-secret">Secret</FieldLabel>
            <Input
              id="webhook-secret"
              value={form.secret}
              onChange={(event) => setForm({ ...form, secret: event.target.value })}
              type="password"
              autoComplete="off"
            />
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
      <Field>
        <FieldLabel>Sessions</FieldLabel>
        <Select
          value={form.routePolicy}
          onValueChange={(value) => value && setForm({ ...form, routePolicy: value as WebhookFormState["routePolicy"] })}
        >
          <SelectTrigger>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="bot">Deliver to the main session</SelectItem>
            <SelectItem value="perKey">One session per key</SelectItem>
            <SelectItem value="perEvent">One session per event</SelectItem>
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
            placeholder="data.issue.number"
            className="font-mono"
          />
          <FieldDescription>
            CEL over event, data, and headers. GitHub triggers default to the PR or issue number.
          </FieldDescription>
        </Field>
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
            min={1}
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
            min={1}
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
            value && setForm({ ...form, whenBusy: value as WebhookFormState["whenBusy"] })
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

function AddTriggerDialog({
  botId,
  open,
  onOpenChange,
}: {
  botId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [kind, setKind] = useState<"schedule" | "webhook" | null>(null);
  const [name, setName] = useState("");
  const [once, setOnce] = useState(false);
  const [at, setAt] = useState("");
  const [cron, setCron] = useState("");
  const [timezone, setTimezone] = useState("UTC");
  const [summary, setSummary] = useState("");
  const [webhook, setWebhook] = useState<WebhookFormState>(defaultWebhookForm);
  const [error, setError] = useState<string | null>(null);
  const nameInvalid = name.trim().length > 0 && !NAME_PATTERN.test(name.trim());
  const cronIssue = kind === "schedule" && !once ? cronProblem(cron) : null;
  const webhookIssue = kind === "webhook" ? webhookFormProblem(webhook) : null;
  const reset = () => {
    setKind(null);
    setName("");
    setOnce(false);
    setAt("");
    setCron("");
    setTimezone("UTC");
    setSummary("");
    setWebhook(defaultWebhookForm);
    setError(null);
  };
  const changeOpen = (next: boolean) => {
    if (!next) reset();
    onOpenChange(next);
  };
  const create = useMutation({
    mutationFn: () => {
      if (!kind) throw new Error("Choose a trigger type.");
      return api(
        "POST",
        `/api/v1/bots/${botId}/triggers`,
        kind === "schedule"
          ? {
              name: name.trim(),
              kind,
              spec: once
                ? { at: new Date(at).toISOString(), timezone: "UTC", summary: summary.trim() }
                : { cron: cron.trim(), timezone: timezone.trim() || "UTC", summary: summary.trim() },
            }
          : { name: name.trim(), kind, ...webhookPayload(webhook) },
      );
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["bot-triggers", botId] });
      reset();
      onOpenChange(false);
    },
    onError: (err) => setError(err.message),
  });
  const incomplete =
    kind === null ||
    !name.trim() ||
    nameInvalid ||
    (kind === "schedule"
      ? (once ? !at || Number.isNaN(new Date(at).getTime()) : !cron.trim() || cronIssue !== null) ||
        !summary.trim()
      : webhookIssue !== null);

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogContent
        className={kind
          ? "h-[min(92dvh,900px)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 p-0 sm:max-w-xl"
          : "max-h-[92dvh] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 p-0 sm:max-w-xl"}
      >
        <DialogHeader className="border-b p-6 pr-14">
          <DialogTitle>{kind ? `Add ${kind}` : "Add trigger"}</DialogTitle>
          <DialogDescription>
            {kind === "schedule"
              ? "Run the bot on a recurring cron or once at a specific time."
              : kind === "webhook"
                ? "Give the bot a protected ingest URL for external events."
                : "Choose how this bot should receive events."}
          </DialogDescription>
        </DialogHeader>
        {!kind ? (
          <>
            <div className="grid min-h-0 content-start gap-3 overflow-y-auto p-6 sm:grid-cols-2">
              <TriggerKindChoice
                icon={<CalendarClock className="size-5" />}
                title="Schedule"
                description="Run on a recurring cron or once at a specific time."
                onClick={() => setKind("schedule")}
              />
              <TriggerKindChoice
                icon={<Webhook className="size-5" />}
                title="Webhook"
                description="Receive token-protected or signed events from external systems."
                onClick={() => setKind("webhook")}
              />
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
                  autoFocus
                />
              </Field>
              {nameInvalid && (
                <p className="-mt-2 text-xs text-destructive">
                  Use lowercase letters, numbers, and dashes, starting with a letter or number.
                </p>
              )}
              {kind === "schedule" ? (
                <>
                  <Field>
                    <FieldLabel>When</FieldLabel>
                    <Select value={once ? "once" : "cron"} onValueChange={(value) => setOnce(value === "once")}>
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="cron">On a recurring cron</SelectItem>
                        <SelectItem value="once">Once, at a specific time</SelectItem>
                      </SelectContent>
                    </Select>
                  </Field>
                  {once ? (
                    <Field>
                      <FieldLabel htmlFor="trigger-at">Fire at</FieldLabel>
                      <Input
                        id="trigger-at"
                        type="datetime-local"
                        value={at}
                        onChange={(event) => setAt(event.target.value)}
                      />
                      <FieldDescription>
                        Local time; the trigger disables itself after firing once.
                      </FieldDescription>
                    </Field>
                  ) : (
                    <>
                      <div className="grid gap-3 sm:grid-cols-[1fr_10rem]">
                        <Field>
                          <FieldLabel htmlFor="trigger-cron">Cron</FieldLabel>
                          <Input
                            id="trigger-cron"
                            value={cron}
                            onChange={(event) => setCron(event.target.value)}
                            placeholder="0 8 * * 1-5"
                            className="font-mono"
                            aria-invalid={cronIssue !== null || undefined}
                          />
                        </Field>
                        <Field>
                          <FieldLabel htmlFor="trigger-timezone">Timezone</FieldLabel>
                          <Input
                            id="trigger-timezone"
                            value={timezone}
                            onChange={(event) => setTimezone(event.target.value)}
                            placeholder="UTC"
                          />
                        </Field>
                      </div>
                      <CronBuilder value={cron} onChange={setCron} />
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
                    <FieldLabel htmlFor="trigger-summary">Task</FieldLabel>
                    <Textarea
                      id="trigger-summary"
                      value={summary}
                      onChange={(event) => setSummary(event.target.value)}
                      rows={4}
                      placeholder="What should the bot do each time this fires?"
                    />
                  </Field>
                </>
              ) : (
                <WebhookFields form={webhook} setForm={setWebhook} />
              )}
            </div>
            <div className="grid gap-2 border-t p-4">
              {webhookIssue && <p className="text-xs text-destructive">{webhookIssue}</p>}
              {error && <p className="text-sm text-destructive">{error}</p>}
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => setKind(null)}>
                  <ArrowLeft data-icon="inline-start" /> Back
                </Button>
                <Button type="submit" disabled={create.isPending || incomplete}>
                  {create.isPending ? "Adding…" : `Add ${kind}`}
                </Button>
              </DialogFooter>
            </div>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}

function TriggerKindChoice({
  icon,
  title,
  description,
  onClick,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-start gap-4 rounded-xl border p-4 text-left transition-colors hover:bg-muted/40 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none"
    >
      <span className="rounded-lg bg-muted p-2 text-foreground">{icon}</span>
      <span className="grid gap-1">
        <span className="font-medium">{title}</span>
        <span className="text-xs text-muted-foreground">{description}</span>
      </span>
    </button>
  );
}

function EditTriggerDialog({
  botId,
  trigger,
  open,
  onOpenChange,
}: {
  botId: string;
  trigger: BotTrigger;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const scheduleSpec = trigger.kind === "schedule" ? (trigger.spec as BotScheduleSpec) : null;
  const oneShotAt = scheduleSpec?.at ?? null;
  const [cron, setCron] = useState(scheduleSpec?.cron ?? "");
  const [timezone, setTimezone] = useState(scheduleSpec?.timezone ?? "UTC");
  const [summary, setSummary] = useState(scheduleSpec?.summary ?? "");
  const [webhook, setWebhook] = useState<WebhookFormState>(() =>
    trigger.kind === "webhook" ? webhookFormFromTrigger(trigger) : defaultWebhookForm,
  );
  const [error, setError] = useState<string | null>(null);
  const cronIssue = trigger.kind === "schedule" && !oneShotAt ? cronProblem(cron) : null;
  const webhookIssue = trigger.kind === "webhook" ? webhookFormProblem(webhook) : null;
  const save = useMutation({
    mutationFn: () =>
      api(
        "PATCH",
        `/api/v1/bots/${botId}/triggers/${trigger.id}`,
        trigger.kind === "schedule"
          ? {
              spec: oneShotAt
                ? { at: oneShotAt, timezone: "UTC", summary: summary.trim() }
                : { cron: cron.trim(), timezone: timezone.trim() || "UTC", summary: summary.trim() },
            }
          : webhookPayload(webhook),
      ),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["bot-triggers", botId] });
      setError(null);
      onOpenChange(false);
    },
    onError: (err) => setError(err.message),
  });
  const incomplete =
    trigger.kind === "schedule"
      ? (!oneShotAt && (!cron.trim() || cronIssue !== null)) || !summary.trim()
      : webhookIssue !== null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="h-[min(92dvh,900px)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 p-0 sm:max-w-xl">
        <DialogHeader className="border-b p-6 pr-14">
          <DialogTitle>Edit {trigger.kind === "schedule" ? "schedule" : "webhook"}</DialogTitle>
          <DialogDescription>
            {trigger.kind === "schedule"
              ? "Changes reconcile to the Temporal Schedule immediately; the next fire uses them."
              : "The ingest URL keeps its token; verification and routing changes apply to the next delivery."}
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
            {trigger.kind === "schedule" ? (
              <>
                {oneShotAt && (
                  <p className="text-xs text-muted-foreground">
                    One-shot trigger at {new Date(oneShotAt).toLocaleString()}; only the task text is editable.
                  </p>
                )}
                <div className={oneShotAt ? "hidden" : "grid gap-3 sm:grid-cols-[1fr_10rem]"}>
                  <Field>
                    <FieldLabel htmlFor="edit-trigger-cron">Cron</FieldLabel>
                    <Input
                      id="edit-trigger-cron"
                      value={cron}
                      onChange={(event) => setCron(event.target.value)}
                      className="font-mono"
                      aria-invalid={cronIssue !== null || undefined}
                      autoFocus
                    />
                  </Field>
                  <Field>
                    <FieldLabel htmlFor="edit-trigger-timezone">Timezone</FieldLabel>
                    <Input
                      id="edit-trigger-timezone"
                      value={timezone}
                      onChange={(event) => setTimezone(event.target.value)}
                    />
                  </Field>
                </div>
                {!oneShotAt && <CronBuilder value={cron} onChange={setCron} />}
                {cronIssue ? (
                  <p className="-mt-2 text-xs text-destructive">{cronIssue}</p>
                ) : (
                  <p className="-mt-2 text-xs text-muted-foreground">
                    5 fields: minute, hour, day, month, weekday — or a macro like @daily.
                  </p>
                )}
                <Field>
                  <FieldLabel htmlFor="edit-trigger-summary">Task</FieldLabel>
                  <Textarea
                    id="edit-trigger-summary"
                    value={summary}
                    onChange={(event) => setSummary(event.target.value)}
                    rows={4}
                  />
                </Field>
              </>
            ) : (
              <WebhookFields form={webhook} setForm={setWebhook} />
            )}
          </div>
          <div className="grid gap-2 border-t p-4">
            {webhookIssue && <p className="text-xs text-destructive">{webhookIssue}</p>}
            {error && <p className="text-sm text-destructive">{error}</p>}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={save.isPending || incomplete}>
                {save.isPending ? "Saving…" : "Save"}
              </Button>
            </DialogFooter>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
