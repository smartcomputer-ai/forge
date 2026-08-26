import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { api, type ChannelAccount, type ChannelsStatus } from "@/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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
import {
  Table,
  TableActionsCell,
  TableBody,
  TableCard,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableTitleCell,
} from "@/components/ui/table";
import { LoadingNote, PageHeader } from "@/components/page";

export function AdminChannelsPage() {
  const queryClient = useQueryClient();
  const accounts = useQuery({
    queryKey: ["channel-accounts"],
    queryFn: () => api<ChannelAccount[]>("GET", "/api/v1/channel-accounts"),
  });
  const status = useQuery({
    queryKey: ["channels-status"],
    queryFn: () => api<ChannelsStatus>("GET", "/api/v1/status/channels"),
    refetchInterval: 10_000,
  });
  const toggle = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      api<ChannelAccount>("PATCH", `/api/v1/channel-accounts/${id}`, { enabled }),
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ["channel-accounts"] }),
  });

  const [createOpen, setCreateOpen] = useState(false);

  return (
    <>
      <PageHeader
        title="Channels"
        description="Provider accounts and live connector state for this deployment."
        actions={
          <Button onClick={() => setCreateOpen(true)}>
            <Plus data-icon="inline-start" />
            Add account
          </Button>
        }
      />
      <CreateAccountDialog open={createOpen} onOpenChange={setCreateOpen} />
      <div className="grid gap-6">
        <Card>
          <CardHeader>
            <CardTitle>Connectors</CardTitle>
            <CardDescription>
              Refreshes every ten seconds from each connector's private health endpoint.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {status.isLoading && <LoadingNote />}
            {status.error && <p className="text-sm text-destructive">{status.error.message}</p>}
            {status.data && (
              <TableCard>
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Connector</TableHead>
                      <TableHead>State</TableHead>
                      <TableHead>Ingress</TableHead>
                      <TableHead>Activities</TableHead>
                      <TableHead>Last change</TableHead>
                      <TableHead>Last error</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {status.data.connectors.map((connector) => {
                      const health = connector.health;
                      return (
                        <TableRow key={connector.url}>
                          <TableCell className="font-medium">
                            {health ? `${health.provider} / ${health.accountId}` : connector.url}
                          </TableCell>
                          <TableCell>
                            <Badge variant={health?.state === "ready" ? "secondary" : "destructive"}>
                              {health?.state ?? "unreachable"}
                            </Badge>
                          </TableCell>
                          <TableCell>{yesNo(health?.ingressConnected)}</TableCell>
                          <TableCell>{yesNo(health?.activityWorkerReady)}</TableCell>
                          <TableCell className="text-muted-foreground">
                            {health ? new Date(health.changedAtMs).toLocaleString() : "—"}
                          </TableCell>
                          <TableCell className="max-w-sm text-sm text-muted-foreground">
                            {health?.lastError ?? connector.error ?? health?.detail ?? "—"}
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </TableCard>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Channel accounts</CardTitle>
            <CardDescription>
              Chat triggers on bots target one stable provider account. Secret values stay outside Postgres.
            </CardDescription>
          </CardHeader>
          <CardContent className="grid gap-5">
            {accounts.isLoading && <LoadingNote />}
            {accounts.error && <p className="text-sm text-destructive">{accounts.error.message}</p>}
            {accounts.data && (
              <TableCard>
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Account</TableHead>
                      <TableHead>Provider</TableHead>
                      <TableHead>Status</TableHead>
                      <TableHead className="w-0" />
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {accounts.data.map((account) => (
                      <TableRow key={account.id}>
                        <TableTitleCell
                          title={account.displayName}
                          subtitle={account.accountId}
                        />
                        <TableCell>{account.provider}</TableCell>
                        <TableCell>
                          <Badge variant={account.enabled ? "secondary" : "outline"}>
                            {account.enabled ? "enabled" : "disabled"}
                          </Badge>
                        </TableCell>
                        <TableActionsCell>
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={toggle.isPending}
                            onClick={() => toggle.mutate({ id: account.id, enabled: !account.enabled })}
                          >
                            {account.enabled ? "Disable" : "Enable"}
                          </Button>
                        </TableActionsCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </TableCard>
            )}
          </CardContent>
        </Card>
      </div>
    </>
  );
}

function CreateAccountDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [provider, setProvider] = useState<"telegram" | "whatsapp">("telegram");
  const [accountId, setAccountId] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [credentialRef, setCredentialRef] = useState("");
  const [stateRef, setStateRef] = useState("");
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setProvider("telegram");
    setAccountId("");
    setDisplayName("");
    setCredentialRef("");
    setStateRef("");
    setError(null);
  };

  const create = useMutation({
    mutationFn: () =>
      api<ChannelAccount>("POST", "/api/v1/channel-accounts", {
        provider,
        accountId: accountId.trim(),
        displayName: displayName.trim(),
        credentialRef: credentialRef.trim() || null,
        stateRef: stateRef.trim() || null,
        settings: {},
      }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["channel-accounts"] });
      onOpenChange(false);
      reset();
    },
    onError: (err) => setError(err.message),
  });
  const submit = (event: FormEvent) => {
    event.preventDefault();
    setError(null);
    create.mutate();
  };

  return (
    <Dialog open={open} onOpenChange={(next) => { onOpenChange(next); if (!next) reset(); }}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add channel account</DialogTitle>
          <DialogDescription>
            Registers a provider account for the Channels connectors. Universe owners
            connect conversations to it with a chat trigger on one of their bots.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <Field>
              <FieldLabel htmlFor="channel-provider">Provider</FieldLabel>
              <Select
                value={provider}
                onValueChange={(value) => setProvider(value as typeof provider)}
              >
                <SelectTrigger id="channel-provider">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="telegram">Telegram</SelectItem>
                  <SelectItem value="whatsapp">WhatsApp</SelectItem>
                </SelectContent>
              </Select>
            </Field>
            <Field>
              <FieldLabel htmlFor="channel-account-id">Stable account id</FieldLabel>
              <Input
                id="channel-account-id"
                value={accountId}
                onChange={(e) => setAccountId(e.target.value)}
                required
                autoFocus
              />
            </Field>
          </div>
          <Field>
            <FieldLabel htmlFor="channel-display-name">Display name</FieldLabel>
            <Input
              id="channel-display-name"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              required
            />
          </Field>
          <div className="grid gap-4 sm:grid-cols-2">
            <Field>
              <FieldLabel htmlFor="channel-credential-ref">Credential reference</FieldLabel>
              <Input
                id="channel-credential-ref"
                value={credentialRef}
                onChange={(e) => setCredentialRef(e.target.value)}
                placeholder="optional"
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="channel-state-ref">State reference</FieldLabel>
              <Input
                id="channel-state-ref"
                value={stateRef}
                onChange={(e) => setStateRef(e.target.value)}
                placeholder="optional"
              />
            </Field>
          </div>
          <FieldDescription>
            References are opaque handles resolved by the connector; secrets never
            pass through this form.
          </FieldDescription>
          {error && <p className="text-sm text-destructive">{error}</p>}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={create.isPending || !accountId.trim() || !displayName.trim()}
            >
              {create.isPending ? "Adding…" : "Add account"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function yesNo(value: boolean | undefined): string {
  return value === undefined ? "—" : value ? "yes" : "no";
}
