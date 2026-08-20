import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";

const NAME_PATTERN = /^[a-z0-9][a-z0-9-]*$/;

export function CreateBotDialog({
  universeId,
  slug,
  open,
  onOpenChange,
}: {
  universeId: string;
  slug: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const profiles = useQuery({
    queryKey: ["profiles", universeId],
    queryFn: () => api<ProfileSummary[]>("GET", `/api/v1/universes/${universeId}/profiles`),
    enabled: open,
  });
  const [name, setName] = useState("");
  const [profileId, setProfileId] = useState("");
  const [brief, setBrief] = useState("");
  const [runsPerDay, setRunsPerDay] = useState("");
  const [error, setError] = useState<string | null>(null);
  const nameInvalid = name.trim().length > 0 && !NAME_PATTERN.test(name.trim());
  const create = useMutation({
    mutationFn: () =>
      api<{ bot: Bot }>("POST", `/api/v1/universes/${universeId}/bots`, {
        name: name.trim(),
        profileId,
        ...(brief.trim() ? { brief: brief.trim() } : {}),
        ...(runsPerDay.trim() ? { runsPerDay: Number(runsPerDay) } : {}),
      }),
    onSuccess: async ({ bot }) => {
      await queryClient.invalidateQueries({ queryKey: ["bots", universeId] });
      setName("");
      setBrief("");
      setRunsPerDay("");
      setError(null);
      onOpenChange(false);
      navigate(`/u/${slug}/bots/${bot.id}`);
    },
    onError: (err) => setError(err.message),
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create bot</DialogTitle>
          <DialogDescription>
            A bot owns a persistent session and turns schedules and events into runs.
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
            <FieldLabel htmlFor="bot-name">Name</FieldLabel>
            <Input
              id="bot-name"
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
          <Field>
            <FieldLabel>Profile</FieldLabel>
            <Select value={profileId} onValueChange={(value) => value && setProfileId(value)}>
              <SelectTrigger>
                <SelectValue placeholder="Select a profile" />
              </SelectTrigger>
              <SelectContent>
                {profiles.data?.map((profile) => (
                  <SelectItem key={profile.profileId} value={profile.profileId}>
                    {profile.displayName ?? profile.profileId}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <FieldDescription>
              Capabilities, instructions, and environment intent come from the profile.
            </FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="bot-brief">Brief (optional)</FieldLabel>
            <Textarea
              id="bot-brief"
              value={brief}
              onChange={(event) => setBrief(event.target.value)}
              rows={4}
              placeholder="Standing instructions for this bot, appended to the profile."
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="bot-runs-per-day">Runs per day (optional)</FieldLabel>
            <Input
              id="bot-runs-per-day"
              type="number"
              min={1}
              value={runsPerDay}
              onChange={(event) => setRunsPerDay(event.target.value)}
              placeholder="Unlimited"
            />
            <FieldDescription>Budget: events beyond the cap wait for the next UTC day.</FieldDescription>
          </Field>
          {error && <p className="text-sm text-destructive">{error}</p>}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={create.isPending || !name.trim() || nameInvalid || !profileId}>
              {create.isPending ? "Creating…" : "Create"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
