import { useEffect, useLayoutEffect, useRef, useState, type FormEvent } from "react";
import {
  type InfiniteData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { NavLink, useNavigate, useParams } from "react-router-dom";
import { Archive, ArrowLeft, Check, Copy, ListFilter, LoaderCircle, Plus, ShieldCheck, SlidersHorizontal, Trash2 } from "lucide-react";
import {
  api,
  type Environment,
  type InlineProfile,
  type ProfileDocument,
  type ProfileSource,
  type ProfileSummary,
  type SessionListPage,
  type SessionRunAccepted,
  SessionRunCancelled,
  SessionRunSteered,
  SessionRunView,
  type SessionSummary,
  type SessionView,
} from "@/api";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { BotFaceIcon } from "@/components/icons/bot";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
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
import { ProfileEnvironmentEditor } from "@/components/session/profile-environment-editor";
import { SessionConfigEditor } from "@/components/session/session-config-editor";
import { SessionSettingsDialog } from "@/components/session/session-settings-sheet";
import { SetupEditorSection } from "@/components/session/setup-editor-section";
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
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
  useMessageScroller,
  useMessageScrollerScrollable,
} from "@/components/ui/message-scroller";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { SessionComposer, type ComposerMode } from "@/components/session/composer";
import { Switch } from "@/components/ui/switch";
import {
  ActiveRunMarker,
  QueuedRunsBar,
  TranscriptEntryView,
  UserBand,
  type QueuedRunItem,
} from "@/components/session/transcript-view";
import { CenteredNote, LoadingNote, UniverseNotFound } from "@/components/page";
import { useSessionTail } from "@/lib/sessions/tail";
import {
  isTerminalToolStatus,
  runInProgress,
  type ActiveRun,
  type TranscriptEntry,
} from "@/lib/sessions/transcript";
import { useSessionConfigEditorOptions } from "@/lib/sessions/editor-options";
import { managedSessionOwnerLabel } from "@/lib/sessions/management";
import {
  hasSessionFeature,
  resourceFeatureDisableReasons,
  setupResourceFeatureError,
} from "@/lib/sessions/resource-features";
import { ProviderReadinessBanner } from "@/components/provider-readiness-banner";
import { canManage, useActiveUniverse } from "@/lib/universes";
import { cn } from "@/lib/utils";

/// U4a+U4d: master-detail session chat. Pane = paged session list plus
/// New session (sub-agent tree expansion arrives with engine D1 parent
/// linkage); detail = live transcript (long-poll tail) with a composer.
export function SessionsPage({ admin }: { admin: boolean }) {
  const { universe, slug, isLoading } = useActiveUniverse();
  const { sessionId } = useParams<{ sessionId: string }>();

  if (isLoading) {
    return <LoadingNote />;
  }
  if (!universe || !canManage(universe, admin)) {
    return (
      <div className="p-6">
        <UniverseNotFound slug={slug} />
      </div>
    );
  }

  return (
    <div className="flex min-h-0 min-w-0 max-w-full flex-1">
      <aside
        className={cn(
          "w-full shrink-0 flex-col border-r md:flex md:w-80",
          sessionId ? "hidden" : "flex",
        )}
      >
        <SessionList universeId={universe.id} slug={slug!} activeId={sessionId} />
      </aside>
      <section className={cn("min-w-0 flex-1 flex-col", sessionId ? "flex" : "hidden md:flex")}>
        <ProviderReadinessBanner universeId={universe.id} slug={slug!} />
        {sessionId ? (
          <SessionDetail
            key={sessionId}
            universeId={universe.id}
            slug={slug!}
            sessionId={sessionId}
          />
        ) : (
          <div className="flex flex-1 items-center justify-center p-6 text-sm text-muted-foreground">
            Select a session, or start a new one.
          </div>
        )}
      </section>
    </div>
  );
}

function SessionList({
  universeId,
  slug,
  activeId,
}: {
  universeId: string;
  slug: string;
  activeId: string | undefined;
}) {
  const pages = useInfiniteQuery({
    queryKey: ["sessions", universeId],
    queryFn: ({ pageParam }) =>
      api<SessionListPage>(
        "GET",
        `/api/v1/universes/${universeId}/sessions?limit=50${
          pageParam ? `&cursor=${encodeURIComponent(pageParam)}` : ""
        }`,
      ),
    initialPageParam: "",
    getNextPageParam: (last) => last.nextCursor ?? undefined,
  });
  const [createOpen, setCreateOpen] = useState(false);
  const [showClosed, setShowClosed] = useState(true);

  const allSessions = pages.data?.pages.flatMap((page) => page.sessions) ?? [];
  const sessions = showClosed
    ? allSessions
    : allSessions.filter((session) => session.lifecycleStatus !== "closed");

  return (
    <>
      <div className="flex h-12 shrink-0 items-center gap-2 border-b px-4">
        <h1 className="text-sm font-semibold">Sessions</h1>
        <span className="text-xs text-muted-foreground">
          {sessions.length}
          {pages.hasNextPage ? "+" : ""}
        </span>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <Button
                variant="ghost"
                size="icon-sm"
                className={cn("ml-auto", !showClosed && "text-primary")}
                aria-label="Session list settings"
              />
            }
          >
            <ListFilter />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-48">
            <DropdownMenuGroup>
              <DropdownMenuLabel>List settings</DropdownMenuLabel>
              <DropdownMenuCheckboxItem
                checked={showClosed}
                onCheckedChange={(checked) => setShowClosed(checked === true)}
              >
                Show closed sessions
              </DropdownMenuCheckboxItem>
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={() => setCreateOpen(true)}
          aria-label="New session"
        >
          <Plus />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {pages.isLoading && <p className="p-4 text-sm text-muted-foreground">Loading…</p>}
        {pages.error && (
          <p className="p-4 text-sm text-destructive">{pages.error.message}</p>
        )}
        {pages.data && allSessions.length === 0 && (
          <p className="p-4 text-sm text-muted-foreground">
            No sessions yet — start one, or bind a chat.
          </p>
        )}
        {pages.data && !showClosed && allSessions.length > 0 && sessions.length === 0 && (
          <p className="p-4 text-sm text-muted-foreground">
            No open sessions in the loaded results.
          </p>
        )}
        <ul>
          {sessions.map((session) => (
            <SessionListItem
              key={session.id}
              session={session}
              slug={slug}
              active={session.id === activeId}
            />
          ))}
        </ul>
        {pages.hasNextPage && (
          <div className="p-3">
            <Button
              variant="outline"
              size="sm"
              className="w-full"
              disabled={pages.isFetchingNextPage}
              onClick={() => void pages.fetchNextPage()}
            >
              {pages.isFetchingNextPage ? "Loading…" : "Load more"}
            </Button>
          </div>
        )}
      </div>
      <NewSessionDialog
        universeId={universeId}
        slug={slug}
        open={createOpen}
        onOpenChange={setCreateOpen}
      />
    </>
  );
}

function SessionListItem({
  session,
  slug,
  active,
}: {
  session: SessionSummary;
  slug: string;
  active: boolean;
}) {
  const botManaged = session.managed && session.id.startsWith("bot:v1:");
  return (
    <li>
      <NavLink
        to={`/u/${slug}/sessions/${session.id}`}
        className={cn(
          "flex flex-col gap-0.5 border-b px-4 py-2.5 text-sm hover:bg-muted/50",
          active && "bg-muted",
        )}
      >
        <span className="flex min-w-0 items-center gap-2">
          <span className="truncate font-medium">
            {session.displayName ?? session.id.slice(0, 18)}
          </span>
          {session.lifecycleStatus === "closed" && (
            <span className="shrink-0 rounded-full bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
              closed
            </span>
          )}
          {session.managed && (
            <Badge variant="secondary" title={botManaged ? "Bot-managed session" : undefined}>
              {botManaged && <BotFaceIcon />}
              {botManaged ? "Bot Managed" : "Managed"}
            </Badge>
          )}
        </span>
        <span className="flex gap-2 font-mono text-xs text-muted-foreground">
          <span className="truncate">{session.id.slice(0, 14)}…</span>
          <span className="ml-auto shrink-0 font-sans">
            {relativeTime(session.updatedAtMs)}
          </span>
        </span>
      </NavLink>
    </li>
  );
}

function NewSessionDialog({
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
  const [displayName, setDisplayName] = useState("");
  const [profileId, setProfileId] = useState("");
  const [step, setStep] = useState<"basics" | "setup">("basics");
  const [inlineProfile, setInlineProfile] = useState<InlineProfile | null>(null);
  const [configError, setConfigError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const profiles = useQuery({
    queryKey: ["profiles", universeId],
    queryFn: () => api<ProfileSummary[]>("GET", `/api/v1/universes/${universeId}/profiles`),
    enabled: open,
  });
  const selectedProfile = useQuery({
    queryKey: ["profile", universeId, profileId],
    queryFn: () =>
      api<ProfileDocument>("GET", `/api/v1/universes/${universeId}/profiles/${profileId}`),
    enabled: open && Boolean(profileId),
  });
  const editorOptions = useSessionConfigEditorOptions(universeId, open && step === "setup");
  const environments = useQuery({
    queryKey: ["environments", universeId],
    queryFn: () => api<Environment[]>("GET", `/api/v1/universes/${universeId}/environments`),
    enabled: open && step === "setup",
  });
  const create = useMutation({
    mutationFn: () =>
      api<SessionView>("POST", `/api/v1/universes/${universeId}/sessions`, {
        ...(displayName.trim() ? { displayName: displayName.trim() } : {}),
        profile: profileForCreate(profileId, inlineProfile, selectedProfile.data),
      }),
    onSuccess: async (session) => {
      await queryClient.invalidateQueries({ queryKey: ["sessions", universeId] });
      onOpenChange(false);
      const target = session.id;
      setDisplayName("");
      setProfileId("");
      setStep("basics");
      setInlineProfile(null);
      setConfigError(null);
      setError(null);
      navigate(`/u/${slug}/sessions/${target}`);
    },
    onError: (err) => setError(err.message),
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    const resourceError = setupResourceFeatureError(
      inlineProfile ?? selectedProfile.data ?? {},
    );
    if (configError || resourceError) {
      setError(configError ? `Config: ${configError}` : resourceError);
      return;
    }
    create.mutate();
  };

  const changeOpen = (next: boolean) => {
    onOpenChange(next);
    if (!next && !create.isPending) {
      setDisplayName("");
      setProfileId("");
      setStep("basics");
      setInlineProfile(null);
      setConfigError(null);
      setError(null);
    }
  };

  const customize = () => {
    if (inlineProfile) {
      setStep("setup");
      return;
    }
    if (profileId && !selectedProfile.data) return;
    setInlineProfile(
      profileId && selectedProfile.data
        ? inlineProfileFromDocument(selectedProfile.data)
        : {},
    );
    setStep("setup");
  };
  const resourceFeatureError = inlineProfile
    ? setupResourceFeatureError(inlineProfile)
    : null;

  return (
    <Dialog open={open} onOpenChange={changeOpen}>
      <DialogContent
        className={step === "setup"
          ? "h-[min(92dvh,900px)] grid-rows-[auto_minmax(0,1fr)_auto] gap-0 p-0 sm:max-w-4xl"
          : undefined}
      >
        {step === "basics" ? (
          <>
            <DialogHeader>
              <DialogTitle>New session</DialogTitle>
              <DialogDescription>
                Start from a named profile or customize an inline setup for this session.
              </DialogDescription>
            </DialogHeader>
            <form onSubmit={submit} className="grid gap-4">
              <Field>
                <FieldLabel htmlFor="new-session-name">Name</FieldLabel>
                <Input
                  id="new-session-name"
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  placeholder="Scratch chat"
                  autoFocus
                />
              </Field>
              <Field>
                <FieldLabel>Profile</FieldLabel>
                <Select
                  value={profileId}
                  onValueChange={(value) => {
                    setProfileId(value as string);
                    setInlineProfile(null);
                    setConfigError(null);
                    setError(null);
                  }}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue>
                      {(value: string) =>
                        value
                          ? (profiles.data?.find((p) => p.profileId === value)?.displayName ?? value)
                          : "No profile (engine defaults)"
                      }
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="">No profile (engine defaults)</SelectItem>
                    {(profiles.data ?? []).map((profile) => (
                      <SelectItem key={profile.profileId} value={profile.profileId}>
                        {profile.displayName ?? profile.profileId}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FieldDescription>
                  The profile is resolved at creation; later profile edits do not change this session.
                </FieldDescription>
              </Field>
              <Button
                type="button"
                variant="outline"
                disabled={Boolean(profileId) && selectedProfile.isLoading}
                onClick={customize}
              >
                {inlineProfile ? "Edit customized setup" : "Customize setup…"}
              </Button>
              {selectedProfile.error && (
                <p className="text-sm text-destructive">{selectedProfile.error.message}</p>
              )}
              {error && <p className="text-sm text-destructive">{error}</p>}
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => changeOpen(false)}>
                  Cancel
                </Button>
                <Button type="submit" disabled={create.isPending}>
                  {create.isPending ? "Creating…" : "Create"}
                </Button>
              </DialogFooter>
            </form>
          </>
        ) : (
          <>
            <DialogHeader className="border-b p-6 pr-14">
              <DialogTitle>Configure new session</DialogTitle>
              <DialogDescription>
                {profileId
                  ? `Customized from ${profiles.data?.find((profile) => profile.profileId === profileId)?.displayName ?? profileId}.`
                  : "Inline setup for this session."}
              </DialogDescription>
            </DialogHeader>
            <div className="min-h-0 overflow-y-auto p-6">
              <InlineSetupEditor
                value={inlineProfile ?? {}}
                options={editorOptions}
                environments={environments.data}
                onValidityChange={setConfigError}
                onChange={setInlineProfile}
              />
            </div>
            <div className="grid gap-2 border-t p-4">
              {resourceFeatureError && (
                <p className="text-sm text-destructive">{resourceFeatureError}</p>
              )}
              {error && <p className="text-sm text-destructive">{error}</p>}
              <DialogFooter>
                <Button type="button" variant="outline" onClick={() => setStep("basics")}>
                  Back
                </Button>
                {profileId && selectedProfile.data && (
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => setInlineProfile(inlineProfileFromDocument(selectedProfile.data!))}
                  >
                    Reset to profile
                  </Button>
                )}
                <Button
                  type="button"
                  disabled={create.isPending || Boolean(configError || resourceFeatureError)}
                  onClick={() => create.mutate()}
                >
                  {create.isPending ? "Creating…" : "Create session"}
                </Button>
              </DialogFooter>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

function InlineSetupEditor({
  value,
  options,
  environments,
  onValidityChange,
  onChange,
}: {
  value: InlineProfile;
  options: ReturnType<typeof useSessionConfigEditorOptions>;
  environments: Environment[] | undefined;
  onValidityChange: (message: string | null) => void;
  onChange: (profile: InlineProfile) => void;
}) {
  const change = (mutate: (next: InlineProfile) => void) => {
    const next = structuredClone(value);
    mutate(next);
    onChange(next);
  };
  const instructions = value.instructions?.type === "text" ? value.instructions.text : "";

  return (
    <div className="grid gap-8">
      <SetupEditorSection title="Instructions" description="System prompt applied when the session starts.">
        {value.instructions?.type === "textRef" ? (
          <p className="text-sm text-muted-foreground">
            This profile uses a blob-backed instruction. Editing replaces it with inline text.
          </p>
        ) : null}
        <textarea
          className="min-h-32 w-full resize-y rounded-lg border border-input bg-transparent p-3 text-sm outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 dark:bg-input/30"
          value={instructions}
          onChange={(event) => change((next) => {
            if (event.target.value) next.instructions = { type: "text", text: event.target.value };
            else delete next.instructions;
          })}
          spellCheck={false}
        />
      </SetupEditorSection>
      <SetupEditorSection
        title="Session config"
        description="Sparse behavior and capability grants. Unset values inherit engine defaults."
      >
        <SessionConfigEditor
          value={value.config}
          mcpServers={options.mcpServers}
          workspaces={options.workspaces}
          workspacesLoading={options.workspacesLoading}
          models={options.models}
          profiles={options.profiles}
          environmentProviders={options.environmentProviders}
          featureDisableReasons={resourceFeatureDisableReasons(value)}
          onValidityChange={onValidityChange}
          onChange={(config) => change((next) => {
            if (config) next.config = config;
            else delete next.config;
          })}
        />
      </SetupEditorSection>
      <ProfileEnvironmentEditor
        value={value.environment}
        environments={environments}
        bindings={options.environmentBindings}
        templates={options.environmentTemplates}
        secrets={options.secrets}
        disabled={!hasSessionFeature(value.config, "environments")}
        onChange={(environment) => change((next) => {
          if (environment) next.environment = environment;
          else delete next.environment;
        })}
      />
    </div>
  );
}

function inlineProfileFromDocument(document: ProfileDocument): InlineProfile {
  const profile: InlineProfile = {};
  if (isRecord(document.config)) profile.config = structuredClone(document.config);
  if (isRecord(document.instructions)) profile.instructions = structuredClone(document.instructions) as InlineProfile["instructions"];
  if (document.environment) profile.environment = structuredClone(document.environment);
  return profile;
}

function profileForCreate(
  profileId: string,
  inlineProfile: InlineProfile | null,
  selectedProfile: ProfileDocument | undefined,
): ProfileSource {
  if (!inlineProfile) {
    return profileId
      ? { kind: "named", profileId }
      : { kind: "inline", profile: {} };
  }
  if (profileId && selectedProfile) {
    const original = inlineProfileFromDocument(selectedProfile);
    if (JSON.stringify(original) === JSON.stringify(inlineProfile)) {
      return { kind: "named", profileId };
    }
  }
  return { kind: "inline", profile: inlineProfile };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export function SessionDetail({
  universeId,
  slug,
  sessionId,
  backTo = `/u/${slug}/sessions`,
}: {
  universeId: string;
  slug: string;
  sessionId: string;
  backTo?: string;
}) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const tail = useSessionTail(universeId, sessionId);
  const session = useQuery({
    queryKey: ["session", universeId, sessionId],
    queryFn: () =>
      api<SessionView>(
        "GET",
        `/api/v1/universes/${universeId}/sessions/${sessionId}`,
      ),
  });
  const [pending, setPending] = useState<PendingMessage[]>([]);
  const [pendingSteers, setPendingSteers] = useState<PendingSteer[]>([]);
  const [notices, setNotices] = useState<{ id: string; text: string }[]>([]);
  const [stoppingRunId, setStoppingRunId] = useState<string | null>(null);
  const [cancellingQueued, setCancellingQueued] = useState<Set<string>>(() => new Set());
  const [sendError, setSendError] = useState<string | null>(null);
  const [sessionIdCopied, setSessionIdCopied] = useState(false);
  const [closeError, setCloseError] = useState<string | null>(null);
  const [closeOpen, setCloseOpen] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);

  const entries = tail.transcript.entries;
  const activeRun = tail.transcript.activeRun;
  const queuedRuns = tail.transcript.queuedRuns;
  const runRevision = tail.transcript.runRevision;
  const activeToolGroup = entries.some(
    (entry) => entry.kind === "tool-group" && !isTerminalToolStatus(entry.status),
  );

  // The session view is authoritative for run state; fold it into the tail
  // whenever it arrives (forward moves only), and refresh it on every run
  // lifecycle change the tail reports so queued-run text and terminal
  // statuses stay current.
  const reconcileRuns = tail.reconcileRuns;
  const sessionRuns = session.data?.runs;
  useEffect(() => {
    if (sessionRuns && tail.phase === "live") {
      reconcileRuns(sessionRuns);
    }
  }, [sessionRuns, tail.phase, reconcileRuns]);
  const refetchSession = session.refetch;
  useEffect(() => {
    if (runRevision > 0) {
      void refetchSession();
    }
  }, [runRevision, refetchSession]);

  // An optimistic send learns its run id from whichever arrives first: the
  // POST response, or the tail's `runAccepted` carrying our submission id.
  const runBySubmission = tail.transcript.runBySubmission;
  useEffect(() => {
    setPending((prev) => {
      let changed = false;
      const next = prev.map((message) => {
        if (message.runId) {
          return message;
        }
        const runId = runBySubmission.get(message.id);
        if (!runId) {
          return message;
        }
        changed = true;
        return { ...message, runId };
      });
      return changed ? next : prev;
    });
  }, [runRevision, runBySubmission]);

  // Reconcile optimistic echoes against the engine's own entries. A sent
  // message is confirmed by the run input entry carrying its run id (the
  // id can arrive after the entry, so this re-runs when pending changes
  // too); a steer by a steering entry with the same text on its run.
  const pendingRunIds = pending.map((message) => message.runId ?? "").join("|");
  useEffect(() => {
    setPending((prev) => {
      if (prev.length === 0) {
        return prev;
      }
      const confirmedRuns = new Set(
        entries
          .filter((entry) => entry.kind === "message" && entry.role === "user" && !entry.steering)
          .map((entry) => (entry as { runId?: string }).runId)
          .filter((runId): runId is string => Boolean(runId)),
      );
      const next = prev.filter((message) => !(message.runId && confirmedRuns.has(message.runId)));
      return next.length === prev.length ? prev : next;
    });
    setPendingSteers((prev) => {
      if (prev.length === 0) {
        return prev;
      }
      const confirmed = new Set(
        entries
          .filter((entry) => entry.kind === "message" && entry.role === "user" && entry.steering)
          .map((entry) => `${(entry as { runId?: string }).runId ?? ""}\u0000${(entry as { text: string }).text.trim()}`),
      );
      const next = prev.filter((steer) => !confirmed.has(`${steer.runId}\u0000${steer.text.trim()}`));
      return next.length === prev.length ? prev : next;
    });
  }, [entries, pendingRunIds]);

  // A pending message whose run the tail now knows is no longer optimistic
  // for status purposes; drop the ones whose run ended without ever
  // materializing input (cancelled while queued).
  useEffect(() => {
    setPending((prev) => {
      const next = prev.filter(
        (message) => !(message.runId && tail.transcript.runPhases.get(message.runId) === "terminal"),
      );
      return next.length === prev.length ? prev : next;
    });
    setPendingSteers((prev) => {
      const dropped = prev.filter(
        (steer) => tail.transcript.runPhases.get(steer.runId) === "terminal",
      );
      if (dropped.length === 0) {
        return prev;
      }
      // A steer that never materialized before its run ended was not
      // seen by the model; say so instead of letting it vanish.
      setNotices((current) => [
        ...current,
        ...dropped.map((steer) => ({
          id: steer.id,
          text: `steering not delivered — the run ended before its next turn: “${steer.text}”`,
        })),
      ]);
      return prev.filter((steer) => !dropped.includes(steer));
    });
    if (stoppingRunId && tail.transcript.runPhases.get(stoppingRunId) === "terminal") {
      setStoppingRunId(null);
    }
    setCancellingQueued((prev) => {
      if (prev.size === 0) {
        return prev;
      }
      const next = new Set(
        [...prev].filter((runId) => tail.transcript.runPhases.get(runId) !== "terminal"),
      );
      return next.size === prev.size ? prev : next;
    });
  }, [runRevision, tail.transcript.runPhases, stoppingRunId]);

  // Resolve run ids synchronously for rendering (the effect above persists
  // them a render later); otherwise the optimistic row and the tail's row
  // coexist for one frame under different keys and the list flickers.
  const resolvedPending: PendingMessage[] = pending.map((message) =>
    message.runId ? message : { ...message, runId: runBySubmission.get(message.id) ?? null },
  );
  const queuedIds = new Set(queuedRuns.map((run) => run.runId));
  // A message sent while a run was already live will be queued by the
  // engine; show it in the queue from the start rather than as a
  // transcript echo that jumps into the queue a moment later.
  const isQueuedPending = (message: PendingMessage) =>
    message.status === "queued" ||
    (message.status === "sending" && message.expectQueued) ||
    (message.runId !== null && queuedIds.has(message.runId));
  // Hide an echo the same frame its engine entry shows (the effect above
  // removes it from state a render later).
  const confirmedInputRuns = new Set(
    entries
      .filter((entry) => entry.kind === "message" && entry.role === "user" && !entry.steering)
      .map((entry) => (entry as { runId?: string }).runId)
      .filter((runId): runId is string => Boolean(runId)),
  );
  const pendingInTranscript = resolvedPending.filter(
    (message) =>
      !isQueuedPending(message) && !(message.runId && confirmedInputRuns.has(message.runId)),
  );
  const confirmedSteers = new Set(
    entries
      .filter((entry) => entry.kind === "message" && entry.role === "user" && entry.steering)
      .map((entry) => `${(entry as { runId?: string }).runId ?? ""}\u0000${(entry as { text: string }).text.trim()}`),
  );
  const visiblePendingSteers = pendingSteers.filter(
    (steer) => !confirmedSteers.has(`${steer.runId}\u0000${steer.text.trim()}`),
  );
  // The run to steer or stop: the tail's active run, or — before the tail
  // has reported it — the run the engine just accepted as running.
  const steerTargetRunId =
    activeRun?.runId ??
    resolvedPending.find(
      (message) =>
        message.runId &&
        (message.status === "running" ||
          tail.transcript.runPhases.get(message.runId) === "running"),
    )?.runId ??
    null;
  const runActive = runInProgress(tail.transcript) || pending.length > 0;
  const stopping = stoppingRunId !== null && steerTargetRunId === stoppingRunId;
  const canSteer = steerTargetRunId !== null && !(activeRun?.cancelling ?? false) && !stopping;
  const queuedItems: QueuedRunItem[] = [
    ...queuedRuns.map((run) => {
      const sent = resolvedPending.find((message) => message.runId === run.runId);
      return {
        key: sent?.id ?? run.runId,
        runId: run.runId,
        text: queuedRunText(run.runId, sessionRuns, resolvedPending),
        cancelling: cancellingQueued.has(run.runId),
      };
    }),
    ...resolvedPending
      .filter(
        (message) =>
          isQueuedPending(message) &&
          !(message.runId && tail.transcript.runPhases.has(message.runId)),
      )
      .map((message) => ({
        key: message.id,
        runId: message.runId ?? null,
        text: message.text,
        pending: true,
      })),
  ];
  const closed = session.data?.status === "closed";
  const management = session.data?.management;
  const managed = session.data?.managed === true;
  const managerLabel = managedSessionOwnerLabel(management);
  // Operator override: the engine happily admits direct runs on a managed
  // session (they queue like any client run), so the gate here is policy,
  // not capability. Off by default because direct input bypasses the
  // manager's ingress; resets when the operator navigates away.
  const managedGate = managed;
  const [directInput, setDirectInput] = useState(false);
  useEffect(() => {
    setDirectInput(false);
  }, [sessionId]);

  useEffect(() => {
    if (!sessionIdCopied) return;
    const timer = window.setTimeout(() => setSessionIdCopied(false), 1_500);
    return () => window.clearTimeout(timer);
  }, [sessionIdCopied]);

  useEffect(() => {
    if (settingsOpen && !runActive) {
      void session.refetch();
    }
  }, [settingsOpen, runActive]);

  const send = async (text: string, mode: ComposerMode | null) => {
    setSendError(null);
    if (mode === "steer") {
      await steer(text);
      return;
    }
    // The submission id doubles as the engine idempotency key: a retried
    // POST returns the original run instead of starting a second one.
    const submissionId = crypto.randomUUID();
    const expectQueued = runActive;
    setPending((prev) => [
      ...prev,
      { id: submissionId, text, runId: null, status: "sending", expectQueued },
    ]);
    try {
      const accepted = await api<SessionRunAccepted>(
        "POST",
        `/api/v1/universes/${universeId}/sessions/${sessionId}/messages`,
        { text, submissionId },
      );
      setPending((prev) =>
        prev.map((message) =>
          message.id === submissionId
            ? {
                ...message,
                runId: message.runId ?? accepted.run.id,
                status: accepted.run.status === "queued" ? "queued" : "running",
              }
            : message,
        ),
      );
    } catch (error) {
      setPending((prev) => prev.filter((message) => message.id !== submissionId));
      setSendError(error instanceof Error ? error.message : String(error));
    }
  };

  const steer = async (text: string) => {
    const runId = steerTargetRunId;
    if (!runId) {
      setSendError(
        "There is no run to steer yet — wait a moment and try again, or press Enter to queue it.",
      );
      return;
    }
    const id = crypto.randomUUID();
    setPendingSteers((prev) => [...prev, { id, runId, text }]);
    try {
      await api<SessionRunSteered>(
        "POST",
        `/api/v1/universes/${universeId}/sessions/${sessionId}/runs/${runId}/steer`,
        { text },
      );
    } catch (error) {
      setPendingSteers((prev) => prev.filter((steer) => steer.id !== id));
      setSendError(error instanceof Error ? error.message : String(error));
    }
  };

  const cancelRun = async (runId: string) => {
    setSendError(null);
    try {
      return await api<SessionRunCancelled>(
        "POST",
        `/api/v1/universes/${universeId}/sessions/${sessionId}/runs/${runId}/cancel`,
        {},
      );
    } catch (error) {
      setSendError(error instanceof Error ? error.message : String(error));
      return null;
    }
  };

  const stop = async () => {
    // Stop the run the engine is executing; if only queued runs exist
    // (the active one just ended), stop the next one instead so the queue
    // does not start behind the reader's back.
    const target =
      steerTargetRunId ?? queuedRuns[0]?.runId ?? resolvedPending.find((m) => m.runId)?.runId;
    if (!target) {
      return;
    }
    if (steerTargetRunId === target) {
      setStoppingRunId(target);
    } else {
      setCancellingQueued((prev) => new Set(prev).add(target));
    }
    const response = await cancelRun(target);
    if (!response) {
      setStoppingRunId((current) => (current === target ? null : current));
      setCancellingQueued((prev) => {
        const next = new Set(prev);
        next.delete(target);
        return next;
      });
      return;
    }
    if (response.run.status === "cancelled") {
      setStoppingRunId((current) => (current === target ? null : current));
    }
    void session.refetch();
  };

  const cancelQueued = async (runId: string) => {
    setCancellingQueued((prev) => new Set(prev).add(runId));
    const response = await cancelRun(runId);
    if (!response) {
      setCancellingQueued((prev) => {
        const next = new Set(prev);
        next.delete(runId);
        return next;
      });
      return;
    }
    void session.refetch();
  };

  const closeSession = useMutation({
    mutationFn: () =>
      api<SessionView>(
        "POST",
        `/api/v1/universes/${universeId}/sessions/${sessionId}/close`,
        { force: true },
      ),
    onSuccess: async (closedSession) => {
      setCloseOpen(false);
      queryClient.setQueryData(
        ["session", universeId, sessionId],
        closedSession,
      );
      await queryClient.invalidateQueries({ queryKey: ["sessions", universeId] });
    },
    onError: (error) => setCloseError(error.message),
  });

  const deleteSession = useMutation({
    mutationFn: () =>
      api<SessionSummary>(
        "DELETE",
        `/api/v1/universes/${universeId}/sessions/${sessionId}`,
      ),
    onSuccess: async () => {
      setDeleteOpen(false);
      queryClient.setQueryData<InfiniteData<SessionListPage>>(
        ["sessions", universeId],
        (current) => current
          ? {
              ...current,
              pages: current.pages.map((page) => ({
                ...page,
                sessions: page.sessions.filter((candidate) => candidate.id !== sessionId),
              })),
            }
          : current,
      );
      navigate(`/u/${slug}/sessions`);
      await queryClient.invalidateQueries({ queryKey: ["sessions", universeId] });
    },
    onError: (error) => setDeleteError(error.message),
  });

  return (
    <>
      <header className="flex h-12 shrink-0 items-center gap-3 border-b px-4">
        <NavLink to={backTo} className="md:hidden">
          <ArrowLeft className="size-4" />
        </NavLink>
        <h1 className="min-w-0 truncate text-sm font-semibold">
          {session.data?.displayName ?? sessionId.slice(0, 24)}
        </h1>
        {closed && (
          <span className="rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground">
            Closed
          </span>
        )}
        {managed && (
          <Tooltip>
            <TooltipTrigger
              render={<button type="button" onClick={() => setSettingsOpen(true)} />}
            >
              <Badge variant="secondary" className="gap-1">
                <ShieldCheck /> Managed by {managerLabel}
              </Badge>
            </TooltipTrigger>
            <TooltipContent>
              {`Lifecycle and chat input are controlled by ${managerLabel}; configuration remains editable.`}
            </TooltipContent>
          </Tooltip>
        )}
        <div className="ml-auto flex items-center gap-1">
          {!closed && !managed && (
            <AlertDialog
              open={closeOpen}
              onOpenChange={(open) => {
                setCloseOpen(open);
                if (open) setCloseError(null);
              }}
            >
              <AlertDialogTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="text-destructive"
                    disabled={closeSession.isPending}
                    aria-label={runActive ? "Force close session" : "Close session"}
                    title={runActive
                      ? "Cancel active work and permanently close this session"
                      : "Permanently close this session"}
                  />
                }
              >
                {closeSession.isPending
                  ? <LoaderCircle className="animate-spin" />
                  : <Archive />}
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>
                    {runActive ? "Force close this session?" : "Close this session?"}
                  </AlertDialogTitle>
                  <AlertDialogDescription>
                    {runActive
                      ? "This cancels active and queued work, then permanently closes the session. Recovery of a stuck workflow can take up to about 90 seconds while the engine terminates it and reconciles the session. The history remains available, but the session cannot be reopened."
                      : "This permanently closes the session. It remains in the session list so its history can be inspected, but it cannot be reopened."}
                  </AlertDialogDescription>
                </AlertDialogHeader>
                {closeSession.isPending && (
                  <p className="text-sm text-muted-foreground">
                    Force close is running in the background. You can hide this dialog and
                    continue using the app.
                  </p>
                )}
                {closeError && <p className="text-sm text-destructive">{closeError}</p>}
                <AlertDialogFooter>
                  <AlertDialogCancel>
                    {closeSession.isPending ? "Hide" : "Cancel"}
                  </AlertDialogCancel>
                  <AlertDialogAction
                    className="bg-destructive text-white hover:bg-destructive/90"
                    disabled={closeSession.isPending}
                    onClick={() => closeSession.mutate()}
                  >
                    {closeSession.isPending
                      ? "Force-closing…"
                      : runActive
                        ? "Force close session"
                        : "Close session"}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          )}
          {closed && !managed && (
            <AlertDialog
              open={deleteOpen}
              onOpenChange={(open) => {
                setDeleteOpen(open);
                if (open) setDeleteError(null);
              }}
            >
              <AlertDialogTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="text-destructive"
                    aria-label="Delete session"
                  />
                }
              >
                <Trash2 />
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>Delete this session permanently?</AlertDialogTitle>
                  <AlertDialogDescription>
                    This removes the session and its retained history. It cannot be undone.
                    Sessions with forks that still inherit their history must be deleted
                    leaf-first.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                {deleteError && <p className="text-sm text-destructive">{deleteError}</p>}
                <AlertDialogFooter>
                  <AlertDialogCancel disabled={deleteSession.isPending}>Cancel</AlertDialogCancel>
                  <AlertDialogAction
                    className="bg-destructive text-white hover:bg-destructive/90"
                    disabled={deleteSession.isPending}
                    onClick={() => deleteSession.mutate()}
                  >
                    {deleteSession.isPending ? "Deleting…" : "Delete permanently"}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          )}
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label="Session settings"
            onClick={() => setSettingsOpen(true)}
          >
            <SlidersHorizontal />
          </Button>
        </div>
        {activeRun && !activeToolGroup && (
          <span className="shrink-0 text-xs text-muted-foreground">{activeRun.label}…</span>
        )}
        <Button
          variant="ghost"
          size="xs"
          className="shrink-0 gap-1.5 px-2 font-mono text-xs text-muted-foreground"
          aria-label={sessionIdCopied ? "Session ID copied" : "Copy session ID"}
          title={sessionIdCopied ? "Copied" : `Copy ${sessionId}`}
          onClick={() => {
            void navigator.clipboard
              .writeText(sessionId)
              .then(() => setSessionIdCopied(true))
              .catch(() => undefined);
          }}
        >
          {sessionIdCopied ? "Copied" : `${sessionId.slice(0, 18)}…`}
          {sessionIdCopied ? <Check /> : <Copy />}
        </Button>
      </header>
      <MessageScrollerProvider autoScroll defaultScrollPosition="end">
        <MessageScroller className="min-h-0 flex-1">
          <MessageScrollerViewport>
            <MessageScrollerContent className="gap-3 px-4 py-6 md:px-8">
              {tail.phase === "loading" && entries.length === 0 && !tail.error && (
                <LoadingNote />
              )}
              {tail.error && entries.length === 0 && (
                <p className="text-sm text-destructive">{tail.error}</p>
              )}
              {tail.truncated && (
                <p className="text-center text-xs text-muted-foreground">
                  Very long session — a stretch of older events was skipped.
                </p>
              )}
              {tail.phase === "live" &&
                entries.length === 0 &&
                pendingInTranscript.length === 0 && (
                  <CenteredNote>No conversation yet — say something below.</CenteredNote>
                )}
              {entries.map((entry) => (
                <MessageScrollerItem
                  key={entry.key}
                  messageId={entry.key}
                >
                  <TranscriptEntryView entry={entry} />
                </MessageScrollerItem>
              ))}
              {pendingInTranscript.map((message) => (
                <MessageScrollerItem key={message.id} messageId={message.id}>
                  <UserBand text={message.text} pending />
                </MessageScrollerItem>
              ))}
              {visiblePendingSteers.map((steer) => (
                <MessageScrollerItem key={steer.id} messageId={steer.id}>
                  <UserBand text={steer.text} pending steering />
                </MessageScrollerItem>
              ))}
              {notices.map((notice) => (
                <MessageScrollerItem key={notice.id} messageId={notice.id}>
                  <TranscriptEntryView
                    entry={{ kind: "marker", key: notice.id, text: notice.text, tone: "muted" }}
                  />
                </MessageScrollerItem>
              ))}
              {activeRun && !activeToolGroup && (
                <MessageScrollerItem messageId="active-run">
                  <ActiveRunMarker run={activeRun} />
                </MessageScrollerItem>
              )}
              {tail.error && entries.length > 0 && (
                <p className="text-center text-xs text-destructive">
                  Connection lost — retrying. ({tail.error})
                </p>
              )}
            </MessageScrollerContent>
          </MessageScrollerViewport>
          <MessageScrollerButton />
        </MessageScroller>
        <SessionScrollFollower
          ready={tail.phase === "live"}
          entries={entries}
          pending={pendingInTranscript}
          activeRun={activeRun}
        />
      </MessageScrollerProvider>
      {!closed && (
        <QueuedRunsBar items={queuedItems} onCancel={(runId) => void cancelQueued(runId)} />
      )}
      <SessionComposer
        runActive={runActive}
        canSteer={canSteer}
        stopping={stopping}
        disabled={closed || (managedGate && !directInput)}
        disabledReason={managedGate && !directInput
          ? `Managed by ${managerLabel} — flip Direct input to message this session anyway.`
          : undefined}
        banner={managedGate && !closed ? (
          <div className="flex min-w-0 items-center gap-2 pb-2 text-xs">
            <Switch
              className="shrink-0"
              checked={directInput}
              onCheckedChange={setDirectInput}
              aria-label="Direct input"
            />
            <span className="shrink-0 font-medium">Direct input</span>
            <span
              className={`min-w-0 truncate ${directInput ? "text-foreground" : "text-muted-foreground"}`}
              title={directInput
                ? `Direct input bypasses ${managerLabel}'s ingress: messages are not tracked as events, skip its budget and delivery policies, and may interleave with its deliveries.`
                : `Managed by ${managerLabel} — flip Direct input to message this session anyway.`}
            >
              {directInput
                ? `Bypasses ${managerLabel}'s ingress: messages are not tracked as events, skip its budget and delivery policies, and may interleave with its deliveries.`
                : `Managed by ${managerLabel} — flip to message this session anyway.`}
            </span>
          </div>
        ) : undefined}
        error={sendError}
        onSend={(text, mode) => void send(text, mode)}
        onStop={() => void stop()}
      />
      <SessionSettingsDialog
        universeId={universeId}
        sessionId={sessionId}
        session={session.data}
        runActive={runActive}
        open={settingsOpen}
        onOpenChange={(open) => {
          setSettingsOpen(open);
          if (open) void session.refetch();
        }}
      />
    </>
  );
}

interface PendingMessage {
  id: string;
  text: string;
  /// Engine run id once the POST returned; null while in flight.
  runId: string | null;
  status: "sending" | "running" | "queued";
  /// Sent while a run was already live, so the engine will queue it.
  expectQueued: boolean;
}

interface PendingSteer {
  id: string;
  runId: string;
  text: string;
}

/// Text for a queued run: from the authoritative session view when it has
/// been refetched, else from the optimistic send that produced it.
function queuedRunText(
  runId: string,
  runs: SessionRunView[] | undefined,
  pending: PendingMessage[],
): string {
  const run = runs?.find((candidate) => String(candidate.id) === runId);
  if (run?.source.type === "input") {
    const text = run.source.items
      .map((item) => (item.type === "text" ? item.text : `[${item.type}]`))
      .join("\n")
      .trim();
    if (text) {
      return text;
    }
  }
  return pending.find((message) => message.runId === runId)?.text ?? "(queued message)";
}

function SessionScrollFollower({
  ready,
  entries,
  pending,
  activeRun,
}: {
  ready: boolean;
  entries: TranscriptEntry[];
  pending: { id: string; text: string }[];
  activeRun: ActiveRun | null;
}) {
  const { scrollToEnd } = useMessageScroller();
  const scrollable = useMessageScrollerScrollable();
  const initialized = useRef(false);

  useLayoutEffect(() => {
    if (!ready) {
      return;
    }

    // On open, always start at the latest message. After that, append only
    // follows when the viewport was already at its end before this render;
    // a reader who scrolled up keeps their position.
    if (!initialized.current || !scrollable.end) {
      initialized.current = true;
      scrollToEnd({ behavior: "auto" });
    }
  }, [ready, entries, pending, activeRun, scrollable.end, scrollToEnd]);

  return null;
}

function relativeTime(ms: number): string {
  const delta = Date.now() - ms;
  if (delta < 60_000) return "now";
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m`;
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h`;
  return `${Math.floor(delta / 86_400_000)}d`;
}
