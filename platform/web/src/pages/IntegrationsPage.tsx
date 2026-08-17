import { useState, type ChangeEvent, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, Copy, Plus, RefreshCw, ShieldOff, Trash2 } from "lucide-react";
import {
  api,
  type GitHubApp,
  type GitHubInstallation,
  type GitHubIntegration,
  type SecretGrant,
  type SubscriptionImportResult,
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
import {
  CODEX_AUTH_JSON_BOOTSTRAP,
  isCodexTokenSet,
  subscriptionAccountLabel,
  subscriptionBinding,
  subscriptionProviderOf,
  type SubscriptionProvider,
} from "@/lib/subscriptions";
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
      <div className="grid gap-10">
        <SubscriptionSection universeId={universe.id} provider="anthropic" />
        <SubscriptionSection universeId={universe.id} provider="openAi" />
        <GitHubSection universeId={universe.id} />
      </div>
    </>
  );
}

const SUBSCRIPTION_COPY: Record<
  SubscriptionProvider,
  {
    title: string;
    description: string;
    connect: string;
    dialogTitle: string;
    steps: string[];
    placeholder: string;
    inputRows: number;
    apiKeyNote: string;
  }
> = {
  anthropic: {
    title: "Anthropic",
    description:
      "Run Claude Code in environments on a Claude Pro, Max, Team, or Enterprise subscription. The token is injected as CLAUDE_CODE_OAUTH_TOKEN.",
    connect: "Connect Claude subscription",
    dialogTitle: "Connect Claude subscription",
    steps: [
      "On your own machine, run `claude setup-token` and complete the browser login.",
      "Copy the token it prints (it starts with sk-ant-oat) and paste it below.",
      "Bind the credential to environments as CLAUDE_CODE_OAUTH_TOKEN (suggested automatically).",
    ],
    placeholder: "sk-ant-oat01-…",
    inputRows: 3,
    apiKeyNote:
      "API keys for Lightspeed sessions are separate: add them under Secrets → Model provider credentials. Do not bind ANTHROPIC_API_KEY next to the subscription token; Claude Code would prefer the key.",
  },
  openAi: {
    title: "OpenAI",
    description:
      "Run Codex in environments on a ChatGPT subscription. Plus/Pro/Team: paste your local ~/.codex/auth.json; Enterprise: paste a Codex access token.",
    connect: "Connect ChatGPT subscription",
    dialogTitle: "Connect ChatGPT subscription",
    steps: [
      "Plus/Pro/Team: on your own machine, run `codex login`, then paste the contents of ~/.codex/auth.json below.",
      "ChatGPT Enterprise: create a Codex access token in your workspace and paste it instead.",
      "Bind the credential to environments as CODEX_AUTH_JSON (token set) or CODEX_ACCESS_TOKEN (Enterprise); the name is suggested automatically.",
    ],
    placeholder: '{ "auth_mode": "chatgpt", "tokens": { … } }  —  or a Codex access token',
    inputRows: 6,
    apiKeyNote:
      "OpenAI API keys for Lightspeed sessions are separate: add them under Secrets → Model provider credentials.",
  },
};

function SubscriptionSection({
  universeId,
  provider,
}: {
  universeId: string;
  provider: SubscriptionProvider;
}) {
  const queryClient = useQueryClient();
  const [connectOpen, setConnectOpen] = useState(false);
  const copy = SUBSCRIPTION_COPY[provider];
  const subscriptions = useQuery({
    queryKey: ["integrations", "subscriptions", universeId],
    queryFn: () =>
      api<SecretGrant[]>("GET", `/api/v1/universes/${universeId}/integrations/subscriptions`),
  });
  const invalidate = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: ["integrations", "subscriptions", universeId] }),
      queryClient.invalidateQueries({ queryKey: ["secrets", universeId] }),
    ]);
  const disconnect = useMutation({
    mutationFn: (grantId: string) =>
      api<SecretGrant>(
        "DELETE",
        `/api/v1/universes/${universeId}/integrations/subscriptions/${encodeURIComponent(grantId)}`,
      ),
    onSuccess: () => void invalidate(),
  });

  const grants = (subscriptions.data ?? [])
    .filter((grant) => subscriptionProviderOf(grant) === provider && grant.status !== "revoked")
    .sort((a, b) => b.createdAtMs - a.createdAtMs);

  return (
    <section>
      <SectionHeader
        title={copy.title}
        description={copy.description}
        actions={
          <Button size="sm" onClick={() => setConnectOpen(true)}>
            <Plus />
            {copy.connect}
          </Button>
        }
      />
      {subscriptions.isLoading && <LoadingNote />}
      {subscriptions.error && (
        <p className="text-sm text-destructive">{subscriptions.error.message}</p>
      )}
      {disconnect.error && (
        <p className="mb-3 text-sm text-destructive">{disconnect.error.message}</p>
      )}
      {subscriptions.data && grants.length === 0 && (
        <p className="rounded-xl border border-dashed p-5 text-sm text-muted-foreground">
          No subscription connected. {copy.apiKeyNote}
        </p>
      )}
      {grants.length > 0 && (
        <TableCard>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Credential</TableHead>
                <TableHead>Account</TableHead>
                <TableHead>Bind as</TableHead>
                <TableHead>Expires</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="w-0" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {grants.map((grant) => {
                const binding = subscriptionBinding(grant);
                return (
                  <TableRow key={grant.grantId}>
                    <TableTitleCell
                      title={grant.displayName ?? binding?.label ?? grant.grantId}
                      subtitle={grant.grantId}
                    />
                    <TableCell className="text-muted-foreground">
                      {subscriptionAccountLabel(grant) || "—"}
                    </TableCell>
                    <TableCell>
                      {binding ? <IdText>{binding.envName}</IdText> : "—"}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {formatExpiry(grant.expiresAtMs)}
                    </TableCell>
                    <TableCell>
                      <SubscriptionStatusBadge status={grant.status} />
                    </TableCell>
                    <TableActionsCell>
                      <AlertDialog>
                        <AlertDialogTrigger
                          render={
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              className="text-destructive"
                              aria-label={`Disconnect ${grant.displayName ?? grant.grantId}`}
                            />
                          }
                        >
                          <ShieldOff />
                        </AlertDialogTrigger>
                        <AlertDialogContent>
                          <AlertDialogHeader>
                            <AlertDialogTitle>Disconnect this subscription?</AlertDialogTitle>
                            <AlertDialogDescription>
                              Environments bound to{" "}
                              <span className="font-mono text-xs">{grant.grantId}</span> stop
                              receiving the credential on their next job. The subscription itself
                              is unaffected; revoke the token with the provider if it leaked.
                            </AlertDialogDescription>
                          </AlertDialogHeader>
                          <AlertDialogFooter>
                            <AlertDialogCancel>Cancel</AlertDialogCancel>
                            <AlertDialogAction
                              className="bg-destructive text-white hover:bg-destructive/90"
                              onClick={() => disconnect.mutate(grant.grantId)}
                            >
                              Disconnect
                            </AlertDialogAction>
                          </AlertDialogFooter>
                        </AlertDialogContent>
                      </AlertDialog>
                    </TableActionsCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </TableCard>
      )}
      {provider === "openAi" && grants.some(isCodexTokenSet) && <CodexBootstrapNote />}
      <ConnectSubscriptionDialog
        universeId={universeId}
        provider={provider}
        open={connectOpen}
        onOpenChange={setConnectOpen}
        onConnected={() => void invalidate()}
      />
    </section>
  );
}

function CodexBootstrapNote() {
  const [copied, setCopied] = useState(false);
  return (
    <div className="mt-3 rounded-xl border bg-muted/15 p-4 text-sm">
      <div className="mb-2 flex items-center justify-between gap-3">
        <p className="text-muted-foreground">
          Codex reads the token set from <span className="font-mono">$CODEX_HOME/auth.json</span>.
          Run this in the environment before Codex (image entrypoint or job pre-command):
        </p>
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            void navigator.clipboard?.writeText(CODEX_AUTH_JSON_BOOTSTRAP).then(() => {
              setCopied(true);
              window.setTimeout(() => setCopied(false), 1500);
            });
          }}
        >
          <Copy />
          {copied ? "Copied" : "Copy"}
        </Button>
      </div>
      <pre className="overflow-x-auto rounded-md bg-background p-3 font-mono text-xs">
        {CODEX_AUTH_JSON_BOOTSTRAP}
      </pre>
    </div>
  );
}

function ConnectSubscriptionDialog({
  universeId,
  provider,
  open,
  onOpenChange,
  onConnected,
}: {
  universeId: string;
  provider: SubscriptionProvider;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConnected: () => void;
}) {
  const copy = SUBSCRIPTION_COPY[provider];
  const [displayName, setDisplayName] = useState("");
  const [credential, setCredential] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<SubscriptionImportResult | null>(null);

  const reset = () => {
    setDisplayName("");
    setCredential("");
    setError(null);
    setResult(null);
    connect.reset();
  };

  const connect = useMutation<SubscriptionImportResult, Error, void>({
    mutationFn: () =>
      api<SubscriptionImportResult>(
        "POST",
        `/api/v1/universes/${universeId}/integrations/subscriptions`,
        {
          provider,
          credential,
          ...(displayName.trim() ? { displayName: displayName.trim() } : {}),
        },
      ),
    onSuccess: (imported) => {
      onConnected();
      setCredential("");
      setResult(imported);
    },
    onError: (reason) => setError(reason.message),
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!credential.trim()) {
      setError("paste the credential first");
      return;
    }
    connect.mutate();
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        onOpenChange(nextOpen);
        if (!nextOpen) reset();
      }}
    >
      <DialogContent className="max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{copy.dialogTitle}</DialogTitle>
          <DialogDescription>
            The credential is sent once to Lightspeed, encrypted, and never returned by an API.
          </DialogDescription>
        </DialogHeader>
        {result ? (
          <div className="grid gap-3 text-sm">
            <p>
              Connected as{" "}
              <span className="font-medium">
                {subscriptionAccountLabel(result.grant) || result.grant.displayName || result.grant.grantId}
              </span>
              .
            </p>
            <p className="text-muted-foreground">
              Bind it to environments as{" "}
              <span className="font-mono">{subscriptionBinding(result.grant)?.envName}</span>
              {result.shape === "codexTokenSet"
                ? "; the value is Codex auth.json content — add the bootstrap line shown on this page to your environment."
                : "."}
            </p>
            <DialogFooter>
              <Button onClick={() => onOpenChange(false)}>Done</Button>
            </DialogFooter>
          </div>
        ) : (
          <form onSubmit={submit} className="grid gap-4">
            <ol className="list-decimal space-y-1 pl-5 text-sm text-muted-foreground">
              {copy.steps.map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ol>
            <Field>
              <FieldLabel htmlFor={`sub-name-${provider}`}>Display name</FieldLabel>
              <Input
                id={`sub-name-${provider}`}
                value={displayName}
                onChange={(event) => setDisplayName(event.target.value)}
                placeholder={provider === "anthropic" ? "Lukas · Max" : "Lukas · ChatGPT Pro"}
              />
            </Field>
            <Field>
              <FieldLabel htmlFor={`sub-cred-${provider}`}>Credential</FieldLabel>
              <Textarea
                id={`sub-cred-${provider}`}
                value={credential}
                onChange={(event) => {
                  setCredential(event.target.value);
                  setError(null);
                }}
                autoComplete="off"
                spellCheck={false}
                rows={copy.inputRows}
                className="max-h-40 resize-y overflow-y-auto font-mono text-xs"
                placeholder={copy.placeholder}
                autoFocus
              />
            </Field>
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
              <Button type="submit" disabled={connect.isPending}>
                {connect.isPending ? "Encrypting…" : "Connect"}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}

function SubscriptionStatusBadge({ status }: { status: SecretGrant["status"] }) {
  if (status === "active") {
    return <Badge variant="secondary">connected</Badge>;
  }
  if (status === "needsReauth" || status === "failed") {
    return (
      <Badge variant="outline" className="border-destructive/50 text-destructive">
        {status === "needsReauth" ? "reconnect" : status}
      </Badge>
    );
  }
  return <Badge variant="outline">revoked</Badge>;
}

export function formatExpiry(expiresAtMs: number | null | undefined, nowMs = Date.now()): string {
  if (!expiresAtMs) return "—";
  const days = Math.floor((expiresAtMs - nowMs) / 86_400_000);
  if (days < 0) return "expired";
  if (days === 0) return "today";
  if (days < 45) return `in ${days} d`;
  return new Date(expiresAtMs).toISOString().slice(0, 10);
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
