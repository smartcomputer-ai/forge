import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Check, ChevronDown, ChevronRight, Trash2 } from "lucide-react";
import { NavLink, useNavigate } from "react-router-dom";
import {
  api,
  type Bot,
  type Environment,
  type ProfileEnvironment,
  type ProfileSummary,
} from "@/api";
import { BotAvatar, botColor } from "@/components/bot/face";
import { botIdFrom } from "@/components/bot/identity";
import { capabilitySummary, otherBotsSummary } from "@/components/bot/setup-summary";
import { BOT_TEMPLATES, type BotTemplate } from "@/components/bot/templates";
import { describeCron } from "@/components/bot/trigger-summary";
import {
  BotMultiSelect,
  NAME_PATTERN,
  TriggerKindFields,
  TriggerKindIcon,
  TriggerKindPicker,
  defaultPollForm,
  defaultTriggerForms,
  defaultTriggerName,
  triggerCreateBody,
  triggerFormProblem,
  type BotEnvStatus,
  type TriggerForms,
  type TriggerKind,
} from "@/components/bot/triggers";
import { ProviderReadinessBanner } from "@/components/provider-readiness-banner";
import { ProfileEnvironmentEditor } from "@/components/session/profile-environment-editor";
import { SessionConfigEditor } from "@/components/session/session-config-editor";
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
import { LoadingNote, UniverseNotFound } from "@/components/page";
import { useSecretsInventory } from "@/lib/environment-credentials";
import { useSessionConfigEditorOptions } from "@/lib/sessions/editor-options";
import { hasSessionFeature, setupResourceFeatureError } from "@/lib/sessions/resource-features";
import { canManage, useActiveUniverse } from "@/lib/universes";
import { cn } from "@/lib/utils";

type Step = "job" | "wakeups" | "bots" | "capabilities" | "guardrails";
/** The same sections as the bot's Setup tab, in the same order. */
const STEPS: Array<{ id: Step; label: string }> = [
  { id: "job", label: "Job" },
  { id: "wakeups", label: "Triggers" },
  { id: "bots", label: "Other bots" },
  { id: "capabilities", label: "Capabilities" },
  { id: "guardrails", label: "Guardrails" },
];

interface WakeupDraft {
  key: string;
  kind: TriggerKind;
  name: string;
  forms: TriggerForms;
}

export { botIdFrom };

export function uniqueTriggerName(base: string, taken: string[]): string {
  if (!taken.includes(base)) return base;
  let index = 2;
  while (taken.includes(`${base}-${index}`)) index += 1;
  return `${base}-${index}`;
}

/** Plain words for a trigger still being drafted, from its form. */
export function wakeupSummary(draft: WakeupDraft): string {
  switch (draft.kind) {
    case "schedule": {
      const form = draft.forms.schedule;
      if (form.once) return form.at ? `Once, at ${new Date(form.at).toLocaleString()}` : "Once — pick when";
      return describeCron(form.cron, form.timezone);
    }
    case "webhook": {
      const form = draft.forms.webhook;
      return `${form.preset ? "GitHub webhook" : "Webhook"} · ${form.scheme === "token" ? "URL token" : "signed"}`;
    }
    case "poll": {
      const form = draft.forms.poll;
      const every = `every ${form.intervalMinutes || "?"} min`;
      return form.sourceKind === "http"
        ? form.url
          ? `Checks ${form.url} ${every}`
          : "Check a URL — enter it"
        : form.argvText.trim()
          ? `Runs ${form.argvText.trim().split(/\n/)[0]} ${every}`
          : "Run a command — enter it";
    }
    case "chat":
      return draft.forms.chat.channelAccountId ? "Messages on the chosen account" : "Chat — pick an account";
    case "bot": {
      const form = draft.forms.inbox;
      return form.fromMode === "any" ? "Messages from any bot" : `Messages from ${form.fromBotIds.join(", ") || "…"}`;
    }
  }
}

/** What a template comes with, in a few words: its triggers, then its capabilities. */
export function templateHighlights(template: BotTemplate): string[] {
  const wakeups = template.triggers.map((trigger) => {
    switch (trigger.kind) {
      case "schedule":
        return describeCron(trigger.cron, trigger.timezone === "UTC" ? null : trigger.timezone);
      case "webhook":
        return trigger.preset === "github" ? "GitHub events" : "Webhook";
      case "chat":
        return "Chat messages";
      case "bot":
        return "Other bots";
    }
  });
  return [...wakeups, ...capabilitySummary({ features: template.features })];
}

export function BotCreatePage({ admin }: { admin: boolean }) {
  const { universe, slug, isLoading } = useActiveUniverse();
  if (isLoading) return <LoadingNote />;
  if (!universe || !canManage(universe, admin)) {
    return (
      <div className="p-6">
        <UniverseNotFound slug={slug} />
      </div>
    );
  }
  return (
    <div className="min-h-0 min-w-0 flex-1 overflow-y-auto">
      <Wizard universeId={universe.id} slug={slug!} />
    </div>
  );
}

function Wizard({ universeId, slug }: { universeId: string; slug: string }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [step, setStep] = useState<Step>("job");
  const [templateId, setTemplateId] = useState("blank");
  // The last name a template suggested: a person's own name is never
  // overwritten, a suggestion is replaced by the next template's.
  const [suggestedName, setSuggestedName] = useState<string | null>(null);
  const [displayName, setDisplayName] = useState("");
  const [botId, setBotId] = useState("");
  const [idTouched, setIdTouched] = useState(false);
  const [description, setDescription] = useState("");
  const [brief, setBrief] = useState("");
  const [wakeups, setWakeups] = useState<WakeupDraft[]>([]);
  const [openWakeup, setOpenWakeup] = useState<string | null>(null);
  const [setupMode, setSetupMode] = useState<"own" | "shared">("own");
  const [sharedProfileId, setSharedProfileId] = useState("");
  const [config, setConfig] = useState<Record<string, unknown> | undefined>(undefined);
  const [configError, setConfigError] = useState<string | null>(null);
  const [environment, setEnvironment] = useState<ProfileEnvironment | undefined>(undefined);
  const [runsPerDay, setRunsPerDay] = useState("50");
  const [selfConfig, setSelfConfig] = useState(true);
  const [emit, setEmit] = useState(false);
  // The inbox is a trigger draft like any other, but it is driven from the
  // "Other bots" block, not the trigger picker, so send and receive sit together.
  const inboxDraft = wakeups.find((draft) => draft.kind === "bot");
  const inboxMode: "off" | "any" | "selected" =
    !inboxDraft ? "off" : inboxDraft.forms.inbox.fromMode === "any" ? "any" : "selected";
  const inboxIds = inboxDraft?.forms.inbox.fromBotIds ?? [];
  const setInbox = (mode: "off" | "any" | "selected", ids: string[] = inboxIds) => {
    setWakeups((current) => {
      const without = current.filter((draft) => draft.kind !== "bot");
      if (mode === "off") return without;
      const existing = current.find((draft) => draft.kind === "bot");
      const forms = existing?.forms ?? defaultTriggerForms();
      const draft: WakeupDraft = {
        key: existing?.key ?? crypto.randomUUID(),
        kind: "bot",
        name: existing?.name ?? uniqueTriggerName("inbox", without.map((entry) => entry.name)),
        forms: { ...forms, inbox: { ...forms.inbox, fromMode: mode === "any" ? "any" : "selected", fromBotIds: ids } },
      };
      return [...without, draft];
    });
  };
  const visibleWakeups = wakeups.filter((draft) => draft.kind !== "bot");

  const profiles = useQuery({
    queryKey: ["profiles", universeId],
    queryFn: () => api<ProfileSummary[]>("GET", `/api/v1/universes/${universeId}/profiles`),
  });
  const bots = useQuery({
    queryKey: ["bots", universeId],
    queryFn: () => api<{ bots: Bot[] }>("GET", `/api/v1/universes/${universeId}/bots`),
  });
  const options = useSessionConfigEditorOptions(universeId, step === "capabilities" || step === "wakeups");
  const environments = useQuery({
    queryKey: ["environments", universeId],
    queryFn: () => api<Environment[]>("GET", `/api/v1/universes/${universeId}/environments`),
    enabled: step === "capabilities" || step === "wakeups",
  });
  const secrets = useSecretsInventory(universeId, step === "capabilities");

  const env: BotEnvStatus =
    setupMode === "shared"
      ? { kind: "unknown" }
      : environment?.type === "existing"
        ? { kind: "existing", environmentId: environment.environmentId }
        : environment?.type === "provision"
          ? { kind: "provision" }
          : { kind: "none" };

  const applyTemplate = (template: BotTemplate) => {
    setTemplateId(template.id);
    // A suggested name is replaced by the next template's, and Blank clears
    // it — a name the person typed themselves is never touched.
    if (displayName.trim() === "" || displayName === suggestedName) {
      setDisplayName(template.suggestedName ?? "");
      if (!idTouched) setBotId(template.suggestedName ? botIdFrom(template.suggestedName) : "");
    }
    setSuggestedName(template.suggestedName);
    setBrief(template.brief);
    setConfig(Object.keys(template.features).length > 0 ? { features: structuredClone(template.features) } : undefined);
    setRunsPerDay(template.runsPerDay === null ? "" : String(template.runsPerDay));
    setSelfConfig(template.selfConfig);
    setEmit(template.emit);
    const drafts: WakeupDraft[] = [];
    for (const trigger of template.triggers) {
      const forms = defaultTriggerForms();
      if (trigger.kind === "schedule") {
        forms.schedule = { ...forms.schedule, once: false, cron: trigger.cron, timezone: trigger.timezone, summary: trigger.summary };
      } else if (trigger.kind === "webhook") {
        forms.webhook = {
          ...forms.webhook,
          preset: trigger.preset === "github",
          routePolicy: trigger.perKey ? "perKey" : "bot",
          ...(trigger.filter ? { filter: trigger.filter } : {}),
        };
      }
      drafts.push({ key: crypto.randomUUID(), kind: trigger.kind, name: trigger.name, forms });
    }
    setWakeups(drafts);
    setOpenWakeup(null);
  };

  const idInvalid = botId.trim().length > 0 && !NAME_PATTERN.test(botId.trim());
  const idTaken = bots.data?.bots.some((bot) => bot.botId === botId.trim()) ?? false;
  const profileTaken =
    setupMode === "own" && (profiles.data?.some((profile) => profile.profileId === botId.trim()) ?? false);
  const wakeupProblems = wakeups.map((draft) => ({
    key: draft.key,
    problem:
      !draft.name.trim() || !NAME_PATTERN.test(draft.name.trim())
        ? "Give it a name in lowercase letters, numbers, and dashes."
        : wakeups.filter((other) => other.name.trim() === draft.name.trim()).length > 1
          ? "Two triggers share this name."
          : triggerFormProblem(draft.kind, draft.forms, env),
  }));
  const setupProblem =
    setupMode === "own"
      ? (configError ? `Capabilities: ${configError}` : setupResourceFeatureError({ config, environment }))
      : sharedProfileId
        ? null
        : "Pick the shared profile this bot applies.";
  const problems = [
    ...(botId.trim() ? [] : ["Give the bot a name."]),
    ...(idInvalid ? ["The id uses lowercase letters, numbers, and dashes."] : []),
    ...(idTaken ? [`A bot named ${botId.trim()} already exists.`] : []),
    ...(profileTaken ? [`A profile named ${botId.trim()} already exists — pick another id, or use it as a shared profile.`] : []),
    ...wakeupProblems.filter((entry) => entry.problem).map((entry) => `Trigger: ${entry.problem}`),
    ...(setupProblem ? [setupProblem] : []),
    ...(runsPerDay.trim() && !(Number(runsPerDay) >= 1) ? ["The daily run limit is at least 1."] : []),
  ];

  const create = useMutation({
    mutationFn: async () => {
      const id = botId.trim();
      let profileId = sharedProfileId;
      if (setupMode === "own") {
        profileId = id;
        await api("PUT", `/api/v1/universes/${universeId}/profiles/${encodeURIComponent(profileId)}`, {
          profileId,
          displayName: displayName.trim() || id,
          description: `Setup of bot ${id}`,
          ...(config ? { config } : {}),
          ...(environment ? { environment } : {}),
        });
      }
      try {
        const { bot } = await api<{ bot: Bot }>("POST", `/api/v1/universes/${universeId}/bots`, {
          botId: id,
          ...(displayName.trim() ? { displayName: displayName.trim() } : {}),
          ...(description.trim() ? { description: description.trim() } : {}),
          profileId,
          ...(brief.trim() ? { brief: brief.trim() } : {}),
          ...(runsPerDay.trim() ? { runsPerDay: Number(runsPerDay) } : {}),
          selfConfig,
          emit,
          triggers: wakeups.map((draft) => triggerCreateBody(draft.kind, draft.name.trim(), draft.forms)),
        });
        return bot;
      } catch (error) {
        if (setupMode === "own") {
          await api("DELETE", `/api/v1/universes/${universeId}/profiles/${encodeURIComponent(profileId)}`).catch(
            () => undefined,
          );
        }
        throw error;
      }
    },
    onSuccess: async (bot) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["bots", universeId] }),
        queryClient.invalidateQueries({ queryKey: ["profiles", universeId] }),
      ]);
      navigate(`/u/${slug}/bots/${bot.botId}`, { state: { introduce: true } });
    },
  });

  const stepIndex = STEPS.findIndex((entry) => entry.id === step);
  const next = () => setStep(STEPS[Math.min(stepIndex + 1, STEPS.length - 1)]!.id);
  const back = () => setStep(STEPS[Math.max(stepIndex - 1, 0)]!.id);
  const label = displayName.trim() || botId.trim() || "your bot";

  const addWakeup = (kind: TriggerKind, pick?: { pollSource: "http" | "exec" }) => {
    const forms = defaultTriggerForms();
    if (kind === "poll") {
      forms.poll = {
        ...defaultPollForm,
        sourceKind: pick?.pollSource ?? "http",
        environmentId: pick?.pollSource === "exec" && env.kind === "existing" ? env.environmentId : "",
      };
    }
    const draft: WakeupDraft = {
      key: crypto.randomUUID(),
      kind,
      name: uniqueTriggerName(defaultTriggerName(kind, pick?.pollSource), wakeups.map((entry) => entry.name)),
      forms,
    };
    setWakeups((current) => [...current, draft]);
    setOpenWakeup(draft.key);
  };
  const updateWakeup = (key: string, mutate: (draft: WakeupDraft) => WakeupDraft) =>
    setWakeups((current) => current.map((draft) => (draft.key === key ? mutate(draft) : draft)));

  return (
    <div className="mx-auto grid w-full max-w-5xl gap-6 px-4 py-6 md:grid-cols-[minmax(0,1fr)_17rem] md:px-8">
      <aside className="grid content-start gap-4 md:sticky md:top-6 md:order-2 md:self-start">
        <NavLink to={`/u/${slug}/bots`} className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground">
          <ArrowLeft className="size-3.5" /> Bots
        </NavLink>
        <ol className="flex flex-wrap gap-x-4 gap-y-2 md:grid md:gap-2">
          {STEPS.map((entry, index) => {
            const done = index < stepIndex;
            const current = entry.id === step;
            return (
              <li key={entry.id}>
                <button
                  type="button"
                  onClick={() => setStep(entry.id)}
                  className={cn(
                    "flex items-center gap-2 text-sm",
                    current ? "font-semibold text-foreground" : "text-muted-foreground hover:text-foreground",
                  )}
                >
                  <span
                    className={cn(
                      "grid size-5 place-items-center rounded-full border text-[10px] font-mono",
                      done && "border-emerald-600 bg-emerald-600 text-white",
                      current && "border-primary text-primary",
                    )}
                  >
                    {done ? <Check className="size-3" /> : index + 1}
                  </span>
                  {entry.label}
                </button>
              </li>
            );
          })}
        </ol>
        <div className="grid gap-3 rounded-lg border bg-muted/30 p-4 text-xs">
          <div className="flex items-center gap-2">
            <BotAvatar
              botId={botId.trim() || "new-bot"}
              size={32}
              className={cn(!botId.trim() && "text-muted-foreground")}
              color={botId.trim() ? undefined : "var(--muted)"}
            />
            <div className="min-w-0">
              <div className="truncate text-sm font-semibold">{displayName.trim() || "Unnamed bot"}</div>
              <div className="truncate font-mono text-muted-foreground">{botId.trim() || "id"}</div>
            </div>
          </div>
          <SummaryRow label="Job">{brief.trim() ? brief.trim().slice(0, 140) + (brief.trim().length > 140 ? "…" : "") : <em>not written yet</em>}</SummaryRow>
          <SummaryRow label="Triggers">
            {visibleWakeups.length === 0 ? <em>only when you message it</em> : visibleWakeups.map((draft) => <span key={draft.key} className="block truncate">{wakeupSummary(draft)}</span>)}
          </SummaryRow>
          {(emit || inboxMode !== "off") && (
            <SummaryRow label="Other bots">
              {otherBotsSummary(emit, inboxMode === "off" ? "off" : inboxMode === "any" ? "any" : inboxIds)}
            </SummaryRow>
          )}
          <SummaryRow label="Can use">
            {setupMode === "shared"
              ? sharedProfileId
                ? `profile ${sharedProfileId}`
                : <em>pick a profile</em>
              : capabilitySummary(config).length > 0
                ? capabilitySummary(config).join(" · ")
                : <em>the default model, no tools</em>}
          </SummaryRow>
          <SummaryRow label="Works in">
            {setupMode === "shared"
              ? <em>per the profile</em>
              : environment?.type === "existing"
                ? (environments.data?.find((entry) => entry.environmentId === environment.environmentId)?.displayName ?? environment.environmentId)
                : environment?.type === "provision"
                  ? "a fresh environment per session"
                  : <em>no environment</em>}
          </SummaryRow>
          <SummaryRow label="Limits">
            {runsPerDay.trim() ? `${runsPerDay} runs a day` : "no daily limit"}
            {selfConfig ? " · can change its own triggers" : ""}
          </SummaryRow>
          {problems.length > 0 && (
            <ul className="grid gap-1 border-t pt-3 text-amber-700 dark:text-amber-400">
              {problems.slice(0, 4).map((problem) => (
                <li key={problem}>{problem}</li>
              ))}
            </ul>
          )}
          {create.error && <p className="border-t pt-3 text-destructive">{create.error.message}</p>}
          <Button
            size="sm"
            className="justify-self-start"
            onClick={() => create.mutate()}
            disabled={create.isPending || problems.length > 0}
          >
            {create.isPending ? "Creating…" : `Create ${label}`}
          </Button>
        </div>
      </aside>

      <main className="grid min-w-0 content-start gap-6 md:order-1">
        {step === "job" && (
          <section className="grid gap-5">
            <header className="grid gap-1">
              <h1 className="text-xl font-semibold tracking-tight">What is this bot's job?</h1>
              <p className="text-sm text-muted-foreground">Start from a template or from scratch; everything stays editable later.</p>
            </header>
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              {BOT_TEMPLATES.map((template) => (
                <TemplateCard
                  key={template.id}
                  template={template}
                  selected={templateId === template.id}
                  onSelect={() => applyTemplate(template)}
                />
              ))}
            </div>
            <div className="grid gap-4 sm:grid-cols-2">
              <Field>
                <FieldLabel htmlFor="new-bot-name">Name</FieldLabel>
                <Input
                  id="new-bot-name"
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
                <FieldLabel htmlFor="new-bot-id">Id</FieldLabel>
                <Input
                  id="new-bot-id"
                  value={botId}
                  onChange={(event) => {
                    setBotId(event.target.value);
                    setIdTouched(event.target.value.length > 0);
                  }}
                  placeholder="triage"
                  className="font-mono"
                  aria-invalid={idInvalid || idTaken || undefined}
                />
                <FieldDescription>
                  {idTaken
                    ? "Taken by another bot."
                    : "What other bots, briefs, and URLs use — it cannot change later."}
                </FieldDescription>
              </Field>
            </div>
            <Field>
              <FieldLabel htmlFor="new-bot-brief">Brief</FieldLabel>
              <Textarea
                id="new-bot-brief"
                value={brief}
                onChange={(event) => setBrief(event.target.value)}
                rows={9}
                placeholder="What this bot is for, how it should behave, what good work looks like. It reads this with every event."
                className="leading-relaxed"
              />
            </Field>
            <Field>
              <FieldLabel htmlFor="new-bot-description">Description for other bots (optional)</FieldLabel>
              <Input
                id="new-bot-description"
                value={description}
                onChange={(event) => setDescription(event.target.value)}
                placeholder="One line other bots read when deciding whether to message this bot."
              />
            </Field>
          </section>
        )}

        {step === "wakeups" && (
          <section className="grid gap-5">
            <header className="grid gap-1">
              <h1 className="text-xl font-semibold tracking-tight">When should {label} wake up?</h1>
              <p className="text-sm text-muted-foreground">
                Pick any that apply. You can always message it from Chat, and add more triggers later
                {selfConfig ? " — or ask it to add them itself" : ""}.
              </p>
            </header>
            <TriggerKindPicker env={env} onPick={addWakeup} exclude={["bot"]} />
            {visibleWakeups.length > 0 && (
              <div className="grid gap-2">
                {visibleWakeups.map((draft) => {
                  const problem = wakeupProblems.find((entry) => entry.key === draft.key)?.problem ?? null;
                  const open = openWakeup === draft.key;
                  return (
                    <div key={draft.key} className={cn("rounded-md border", problem && "border-amber-500/60")}>
                      <div className="flex items-center gap-2 px-3 py-2 text-sm">
                        <button
                          type="button"
                          className="flex min-w-0 flex-1 items-center gap-2 text-left"
                          onClick={() => setOpenWakeup(open ? null : draft.key)}
                          aria-expanded={open}
                        >
                          {open ? <ChevronDown className="size-4 text-muted-foreground" /> : <ChevronRight className="size-4 text-muted-foreground" />}
                          <TriggerKindIcon kind={draft.kind} exec={draft.forms.poll.sourceKind === "exec"} />
                          <span className="min-w-0">
                            <span className="block truncate font-medium">{draft.name}</span>
                            <span className={cn("block truncate text-xs text-muted-foreground", problem && "text-amber-700 dark:text-amber-400")}>
                              {problem ?? wakeupSummary(draft)}
                            </span>
                          </span>
                        </button>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label="Remove trigger"
                          onClick={() => setWakeups((current) => current.filter((entry) => entry.key !== draft.key))}
                        >
                          <Trash2 />
                        </Button>
                      </div>
                      {open && (
                        <div className="grid gap-4 border-t p-3">
                          <Field>
                            <FieldLabel htmlFor={`wakeup-name-${draft.key}`}>Name</FieldLabel>
                            <Input
                              id={`wakeup-name-${draft.key}`}
                              value={draft.name}
                              onChange={(event) => updateWakeup(draft.key, (current) => ({ ...current, name: event.target.value }))}
                              className="font-mono"
                            />
                          </Field>
                          <TriggerKindFields
                            kind={draft.kind}
                            forms={draft.forms}
                            patch={(key, value) =>
                              updateWakeup(draft.key, (current) => ({ ...current, forms: { ...current.forms, [key]: value } }))
                            }
                            botId={botId.trim() || "new-bot"}
                            bots={(bots.data?.bots ?? []).map((bot) => ({ ...bot, triggerCount: 0, pendingCount: 0, lastEvent: null }))}
                          />
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </section>
        )}

        {step === "bots" && (
          <section className="grid gap-5">
            <header className="grid gap-1">
              <h1 className="text-xl font-semibold tracking-tight">Should {label} talk to other bots?</h1>
              <p className="text-sm text-muted-foreground">
                Bots in this universe can message each other; every message is an event, and each side decides for
                itself. Skip this if {label} works alone.
              </p>
            </header>
            <div className="grid gap-2">
              <div className="flex items-center justify-between gap-3 rounded-md border p-3">
                <Label htmlFor="new-bot-emit" className="text-sm">
                  Can message other bots
                  <span className="block text-xs font-normal text-muted-foreground">
                    Sees which bots accept it and addresses them by id; rate-capped. Turning this on also opens the inbox —
                    sending without listening is the rare case.
                  </span>
                </Label>
                <Switch
                  id="new-bot-emit"
                  checked={emit}
                  onCheckedChange={(checked) => {
                    setEmit(checked);
                    if (checked && inboxMode === "off") setInbox("any");
                  }}
                />
              </div>
              <div className="grid gap-2 rounded-md border p-3">
                <div className="flex items-center justify-between gap-3">
                  <Label htmlFor="new-bot-inbox" className="text-sm">
                    Accepts messages from
                    <span className="block text-xs font-normal text-muted-foreground">
                      Which bots may address this one. Routing and batching can be tuned under Setup later.
                    </span>
                  </Label>
                  <Select value={inboxMode} onValueChange={(value) => value && setInbox(value as "off" | "any" | "selected")}>
                    <SelectTrigger id="new-bot-inbox" size="sm" className="w-40">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="off">Nobody</SelectItem>
                      <SelectItem value="any">Any bot here</SelectItem>
                      <SelectItem value="selected">Only these bots</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                {inboxMode === "selected" && (
                  <BotMultiSelect
                    currentBotId={botId.trim() || "new-bot"}
                    bots={(bots.data?.bots ?? []).map((bot) => ({ ...bot, triggerCount: 0, pendingCount: 0, lastEvent: null }))}
                    value={inboxIds}
                    onChange={(ids) => setInbox("selected", ids)}
                  />
                )}
              </div>
            </div>
          </section>
        )}

        {step === "capabilities" && (
          <section className="grid gap-5">
            <header className="grid gap-1">
              <h1 className="text-xl font-semibold tracking-tight">What can {label} use?</h1>
              <p className="text-sm text-muted-foreground">
                The model, tools, and environment its sessions get. Saved as a profile named after the bot, editable from its Setup tab.
              </p>
            </header>
            <ProviderReadinessBanner universeId={universeId} slug={slug} />
            <div className="grid gap-2 sm:grid-cols-2">
              <SetupModeChoice
                active={setupMode === "own"}
                title="Its own setup"
                description="A profile named after the bot; edit it from the bot's Setup tab."
                onClick={() => setSetupMode("own")}
              />
              <SetupModeChoice
                active={setupMode === "shared"}
                title="A shared profile"
                description="Apply an existing profile; changes to it reach every bot that uses it."
                onClick={() => setSetupMode("shared")}
              />
            </div>
            {setupMode === "shared" ? (
              <Field>
                <FieldLabel>Profile</FieldLabel>
                <Select value={sharedProfileId} onValueChange={(value) => value && setSharedProfileId(value)}>
                  <SelectTrigger className="w-full sm:w-80">
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
                {profiles.data?.length === 0 && (
                  <FieldDescription>No profiles yet — give the bot its own setup instead.</FieldDescription>
                )}
              </Field>
            ) : (
              <>
                <SessionConfigEditor
                  value={config}
                  mcpServers={options.mcpServers}
                  workspaces={options.workspaces}
                  workspacesLoading={options.workspacesLoading}
                  models={options.models}
                  profiles={options.profiles}
                  environmentProviders={options.environmentProviders}
                  onValidityChange={setConfigError}
                  onChange={(next) => setConfig(next as Record<string, unknown> | undefined)}
                />
                <ProfileEnvironmentEditor
                  value={environment}
                  environments={environments.data}
                  bindings={options.environmentBindings}
                  templates={options.environmentTemplates}
                  secrets={secrets.data}
                  disabled={!hasSessionFeature(config, "environments")}
                  title="Environment"
                  description="Where the bot works: an environment shared across its sessions, or a fresh one per session. Command polls need a lasting one."
                  onChange={setEnvironment}
                />
                {environments.data?.length === 0 && (
                  <p className="text-xs text-muted-foreground">
                    No environments yet — create one under{" "}
                    <NavLink to={`/u/${slug}/settings/environments`} className="underline">
                      Settings › Environments
                    </NavLink>{" "}
                    if the bot needs a machine.
                  </p>
                )}
              </>
            )}
          </section>
        )}

        {step === "guardrails" && (
          <section className="grid gap-5">
            <header className="grid gap-1">
              <h1 className="text-xl font-semibold tracking-tight">Limits and permissions</h1>
              <p className="text-sm text-muted-foreground">Sensible defaults; all of it lives under Setup › Guardrails later.</p>
            </header>
            <Field className="sm:w-64">
              <FieldLabel htmlFor="new-bot-runs">Daily run limit</FieldLabel>
              <Input
                id="new-bot-runs"
                type="number"
                min={1}
                value={runsPerDay}
                onChange={(event) => setRunsPerDay(event.target.value)}
                placeholder="No limit"
              />
              <FieldDescription>Runs and sub-agents count; events beyond it wait for the next UTC day.</FieldDescription>
            </Field>
            <div className="grid gap-2">
              <div className="flex items-center justify-between gap-3 rounded-md border p-3">
                <Label htmlFor="new-bot-self-config" className="text-sm">
                  Can change its own brief and triggers
                  <span className="block text-xs font-normal text-muted-foreground">
                    Ask it in Chat to add a schedule or rewrite its job. Off: it can only look.
                  </span>
                </Label>
                <Switch id="new-bot-self-config" checked={selfConfig} onCheckedChange={setSelfConfig} />
              </div>
            </div>
          </section>
        )}

        <div className="flex items-center justify-between gap-3 border-t pt-4">
          <Button variant="outline" onClick={back} disabled={stepIndex === 0}>
            Back
          </Button>
          {stepIndex < STEPS.length - 1 ? (
            <Button onClick={next}>Next: {STEPS[stepIndex + 1]!.label}</Button>
          ) : (
            <Button onClick={() => create.mutate()} disabled={create.isPending || problems.length > 0}>
              {create.isPending ? "Creating…" : `Create ${label}`}
            </Button>
          )}
        </div>
      </main>
    </div>
  );
}



/**
 * A template is a colleague you could hire, so its card carries a face and a
 * line of what it comes with — heavier than the option cards further on,
 * without shouting.
 */
function TemplateCard({
  template,
  selected,
  onSelect,
}: {
  template: BotTemplate;
  selected: boolean;
  onSelect: () => void;
}) {
  const blank = template.id === "blank";
  const highlights = templateHighlights(template);
  // The face is the bot's, not the template's: colour it by the id the bot
  // will get, so it does not change the moment the bot exists.
  const faceId = template.suggestedName ? botIdFrom(template.suggestedName) : "new-bot";
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      className={cn(
        "grid min-h-32 content-start gap-2.5 rounded-xl border bg-card p-4 text-left shadow-sm transition-all",
        "hover:-translate-y-0.5 hover:shadow-md motion-reduce:transition-none motion-reduce:hover:translate-y-0",
        selected ? "border-primary ring-2 ring-primary/40" : "border-border",
      )}
    >
      <span className="flex items-center gap-3">
        <BotAvatar
          botId={faceId}
          size={36}
          className={cn("rounded-lg", blank && "text-muted-foreground")}
          color={blank ? "var(--muted)" : botColor(faceId)}
        />
        <span className="grid min-w-0 gap-0.5">
          <span className="truncate text-sm font-semibold">{template.name}</span>
          {template.suggestedName && (
            <span className="truncate text-[11px] text-muted-foreground">
              named {template.suggestedName} · <span className="font-mono">{botIdFrom(template.suggestedName)}</span>
            </span>
          )}
        </span>
      </span>
      <span className="text-xs leading-relaxed text-muted-foreground">{template.description}</span>
      {highlights.length > 0 && (
        <span className="mt-auto flex flex-wrap gap-1 pt-1">
          {highlights.map((highlight) => (
            <span
              key={highlight}
              className="rounded-full border bg-background px-2 py-0.5 text-[10px] font-medium text-foreground/80"
            >
              {highlight}
            </span>
          ))}
        </span>
      )}
    </button>
  );
}

function SummaryRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-0.5">
      <span className="text-[10px] font-semibold tracking-wider text-muted-foreground uppercase">{label}</span>
      <span className="min-w-0 text-foreground">{children}</span>
    </div>
  );
}

function SetupModeChoice({
  active,
  title,
  description,
  onClick,
}: {
  active: boolean;
  title: string;
  description: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "grid content-start gap-1 rounded-lg border p-3 text-left transition-colors hover:bg-muted/40",
        active && "border-primary bg-primary/5",
      )}
    >
      <span className="text-sm font-medium">{title}</span>
      <span className="text-xs text-muted-foreground">{description}</span>
    </button>
  );
}
