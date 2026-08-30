import { useMemo, useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import {
  api,
  type ChannelAccountInput,
  type ChannelsStatus,
  type OperatorChannelAccountListResponse,
  type OperatorChannelAccountView,
  type Universe,
} from "@/api";
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
import { useUniverses } from "@/lib/universes";

/// Channel accounts are universe resources in the core; this admin page
/// reads the deployment-wide operator listing and manages each row through
/// its universe's routes. Operator rows carry the CORE universe id, so
/// management calls map it to the platform universe that links to it.
function accountInputOf(row: OperatorChannelAccountView): ChannelAccountInput {
  return {
    accountId: row.accountId,
    provider: row.provider,
    providerAccountId: row.providerAccountId,
    displayName: row.displayName,
    credentialGrantId: row.credentialGrantId ?? null,
    enabled: row.enabled ?? true,
    ...(row.settings ? { settings: row.settings } : {}),
  };
}

export function AdminChannelsPage() {
  const queryClient = useQueryClient();
  const accounts = useQuery({
    queryKey: ["admin-channel-accounts"],
    queryFn: () => api<OperatorChannelAccountListResponse>("GET", "/api/v1/channel-accounts"),
  });
  const universes = useUniverses();
  const status = useQuery({
    queryKey: ["channels-status"],
    queryFn: () => api<ChannelsStatus>("GET", "/api/v1/status/channels"),
    refetchInterval: 10_000,
  });
  const universeByCoreId = useMemo(
    () => new Map((universes.data ?? []).map((universe) => [universe.lightspeedUniverseId, universe])),
    [universes.data],
  );
  const toggle = useMutation({
    mutationFn: (row: OperatorChannelAccountView) => {
      const universe = universeByCoreId.get(row.universeId);
      if (!universe) throw new Error(`No platform universe links to ${row.universeId}.`);
      return api("PUT", `/api/v1/universes/${universe.id}/channel-accounts/${row.accountId}`, {
        account: { ...accountInputOf(row), enabled: !(row.enabled ?? true) },
        expectedRevision: row.revision,
      });
    },
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ["admin-channel-accounts"] }),
  });

  const [createOpen, setCreateOpen] = useState(false);
  const rows = accounts.data?.accounts ?? [];

  return (
    <>
      <PageHeader
        title="Channels"
        description="Provider accounts across universes and live connector state for this deployment."
        actions={
          <Button onClick={() => setCreateOpen(true)}>
            <Plus data-icon="inline-start" />
            Add account
          </Button>
        }
      />
      <CreateAccountDialog open={createOpen} onOpenChange={setCreateOpen} universes={universes.data ?? []} />
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
              Chat triggers on bots target one stable provider account in their universe. Secret values stay
              outside Postgres.
            </CardDescription>
          </CardHeader>
          <CardContent className="grid gap-5">
            {accounts.isLoading && <LoadingNote />}
            {accounts.error && <p className="text-sm text-destructive">{accounts.error.message}</p>}
            {toggle.error && <p className="text-sm text-destructive">{toggle.error.message}</p>}
            {accounts.data && (
              <TableCard>
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Account</TableHead>
                      <TableHead>Provider</TableHead>
                      <TableHead>Universe</TableHead>
                      <TableHead>Status</TableHead>
                      <TableHead className="w-0" />
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {rows.map((account) => {
                      const universe = universeByCoreId.get(account.universeId);
                      return (
                        <TableRow key={`${account.universeId}/${account.accountId}`}>
                          <TableTitleCell
                            title={account.displayName}
                            subtitle={account.accountId}
                          />
                          <TableCell>
                            {account.provider}
                            <span className="block text-xs text-muted-foreground">{account.providerAccountId}</span>
                          </TableCell>
                          <TableCell className="text-muted-foreground">
                            {universe?.name ?? account.universeId}
                          </TableCell>
                          <TableCell>
                            <Badge variant={(account.enabled ?? true) ? "secondary" : "outline"}>
                              {(account.enabled ?? true) ? "enabled" : "disabled"}
                            </Badge>
                          </TableCell>
                          <TableActionsCell>
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled={toggle.isPending || !universe}
                              title={universe ? undefined : "No platform universe links to this account's universe."}
                              onClick={() => toggle.mutate(account)}
                            >
                              {(account.enabled ?? true) ? "Disable" : "Enable"}
                            </Button>
                          </TableActionsCell>
                        </TableRow>
                      );
                    })}
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
  universes,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  universes: Universe[];
}) {
  const queryClient = useQueryClient();
  const [universeId, setUniverseId] = useState("");
  const [provider, setProvider] = useState<"telegram" | "whatsapp">("telegram");
  const [accountId, setAccountId] = useState("");
  const [providerAccountId, setProviderAccountId] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [credentialGrantId, setCredentialGrantId] = useState("");
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setUniverseId("");
    setProvider("telegram");
    setAccountId("");
    setProviderAccountId("");
    setDisplayName("");
    setCredentialGrantId("");
    setError(null);
  };

  const create = useMutation({
    mutationFn: () =>
      api("POST", `/api/v1/universes/${universeId}/channel-accounts`, {
        account: {
          accountId: accountId.trim(),
          provider,
          providerAccountId: providerAccountId.trim(),
          displayName: displayName.trim(),
          ...(credentialGrantId.trim() ? { credentialGrantId: credentialGrantId.trim() } : {}),
        } satisfies ChannelAccountInput,
      }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["admin-channel-accounts"] });
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
            Registers a provider account in one universe. Its owners connect conversations to it
            with a chat trigger on one of their bots.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <Field>
              <FieldLabel htmlFor="channel-universe">Universe</FieldLabel>
              <Select value={universeId} onValueChange={(value) => value && setUniverseId(value)}>
                <SelectTrigger id="channel-universe">
                  <SelectValue placeholder="Select a universe" />
                </SelectTrigger>
                <SelectContent>
                  {universes.map((universe) => (
                    <SelectItem key={universe.id} value={universe.id}>
                      {universe.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
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
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <Field>
              <FieldLabel htmlFor="channel-account-id">Stable account id</FieldLabel>
              <Input
                id="channel-account-id"
                value={accountId}
                onChange={(e) => setAccountId(e.target.value)}
                required
                autoFocus
              />
              <FieldDescription>How triggers and the API name this account.</FieldDescription>
            </Field>
            <Field>
              <FieldLabel htmlFor="channel-provider-account-id">Provider account</FieldLabel>
              <Input
                id="channel-provider-account-id"
                value={providerAccountId}
                onChange={(e) => setProviderAccountId(e.target.value)}
                required
              />
              <FieldDescription>
                The Telegram bot username or id, or the WhatsApp phone number.
              </FieldDescription>
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
          <Field>
            <FieldLabel htmlFor="channel-credential-grant">Credential grant id</FieldLabel>
            <Input
              id="channel-credential-grant"
              value={credentialGrantId}
              onChange={(e) => setCredentialGrantId(e.target.value)}
              placeholder="optional"
              className="font-mono"
              autoComplete="off"
            />
            <FieldDescription>
              Retrievable grant holding the provider token (Telegram); WhatsApp accounts keep their
              session state on the connector instead. Secrets never pass through this form.
            </FieldDescription>
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
                !universeId ||
                !accountId.trim() ||
                !providerAccountId.trim() ||
                !displayName.trim()
              }
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
