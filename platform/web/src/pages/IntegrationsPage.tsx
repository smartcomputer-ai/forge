import { useState, type ChangeEvent, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, Plus, RefreshCw, Trash2 } from "lucide-react";
import {
  api,
  type GitHubApp,
  type GitHubInstallation,
  type GitHubIntegration,
  type SecretGrant,
} from "@/api";
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
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
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
import {
  IdText,
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
import {
  LoadingNote,
  PageHeader,
  SectionHeader,
  UniverseNotFound,
} from "@/components/page";
import { canManage, useActiveUniverse } from "@/lib/universes";

const DEFAULT_GITHUB_API_BASE_URL = "https://api.github.com";

export function IntegrationsPage({ admin }: { admin: boolean }) {
  const { universe, slug, isLoading } = useActiveUniverse();

  if (isLoading) {
    return <LoadingNote />;
  }
  if (!universe || !canManage(universe, admin)) {
    return <UniverseNotFound slug={slug} />;
  }

  return (
    <>
      <PageHeader
        title="Integrations"
        description="Third-party services connected to this universe. Each connection stores its credentials in Lightspeed and exposes them to sessions and tools without revealing their values."
      />
      <GitHubSection universeId={universe.id} />
    </>
  );
}

function GitHubSection({ universeId }: { universeId: string }) {
  const queryClient = useQueryClient();
  const [addOpen, setAddOpen] = useState(false);
  const integration = useQuery({
    queryKey: ["integrations", "github", universeId],
    queryFn: () =>
      api<GitHubIntegration>("GET", `/api/v1/universes/${universeId}/integrations/github`),
  });
  const invalidate = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: ["integrations", "github", universeId] }),
      queryClient.invalidateQueries({ queryKey: ["secrets", universeId] }),
    ]);
  const removeApp = useMutation({
    mutationFn: (providerId: string) =>
      api<GitHubApp>(
        "DELETE",
        `/api/v1/universes/${universeId}/integrations/github/apps/${encodeURIComponent(providerId)}`,
      ),
    onSuccess: () => void invalidate(),
  });

  const apps = integration.data?.apps ?? [];
  const grants = integration.data?.grants ?? [];

  return (
    <section>
      <SectionHeader
        title="GitHub"
        description="Bring your own GitHub App: register it with its private key, then grant access to the accounts where it is installed. Installation tokens are minted on demand and never stored."
        actions={
          <Button size="sm" onClick={() => setAddOpen(true)}>
            <Plus />
            Add GitHub App
          </Button>
        }
      />
      {integration.isLoading && <LoadingNote />}
      {integration.error && (
        <p className="text-sm text-destructive">{integration.error.message}</p>
      )}
      {removeApp.error && (
        <p className="mb-3 text-sm text-destructive">{removeApp.error.message}</p>
      )}
      {integration.data && apps.length === 0 && (
        <p className="rounded-xl border border-dashed p-5 text-sm text-muted-foreground">
          No GitHub Apps registered. Create one in GitHub (Settings → Developer settings → GitHub
          Apps), generate a private key, and add it here.
        </p>
      )}
      <div className="grid gap-4">
        {apps.map((app) => (
          <GitHubAppCard
            key={app.providerId}
            universeId={universeId}
            app={app}
            grants={grants.filter((grant) => grant.providerId === app.providerId)}
            onChanged={() => void invalidate()}
            onRemove={() => removeApp.mutate(app.providerId)}
            removing={removeApp.isPending && removeApp.variables === app.providerId}
          />
        ))}
      </div>
      <AddGitHubAppDialog
        universeId={universeId}
        open={addOpen}
        onOpenChange={setAddOpen}
        onCreated={() => void invalidate()}
      />
    </section>
  );
}

function GitHubAppCard({
  universeId,
  app,
  grants,
  onChanged,
  onRemove,
  removing,
}: {
  universeId: string;
  app: GitHubApp;
  grants: SecretGrant[];
  onChanged: () => void;
  onRemove: () => void;
  removing: boolean;
}) {
  const [installationsOpen, setInstallationsOpen] = useState(false);
  const activeGrants = grants.filter((grant) => grant.status !== "revoked");

  return (
    <div className="rounded-xl border">
      <div className="flex flex-wrap items-start justify-between gap-3 p-4">
        <div className="grid gap-1">
          <div className="flex items-center gap-2">
            <span className="font-medium">{app.displayName ?? `GitHub App ${app.config.appId}`}</span>
            <AppStatusBadge app={app} />
          </div>
          <div className="text-sm text-muted-foreground">
            App ID <span className="font-mono">{app.config.appId}</span>
            {app.config.apiBaseUrl !== DEFAULT_GITHUB_API_BASE_URL && (
              <>
                {" · "}
                <span className="font-mono">{app.config.apiBaseUrl}</span>
              </>
            )}
            {" · "}
            {activeGrants.length === 1
              ? "1 installation granted"
              : `${activeGrants.length} installations granted`}
          </div>
          <IdText className="text-xs text-muted-foreground">{app.providerId}</IdText>
        </div>
        <div className="flex items-center gap-1">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setInstallationsOpen((open) => !open)}
          >
            Installations
            <ChevronDown
              className={`size-3.5 transition-transform ${installationsOpen ? "rotate-180" : ""}`}
            />
          </Button>
          <AlertDialog>
            <AlertDialogTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="text-destructive"
                  aria-label={`Remove ${app.displayName ?? app.providerId}`}
                  disabled={removing}
                />
              }
            >
              <Trash2 />
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>Remove this GitHub App?</AlertDialogTitle>
                <AlertDialogDescription>
                  The stored private key is deleted and installation credentials of{" "}
                  <span className="font-mono text-xs">{app.providerId}</span> stop resolving
                  tokens. The App itself stays installed on GitHub.
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>Cancel</AlertDialogCancel>
                <AlertDialogAction
                  className="bg-destructive text-white hover:bg-destructive/90"
                  onClick={onRemove}
                >
                  Remove GitHub App
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        </div>
      </div>
      {installationsOpen && (
        <InstallationsPanel
          universeId={universeId}
          app={app}
          grants={grants}
          onChanged={onChanged}
        />
      )}
    </div>
  );
}

function InstallationsPanel({
  universeId,
  app,
  grants,
  onChanged,
}: {
  universeId: string;
  app: GitHubApp;
  grants: SecretGrant[];
  onChanged: () => void;
}) {
  const installations = useQuery({
    queryKey: ["integrations", "github", universeId, app.providerId, "installations"],
    queryFn: () =>
      api<GitHubInstallation[]>(
        "GET",
        `/api/v1/universes/${universeId}/integrations/github/apps/${encodeURIComponent(app.providerId)}/installations`,
      ),
  });
  const grant = useMutation({
    mutationFn: (installation: GitHubInstallation) =>
      api<SecretGrant>(
        "POST",
        `/api/v1/universes/${universeId}/integrations/github/apps/${encodeURIComponent(app.providerId)}/installations/${installation.installationId}/grant`,
        installation.accountLogin ? { displayName: `GitHub: ${installation.accountLogin}` } : {},
      ),
    onSuccess: onChanged,
  });

  return (
    <div className="border-t bg-muted/15 p-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm text-muted-foreground">
          Accounts where this App is installed, live from GitHub. Grant an installation to let
          this universe mint tokens for its repositories.
        </p>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => void installations.refetch()}
          disabled={installations.isFetching}
        >
          <RefreshCw className={installations.isFetching ? "animate-spin" : undefined} />
          Refresh
        </Button>
      </div>
      {installations.isLoading && <LoadingNote />}
      {installations.error && (
        <p className="text-sm text-destructive">{installations.error.message}</p>
      )}
      {grant.error && <p className="mb-2 text-sm text-destructive">{grant.error.message}</p>}
      {installations.data && installations.data.length === 0 && (
        <p className="text-sm text-muted-foreground">
          This App has no installations yet. Install it on a GitHub account or organization,
          then refresh.
        </p>
      )}
      {installations.data && installations.data.length > 0 && (
        <TableCard>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Account</TableHead>
                <TableHead>Repositories</TableHead>
                <TableHead>Permissions</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="w-0" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {installations.data.map((installation) => {
                const existing = installationGrantFor(grants, installation.installationId);
                const granting =
                  grant.isPending &&
                  grant.variables?.installationId === installation.installationId;
                return (
                  <TableRow key={installation.installationId}>
                    <TableTitleCell
                      title={installation.accountLogin ?? "Unknown account"}
                      subtitle={`installation ${installation.installationId}`}
                    />
                    <TableCell className="text-muted-foreground">
                      {installation.repositorySelection ?? "—"}
                    </TableCell>
                    <TableCell className="whitespace-normal text-muted-foreground">
                      <PermissionList permissions={installation.permissions} />
                    </TableCell>
                    <TableCell>
                      {existing ? (
                        <GrantStatusBadge status={existing.status} />
                      ) : (
                        <Badge variant="outline">not granted</Badge>
                      )}
                    </TableCell>
                    <TableActionsCell>
                      {(!existing || existing.status !== "active") && (
                        <Button
                          size="sm"
                          variant={existing ? "outline" : "default"}
                          disabled={granting}
                          onClick={() => grant.mutate(installation)}
                        >
                          {granting ? "Granting…" : existing ? "Re-grant" : "Grant"}
                        </Button>
                      )}
                    </TableActionsCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </TableCard>
      )}
    </div>
  );
}

function AddGitHubAppDialog({
  universeId,
  open,
  onOpenChange,
  onCreated,
}: {
  universeId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: () => void;
}) {
  const [displayName, setDisplayName] = useState("");
  const [appId, setAppId] = useState("");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [apiBaseUrl, setApiBaseUrl] = useState(DEFAULT_GITHUB_API_BASE_URL);
  const [providerId, setProviderId] = useState("");
  const [privateKey, setPrivateKey] = useState("");
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setDisplayName("");
    setAppId("");
    setAdvancedOpen(false);
    setApiBaseUrl(DEFAULT_GITHUB_API_BASE_URL);
    setProviderId("");
    setPrivateKey("");
    setError(null);
    create.reset();
  };

  const create = useMutation<GitHubApp, Error, void>({
    mutationFn: () =>
      api<GitHubApp>("POST", `/api/v1/universes/${universeId}/integrations/github/apps`, {
        appId: appId.trim(),
        privateKey,
        ...(displayName.trim() ? { displayName: displayName.trim() } : {}),
        ...(providerId.trim() ? { providerId: providerId.trim() } : {}),
        ...(apiBaseUrl.trim() && apiBaseUrl.trim() !== DEFAULT_GITHUB_API_BASE_URL
          ? { apiBaseUrl: apiBaseUrl.trim() }
          : {}),
      }),
    onSuccess: () => {
      onCreated();
      onOpenChange(false);
      reset();
    },
    onError: (reason) => setError(reason.message),
  });

  const readPemFile = (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) {
      return;
    }
    file
      .text()
      .then((text) => {
        setPrivateKey(text);
        setError(null);
      })
      .catch(() => setError("could not read the selected file"));
    event.target.value = "";
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const validation = validateGitHubAppForm({ appId, privateKey });
    if (validation) {
      setError(validation);
      return;
    }
    create.mutate();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        onOpenChange(nextOpen);
        if (!nextOpen) {
          reset();
        }
      }}
    >
      <DialogContent className="max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Add GitHub App</DialogTitle>
          <DialogDescription>
            Register a GitHub App you already created. The private key is sent once to
            Lightspeed, encrypted, and never returned. Afterwards, install the App on the GitHub
            accounts you want to reach and grant those installations here.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={submit} className="grid gap-4">
          <Field>
            <FieldLabel htmlFor="github-app-name">Display name</FieldLabel>
            <Input
              id="github-app-name"
              value={displayName}
              onChange={(event) => setDisplayName(event.target.value)}
              placeholder="Acme Lightspeed"
              autoFocus
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="github-app-id">App ID</FieldLabel>
            <Input
              id="github-app-id"
              value={appId}
              onChange={(event) => {
                setAppId(event.target.value);
                setError(null);
              }}
              inputMode="numeric"
              placeholder="123456"
              className="font-mono"
            />
            <FieldDescription>
              The numeric App ID shown on the App's settings page (not the client ID or slug).
            </FieldDescription>
          </Field>
          <Field>
            <FieldLabel htmlFor="github-app-key">Private key (PEM)</FieldLabel>
            <Textarea
              id="github-app-key"
              value={privateKey}
              onChange={(event) => {
                setPrivateKey(event.target.value);
                setError(null);
              }}
              autoComplete="off"
              spellCheck={false}
              rows={4}
              className="max-h-24 min-h-20 resize-y overflow-y-auto font-mono text-xs"
              placeholder="-----BEGIN RSA PRIVATE KEY-----"
            />
            <div className="flex items-center gap-2">
              <Input
                id="github-app-key-file"
                type="file"
                accept=".pem,application/x-pem-file,text/plain"
                onChange={readPemFile}
                className="max-w-xs"
                aria-label="Upload private key file"
              />
              <FieldDescription>Or upload the .pem file downloaded from GitHub.</FieldDescription>
            </div>
          </Field>
          <Collapsible open={advancedOpen} onOpenChange={setAdvancedOpen}>
            <CollapsibleTrigger className="inline-flex h-8 items-center gap-1.5 rounded-md px-2.5 text-sm font-medium text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50">
              Advanced options
              <ChevronDown
                className={`size-3.5 transition-transform ${advancedOpen ? "rotate-180" : ""}`}
              />
            </CollapsibleTrigger>
            <CollapsibleContent>
              <div className="mt-2 grid gap-4 rounded-lg border bg-muted/15 p-3">
                <Field>
                  <FieldLabel htmlFor="github-app-api-url">API base URL</FieldLabel>
                  <Input
                    id="github-app-api-url"
                    value={apiBaseUrl}
                    onChange={(event) => setApiBaseUrl(event.target.value)}
                    className="font-mono"
                  />
                  <FieldDescription>
                    Change only for GitHub Enterprise Server, e.g.{" "}
                    <span className="font-mono">https://github.example.com/api/v3</span>.
                  </FieldDescription>
                </Field>
                <Field>
                  <FieldLabel htmlFor="github-app-provider-id">Custom provider ID</FieldLabel>
                  <Input
                    id="github-app-provider-id"
                    value={providerId}
                    onChange={(event) => setProviderId(event.target.value)}
                    placeholder="github-app:<App ID> when blank"
                    className="font-mono"
                  />
                  <FieldDescription>
                    Stable identifier referenced by installation credentials and automation.
                  </FieldDescription>
                </Field>
              </div>
            </CollapsibleContent>
          </Collapsible>
          {error && <p className="text-sm text-destructive">{error}</p>}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                onOpenChange(false);
                reset();
              }}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={create.isPending}>
              {create.isPending ? "Encrypting…" : "Add GitHub App"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function AppStatusBadge({ app }: { app: GitHubApp }) {
  if (app.status === "active" && app.hasCredential) {
    return <Badge variant="secondary">active</Badge>;
  }
  if (app.status === "disabled") {
    return <Badge variant="outline">disabled</Badge>;
  }
  return (
    <Badge variant="outline" className="border-destructive/50 text-destructive">
      needs private key
    </Badge>
  );
}

function GrantStatusBadge({ status }: { status: SecretGrant["status"] }) {
  if (status === "active") {
    return <Badge variant="secondary">granted</Badge>;
  }
  if (status === "needsReauth" || status === "failed") {
    return (
      <Badge variant="outline" className="border-destructive/50 text-destructive">
        {status === "needsReauth" ? "needs reinstall" : status}
      </Badge>
    );
  }
  return <Badge variant="outline">revoked</Badge>;
}

/// The grant for one installation, preferring a live one when a revoked and
/// an active grant both exist (a re-grant after revocation).
export function installationGrantFor(
  grants: Pick<SecretGrant, "status" | "metadata">[],
  installationId: number,
): Pick<SecretGrant, "status" | "metadata"> | undefined {
  const matching = grants.filter(
    (grant) => Number(grant.metadata?.installation_id) === installationId,
  );
  return (
    matching.find((grant) => grant.status === "active") ??
    matching.find((grant) => grant.status !== "revoked") ??
    matching[0]
  );
}

function PermissionList({ permissions }: { permissions: Record<string, unknown> | undefined }) {
  const entries = permissionEntries(permissions);
  if (entries.length === 0) {
    return <>—</>;
  }
  return (
    <div className="flex max-w-md flex-wrap gap-1">
      {entries.map(([name, level]) => (
        <Badge key={name} variant="outline" className="font-mono text-[11px] font-normal">
          {name}: {level}
        </Badge>
      ))}
    </div>
  );
}

/// Sorted `[name, level]` pairs from GitHub's permission map; the raw map
/// stays in grant metadata.
export function permissionEntries(
  permissions: Record<string, unknown> | undefined,
): [string, string][] {
  if (!permissions) {
    return [];
  }
  return Object.entries(permissions)
    .filter((entry): entry is [string, string] => typeof entry[1] === "string" && entry[1] !== "")
    .sort(([a], [b]) => a.localeCompare(b));
}

/// Compact "contents: read, pull_requests: write" rendering.
export function permissionSummary(permissions: Record<string, unknown> | undefined): string {
  const entries = permissionEntries(permissions).map(([name, level]) => `${name}: ${level}`);
  return entries.length ? entries.join(", ") : "—";
}

export function validateGitHubAppForm(input: {
  appId: string;
  privateKey: string;
}): string | null {
  if (!/^[0-9]+$/.test(input.appId.trim())) {
    return "the App ID must be the numeric ID from the GitHub App settings page";
  }
  if (!input.privateKey.trim()) {
    return "a private key is required";
  }
  if (!/-----BEGIN [A-Z ]*PRIVATE KEY-----/.test(input.privateKey)) {
    return "the private key must be a PEM file (it starts with -----BEGIN … PRIVATE KEY-----)";
  }
  return null;
}
