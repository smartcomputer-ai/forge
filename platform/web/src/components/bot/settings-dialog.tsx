import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type Bot, type ProfileSummary } from "@/api";
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
}: {
  universeId: string;
  bot: Bot;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const profiles = useQuery({
    queryKey: ["profiles", universeId],
    queryFn: () => api<ProfileSummary[]>("GET", `/api/v1/universes/${universeId}/profiles`),
    enabled: open,
  });
  const [profileId, setProfileId] = useState(bot.profileId);
  const [brief, setBrief] = useState(bot.brief ?? "");
  const [runsPerDay, setRunsPerDay] = useState(bot.runsPerDay?.toString() ?? "");
  const [enabled, setEnabled] = useState(bot.enabled);
  const [error, setError] = useState<string | null>(null);
  const save = useMutation({
    mutationFn: () =>
      api<{ bot: Bot }>("PATCH", `/api/v1/bots/${bot.id}`, {
        profileId,
        brief: brief.trim() ? brief.trim() : null,
        runsPerDay: runsPerDay.trim() ? Number(runsPerDay) : null,
        enabled,
      }),
    onSuccess: async ({ bot: updated }) => {
      queryClient.setQueryData(["bot", bot.id], { bot: updated });
      await queryClient.invalidateQueries({ queryKey: ["bots", updated.universeId] });
      await queryClient.invalidateQueries({ queryKey: ["bot-state", bot.id] });
      setError(null);
      onOpenChange(false);
    },
    onError: (err) => setError(err.message),
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Bot configuration</DialogTitle>
          <DialogDescription>
            Changes are applied to the bot's session at its next idle boundary.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-4">
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
          <div className="flex items-center justify-between rounded-md border p-3">
            <Label htmlFor="bot-settings-enabled" className="text-sm">
              Enabled
              <span className="block text-xs font-normal text-muted-foreground">
                Disabling pauses all schedules and stops event delivery.
              </span>
            </Label>
            <Switch id="bot-settings-enabled" checked={enabled} onCheckedChange={setEnabled} />
          </div>
          {error && <p className="text-sm text-destructive">{error}</p>}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={() => save.mutate()} disabled={save.isPending || !profileId}>
            {save.isPending ? "Saving…" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
