import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowUpRight, ChevronRight, SlidersHorizontal } from "lucide-react";
import { Link, useNavigate } from "react-router-dom";
import {
  api,
  botLabel,
  type Bot,
  type BotInboxSpec,
  type BotListItem,
  type BotState,
  type BotTrigger,
  type Environment,
  type ProfileDocument,
  type ProfileEnvironment,
  type ProfileSummary,
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
import { Button } from "@/components/ui/button";
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
import { ProfileEnvironmentEditor } from "@/components/session/profile-environment-editor";
import { SessionConfigEditor } from "@/components/session/session-config-editor";
import { useSecretsInventory } from "@/lib/environment-credentials";
import { useSessionConfigEditorOptions } from "@/lib/sessions/editor-options";
import {
  hasSessionFeature,
  resourceFeatureDisableReasons,
  setupResourceFeatureError,
} from "@/lib/sessions/resource-features";
import { cn } from "@/lib/utils";
import { BotEnvironmentCard } from "./environment-card";
import { BotAvatar } from "./face";
import { briefSummary, capabilitySummary, environmentSummary, guardrailsSummary, otherBotsSummary } from "./setup-summary";
import { triggerSummary } from "./trigger-summary";
import {
  BotMultiSelect,
  EditTriggerDialog,
  TriggersSection,
  inboxSelectionSpec,
  type BotEnvStatus,
} from "./triggers";

/**
 * Everything about how a bot works, as a stack of sections that each read
 * as one line when closed — so the page at a glance is a summary, and you
 * open only what you are editing. Each section saves on its own.
 */
export function BotSetup({
  slug,
  bot,
  state,
  manage,
}: {
  slug: string;
  bot: Bot;
  state?: BotState;
  manage: boolean;
}) {
  const profile = useQuery({
    queryKey: ["profile", bot.universeId, bot.profileId],
    queryFn: () =>
      api<ProfileDocument>(
        "GET",
        `/api/v1/universes/${bot.universeId}/profiles/${encodeURIComponent(bot.profileId)}`,
      ),
    staleTime: 0,
    retry: false,
  });
  const triggers = useQuery({
    queryKey: ["bot-triggers", bot.universeId, bot.botId],
    queryFn: () =>
      api<{ triggers: BotTrigger[] }>("GET", `/api/v1/universes/${bot.universeId}/bots/${bot.botId}/triggers`),
  });
  const env: BotEnvStatus =
    profile.isLoading || profile.isError || profile.data === undefined
      ? { kind: "unknown" }
      : profile.data.environment == null
        ? { kind: "none" }
        : profile.data.environment.type === "existing"
          ? { kind: "existing", environmentId: profile.data.environment.environmentId }
          : { kind: "provision" };
  const triggerList = triggers.data?.triggers ?? [];
  const wakeups = triggerList.filter((trigger) => trigger.kind !== "bot");
  const triggersLine =
    wakeups.length === 0
      ? "Nothing wakes it yet — you can always message it"
      : `${wakeups.length} ${wakeups.length === 1 ? "trigger" : "triggers"} · ${wakeups
          .slice(0, 2)
          .map((trigger) => `${trigger.name}: ${triggerSummary(trigger)}`)
          .join(" · ")}${wakeups.length > 2 ? " · …" : ""}`;

  return (
    <div className="min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto">
      <div className="mx-auto grid w-full min-w-0 max-w-4xl gap-3 px-4 py-5 text-sm md:px-8">
        <IdentitySection bot={bot} manage={manage} />
        <BriefSection bot={bot} manage={manage} />
        <SetupSection
          id="triggers"
          title="Triggers"
          description="When this bot wakes up: schedules, webhooks, polls, chats, other bots."
          summary={triggersLine}
          defaultOpen
        >
          <TriggersSection
            universeId={bot.universeId}
            botId={bot.botId}
            manage={manage}
            env={env}
            headless
            hideKinds={["bot"]}
          />
        </SetupSection>
        <OtherBotsSection bot={bot} manage={manage} inbox={triggerList.find((trigger) => trigger.kind === "bot")} />
        <ProfileSections slug={slug} bot={bot} manage={manage} profile={profile.data} profileError={profile.error?.message} />
        <GuardrailsSection bot={bot} state={state} manage={manage} />
        <DangerSection slug={slug} bot={bot} manage={manage} />
      </div>
    </div>
  );
}

function SetupSection({
  id,
  title,
  description,
  summary,
  defaultOpen = false,
  actions,
  tone,
  children,
}: {
  id: string;
  title: string;
  description: string;
  /** One line shown while closed: what is set, not what the section is. */
  summary: string;
  defaultOpen?: boolean;
  actions?: React.ReactNode;
  tone?: "danger";
  children: React.ReactNode;
}) {
  const [open, setOpen] = useState(() => defaultOpen || window.location.hash === `#${id}`);
  useEffect(() => {
    if (window.location.hash !== `#${id}`) return;
    setOpen(true);
    document.getElementById(id)?.scrollIntoView({ block: "start" });
  }, [id]);
  return (
    <section
      id={id}
      className={cn("scroll-mt-4 rounded-lg border bg-card", tone === "danger" && "border-destructive/40")}
    >
      <div className="flex items-center gap-2 pr-3">
        <button
          type="button"
          onClick={() => setOpen((value) => !value)}
          aria-expanded={open}
          className="flex min-w-0 flex-1 items-center gap-3 px-4 py-3 text-left"
        >
          <ChevronRight
            className={cn("size-4 shrink-0 text-muted-foreground transition-transform", open && "rotate-90")}
          />
          <span className="min-w-0 flex-1">
            <span className={cn("block text-sm font-semibold", tone === "danger" && "text-destructive")}>{title}</span>
            <span className="block truncate text-xs text-muted-foreground" title={open ? undefined : summary}>
              {open ? description : summary}
            </span>
          </span>
        </button>
        {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      </div>
      {open && <div className="grid gap-4 border-t px-4 py-4">{children}</div>}
    </section>
  );
}

function useBotPatch(bot: Bot) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (fields: Record<string, unknown>) =>
      api<{ bot: Bot }>("PATCH", `/api/v1/universes/${bot.universeId}/bots/${bot.botId}`, fields),
    onSuccess: async ({ bot: updated }) => {
      queryClient.setQueryData(["bot", bot.universeId, bot.botId], { bot: updated });
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["bots", bot.universeId] }),
        queryClient.invalidateQueries({ queryKey: ["bot-state", bot.universeId, bot.botId] }),
      ]);
    },
  });
}

function SaveRow({
  dirty,
  pending,
  error,
  onSave,
  disabled = false,
  note,
}: {
  dirty: boolean;
  pending: boolean;
  error: string | null | undefined;
  onSave: () => void;
  disabled?: boolean;
  note?: string;
}) {
  return (
    <div className="flex flex-wrap items-center gap-3">
      <Button size="sm" disabled={!dirty || pending || disabled} onClick={onSave}>
        {pending ? "Saving…" : dirty ? "Save" : "Saved"}
      </Button>
      {note && !error && <span className="text-xs text-muted-foreground">{note}</span>}
      {error && <span className="text-xs text-destructive">{error}</span>}
    </div>
  );
}

function IdentitySection({ bot, manage }: { bot: Bot; manage: boolean }) {
  const [displayName, setDisplayName] = useState(bot.displayName ?? "");
  const [description, setDescription] = useState(bot.description ?? "");
  useEffect(() => {
    setDisplayName(bot.displayName ?? "");
    setDescription(bot.description ?? "");
  }, [bot.displayName, bot.description]);
  const save = useBotPatch(bot);
  const dirty = displayName.trim() !== (bot.displayName ?? "") || description.trim() !== (bot.description ?? "");
  return (
    <SetupSection
      id="identity"
      title="Identity"
      description="How people and other bots know it."
      summary={`${botLabel(bot)} · ${bot.botId}${bot.description ? ` · ${bot.description}` : ""}`}
    >
      <div className="grid gap-4 sm:grid-cols-[auto_minmax(0,1fr)]">
        <BotAvatar botId={bot.botId} size={56} className="rounded-xl" />
        <div className="grid gap-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <Field>
              <FieldLabel htmlFor="bot-display-name">Name</FieldLabel>
              <Input
                id="bot-display-name"
                value={displayName}
                onChange={(event) => setDisplayName(event.target.value)}
                placeholder={bot.botId}
                disabled={!manage}
              />
            </Field>
            <Field>
              <FieldLabel>Id</FieldLabel>
              <Input value={bot.botId} readOnly className="font-mono" />
              <FieldDescription>What other bots, briefs, and URLs use. It cannot change.</FieldDescription>
            </Field>
          </div>
          <Field>
            <FieldLabel htmlFor="bot-description">Description</FieldLabel>
            <Input
              id="bot-description"
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              placeholder="One line other bots read when deciding whether to message this bot."
              disabled={!manage}
            />
          </Field>
          {manage && (
            <SaveRow
              dirty={dirty}
              pending={save.isPending}
              error={save.error?.message}
              onSave={() =>
                save.mutate({
                  displayName: displayName.trim() ? displayName.trim() : null,
                  description: description.trim() ? description.trim() : null,
                })
              }
            />
          )}
        </div>
      </div>
    </SetupSection>
  );
}

function BriefSection({ bot, manage }: { bot: Bot; manage: boolean }) {
  const [brief, setBrief] = useState(bot.brief ?? "");
  useEffect(() => setBrief(bot.brief ?? ""), [bot.brief]);
  const save = useBotPatch(bot);
  const dirty = brief.trim() !== (bot.brief ?? "");
  const closed = bot.closedAt !== null;
  return (
    <SetupSection
      id="brief"
      title="Brief"
      description="The job. Sent with every event, after the profile's base instructions."
      summary={briefSummary(bot.brief)}
      defaultOpen
    >
      <Textarea
        value={brief}
        onChange={(event) => setBrief(event.target.value)}
        rows={10}
        placeholder="What this bot is for, how it should behave, what good work looks like."
        disabled={!manage || closed}
        className="leading-relaxed"
      />
      {manage && (
        <SaveRow
          dirty={dirty}
          pending={save.isPending}
          error={save.error?.message}
          disabled={closed}
          onSave={() => save.mutate({ brief: brief.trim() ? brief.trim() : null })}
          note={
            bot.selfConfig
              ? "The bot can rewrite this itself when asked in a conversation; changes reach it at its next idle moment."
              : "Changes reach the bot at its next idle moment."
          }
        />
      )}
    </SetupSection>
  );
}

type SaveableFields = {
  config?: Record<string, unknown> | undefined;
  instructions?: { type: "text"; text: string } | undefined;
  environment?: ProfileEnvironment | undefined;
};

/**
 * Capabilities and Environment both live in the bot's profile, one
 * document with one revision. Each section saves only its own fields onto
 * the latest revision, so saving one never clobbers the other.
 */
function ProfileSections({
  slug,
  bot,
  manage,
  profile,
  profileError,
}: {
  slug: string;
  bot: Bot;
  manage: boolean;
  profile: ProfileDocument | undefined;
  profileError: string | undefined;
}) {
  const queryClient = useQueryClient();
  const universeId = bot.universeId;
  const profileUrl = `/api/v1/universes/${universeId}/profiles/${encodeURIComponent(bot.profileId)}`;
  const options = useSessionConfigEditorOptions(universeId);
  const environments = useQuery({
    queryKey: ["environments", universeId],
    queryFn: () => api<Environment[]>("GET", `/api/v1/universes/${universeId}/environments`),
  });
  const secrets = useSecretsInventory(universeId);
  const bots = useQuery({
    queryKey: ["bots", universeId],
    queryFn: () => api<{ bots: BotListItem[] }>("GET", `/api/v1/universes/${universeId}/bots`),
  });
  const sharedWith = (bots.data?.bots ?? []).filter(
    (other) => other.botId !== bot.botId && other.profileId === bot.profileId && !other.closedAt,
  );

  const [configDraft, setConfigDraft] = useState<Record<string, unknown> | undefined>();
  const [instructionsDraft, setInstructionsDraft] = useState("");
  const [environmentDraft, setEnvironmentDraft] = useState<ProfileEnvironment | undefined>();
  const [configError, setConfigError] = useState<string | null>(null);
  const revision = profile?.revision;
  useEffect(() => {
    if (!profile) return;
    setConfigDraft(profile.config ? structuredClone(profile.config as Record<string, unknown>) : undefined);
    const instructions = profile.instructions as { type: "text"; text: string } | { type: "textRef" } | undefined;
    setInstructionsDraft(instructions?.type === "text" ? instructions.text : "");
    setEnvironmentDraft(profile.environment ?? undefined);
    // Re-sync on a new revision only; an unrelated refetch must not wipe edits.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [revision, bot.profileId]);

  const baseConfig = JSON.stringify(profile?.config ?? null);
  const baseInstructions = (profile?.instructions as { type?: string; text?: string } | undefined)?.type === "text"
    ? ((profile?.instructions as { text: string }).text ?? "")
    : "";
  const configDirty = profile !== undefined && JSON.stringify(configDraft ?? null) !== baseConfig;
  const instructionsDirty = profile !== undefined && instructionsDraft !== baseInstructions;
  const environmentDirty =
    profile !== undefined && JSON.stringify(environmentDraft ?? null) !== JSON.stringify(profile.environment ?? null);

  const save = useMutation({
    mutationFn: async (fields: SaveableFields) => {
      const latest = await api<ProfileDocument>("GET", profileUrl);
      const { createdAtMs: _created, updatedAtMs: _updated, ...document } = latest;
      const next: Record<string, unknown> = { ...document };
      for (const [key, value] of Object.entries(fields)) {
        if (value === undefined) delete next[key];
        else next[key] = value;
      }
      const problem = setupResourceFeatureError(next);
      if (problem) throw new Error(problem);
      await api("PUT", profileUrl, next);
      // The profile's revision moved; tell every bot on it to re-read it
      // now rather than at the next unrelated config change.
      await api("POST", `/api/v1/universes/${universeId}/bots/reconcile`, { profileId: bot.profileId });
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["profile", universeId, bot.profileId] }),
        queryClient.invalidateQueries({ queryKey: ["profiles", universeId] }),
        queryClient.invalidateQueries({ queryKey: ["bot-state", universeId, bot.botId] }),
      ]);
    },
  });
  const merged = useMemo(
    () => ({ ...(profile ?? {}), config: configDraft, environment: environmentDraft }),
    [profile, configDraft, environmentDraft],
  );
  const closed = bot.closedAt !== null;
  const readOnly = !manage || closed;
  const capabilities = capabilitySummary(profile?.config as Record<string, unknown> | undefined);
  const textRef = (profile?.instructions as { type?: string } | undefined)?.type === "textRef";

  return (
    <>
      <SetupSection
        id="capabilities"
        title="Capabilities"
        description="The model and tools this bot's sessions get, from its profile."
        summary={
          profileError
            ? `Profile ${bot.profileId} could not be read`
            : `${capabilities.length > 0 ? capabilities.join(" · ") : "Default model, no tools"} · profile ${bot.profileId}`
        }
      >
        <ProfileSwitcher bot={bot} slug={slug} manage={manage && !closed} sharedWith={sharedWith} />
        {profileError && (
          <p className="rounded-md bg-destructive/10 p-2 text-xs text-destructive">
            Profile <code>{bot.profileId}</code> could not be read: {profileError}
          </p>
        )}
        {profile && (
          <>
            <SessionConfigEditor
              value={configDraft}
              mcpServers={options.mcpServers}
              workspaces={options.workspaces}
              workspacesLoading={options.workspacesLoading}
              models={options.models}
              profiles={options.profiles}
              environmentProviders={options.environmentProviders}
              featureDisableReasons={resourceFeatureDisableReasons(merged)}
              onValidityChange={setConfigError}
              onChange={(config) => setConfigDraft(config as Record<string, unknown> | undefined)}
            />
            <Field>
              <FieldLabel htmlFor="bot-base-instructions">Base instructions</FieldLabel>
              <Textarea
                id="bot-base-instructions"
                value={instructionsDraft}
                onChange={(event) => setInstructionsDraft(event.target.value)}
                rows={4}
                placeholder="Usually empty for a bot: the brief above is its job. Use this for a system prompt the profile should carry on its own."
                disabled={readOnly || textRef}
              />
              {textRef && <FieldDescription>Stored as a blob reference; edit it on the Profiles page.</FieldDescription>}
            </Field>
            {manage && (
              <SaveRow
                dirty={configDirty || instructionsDirty}
                pending={save.isPending}
                error={configError ? `Config: ${configError}` : save.error?.message}
                disabled={closed || configError !== null}
                onSave={() =>
                  save.mutate({
                    config: configDraft,
                    ...(instructionsDirty
                      ? {
                          instructions: instructionsDraft.trim()
                            ? { type: "text", text: instructionsDraft }
                            : undefined,
                        }
                      : {}),
                  })
                }
                note="Applies to Main at its next idle moment; open threads keep their setup until they close."
              />
            )}
          </>
        )}
      </SetupSection>

      <SetupSection
        id="environment"
        title="Environment"
        description="Where the bot works: an environment shared across its sessions, or a fresh one per session. Command polls need a lasting one."
        summary={environmentSummary(profile?.environment, environments.data)}
      >
        {profile && (
          <ProfileEnvironmentEditor
            value={environmentDraft}
            environments={environments.data}
            bindings={options.environmentBindings}
            templates={options.environmentTemplates}
            secrets={secrets.data}
            disabled={!hasSessionFeature(configDraft, "environments")}
            title=""
            description=""
            onChange={setEnvironmentDraft}
          />
        )}
        {profile && manage && (
          <SaveRow
            dirty={environmentDirty}
            pending={save.isPending}
            error={save.error?.message}
            disabled={closed}
            onSave={() => save.mutate({ environment: environmentDraft })}
          />
        )}
        {profile?.environment?.type === "existing" && (
          <BotEnvironmentCard
            slug={slug}
            universeId={universeId}
            environmentId={profile.environment.environmentId}
            manage={manage}
          />
        )}
      </SetupSection>
    </>
  );
}

function ProfileSwitcher({
  bot,
  slug,
  manage,
  sharedWith,
}: {
  bot: Bot;
  slug: string;
  manage: boolean;
  sharedWith: BotListItem[];
}) {
  const profiles = useQuery({
    queryKey: ["profiles", bot.universeId],
    queryFn: () => api<ProfileSummary[]>("GET", `/api/v1/universes/${bot.universeId}/profiles`),
  });
  const [selected, setSelected] = useState(bot.profileId);
  useEffect(() => setSelected(bot.profileId), [bot.profileId]);
  const patch = useBotPatch(bot);
  const known = profiles.data?.some((profile) => profile.profileId === bot.profileId) ?? true;
  return (
    <Field>
      <FieldLabel htmlFor="bot-profile">Profile</FieldLabel>
      <div className="flex flex-wrap items-center gap-2">
        <Select value={selected} onValueChange={(value) => value && setSelected(value)} disabled={!manage}>
          <SelectTrigger id="bot-profile" className="w-64">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {!known && <SelectItem value={bot.profileId}>{bot.profileId}</SelectItem>}
            {profiles.data?.map((profile) => (
              <SelectItem key={profile.profileId} value={profile.profileId}>
                {profile.displayName ?? profile.profileId}
                {profile.displayName && profile.displayName !== profile.profileId ? ` · ${profile.profileId}` : ""}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {selected !== bot.profileId ? (
          <>
            <Button size="sm" disabled={patch.isPending} onClick={() => patch.mutate({ profileId: selected })}>
              {patch.isPending ? "Switching…" : "Use this profile"}
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setSelected(bot.profileId)}>
              Cancel
            </Button>
          </>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            render={<Link to={`/u/${slug}/profiles/${encodeURIComponent(bot.profileId)}`} />}
          >
            Open on the Profiles page <ArrowUpRight data-icon="inline-end" />
          </Button>
        )}
      </div>
      <FieldDescription>
        {patch.error ? (
          <span className="text-destructive">{patch.error.message}</span>
        ) : sharedWith.length > 0 ? (
          <>
            Shared: {sharedWith.map(botLabel).join(", ")} use{sharedWith.length === 1 ? "s" : ""} this profile too, so
            changes below reach {sharedWith.length === 1 ? "it" : "them"} as well.
          </>
        ) : (
          "The model, tools, and environment below are this profile's; switching profiles swaps all of them at the bot's next idle moment."
        )}
      </FieldDescription>
    </Field>
  );
}

function GuardrailsSection({ bot, state, manage }: { bot: Bot; state?: BotState; manage: boolean }) {
  const queryClient = useQueryClient();
  const universeId = bot.universeId;

  const [runsPerDay, setRunsPerDay] = useState(bot.runsPerDay?.toString() ?? "");
  const [breakerFires, setBreakerFires] = useState(bot.breaker?.fires.toString() ?? "");
  const [breakerWindow, setBreakerWindow] = useState(bot.breaker ? String(Math.round(bot.breaker.windowMs / 60_000)) : "");
  const [ttlDays, setTtlDays] = useState(bot.routedSessionTtlMs ? String(Math.round(bot.routedSessionTtlMs / 86_400_000)) : "");
  const [selfConfig, setSelfConfig] = useState(bot.selfConfig);
  useEffect(() => {
    setRunsPerDay(bot.runsPerDay?.toString() ?? "");
    setBreakerFires(bot.breaker?.fires.toString() ?? "");
    setBreakerWindow(bot.breaker ? String(Math.round(bot.breaker.windowMs / 60_000)) : "");
    setTtlDays(bot.routedSessionTtlMs ? String(Math.round(bot.routedSessionTtlMs / 86_400_000)) : "");
    setSelfConfig(bot.selfConfig);
  }, [bot.runsPerDay, bot.breaker, bot.routedSessionTtlMs, bot.selfConfig]);

  const fields = () => ({
    runsPerDay: runsPerDay.trim() ? Number(runsPerDay) : null,
    breaker: breakerFires.trim()
      ? { fires: Number(breakerFires), windowMs: Math.round(Number(breakerWindow.trim() || "10") * 60_000) }
      : null,
    routedSessionTtlMs: ttlDays.trim() ? Math.round(Number(ttlDays) * 86_400_000) : null,
    selfConfig,
  });
  const botDirty =
    JSON.stringify(fields()) !==
    JSON.stringify({
      runsPerDay: bot.runsPerDay,
      breaker: bot.breaker,
      routedSessionTtlMs: bot.routedSessionTtlMs,
      selfConfig: bot.selfConfig,
    });
  const problem = ttlDays.trim() && !(Number(ttlDays) >= 1) ? "Thread retention is at least one day." : null;

  const save = useMutation({
    mutationFn: () =>
      api<{ bot: Bot }>("PATCH", `/api/v1/universes/${universeId}/bots/${bot.botId}`, fields()),
    onSuccess: async ({ bot: updated }) => {
      queryClient.setQueryData(["bot", universeId, bot.botId], { bot: updated });
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["bots", universeId] }),
        queryClient.invalidateQueries({ queryKey: ["bot-state", universeId, bot.botId] }),
      ]);
    },
  });
  const closed = bot.closedAt !== null;
  const readOnly = !manage || closed;
  const usedToday = (state?.runsToday ?? 0) + (state?.descendantsToday ?? 0);

  return (
    <SetupSection
      id="guardrails"
      title="Guardrails"
      description="Limits and permissions."
      summary={guardrailsSummary(bot)}
    >
      <div className="grid gap-4 sm:grid-cols-2">
        <Field>
          <FieldLabel htmlFor="bot-runs-per-day">Daily run limit</FieldLabel>
          <Input
            id="bot-runs-per-day"
            type="number"
            min={1}
            value={runsPerDay}
            onChange={(event) => setRunsPerDay(event.target.value)}
            placeholder="No limit"
            disabled={readOnly}
          />
          <FieldDescription>
            {state ? `${usedToday} used today. ` : ""}Runs and sub-agents count; events beyond the limit wait for the
            next UTC day.
          </FieldDescription>
        </Field>
        <Field>
          <FieldLabel htmlFor="bot-ttl-days">Thread retention (days)</FieldLabel>
          <Input
            id="bot-ttl-days"
            type="number"
            min={1}
            value={ttlDays}
            onChange={(event) => setTtlDays(event.target.value)}
            placeholder="Keep forever"
            disabled={readOnly}
          />
          <FieldDescription>
            Threads idle this long are closed; a later event for the same key opens a fresh one. Triggers can
            override it.
          </FieldDescription>
        </Field>
        <div className="grid gap-3 sm:col-span-2 sm:grid-cols-[1fr_1fr]">
          <Field>
            <FieldLabel htmlFor="bot-breaker-fires">Flood protection: events</FieldLabel>
            <Input
              id="bot-breaker-fires"
              type="number"
              min={1}
              value={breakerFires}
              onChange={(event) => setBreakerFires(event.target.value)}
              placeholder="Off"
              disabled={readOnly}
            />
          </Field>
          <Field>
            <FieldLabel htmlFor="bot-breaker-window">per window (minutes)</FieldLabel>
            <Input
              id="bot-breaker-window"
              type="number"
              min={1}
              value={breakerWindow}
              onChange={(event) => setBreakerWindow(event.target.value)}
              placeholder="10"
              disabled={readOnly}
            />
          </Field>
          <p className="-mt-1 text-xs text-muted-foreground sm:col-span-2">
            A trigger exceeding this rate pauses itself until someone resumes it.
          </p>
        </div>
      </div>
      <div className="grid gap-2">
        <ToggleRow
          id="bot-self-config"
          label="Can change its own brief and triggers"
          hint="Ask it in a conversation to add a schedule or rewrite its job. Off: it can only look."
          checked={selfConfig}
          onCheckedChange={setSelfConfig}
          disabled={readOnly}
        />
      </div>
      {manage && (
        <SaveRow
          dirty={botDirty}
          pending={save.isPending}
          error={problem ?? save.error?.message}
          disabled={closed || problem !== null}
          onSave={() => save.mutate()}
          note="Grants change the bot's toolset; its sessions pick that up at their next idle moment."
        />
      )}
    </SetupSection>
  );
}

type InboxMode = "off" | "any" | "selected";

/**
 * Talking to other bots, both directions in one place: sending is a grant
 * on this bot (`emit`); receiving is its inbox — a trigger under the hood,
 * with the routing and batching of one, shown here in the person's words.
 */
function OtherBotsSection({ bot, manage, inbox }: { bot: Bot; manage: boolean; inbox: BotTrigger | undefined }) {
  const queryClient = useQueryClient();
  const universeId = bot.universeId;
  const bots = useQuery({
    queryKey: ["bots", universeId],
    queryFn: () => api<{ bots: BotListItem[] }>("GET", `/api/v1/universes/${universeId}/bots`),
  });
  const inboxSpec = inbox?.spec as BotInboxSpec | undefined;
  // A paused inbox reads as "Nobody" but keeps its sender list and routing,
  // so switching receiving back on restores them.
  const paused = inbox !== undefined && !inbox.enabled;
  const baseMode: InboxMode = !inbox || paused ? "off" : inboxSpec?.from === undefined ? "any" : "selected";
  const baseIds = inboxSpec?.from ?? [];
  const [emit, setEmit] = useState(bot.emit);
  const [mode, setMode] = useState<InboxMode>(baseMode);
  const [ids, setIds] = useState<string[]>(baseIds);
  const [editingInbox, setEditingInbox] = useState(false);
  useEffect(() => setEmit(bot.emit), [bot.emit]);
  useEffect(() => {
    setMode(baseMode);
    setIds(baseIds);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inbox?.name, baseMode, JSON.stringify(baseIds)]);

  const emitDirty = emit !== bot.emit;
  const inboxDirty =
    mode !== baseMode || (mode === "selected" && JSON.stringify([...ids].sort()) !== JSON.stringify([...baseIds].sort()));
  const problem = mode === "selected" && ids.length === 0 ? "Choose at least one bot, or allow any bot." : null;
  const save = useMutation({
    mutationFn: async () => {
      if (emitDirty) {
        await api<{ bot: Bot }>("PATCH", `/api/v1/universes/${universeId}/bots/${bot.botId}`, { emit });
      }
      if (inboxDirty) {
        const url = `/api/v1/universes/${universeId}/bots/${bot.botId}/triggers`;
        if (mode === "off") {
          // Pause, never delete: the sender list and routing survive.
          if (inbox && inbox.enabled) await api("PATCH", `${url}/${inbox.name}`, { enabled: false });
        } else {
          const spec = inboxSelectionSpec(mode === "any" ? "any" : "selected", ids);
          if (inbox) await api("PATCH", `${url}/${inbox.name}`, { spec, enabled: true });
          else await api("POST", url, { name: "inbox", kind: "bot", spec });
        }
      }
    },
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["bot", universeId, bot.botId] }),
        queryClient.invalidateQueries({ queryKey: ["bots", universeId] }),
        queryClient.invalidateQueries({ queryKey: ["bot-state", universeId, bot.botId] }),
        queryClient.invalidateQueries({ queryKey: ["bot-triggers", universeId, bot.botId] }),
      ]);
    },
  });
  const closed = bot.closedAt !== null;
  const readOnly = !manage || closed;
  const others = (bots.data?.bots ?? []).filter((other) => other.botId !== bot.botId && !other.closedAt);

  return (
    <SetupSection
      id="other-bots"
      title="Other bots"
      description="Bots in this universe can message each other; every message is an event, and each side decides for itself."
      summary={otherBotsSummary(bot.emit, baseMode === "off" ? "off" : baseMode === "any" ? "any" : baseIds)}
    >
      <ToggleRow
        id="bot-emit"
        label="Can message other bots"
        hint="Sees which bots accept it and addresses them by id; may also post to itself. Rate-capped, and a message travels at most a few hops. Turning this on also opens the inbox — sending without listening is the rare case."
        checked={emit}
        onCheckedChange={(checked) => {
          setEmit(checked);
          if (checked && mode === "off") setMode("any");
        }}
        disabled={readOnly}
      />
      <div className="grid gap-2 rounded-md border p-3">
        <div className="flex items-center justify-between gap-3">
          <Label htmlFor="bot-inbox-mode" className="text-sm">
            Accepts messages from
            <span className="block text-xs font-normal text-muted-foreground">
              {others.length === 0
                ? "There are no other bots in this universe yet."
                : "Which bots may address this one. Messages from them arrive like any other event."}
            </span>
          </Label>
          <Select value={mode} onValueChange={(value) => value && setMode(value as InboxMode)} disabled={readOnly}>
            <SelectTrigger id="bot-inbox-mode" size="sm" className="w-40">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="off">Nobody</SelectItem>
              <SelectItem value="any">Any bot here</SelectItem>
              <SelectItem value="selected">Only these bots</SelectItem>
            </SelectContent>
          </Select>
        </div>
        {mode === "selected" && (
          <BotMultiSelect currentBotId={bot.botId} bots={bots.data?.bots ?? []} value={ids} onChange={setIds} />
        )}
        {mode !== "off" && manage && (
          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={!inbox || inboxDirty}
              onClick={() => setEditingInbox(true)}
              title={!inbox || inboxDirty ? "Save first, then set routing and batching." : undefined}
            >
              <SlidersHorizontal data-icon="inline-start" /> Routing &amp; batching…
            </Button>
            <span className="text-xs text-muted-foreground">
              {!inbox || inboxDirty
                ? "Available after you save."
                : inbox.route?.policy === "perKey"
                  ? "One thread per key."
                  : inbox.route?.policy === "perEvent"
                    ? "One thread per message."
                    : "Messages arrive in Main."}
            </span>
          </div>
        )}
        {mode === "off" && paused && (
          <p className="text-xs text-muted-foreground">
            Inbox paused{inbox?.disabledReason === "breaker" ? " by flood protection" : ""}; its sender list and
            routing are kept.
          </p>
        )}
      </div>
      {manage && (
        <SaveRow
          dirty={emitDirty || inboxDirty}
          pending={save.isPending}
          error={problem ?? save.error?.message}
          disabled={closed || problem !== null}
          onSave={() => save.mutate()}
          note="Sending is a tool grant, picked up at the bot's next idle moment; receiving applies to the next message."
        />
      )}
      {manage && inbox && editingInbox && (
        <EditTriggerDialog
          universeId={universeId}
          botId={bot.botId}
          bots={bots.data?.bots ?? []}
          trigger={inbox}
          open
          deliveryOnly
          onOpenChange={(open) => {
            if (!open) setEditingInbox(false);
          }}
        />
      )}
    </SetupSection>
  );
}

function ToggleRow({
  id,
  label,
  hint,
  checked,
  onCheckedChange,
  disabled,
}: {
  id: string;
  label: string;
  hint: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-md border p-3">
      <Label htmlFor={id} className="text-sm">
        {label}
        <span className="block text-xs font-normal text-muted-foreground">{hint}</span>
      </Label>
      <Switch id={id} checked={checked} onCheckedChange={onCheckedChange} disabled={disabled} />
    </div>
  );
}

function DangerSection({ slug, bot, manage }: { slug: string; bot: Bot; manage: boolean }) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const universeId = bot.universeId;
  const closed = bot.closedAt !== null;
  const close = useMutation({
    mutationFn: () =>
      api<{ bot: Bot; completed: boolean }>("POST", `/api/v1/universes/${universeId}/bots/${bot.botId}/close`),
    onSuccess: async ({ bot: updated }) => {
      queryClient.setQueryData(["bot", universeId, bot.botId], { bot: updated });
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["bots", universeId] }),
        queryClient.invalidateQueries({ queryKey: ["bot-state", universeId, bot.botId] }),
      ]);
    },
  });
  const remove = useMutation({
    mutationFn: () => api<{ deleted: boolean }>("DELETE", `/api/v1/universes/${universeId}/bots/${bot.botId}`),
    onSuccess: async () => {
      queryClient.removeQueries({ queryKey: ["bot", universeId, bot.botId] });
      await queryClient.invalidateQueries({ queryKey: ["bots", universeId] });
      navigate(`/u/${slug}/bots`);
    },
  });
  if (!manage) return null;
  return (
    <SetupSection
      id="danger"
      title="Danger zone"
      description="Pausing is reversible and lives in the header. These are not."
      summary={closed ? `Closed ${new Date(bot.closedAt ?? "").toLocaleString()} · delete to free the id` : "Close or delete this bot"}
      tone="danger"
    >
      {closed ? (
        <p className="text-xs text-muted-foreground">
          Closed {new Date(bot.closedAt ?? "").toLocaleString()}: conversations and schedules were released and
          events are refused. The record and its history stay until the bot is deleted.
        </p>
      ) : (
        <div className="flex items-start justify-between gap-3">
          <p className="text-xs text-muted-foreground">
            <b className="font-medium text-foreground">Close</b> is final: in-flight runs are cancelled, every
            conversation is closed, schedules are dropped, and new events are refused. The record, its history, the
            id, and its environment stay.
          </p>
          <AlertDialog>
            <AlertDialogTrigger render={<Button variant="outline" size="sm" disabled={close.isPending} />}>
              {close.isPending ? "Closing…" : "Close bot"}
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>Close {botLabel(bot)}?</AlertDialogTitle>
                <AlertDialogDescription>
                  This cannot be undone. Pending events are archived, active runs are cancelled, all conversations
                  are closed, and webhooks and other bots are refused from now on. The environment is left alone.
                  To pause instead, use Pause in the header.
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
          <b className="font-medium text-foreground">Delete</b> erases the bot, its triggers, its event history, and
          its conversations, and frees the id{closed ? "." : " — it closes the bot first."} Environments and
          profiles are never deleted with a bot.
        </p>
        <AlertDialog>
          <AlertDialogTrigger render={<Button variant="destructive" size="sm" disabled={remove.isPending} />}>
            {remove.isPending ? "Deleting…" : "Delete bot"}
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Delete {botLabel(bot)}?</AlertDialogTitle>
              <AlertDialogDescription>
                {closed
                  ? "The record, its event history, and its conversations are erased; the id becomes available again."
                  : "The bot is closed first (runs cancelled, conversations closed, events refused), then the record, its event history, and its conversations are erased and the id becomes available again."}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Keep</AlertDialogCancel>
              <AlertDialogAction onClick={() => remove.mutate()}>Delete bot</AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
      {(close.error || remove.error) && (
        <p className="text-xs text-destructive">{close.error?.message ?? remove.error?.message}</p>
      )}
    </SetupSection>
  );
}
