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
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";

const ID_PATTERN = /^[a-z0-9][a-z0-9-]*$/;

/** A bot id is authored like a profile id: derived from the display name until edited, then immutable. */
function botIdFrom(displayName: string): string {
  return displayName
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
}

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
  const [displayName, setDisplayName] = useState("");
  const [botId, setBotId] = useState("");
  const [idTouched, setIdTouched] = useState(false);
  const [description, setDescription] = useState("");
  const [profileId, setProfileId] = useState("");
  const [brief, setBrief] = useState("");
  const [runsPerDay, setRunsPerDay] = useState("");
  const [acceptsBotEvents, setAcceptsBotEvents] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const idInvalid = botId.trim().length > 0 && !ID_PATTERN.test(botId.trim());
  const reset = () => {
    setDisplayName("");
    setBotId("");
    setIdTouched(false);
    setDescription("");
    setBrief("");
    setRunsPerDay("");
    setAcceptsBotEvents(false);
    setError(null);
  };
  const create = useMutation({
    mutationFn: () =>
      api<{ bot: Bot }>("POST", `/api/v1/universes/${universeId}/bots`, {
        botId: botId.trim(),
        ...(displayName.trim() ? { displayName: displayName.trim() } : {}),
        ...(description.trim() ? { description: description.trim() } : {}),
        profileId,
        ...(brief.trim() ? { brief: brief.trim() } : {}),
        ...(runsPerDay.trim() ? { runsPerDay: Number(runsPerDay) } : {}),
        ...(acceptsBotEvents ? { acceptsBotEvents: true } : {}),
      }),
    onSuccess: async ({ bot }) => {
      await queryClient.invalidateQueries({ queryKey: ["bots", universeId] });
      reset();
      onOpenChange(false);
      navigate(`/u/${slug}/bots/${bot.botId}`);
    },
    onError: (err) => setError(err.message),
  });

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="h-[min(92dvh,900px)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 p-0 sm:max-w-xl">
        <DialogHeader className="border-b p-6 pr-14">
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
          className="contents"
        >
          <div className="grid min-h-0 content-start gap-4 overflow-y-auto p-6">
            <Field>
              <FieldLabel htmlFor="bot-display-name">Display name</FieldLabel>
              <Input
                id="bot-display-name"
                value={displayName}
                onChange={(event) => {
                  setDisplayName(event.target.value);
                  if (!idTouched) setBotId(event.target.value ? botIdFrom(event.target.value) : "");
                }}
                placeholder="Triage"
                autoFocus
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="bot-id">Bot id</FieldLabel>
              <Input
                id="bot-id"
                value={botId}
                onChange={(event) => {
                  setBotId(event.target.value);
                  setIdTouched(event.target.value.length > 0);
                }}
                placeholder="triage"
                className="font-mono"
                aria-invalid={idInvalid || undefined}
              />
              {idInvalid ? (
                <p className="text-xs text-destructive">
                  Use lowercase letters, numbers, and dashes, starting with a letter or number.
                </p>
              ) : (
                <FieldDescription>
                  What other bots, briefs, and URLs reference — cannot be changed later.
                </FieldDescription>
              )}
            </Field>
            <Field>
              <FieldLabel htmlFor="bot-description">Description (optional)</FieldLabel>
              <Input
                id="bot-description"
                value={description}
                onChange={(event) => setDescription(event.target.value)}
                placeholder="One line other bots read when deciding whether to address this bot."
              />
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
            <div className="flex items-center justify-between rounded-md border p-3">
              <Label htmlFor="bot-accepts-bot-events" className="text-sm">
                Accept events from other bots
                <span className="block text-xs font-normal text-muted-foreground">
                  Creates an inbox trigger so other bots in this universe can address this one.
                  Narrow the senders on the trigger later.
                </span>
              </Label>
              <Switch
                id="bot-accepts-bot-events"
                checked={acceptsBotEvents}
                onCheckedChange={setAcceptsBotEvents}
              />
            </div>
          </div>
          <div className="grid gap-2 border-t p-4">
            {error && <p className="text-sm text-destructive">{error}</p>}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={create.isPending || !botId.trim() || idInvalid || !profileId}>
                {create.isPending ? "Creating…" : "Create"}
              </Button>
            </DialogFooter>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
