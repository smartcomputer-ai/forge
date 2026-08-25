import { useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, CalendarClock, Check, Copy, Pause, Pencil, Play, Plus, RefreshCw, Terminal, Trash2, Webhook } from "lucide-react";
import {
  api,
  type BotPollSpec,
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

/** The bot profile's environment intent, as it affects exec pollers. */
export type BotEnvStatus =
  | { kind: "unknown" }
  | { kind: "none" }
  | { kind: "provision" }
  | { kind: "existing"; environmentId: string };

export function TriggersSection({
  botId,
  manage,
  env,
}: {
  botId: string;
  manage: boolean;
  env: BotEnvStatus;
}) {
  const queryClient = useQueryClient();
  const [addOpen, setAddOpen] = useState(false);
  const [editing, setEditing] = useState<BotTrigger | null>(null);
  const [pendingDelete, setPendingDelete] = useState<string | null>(null);
  const triggers = useQuery({
    queryKey: ["bot-triggers", botId],
    queryFn: () => api<{ triggers: BotTrigger[] }>("GET", `/api/v1/bots/${botId}/triggers`),
  });
  const invalidate = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: ["bot-triggers", botId] }),
    queryClient.invalidateQueries({ queryKey: ["bots"] }),
  ]);
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
          ) : trigger.kind === "poll" ? (
            <PollRowDetail trigger={trigger} />
          ) : (
            <WebhookRowDetail trigger={trigger} manage={manage} />
          )}
        </div>
      ))}
      {manage && <AddTriggerDialog botId={botId} env={env} open={addOpen} onOpenChange={setAddOpen} />}
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

interface DeliveryFormState {
  routePolicy: "bot" | "perKey" | "perEvent";
  routeKey: string;
  filter: string;
  whenBusy: "queue" | "steer" | "append";
  debounceSeconds: string;
  maxWaitSeconds: string;
  maxCount: string;
}

interface WebhookFormState extends DeliveryFormState {
  scheme: "token" | "hmac-sha256";
  grantId: string;
  header: string;
  prefix: string;
  preset: boolean;
}

interface PollFormState extends DeliveryFormState {
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

const defaultDeliveryForm: DeliveryFormState = {
  routePolicy: "bot",
  routeKey: "",
  filter: "",
  whenBusy: "queue",
  debounceSeconds: "",
  maxWaitSeconds: "",
  maxCount: "",
};

const defaultPollForm: PollFormState = {
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
    environmentId: spec.source.kind === "exec" ? spec.source.environmentId : "",
    argvText: spec.source.kind === "exec" ? spec.source.argv.join("\n") : "",
    cwd: spec.source.kind === "exec" ? (spec.source.cwd ?? "") : "",
    intervalMinutes: String(Math.round(spec.intervalMs / 60_000)),
    items: spec.items ?? "",
    dedupe: spec.cursor.kind,
    dedupeField: spec.cursor.kind === "idSet" ? spec.cursor.id : spec.cursor.field,
    routePolicy: trigger.route?.policy ?? "bot",
    routeKey: trigger.route?.policy === "perKey" ? (trigger.route.key ?? "") : "",
    filter: trigger.filter ?? "",
    whenBusy: trigger.deliver?.whenBusy ?? "queue",
    debounceSeconds: trigger.coalesce ? String(trigger.coalesce.debounceMs / 1000) : "",
    maxWaitSeconds: trigger.coalesce ? String(trigger.coalesce.maxWaitMs / 1000) : "",
    maxCount: trigger.coalesce ? String(trigger.coalesce.maxCount) : "",
  };
}

function deliveryPayload(form: DeliveryFormState) {
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
  };
}

function pollArgv(form: PollFormState): string[] {
  return form.argvText
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

function pollPayload(form: PollFormState) {
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
              environmentId: form.environmentId.trim(),
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

function pollFormProblem(form: PollFormState): string | null {
  if (form.sourceKind === "http") {
    if (!/^https?:\/\//.test(form.url.trim())) return "The poll URL must be http(s).";
  } else {
    if (!form.environmentId.trim()) return "Name the environment the command runs in.";
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

function deliveryFormProblem(form: DeliveryFormState): string | null {
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

const defaultWebhookForm: WebhookFormState = {
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

function webhookFormProblem(form: WebhookFormState): string | null {
  if (form.scheme === "hmac-sha256" && !form.grantId.trim()) {
    return "Choose a retrievable credential grant for HMAC verification.";
  }
  return deliveryFormProblem(form);
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

function DeliveryFields<T extends DeliveryFormState>({
  form,
  setForm,
}: {
  form: T;
  setForm: (next: T) => void;
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

function PollFields({
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
              placeholder="environment_…"
              className="font-mono"
            />
            <FieldDescription>
              The command runs as a one-shot job here with the environment's credentials; a
              sleeping environment wakes for the poll and idles back down after.
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

function AddTriggerDialog({
  botId,
  env,
  open,
  onOpenChange,
}: {
  botId: string;
  env: BotEnvStatus;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [kind, setKind] = useState<"schedule" | "webhook" | "poll" | null>(null);
  const [name, setName] = useState("");
  const [once, setOnce] = useState(false);
  const [at, setAt] = useState("");
  const [cron, setCron] = useState("");
  const [timezone, setTimezone] = useState("UTC");
  const [summary, setSummary] = useState("");
  const [webhook, setWebhook] = useState<WebhookFormState>(defaultWebhookForm);
  const [poll, setPoll] = useState<PollFormState>(defaultPollForm);
  const [error, setError] = useState<string | null>(null);
  const nameInvalid = name.trim().length > 0 && !NAME_PATTERN.test(name.trim());
  const cronIssue = kind === "schedule" && !once ? cronProblem(cron) : null;
  const webhookIssue = kind === "webhook" ? webhookFormProblem(webhook) : null;
  const pollIssue = kind === "poll" ? pollFormProblem(poll) : null;
  const reset = () => {
    setKind(null);
    setName("");
    setOnce(false);
    setAt("");
    setCron("");
    setTimezone("UTC");
    setSummary("");
    setWebhook(defaultWebhookForm);
    setPoll(defaultPollForm);
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
          : kind === "poll"
            ? { name: name.trim(), kind, ...pollPayload(poll) }
            : { name: name.trim(), kind, ...webhookPayload(webhook) },
      );
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["bot-triggers", botId] }),
        queryClient.invalidateQueries({ queryKey: ["bots"] }),
      ]);
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
      : kind === "poll"
        ? pollIssue !== null
        : webhookIssue !== null);

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
              ? poll.sourceKind === "exec"
                ? "Add command poll"
                : "Add HTTP/API poll"
              : kind
                ? `Add ${kind}`
                : "Add trigger"}
          </DialogTitle>
          <DialogDescription>
            {kind === "schedule"
              ? "Run the bot on a recurring cron or once at a specific time."
              : kind === "webhook"
                ? "Give the bot a protected ingest URL for external events."
                : kind === "poll"
                  ? "Fetch a source on an interval and wake the bot with new items."
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
              <TriggerKindChoice
                icon={<RefreshCw className="size-5" />}
                title="HTTP/API poll"
                description="Check an HTTP endpoint on an interval and deliver only new items."
                onClick={() => {
                  setPoll({ ...defaultPollForm, sourceKind: "http" });
                  setKind("poll");
                }}
              />
              <TriggerKindChoice
                icon={<Terminal className="size-5" />}
                title="Command poll"
                description="Run a command in the bot's environment on an interval; its JSON output is diffed."
                disabled={env.kind !== "existing"}
                disabledReason={
                  env.kind === "none"
                    ? "The bot's profile has no environment attached."
                    : env.kind === "provision"
                      ? "The profile provisions per-session environments; pollers need a stable existing environment."
                      : env.kind === "unknown"
                        ? "Checking the profile's environment…"
                        : undefined
                }
                onClick={() => {
                  setPoll({
                    ...defaultPollForm,
                    sourceKind: "exec",
                    environmentId: env.kind === "existing" ? env.environmentId : "",
                  });
                  setKind("poll");
                }}
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
              ) : kind === "poll" ? (
                <PollFields form={poll} setForm={setPoll} />
              ) : (
                <WebhookFields form={webhook} setForm={setWebhook} />
              )}
            </div>
            <div className="grid gap-2 border-t p-4">
              {(webhookIssue ?? pollIssue) && (
                <p className="text-xs text-destructive">{webhookIssue ?? pollIssue}</p>
              )}
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
  const [poll, setPoll] = useState<PollFormState>(() =>
    trigger.kind === "poll" ? pollFormFromTrigger(trigger) : defaultPollForm,
  );
  const [error, setError] = useState<string | null>(null);
  const cronIssue = trigger.kind === "schedule" && !oneShotAt ? cronProblem(cron) : null;
  const webhookIssue = trigger.kind === "webhook" ? webhookFormProblem(webhook) : null;
  const pollIssue = trigger.kind === "poll" ? pollFormProblem(poll) : null;
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
          : trigger.kind === "poll"
            ? pollPayload(poll)
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
      : trigger.kind === "poll"
        ? pollIssue !== null
        : webhookIssue !== null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="h-[min(92dvh,900px)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 p-0 sm:max-w-xl">
        <DialogHeader className="border-b p-6 pr-14">
          <DialogTitle>Edit {trigger.kind}</DialogTitle>
          <DialogDescription>
            {trigger.kind === "schedule"
              ? "Changes reconcile to the Temporal Schedule immediately; the next fire uses them."
              : trigger.kind === "poll"
                ? "Spec changes reset the cursor: the next fire re-baselines against the source."
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
            ) : trigger.kind === "poll" ? (
              <PollFields form={poll} setForm={setPoll} />
            ) : (
              <WebhookFields form={webhook} setForm={setWebhook} />
            )}
          </div>
          <div className="grid gap-2 border-t p-4">
            {(webhookIssue ?? pollIssue) && (
              <p className="text-xs text-destructive">{webhookIssue ?? pollIssue}</p>
            )}
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
