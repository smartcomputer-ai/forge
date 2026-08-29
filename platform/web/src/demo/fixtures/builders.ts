/// Building blocks shared by the use-case fixtures. A new use-case is one
/// fixture module built from these helpers plus one `seed*` call in
/// `./index.ts`: the module holds the data (names, instructions, files,
/// transcripts, event logs), and everything a universe needs beyond data —
/// clocks, ids, tool calls as transcripts show them, records with their
/// defaults, bot event logs, bot state — lives here.
import type {
  Bot,
  BotChatSpec,
  BotEventEnvelope,
  BotEventOutcome,
  BotManagedSession,
  BotPollSpec,
  BotRecentEvent,
  BotSessionLineage,
  BotState,
  BotTrigger,
  BotWebhookSpec,
  EnvironmentProviderBinding,
  EnvironmentTemplate,
  ManagedWorkflowTool,
  McpServer,
  Member,
  ModelOption,
  ModelProviderDiscovery,
  ProfileDocument,
  SecretProvider,
  SessionOrigin,
  VfsDirEntry,
  VfsTreeEntry,
  WorkspaceTree,
} from "@/api";
import type { ModelConfig, ToolCallDisplayView } from "@lightspeed/agent-client";
import { DEFAULT_MODEL, newSession } from "../engine";
import type { DemoStore, DemoToolCall, SessionRecord, UniverseState } from "../store";
import { INCUS_PROVIDER_ID } from "./platform";

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

export const MINUTE_MS = 60_000;
export const HOUR_MS = 60 * MINUTE_MS;
export const DAY_MS = 24 * HOUR_MS;

/// Boot time; every fixture timestamp hangs off it so the demo always looks
/// lived-in today.
export const NOW = Date.now();
export const ago = (ms: number): number => NOW - ms;
export const atIso = (ms: number): string => new Date(ms).toISOString();
export const agoIso = (ms: number): string => atIso(NOW - ms);

/// A moment `daysAgo` days back at `hh:mm` local time, so transcripts read
/// like a working day. Today's moments use `ago` so they never land in the
/// future.
export function at(daysAgo: number, hh: number, mm: number): number {
  const date = new Date(NOW - daysAgo * DAY_MS);
  date.setHours(hh, mm, 0, 0);
  return date.getTime();
}

function pad(value: number): string {
  return String(value).padStart(2, "0");
}

/// `HH:MM` local time.
export function clockLabel(ms: number): string {
  const date = new Date(ms);
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

/// `HH:MM` today, `Mon D HH:MM` otherwise.
function compactTime(ms: number): string {
  const date = new Date(ms);
  const today = date.toDateString() === new Date(NOW).toDateString();
  const time = clockLabel(ms);
  return today ? time : `${date.toLocaleDateString("en-US", { month: "short", day: "numeric" })} ${time}`;
}

// ---------------------------------------------------------------------------
// Ids
// ---------------------------------------------------------------------------

/// Deterministic pseudo-digest so refs and ids look real and stay stable
/// across reloads (FNV-1a, folded; nothing cryptographic).
export function hex(seed: string, length = 16): string {
  let out = "";
  for (let round = 0; out.length < length; round++) {
    let h = (0x811c9dc5 ^ round) >>> 0;
    for (let i = 0; i < seed.length; i++) {
      h ^= seed.charCodeAt(i);
      h = Math.imul(h, 0x01000193) >>> 0;
    }
    out += h.toString(16).padStart(8, "0");
  }
  return out.slice(0, length);
}

/// GitHub-delivery-shaped id.
export function uuidLike(seed: string): string {
  const h = hex(seed, 32);
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-4${h.slice(13, 16)}-8${h.slice(17, 20)}-${h.slice(20, 32)}`;
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

export const OPUS: ModelConfig = DEFAULT_MODEL;
export const SONNET: ModelConfig = { providerId: "anthropic", apiKind: "anthropic:messages", model: "claude-sonnet-5" };
export const GPT: ModelConfig = { providerId: "openai", apiKind: "openai:responses", model: "gpt-5.4" };

// ---------------------------------------------------------------------------
// Tool calls as the transcript shows them
// ---------------------------------------------------------------------------

export function tool(
  name: string,
  args: Record<string, unknown>,
  display: ToolCallDisplayView,
  output: string,
  isError = false,
): DemoToolCall {
  return { name, arguments: args, display, output, ...(isError ? { isError: true } : {}) };
}

export interface RunCommandOptions {
  detail?: string;
  cwd?: string;
  timeoutMs?: number;
  isError?: boolean;
}

/// `exec_command` against the active environment.
export function runCommand(argv: string[], output: string, options: RunCommandOptions = {}): DemoToolCall {
  const { detail, cwd, timeoutMs, isError = false } = options;
  return tool(
    "exec_command",
    { argv, ...(cwd === undefined ? {} : { cwd }), ...(timeoutMs === undefined ? {} : { timeoutMs }) },
    { group: "execute", verb: "Run", target: argv.join(" "), ...(detail === undefined ? {} : { detail }) },
    output,
    isError,
  );
}

/// `read_file` against the active environment.
export function readFile(path: string, output: string): DemoToolCall {
  return tool("read_file", { path }, { group: "explore", verb: "Read", target: path }, output);
}

/// `vfs_read_file` against a linked workspace.
export function vfsReadFile(path: string, output: string): DemoToolCall {
  return tool("vfs_read_file", { path }, { group: "explore", verb: "Read", target: path }, output);
}

export function writeFile(path: string, content: string, detail: string): DemoToolCall {
  return tool(
    "write_file",
    { path, content },
    { group: "edit", verb: "Write", target: path, detail },
    `wrote ${new TextEncoder().encode(content).length} bytes to ${path}`,
  );
}

/// `vfs_write_file` into a writable workspace link.
export function vfsWriteFile(path: string, content: string, detail: string): DemoToolCall {
  return tool(
    "vfs_write_file",
    { path, content },
    { group: "edit", verb: "Write", target: path, detail },
    `wrote ${new TextEncoder().encode(content).length} bytes to ${path}`,
  );
}

export function webFetch(url: string, detail: string, output: string, isError = false): DemoToolCall {
  return tool("web_fetch", { url }, { group: "explore", verb: "Fetch", target: url, detail }, output, isError);
}

/// A remote MCP tool; object outputs are shown pretty-printed.
export function mcpCall(name: string, args: Record<string, unknown>, output: unknown): DemoToolCall {
  return tool(
    name,
    args,
    { group: "other", verb: "MCP", target: name },
    typeof output === "string" ? output : JSON.stringify(output, null, 2),
  );
}

/// A GitHub MCP tool, shown under the GitHub verb.
export function github(name: string, args: Record<string, unknown>, detail: string, output: string): DemoToolCall {
  return tool(`github.${name}`, args, { group: "other", verb: "GitHub", target: name, detail }, output);
}

/// `agent_run`: a joined sub-agent delegation.
export function agentRun(profileId: string, task: string, output: string): DemoToolCall {
  return tool(
    "agent_run",
    { profileId, input: task },
    { group: "other", verb: "Delegate", target: profileId, detail: task },
    output,
  );
}

/// `agent_spawn`: a sub-agent started for a promise the run joins later.
export function agentSpawn(profileId: string, task: string, promiseId: string): DemoToolCall {
  return tool(
    "agent_spawn",
    { profileId, input: task },
    { group: "other", verb: "Spawn", target: profileId, detail: task },
    JSON.stringify({ promise: promiseId, agent: profileId }),
  );
}

/// `await`: parks the run on promises; the output is each child's result.
export function awaitPromises(
  promises: string[],
  results: Array<{ agent: string; sessionId: string; output: string }>,
): DemoToolCall {
  return tool(
    "await",
    { promises, mode: "all" },
    { group: "other", verb: "Await", target: promises.join(", ") },
    JSON.stringify(Object.fromEntries(promises.map((id, i) => [id, { status: "completed", ...results[i] }])), null, 2),
  );
}

export type BotEmitArgs = {
  to: string;
  kind: string;
  summary: string;
  data?: unknown;
  reply?: boolean;
};

/// `bot_emit` to another bot's inbox; `seq` is the receiver's #N for it.
export function botEmit(args: BotEmitArgs, seq: number | null): DemoToolCall {
  return tool(
    "bot_emit",
    { ...args },
    { group: "execute", verb: "Emit", target: args.to, detail: args.kind },
    JSON.stringify({ to: args.to, seq }),
  );
}

export function briefPut(brief: string): DemoToolCall {
  return tool(
    "bot_brief_put",
    { brief },
    { group: "edit", verb: "Update", target: "brief" },
    JSON.stringify({ brief, appliesAt: "next idle boundary" }),
  );
}

/// `message_send` in a chat conversation; `sent` is the bot's #N for the
/// archived send. A push (a brief, a reminder) replies to nothing.
export function messageSend(
  conversation: { label: string },
  text: string,
  replyTo: number | null,
  sent: number,
): DemoToolCall {
  return tool(
    "message_send",
    { text, ...(replyTo === null ? {} : { replyTo }) },
    { group: "execute", verb: "Send", target: conversation.label, detail: replyTo === null ? "push" : `reply to #${replyTo}` },
    JSON.stringify({ sent }),
  );
}

export function messageNoop(conversation: { label: string }, reason: string): DemoToolCall {
  return tool(
    "message_noop",
    { reason },
    { group: "other", verb: "Skip", target: conversation.label, detail: reason },
    JSON.stringify({ accepted: true }),
  );
}

// ---------------------------------------------------------------------------
// Universe records
// ---------------------------------------------------------------------------

export interface ProfileInit {
  profileId: string;
  displayName: string;
  description: string;
  instructions: string;
  config: Record<string, unknown>;
  environment?: ProfileDocument["environment"];
  revision: number;
  createdAtMs: number;
  updatedAtMs: number;
}

export function profile(init: ProfileInit): ProfileDocument {
  return {
    profileId: init.profileId,
    displayName: init.displayName,
    description: init.description,
    instructions: { type: "text", text: init.instructions },
    config: structuredClone(init.config),
    ...(init.environment === undefined ? {} : { environment: init.environment }),
    revision: init.revision,
    createdAtMs: init.createdAtMs,
    updatedAtMs: init.updatedAtMs,
  };
}

/// Membership of a platform user; the demo admin joins through `addUniverse`.
export function member(
  store: DemoStore,
  universe: UniverseState,
  userId: string,
  role: string,
  joinedAtMs: number,
): Member {
  const user = store.users.get(userId);
  return {
    id: store.nextId("member"),
    userId,
    role,
    email: user?.email ?? `${userId}@${universe.universe.slug}.example`,
    name: user?.name ?? userId,
    createdAt: atIso(joinedAtMs),
  };
}

const MEDIA_TYPES: Record<string, string> = {
  md: "text/markdown",
  json: "application/json",
  ts: "text/typescript",
};

export interface WorkspaceInit {
  id: string;
  displayName: string;
  /// Path → text; nested directories come from the slashes.
  files: Record<string, string>;
  revision: number;
  createdAtMs: number;
  updatedAtMs: number;
}

/// Builds a nested manifest from flat paths; file bytes go to the blob store
/// so the workspace browser can open them.
export function workspace(store: DemoStore, universe: UniverseState, init: WorkspaceInit): void {
  const root: Record<string, VfsTreeEntry> = {};
  let files = 0;
  let bytes = 0;
  for (const [path, text] of Object.entries(init.files)) {
    const size = new TextEncoder().encode(text).length;
    const parts = path.split("/");
    const name = parts.pop() ?? path;
    let dir = root;
    for (const part of parts) {
      const existing = dir[part];
      if (existing?.kind === "directory") {
        dir = existing.entries;
        continue;
      }
      const created: VfsDirEntry = { kind: "directory", entries: {} };
      dir[part] = created;
      dir = created.entries;
    }
    dir[name] = {
      kind: "file",
      blob_ref: store.putText(text),
      size_bytes: size,
      media_type: MEDIA_TYPES[name.split(".").pop() ?? ""] ?? "text/plain",
      executable: false,
    };
    files += 1;
    bytes += size;
  }
  const manifest: WorkspaceTree["manifest"] = {
    schema_version: "lightspeed.vfs.snapshot.v1",
    root: { entries: root },
    totals: { files, bytes },
  };
  universe.workspaces.set(init.id, {
    row: {
      workspaceId: init.id,
      displayName: init.displayName,
      headSnapshotRef: `snap-${hex(`${init.id}:${init.revision}`, 12)}`,
      revision: init.revision,
      files,
      bytes,
      createdAtMs: init.createdAtMs,
      updatedAtMs: init.updatedAtMs,
    },
    manifest,
  });
}

export interface ProviderBindingInit {
  /// Defaults to the platform's Incus provider.
  providerId?: string;
  revision: number;
  metadata?: Record<string, string>;
  createdAtMs: number;
  updatedAtMs: number;
}

/// The universe's enabled binding to an operator-registered provider.
export function providerBinding(init: ProviderBindingInit): EnvironmentProviderBinding {
  const providerId = init.providerId ?? INCUS_PROVIDER_ID;
  return {
    bindingId: providerId,
    providerId,
    status: "enabled",
    revision: init.revision,
    ...(init.metadata === undefined ? {} : { metadata: init.metadata }),
    createdAtMs: init.createdAtMs,
    updatedAtMs: init.updatedAtMs,
  };
}

export interface TemplateInit {
  /// Defaults to the platform's Incus provider.
  providerId?: string;
  templateId: string;
  displayName: string;
  description: string;
  publicIngress: boolean;
  deprecated: boolean;
  metadata: Record<string, string>;
}

export function template(init: TemplateInit): EnvironmentTemplate {
  const providerId = init.providerId ?? INCUS_PROVIDER_ID;
  return {
    bindingId: providerId,
    providerId,
    templateId: init.templateId,
    displayName: init.displayName,
    description: init.description,
    publicIngress: init.publicIngress,
    deprecated: init.deprecated,
    metadata: init.metadata,
  };
}

export type McpServerInit = Partial<McpServer> &
  Pick<McpServer, "serverId" | "serverUrl" | "authPolicy" | "status" | "createdAtMs" | "updatedAtMs">;

export function mcpServer(init: McpServerInit): McpServer {
  return {
    displayName: null,
    transport: "streamableHttp",
    defaultServerLabel: init.serverId,
    description: null,
    allowedTools: null,
    approvalDefault: "providerDefault",
    deferLoadingDefault: null,
    credential: null,
    revision: 1,
    ...init,
  };
}

/// A `model:<providerId>` credential row.
export function modelProvider(
  providerId: string,
  displayName: string,
  config: SecretProvider["config"],
  hasCredential: boolean,
  createdAtMs: number,
  updatedAtMs: number,
): SecretProvider {
  const providerKind: SecretProvider["providerKind"] =
    config.type === "modelApiKey" ? "modelApiKey" : config.type === "modelEndpoint" ? "modelEndpoint" : "modelOAuth";
  return {
    providerId,
    credentialId: `model:${providerId}`,
    usableForModels: true,
    providerKind,
    displayName,
    config,
    hasCredential,
    status: "active",
    createdAtMs,
    updatedAtMs,
  };
}

/// One discovered model, as `models/list` reports it.
export function modelOption(
  config: ModelConfig,
  displayName: string,
  capabilities: ModelOption["capabilities"],
  fetchedAtMs: number,
): ModelOption {
  return { ...config, displayName, capabilities, source: "provider", fetchedAtMs };
}

export function modelDiscovery(
  providerId: string,
  apiKinds: string[],
  credential: ModelProviderDiscovery["credential"],
  credentialSource: ModelProviderDiscovery["credentialSource"],
  fetchedAtMs: number,
): ModelProviderDiscovery {
  return { providerId, apiKinds, fetchedAtMs, error: null, credential, credentialSource };
}

// ---------------------------------------------------------------------------
// Bots
// ---------------------------------------------------------------------------

/// The workflow tools every bot session carries.
export const BOT_TOOLS: ManagedWorkflowTool[] = [
  { toolId: "bots.event.read", name: "bot_event_read", semanticType: "bots.event.read.v1", target: "bound", completion: "accepted" },
  { toolId: "bots.event.list", name: "bot_event_list", semanticType: "bots.event.list.v1", target: "bound", completion: "accepted" },
  { toolId: "bots.brief.put", name: "bot_brief_put", semanticType: "bots.brief.put.v1", target: "bound", completion: "accepted" },
];
/// Added when the bot may emit.
export const EMIT_TOOL: ManagedWorkflowTool = {
  toolId: "bots.emit",
  name: "bot_emit",
  semanticType: "bots.emit.v1",
  target: "bound",
  completion: "accepted",
};
/// Added when the bot may change its own triggers (`selfConfig`).
export const SELF_CONFIG_TOOLS: ManagedWorkflowTool[] = [
  { toolId: "bots.trigger.put", name: "bot_trigger_put", semanticType: "bots.trigger.put.v1", target: "bound", completion: "accepted" },
  { toolId: "bots.trigger.delete", name: "bot_trigger_delete", semanticType: "bots.trigger.delete.v1", target: "bound", completion: "accepted" },
];
/// Carried by chat-trigger events into their conversation sessions.
export const MESSAGE_TOOLS: ManagedWorkflowTool[] = [
  { toolId: "channels.message.send", name: "message_send", semanticType: "channels.message.send.v1", target: "bound", completion: "accepted" },
  { toolId: "channels.message.edit", name: "message_edit", semanticType: "channels.message.edit.v1", target: "bound", completion: "accepted" },
  { toolId: "channels.message.react", name: "message_react", semanticType: "channels.message.react.v1", target: "bound", completion: "accepted" },
  { toolId: "channels.message.noop", name: "message_noop", semanticType: "channels.message.noop.v1", target: "bound", completion: "accepted" },
];

/// The first thing a freshly created bot is asked.
export const INTRODUCTION_PROMPT =
  "You were just created. Introduce yourself in two sentences and confirm your setup: the triggers that wake you, the tools and environment you can use. Ask about anything that is unclear or missing.";

export interface BotInit {
  botId: string;
  displayName: string;
  description: string;
  profileId: string;
  brief: string;
  runsPerDay: number | null;
  breaker: Bot["breaker"];
  routedSessionTtlMs?: number | null;
  selfConfig?: boolean;
  emit: boolean;
  createdAtMs: number;
  updatedAtMs: number;
}

export function bot(universe: UniverseState, init: BotInit): Bot {
  return {
    botId: init.botId,
    universeId: universe.universe.id,
    displayName: init.displayName,
    description: init.description,
    profileId: init.profileId,
    brief: init.brief,
    runsPerDay: init.runsPerDay,
    breaker: init.breaker,
    routedSessionTtlMs: init.routedSessionTtlMs ?? null,
    selfConfig: init.selfConfig ?? false,
    emit: init.emit,
    enabled: true,
    closedAt: null,
    closedSessions: null,
    createdAt: atIso(init.createdAtMs),
    updatedAt: atIso(init.updatedAtMs),
  };
}

export type TriggerInit = Pick<BotTrigger, "name" | "kind" | "spec"> &
  Partial<Omit<BotTrigger, "name" | "kind" | "spec" | "createdAt" | "updatedAt">> & {
    createdAtMs: number;
    updatedAtMs?: number;
  };

/// Everything a trigger may carry beyond what its kind fixes.
export type TriggerRest = Omit<TriggerInit, "name" | "kind" | "spec">;

export function trigger(init: TriggerInit): BotTrigger {
  const { createdAtMs, updatedAtMs, ...rest } = init;
  return {
    filter: null,
    route: null,
    coalesce: null,
    deliver: null,
    sessionTtlMs: null,
    enabled: true,
    disabledReason: null,
    disabledAt: null,
    lastFilterError: null,
    lastFilterErrorAt: null,
    ...rest,
    createdAt: atIso(createdAtMs),
    updatedAt: atIso(updatedAtMs ?? createdAtMs),
  };
}

/// A webhook trigger with its capability-URL ingest path.
export function webhookTrigger(botId: string, name: string, spec: BotWebhookSpec, rest: TriggerRest): BotTrigger {
  return trigger({
    ...rest,
    name,
    kind: "webhook",
    spec,
    ingestPath: `/api/v1/hooks/bots/${botId}--${name}/${spec.token}`,
  });
}

export function scheduleTrigger(
  name: string,
  spec: { cron: string; summary: string; timezone?: string },
  rest: TriggerRest,
): BotTrigger {
  return trigger({
    ...rest,
    name,
    kind: "schedule",
    spec: { cron: spec.cron, at: null, timezone: spec.timezone ?? "Europe/Berlin", summary: spec.summary },
  });
}

export function pollTrigger(name: string, spec: BotPollSpec, rest: TriggerRest): BotTrigger {
  return trigger({ ...rest, name, kind: "poll", spec });
}

/// The bot's single inbox; `from` undefined accepts every bot in the universe.
export function inboxTrigger(from: string[] | undefined, rest: TriggerRest): BotTrigger {
  return trigger({ ...rest, name: "inbox", kind: "bot", spec: from === undefined ? {} : { from } });
}

/// A chat connection: one session per conversation, kept forever.
export function chatTrigger(store: DemoStore, name: string, spec: BotChatSpec, rest: TriggerRest): BotTrigger {
  const account = store.channelAccounts.get(spec.channelAccountId);
  return trigger({
    name,
    kind: "chat",
    spec,
    route: { policy: "perKey", key: null },
    sessionTtlMs: 0,
    channelAccount: account
      ? { id: account.id, provider: account.provider, accountId: account.accountId, displayName: account.displayName }
      : null,
    ...rest,
  });
}

export interface ManagedInit {
  id: string;
  botId: string;
  displayName: string;
  profile: Pick<ProfileInit, "config" | "instructions">;
  tools: ManagedWorkflowTool[];
  createdAtMs: number;
  environmentId?: string;
}

/// A bot-owned session: the controller is its lifecycle owner and `tools`
/// are its caller-declared workflow tools.
export function managedSession(store: DemoStore, universe: UniverseState, init: ManagedInit): SessionRecord {
  return newSession(store, universe, {
    id: init.id,
    displayName: init.displayName,
    managed: true,
    management: {
      version: 1,
      lifecycleController: { workflowId: `bot:v1:${init.botId}`, workflowKind: "bot_controller_v1" },
      tools: init.tools,
    },
    config: structuredClone(init.profile.config),
    instructions: init.profile.instructions,
    activeEnvironmentId: init.environmentId ?? null,
    createdAtMs: init.createdAtMs,
  });
}

export interface RenderedEvent {
  seq: number;
  kind: string;
  source: string;
  at: number;
  summary: string;
  /// Extra lines after the summary (a payload projection).
  body?: string[];
  inReplyTo?: { bot: string; seq: number } | null;
}

/// The model-facing text of one delivered event, as the bot's session sees
/// it: a header the model can quote back by #N, then the summary.
export function renderEvent(input: RenderedEvent): string {
  const lines = [
    `── event #${input.seq} · ${input.kind} · ${input.source} · ${compactTime(input.at)}`,
    input.summary,
    ...(input.body ?? []),
  ];
  if (input.inReplyTo) lines.push(`reply to your #${input.inReplyTo.seq} at ${input.inReplyTo.bot}`);
  return lines.join("\n");
}

export interface EventInit {
  kind: string;
  source: string;
  /// When it happened; received 900 ms later.
  at: number;
  summary: string;
  body?: string[];
  data?: unknown;
  eventId?: string;
  session?: { sessionId: string; label: string } | null;
  sender?: string;
  hops?: number;
  inReplyTo?: { bot: string; seq: number };
  outcome: BotEventOutcome | null;
  detail?: string;
  /// Shared by every event of one coalesced delivery.
  deliveryId?: string;
  resolvedAfterMs?: number;
}

export interface ScriptedEvent {
  envelope: BotEventEnvelope;
  /// The event as the session read it.
  prompt: string;
}

export interface EventLog {
  botId: string;
  /// Newest last; `seq` is the position in the log.
  events: BotEventEnvelope[];
  add(init: EventInit): ScriptedEvent;
}

/// A bot's numbered event log. `add` stores the prompt and the event
/// document as blobs, like admission does; `runId` is set by whoever
/// appends the run that handled it.
export function eventLog(store: DemoStore, botId: string): EventLog {
  const events: BotEventEnvelope[] = [];
  const add = (init: EventInit): ScriptedEvent => {
    const seq = events.length + 1;
    const inReplyTo = init.inReplyTo ?? null;
    const prompt = renderEvent({
      seq,
      kind: init.kind,
      source: init.source,
      at: init.at,
      summary: init.summary,
      ...(init.body === undefined ? {} : { body: init.body }),
      inReplyTo,
    });
    const document = {
      version: 1,
      kind: init.kind,
      source: init.source,
      occurredAt: atIso(init.at),
      summary: init.summary,
      ...(init.data === undefined ? {} : { data: init.data }),
      ...(init.sender === undefined ? {} : { sender: { bot: init.sender } }),
      ...(init.hops ? { hops: init.hops } : {}),
      ...(inReplyTo ? { inReplyTo } : {}),
    };
    const envelope: BotEventEnvelope = {
      id: `${botId}-evt-${seq}`,
      eventId: init.eventId ?? `${init.kind}:${botId}:${seq}`,
      seq,
      promptRef: store.putText(prompt),
      kind: init.kind,
      source: init.source,
      occurredAt: atIso(init.at),
      ref: store.putText(JSON.stringify(document, null, 2)),
      session: init.session ? { sessionId: init.session.sessionId, label: init.session.label } : null,
      sender: init.sender ?? null,
      hops: init.hops ?? 0,
      inReplyTo,
      receivedAt: atIso(init.at + 900),
      outcome: init.outcome,
      outcomeDetail: init.detail ?? null,
      deliveryId: init.outcome === "blocked" ? null : (init.deliveryId ?? `dlv-${botId}-${seq}`),
      runId: null,
      resolvedAt: init.outcome === null ? null : atIso(init.at + (init.resolvedAfterMs ?? 40_000)),
    };
    events.push(envelope);
    return { envelope, prompt };
  };
  return { botId, events, add };
}

/// One chat conversation a chat trigger routes into its own session.
export interface Conversation {
  sessionId: string;
  label: string;
  provider: "telegram" | "whatsapp";
  source: string;
  chatId: string;
  scope: "direct" | "group";
}

/// An inbound chat message as the chat trigger admits it.
export function chatMessage(
  log: EventLog,
  conversation: Conversation,
  trigger: string,
  sender: string,
  text: string,
  at: number,
  outcome: BotEventOutcome | null,
  detail?: string,
): ScriptedEvent {
  const messageId = String(1_000 + log.events.length * 7 + conversation.chatId.length);
  return log.add({
    kind: "chat.message",
    source: conversation.source,
    at,
    summary: `${sender} (${clockLabel(at)}): ${text}`,
    eventId: `chat:${trigger}:${conversation.chatId}:${messageId}`,
    session: conversation,
    outcome,
    ...(detail === undefined ? {} : { detail }),
    data: {
      conversation: {
        key: conversation.sessionId.slice(`bot:v1:${log.botId}:`.length),
        label: conversation.label,
        scope: conversation.scope,
        provider: conversation.provider,
        chatId: conversation.chatId,
      },
      sender: { name: sender, memberRole: "member" },
      messageId,
      text,
      isDirect: conversation.scope === "direct",
      mentionedBot: conversation.scope === "group",
    },
  });
}

/// A send the bot made, archived so the model can refer to it by #N.
export function chatSent(
  log: EventLog,
  conversation: Conversation,
  text: string,
  at: number,
  replyTo: number | null,
): ScriptedEvent {
  const line = `sent: ${text}`;
  return log.add({
    kind: "chat.sent",
    source: conversation.source,
    at,
    summary: line,
    eventId: `chat:${conversation.provider}:${conversation.chatId}:sent:${log.events.length + 1}`,
    session: conversation,
    outcome: "archived",
    detail: line.length > 120 ? `${line.slice(0, 119)}…` : line,
    resolvedAfterMs: 0,
    data: {
      conversation: { label: conversation.label, provider: conversation.provider, chatId: conversation.chatId },
      text,
      fromMe: true,
      replyTo,
    },
  });
}

export interface ReceiptInit {
  /// The bot that handled our event.
  from: string;
  /// Our event's #N at that bot.
  askedSeq: number;
  status: BotEventOutcome;
  /// The answering delivery's one-line summary.
  summary: string;
  at: number;
  hops: number;
  session: { sessionId: string; label: string };
  outcome: BotEventOutcome;
  detail: string;
  resolvedAfterMs?: number;
}

/// The deterministic `bot.reply` receipt a receiver's controller sends when
/// a delivery finishes: the outcome, never a model-authored message.
export function receipt(log: EventLog, init: ReceiptInit): ScriptedEvent {
  return log.add({
    kind: "bot.reply",
    source: `bot:${init.from}`,
    at: init.at,
    summary: `#${init.askedSeq} at ${init.from} finished ${init.status}: ${init.summary}`,
    eventId: `reply:${init.from}:${hex(`${log.botId}:${init.from}:${init.askedSeq}`, 12)}`,
    session: init.session,
    sender: init.from,
    hops: init.hops,
    inReplyTo: { bot: init.from, seq: init.askedSeq },
    outcome: init.outcome,
    detail: init.detail,
    resolvedAfterMs: init.resolvedAfterMs ?? 20_000,
    data: { status: init.status },
  });
}

export interface SubagentInit {
  id: string;
  displayName: string;
  /// The pinned profile: its config, instructions, and revision.
  profile: Pick<ProfileInit, "profileId" | "config" | "instructions" | "revision">;
  parent: SessionRecord;
  parentRunId: string;
  root: string;
  depth: number;
  limits: SessionOrigin["limits"];
  environmentId?: string;
  createdAtMs: number;
}

/// A sub-agent session: `origin` records who delegated it, under which
/// root, at what depth, from which pinned profile revision.
export function subagentSession(store: DemoStore, universe: UniverseState, init: SubagentInit): SessionRecord {
  const origin: SessionOrigin = {
    kind: "subagent",
    parentSessionId: init.parent.view.id,
    parentRunId: init.parentRunId,
    rootSessionId: init.root,
    depth: init.depth,
    invocationId: `inv-${hex(init.id, 10)}`,
    agent: { profileId: init.profile.profileId, revision: init.profile.revision },
    limits: init.limits,
  };
  return newSession(store, universe, {
    id: init.id,
    displayName: init.displayName,
    config: structuredClone(init.profile.config),
    instructions: init.profile.instructions,
    origin,
    activeEnvironmentId: init.environmentId ?? null,
    createdAtMs: init.createdAtMs,
  });
}

/// One descendant as the bot page's lineage lists it.
export function lineageChild(
  session: SessionRecord,
  profileId: string,
  depth: number,
): BotSessionLineage["children"][number] {
  return {
    id: session.view.id,
    displayName: session.view.displayName ?? null,
    lifecycleStatus: session.view.status === "closed" ? "closed" : "open",
    profileId,
    depth,
    updatedAtMs: session.view.updatedAtMs,
  };
}

/// A session as the controller lists it; the label defaults to "Main" for
/// the main session and to the display name for a keyed one.
export function botSession(
  session: SessionRecord,
  kind: BotManagedSession["kind"],
  label?: string,
): BotManagedSession {
  return {
    sessionId: session.view.id,
    label: label ?? (kind === "main" ? "Main" : (session.view.displayName ?? session.view.id)),
    kind,
    lastActiveAtMs: session.view.updatedAtMs,
  };
}

/// The delivery's decision as the controller remembers it, so the activity
/// page can fill in cache figures beside the stored outcome.
export function recent(
  envelope: BotEventEnvelope,
  usage?: { inputTokens: number; cachedInputTokens: number },
): BotRecentEvent {
  const outcome = envelope.outcome ?? "unresolved";
  return {
    id: envelope.eventId,
    ref: envelope.ref,
    seqs: envelope.seq === null ? [] : [envelope.seq],
    ...(usage === undefined ? {} : { usage }),
    outcome,
    eventCount: 1,
    ...(envelope.runId === null ? {} : { runId: envelope.runId }),
    ...(outcome === "run_failed"
      ? { failure: envelope.outcomeDetail ?? "run failed" }
      : envelope.outcomeDetail === null
        ? {}
        : { summary: envelope.outcomeDetail }),
  };
}

export interface StateInit {
  bot: Bot;
  /// The main session first.
  sessions: BotManagedSession[];
  recentEvents: BotRecentEvent[];
  eventsProcessed: number;
  duplicateEventCount?: number;
  appliedProfileRevision: number;
  runsToday: number;
  descendantsToday?: number;
}

/// The controller's live snapshot for an idle bot.
export function botState(init: StateInit): BotState {
  return {
    botName: init.bot.botId,
    displayName: init.bot.displayName,
    profileId: init.bot.profileId,
    sessionId: init.sessions.find((session) => session.kind === "main")?.sessionId ?? `bot:v1:${init.bot.botId}`,
    sessions: init.sessions,
    controllerStatus: "idle",
    activeDeliveries: [],
    sessionReady: true,
    pendingEventCount: 0,
    pendingDeliveryCount: 0,
    buffers: [],
    recentEvents: init.recentEvents,
    eventsProcessed: init.eventsProcessed,
    duplicateEventCount: init.duplicateEventCount ?? 0,
    duplicateEmissionCount: 0,
    appliedProfileRevision: init.appliedProfileRevision,
    runsPerDay: init.bot.runsPerDay,
    runsToday: init.runsToday,
    descendantsToday: init.descendantsToday ?? 0,
    lastError: null,
  };
}
