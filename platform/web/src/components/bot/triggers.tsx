import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CalendarClock, Pause, Pencil, Play, Plus, Trash2 } from "lucide-react";
import { api, type BotTrigger } from "@/api";
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
import { Textarea } from "@/components/ui/textarea";
import { PanelHeading } from "./status";

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
    <section className="grid gap-2">
      <div className="flex items-center gap-2">
        <PanelHeading title="Triggers" />
        {manage && (
          <Button variant="outline" size="xs" className="ml-auto" onClick={() => setAddOpen(true)}>
            <Plus data-icon="inline-start" /> Schedule
          </Button>
        )}
      </div>
      {triggers.data?.triggers.length === 0 && (
        <p className="text-xs text-muted-foreground">
          No triggers yet{manage ? " — add a schedule to make this bot proactive." : "."}
        </p>
      )}
      {triggers.error && <p className="text-xs text-destructive">{triggers.error.message}</p>}
      {triggers.data?.triggers.map((trigger) => (
        <div key={trigger.id} className="rounded-md border p-2 text-xs">
          <div className="flex items-center gap-2">
            <CalendarClock className="size-3.5 shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate font-medium">{trigger.name}</span>
            {!trigger.enabled && <Badge variant="outline">paused</Badge>}
            {manage && (
              <span className="flex items-center">
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
          <p className="mt-1 text-muted-foreground wrap-anywhere">
            <code>{trigger.cron}</code> · {trigger.timezone}
          </p>
          <p className="mt-1 line-clamp-2 text-muted-foreground wrap-anywhere">
            {trigger.summary}
          </p>
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
  const [cron, setCron] = useState(trigger.cron);
  const [timezone, setTimezone] = useState(trigger.timezone);
  const [summary, setSummary] = useState(trigger.summary);
  const [error, setError] = useState<string | null>(null);
  const cronIssue = cronProblem(cron);
  const save = useMutation({
    mutationFn: () =>
      api("PATCH", `/api/v1/bots/${botId}/triggers/${trigger.id}`, {
        cron: cron.trim(),
        timezone: timezone.trim() || "UTC",
        summary: summary.trim(),
      }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["bot-triggers", botId] });
      setError(null);
      onOpenChange(false);
    },
    onError: (err) => setError(err.message),
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Edit schedule</DialogTitle>
          <DialogDescription>
            Changes reconcile to the Temporal Schedule immediately; the next fire uses them.
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            setError(null);
            save.mutate();
          }}
          className="grid gap-4"
        >
          <div className="grid grid-cols-[1fr_10rem] gap-3">
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
          {error && <p className="text-sm text-destructive">{error}</p>}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={save.isPending || !cron.trim() || cronIssue !== null || !summary.trim()}
            >
              {save.isPending ? "Saving…" : "Save"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
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
  const [name, setName] = useState("");
  const [cron, setCron] = useState("");
  const [timezone, setTimezone] = useState("UTC");
  const [summary, setSummary] = useState("");
  const [error, setError] = useState<string | null>(null);
  const nameInvalid = name.trim().length > 0 && !NAME_PATTERN.test(name.trim());
  const cronIssue = cronProblem(cron);
  const create = useMutation({
    mutationFn: () =>
      api("POST", `/api/v1/bots/${botId}/triggers`, {
        name: name.trim(),
        kind: "schedule",
        cron: cron.trim(),
        timezone: timezone.trim() || "UTC",
        summary: summary.trim(),
      }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["bot-triggers", botId] });
      setName("");
      setCron("");
      setSummary("");
      setError(null);
      onOpenChange(false);
    },
    onError: (err) => setError(err.message),
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add schedule</DialogTitle>
          <DialogDescription>
            A Temporal Schedule fires this trigger; each fire delivers the summary below to the
            bot's session as its task.
          </DialogDescription>
        </DialogHeader>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            setError(null);
            create.mutate();
          }}
          className="grid gap-4"
        >
          <Field>
            <FieldLabel htmlFor="trigger-name">Name</FieldLabel>
            <Input
              id="trigger-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              aria-invalid={nameInvalid || undefined}
              autoFocus
            />
            {nameInvalid ? (
              <p className="text-xs text-destructive">
                Use lowercase letters, numbers, and dashes, starting with a letter or number.
              </p>
            ) : (
              <FieldDescription>Lowercase letters, numbers, and dashes.</FieldDescription>
            )}
          </Field>
          <div className="grid grid-cols-[1fr_10rem] gap-3">
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
          {cronIssue ? (
            <p className="-mt-2 text-xs text-destructive">{cronIssue}</p>
          ) : (
            <p className="-mt-2 text-xs text-muted-foreground">
              5 fields: minute, hour, day, month, weekday — or a macro like @daily.
            </p>
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
          {error && <p className="text-sm text-destructive">{error}</p>}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                create.isPending ||
                !name.trim() ||
                nameInvalid ||
                !cron.trim() ||
                cronIssue !== null ||
                !summary.trim()
              }
            >
              {create.isPending ? "Adding…" : "Add schedule"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
