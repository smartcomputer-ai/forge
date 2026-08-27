import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type Bot, type ProfileSummary } from "@/api";
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
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";

export function BotSettingsDialog({
  universeId,
  bot,
  open,
  onOpenChange,
  onDeleted,
}: {
  universeId: string;
  bot: Bot;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Called after a successful delete, once the record is gone. */
  onDeleted?: () => void;
}) {
  const queryClient = useQueryClient();
  const closed = bot.closedAt !== null;
  const refreshBot = async (updated: Bot | null) => {
    if (updated) queryClient.setQueryData(["bot", universeId, bot.botId], { bot: updated });
    await queryClient.invalidateQueries({ queryKey: ["bots", bot.universeId] });
    await queryClient.invalidateQueries({ queryKey: ["bot-state", universeId, bot.botId] });
  };
  const close = useMutation({
    mutationFn: () =>
      api<{ bot: Bot; completed: boolean }>(
        "POST",
        `/api/v1/universes/${universeId}/bots/${bot.botId}/close`,
      ),
    onSuccess: async ({ bot: updated }) => {
      await refreshBot(updated);
      setError(null);
      onOpenChange(false);
    },
    onError: (err) => setError(err.message),
  });
  const remove = useMutation({
    mutationFn: () =>
      api<{ deleted: boolean }>("DELETE", `/api/v1/universes/${universeId}/bots/${bot.botId}`),
    onSuccess: async () => {
      queryClient.removeQueries({ queryKey: ["bot", universeId, bot.botId] });
      await queryClient.invalidateQueries({ queryKey: ["bots", bot.universeId] });
      setError(null);
      onOpenChange(false);
      onDeleted?.();
    },
    onError: (err) => setError(err.message),
  });
  const profiles = useQuery({
    queryKey: ["profiles", universeId],
    queryFn: () => api<ProfileSummary[]>("GET", `/api/v1/universes/${universeId}/profiles`),
    enabled: open,
  });
  const [displayName, setDisplayName] = useState(bot.displayName ?? "");
  const [description, setDescription] = useState(bot.description ?? "");
  const [profileId, setProfileId] = useState(bot.profileId);
  const [brief, setBrief] = useState(bot.brief ?? "");
  const [runsPerDay, setRunsPerDay] = useState(bot.runsPerDay?.toString() ?? "");
  const [breakerFires, setBreakerFires] = useState(bot.breaker?.fires.toString() ?? "");
  const [breakerWindow, setBreakerWindow] = useState(
    bot.breaker ? String(Math.round(bot.breaker.windowMs / 60_000)) : "",
  );
  const [routedTtlHours, setRoutedTtlHours] = useState(
    bot.routedSessionTtlMs ? String(Math.round(bot.routedSessionTtlMs / 3_600_000)) : "",
  );
  const [selfConfig, setSelfConfig] = useState(bot.selfConfig);
  const [emit, setEmit] = useState(bot.emit);
  const [enabled, setEnabled] = useState(bot.enabled);
  const [error, setError] = useState<string | null>(null);
  const save = useMutation({
    mutationFn: () =>
      api<{ bot: Bot }>("PATCH", `/api/v1/universes/${universeId}/bots/${bot.botId}`, {
        displayName: displayName.trim() ? displayName.trim() : null,
        description: description.trim() ? description.trim() : null,
        profileId,
        brief: brief.trim() ? brief.trim() : null,
        runsPerDay: runsPerDay.trim() ? Number(runsPerDay) : null,
        breaker: breakerFires.trim()
          ? {
              fires: Number(breakerFires),
              windowMs: Math.round(Number(breakerWindow.trim() || "10") * 60_000),
            }
          : null,
        routedSessionTtlMs: routedTtlHours.trim()
          ? Math.round(Number(routedTtlHours) * 3_600_000)
          : null,
        selfConfig,
        emit,
        enabled,
      }),
    onSuccess: async ({ bot: updated }) => {
      queryClient.setQueryData(["bot", universeId, bot.botId], { bot: updated });
      await queryClient.invalidateQueries({ queryKey: ["bots", updated.universeId] });
      await queryClient.invalidateQueries({ queryKey: ["bot-state", universeId, bot.botId] });
      setError(null);
      onOpenChange(false);
    },
    onError: (err) => setError(err.message),
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="h-[min(92dvh,900px)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 p-0 sm:max-w-xl">
        <DialogHeader className="border-b p-6 pr-14">
          <DialogTitle>Bot configuration</DialogTitle>
          <DialogDescription>
            Changes are applied to the bot's session at its next idle boundary.
          </DialogDescription>
        </DialogHeader>
        <div className="grid min-h-0 content-start gap-4 overflow-y-auto p-6">
          <Field>
            <FieldLabel htmlFor="bot-settings-display-name">Display name</FieldLabel>
            <Input
              id="bot-settings-display-name"
              value={displayName}
              onChange={(event) => setDisplayName(event.target.value)}
              placeholder={bot.botId}
            />
            <FieldDescription>
              Label only. The bot id <code className="font-mono">{bot.botId}</code> is what other
              bots, briefs, and URLs reference and cannot change.
            </FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="bot-settings-description">Description</FieldLabel>
            <Input
              id="bot-settings-description"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              placeholder="One line other bots read when deciding whether to address this bot."
            />
          </Field>
          <Field>
            <FieldLabel>Profile</FieldLabel>
            <Select value={profileId} onValueChange={(value) => value && setProfileId(value)}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {!profiles.data?.some((profile) => profile.profileId === bot.profileId) && (
                  <SelectItem value={bot.profileId}>{bot.profileId}</SelectItem>
                )}
                {profiles.data?.map((profile) => (
                  <SelectItem key={profile.profileId} value={profile.profileId}>
                    {profile.displayName ?? profile.profileId}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
          <Field>
            <FieldLabel htmlFor="bot-settings-brief">Brief</FieldLabel>
            <Textarea
              id="bot-settings-brief"
              value={brief}
              onChange={(event) => setBrief(event.target.value)}
              rows={4}
              placeholder="Standing instructions for this bot, appended to the profile."
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="bot-settings-runs">Runs per day</FieldLabel>
            <Input
              id="bot-settings-runs"
              type="number"
              min={1}
              value={runsPerDay}
              onChange={(event) => setRunsPerDay(event.target.value)}
              placeholder="Unlimited"
            />
            <FieldDescription>Budget: events beyond the cap wait for the next UTC day.</FieldDescription>
          </Field>
          <div className="grid gap-3 sm:grid-cols-2">
            <Field>
              <FieldLabel htmlFor="bot-breaker-fires">Breaker: events</FieldLabel>
              <Input
                id="bot-breaker-fires"
                type="number"
                min={1}
                value={breakerFires}
                onChange={(event) => setBreakerFires(event.target.value)}
                placeholder="Off"
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="bot-breaker-window">per window (min)</FieldLabel>
              <Input
                id="bot-breaker-window"
                type="number"
                min={1}
                value={breakerWindow}
                onChange={(event) => setBreakerWindow(event.target.value)}
                placeholder="10"
              />
            </Field>
          </div>
          <p className="-mt-2 text-xs text-muted-foreground">
            Flood breaker: a trigger exceeding this rate is disabled until re-enabled by hand.
          </p>
          <Field>
            <FieldLabel htmlFor="bot-routed-ttl">Close routed sessions after (hours)</FieldLabel>
            <Input
              id="bot-routed-ttl"
              type="number"
              min={1}
              value={routedTtlHours}
              onChange={(event) => setRoutedTtlHours(event.target.value)}
              placeholder="Keep forever"
            />
            <FieldDescription>
              Per-key and per-event sessions idle this long are closed; a later event for the
              same key opens a fresh session.
            </FieldDescription>
          </Field>
          <div className="flex items-center justify-between rounded-md border p-3">
            <Label htmlFor="bot-settings-self-config" className="text-sm">
              Self-configuration
              <span className="block text-xs font-normal text-muted-foreground">
                Lets the bot create and delete its own triggers and rewrite its brief. Off: it
                can only inspect them.
              </span>
            </Label>
            <Switch
              id="bot-settings-self-config"
              checked={selfConfig}
              onCheckedChange={setSelfConfig}
            />
          </div>
          <div className="flex items-center justify-between rounded-md border p-3">
            <Label htmlFor="bot-settings-self-emit" className="text-sm">
              Send events to other bots
              <span className="block text-xs font-normal text-muted-foreground">
                Lets this bot discover eligible receivers and address them with bot_emit. It may
                also post events to itself. Rate-capped to break feedback loops.
              </span>
            </Label>
            <Switch id="bot-settings-self-emit" checked={emit} onCheckedChange={setEmit} />
          </div>
          {!closed && (
            <div className="flex items-center justify-between rounded-md border p-3">
              <Label htmlFor="bot-settings-enabled" className="text-sm">
                Enabled
                <span className="block text-xs font-normal text-muted-foreground">
                  Disabling pauses all schedules and stops event delivery; sessions and their
                  context stay, and the bot can be enabled again.
                </span>
              </Label>
              <Switch id="bot-settings-enabled" checked={enabled} onCheckedChange={setEnabled} />
            </div>
          )}
          <div className="grid gap-3 rounded-md border border-destructive/40 p-3">
            <p className="text-sm font-medium">Lifecycle</p>
            {closed ? (
              <p className="text-xs text-muted-foreground">
                Closed {new Date(bot.closedAt ?? "").toLocaleString()}. Sessions and schedules were
                released and events are refused; the record and its history stay until the bot is
                deleted.
              </p>
            ) : (
              <div className="flex items-start justify-between gap-3">
                <p className="text-xs text-muted-foreground">
                  Closing is final: in-flight runs are cancelled, every session is closed,
                  schedules are dropped, and new events are refused. The record, its event history,
                  the bot id, and its environment stay.
                </p>
                <AlertDialog>
                  <AlertDialogTrigger render={<Button variant="outline" size="sm" disabled={close.isPending} />}>
                    {close.isPending ? "Closing…" : "Close bot"}
                  </AlertDialogTrigger>
                  <AlertDialogContent>
                    <AlertDialogHeader>
                      <AlertDialogTitle>Close {bot.displayName ?? bot.botId}?</AlertDialogTitle>
                      <AlertDialogDescription>
                        This cannot be undone. Pending events are archived, active runs are
                        cancelled, all sessions are closed, and webhooks and other bots are
                        refused from now on. The environment is left alone. To pause instead,
                        turn the bot off.
                      </AlertDialogDescription>
                    </AlertDialogHeader>
                    <AlertDialogFooter>
                      <AlertDialogCancel>Keep running</AlertDialogCancel>
                      <AlertDialogAction onClick={() => close.mutate()}>Close bot</AlertDialogAction>
                    </AlertDialogFooter>
                  </AlertDialogContent>
                </AlertDialog>
              </div>
            )}
            <div className="flex items-start justify-between gap-3">
              <p className="text-xs text-muted-foreground">
                Deleting erases the bot, its triggers, its event history, and its sessions, and
                frees the id{closed ? "." : " — it closes the bot first."} Environments are never
                deleted with a bot.
              </p>
              <AlertDialog>
                <AlertDialogTrigger render={<Button variant="destructive" size="sm" disabled={remove.isPending} />}>
                  {remove.isPending ? "Deleting…" : "Delete bot"}
                </AlertDialogTrigger>
                <AlertDialogContent>
                  <AlertDialogHeader>
                    <AlertDialogTitle>Delete {bot.displayName ?? bot.botId}?</AlertDialogTitle>
                    <AlertDialogDescription>
                      {closed
                        ? "The record, its event history, and its sessions are erased; the id becomes available again."
                        : "The bot is closed first (runs cancelled, sessions closed, events refused), then the record, its event history, and its sessions are erased and the id becomes available again."}
                    </AlertDialogDescription>
                  </AlertDialogHeader>
                  <AlertDialogFooter>
                    <AlertDialogCancel>Keep</AlertDialogCancel>
                    <AlertDialogAction onClick={() => remove.mutate()}>Delete bot</AlertDialogAction>
                  </AlertDialogFooter>
                </AlertDialogContent>
              </AlertDialog>
            </div>
          </div>
        </div>
        <div className="grid gap-2 border-t p-4">
          {error && <p className="text-sm text-destructive">{error}</p>}
          <DialogFooter>
            <Button variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button onClick={() => save.mutate()} disabled={save.isPending || !profileId || closed}>
              {save.isPending ? "Saving…" : "Save"}
            </Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}
