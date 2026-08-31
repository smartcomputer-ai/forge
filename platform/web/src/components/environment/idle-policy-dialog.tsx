import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api, type Environment } from "@/api";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { IdlePolicyFields, idlePolicyIsMonotone, type IdlePolicy } from "./idle-policy-fields";

/**
 * Edit (or clear) the idle policy of an existing provisioned environment
 * (`environments/idle-policy/put`). The policy lives on the environment,
 * not on any profile or bot that uses it — this is the one place to set it
 * for a box that long-lived sessions share.
 */
export function EnvironmentIdlePolicyDialog({
  universeId,
  environment,
  open,
  onOpenChange,
  onChanged,
}: {
  universeId: string;
  environment: Environment;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onChanged?: () => void;
}) {
  const queryClient = useQueryClient();
  const [policy, setPolicy] = useState<IdlePolicy | undefined>(environment.idlePolicy ?? undefined);
  const [error, setError] = useState<string | null>(null);
  const save = useMutation({
    mutationFn: (next: IdlePolicy | undefined) =>
      api<Environment>(
        "PUT",
        `/api/v1/universes/${universeId}/environments/${encodeURIComponent(environment.environmentId)}/idle-policy`,
        { idlePolicy: next ?? null },
      ),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["environments", universeId] });
      onChanged?.();
      setError(null);
      onOpenChange(false);
    },
    onError: (err) => setError(err.message),
  });
  const label = environment.displayName ?? environment.environmentId;
  const valid = idlePolicyIsMonotone(policy);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Idle policy for {label}</DialogTitle>
          <DialogDescription>
            When nothing has used this environment for the given time — no session, bot, poller,
            or shell — it is powered down stage by stage, and wakes on the next use. Leave every
            stage empty to keep it running forever.
          </DialogDescription>
        </DialogHeader>
        <IdlePolicyFields
          value={policy}
          warning={
            policy === undefined
              ? "No stages: this environment never pauses, suspends, or stops while idle."
              : policy.closeAfterMs !== undefined && policy.closeAfterMs !== null
                ? "A close stage destroys the environment; profiles and bots pointing at it will fail until they are repointed."
                : undefined
          }
          onChange={setPolicy}
        />
        {error && <p className="text-sm text-destructive">{error}</p>}
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            variant="outline"
            disabled={save.isPending || environment.idlePolicy == null}
            onClick={() => save.mutate(undefined)}
          >
            Clear policy
          </Button>
          <Button disabled={save.isPending || !valid} onClick={() => save.mutate(policy)}>
            {save.isPending ? "Saving…" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
