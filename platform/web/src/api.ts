import type {
  ContextEntryView,
  RunStatus,
  RunView,
  EnvironmentCredentialSourceView,
  EnvironmentCredentialView,
  EnvironmentProviderBindingView,
  EnvironmentTemplateView,
  EnvironmentView,
  ProfileEnvironment as ProfileEnvironmentView,
  ProfileEnvironmentCredential as ProfileEnvironmentCredentialView,
  SessionEventView,
  SessionEventsReadResponse,
  ToolCallDisplayView,
} from "@lightspeed/agent-client";

/// Thin fetch wrapper for /api/v1 (cookie-authenticated, same origin).

export class ApiError extends Error {
  constructor(
    public status: number,
    public body: unknown,
  ) {
    super(extractMessage(body) ?? `API error ${status}`);
  }
}

function extractMessage(body: unknown): string | null {
  if (!body || typeof body !== "object" || !("error" in body)) return null;
  const { error, issues, failure } = body as {
    error: unknown;
    issues?: unknown;
    failure?: unknown;
  };
  if (typeof error !== "string") return null;
  if (typeof failure === "string" && failure) return `${error}: ${failure}`;
  if (Array.isArray(issues)) {
    const details = issues
      .slice(0, 3)
      .map((issue) => {
        if (!issue || typeof issue !== "object") return null;
        const { path, message } = issue as { path?: unknown; message?: unknown };
        if (typeof message !== "string") return null;
        const at = Array.isArray(path) && path.length > 0 ? `${path.join(".")}: ` : "";
        return `${at}${message}`;
      })
      .filter((detail): detail is string => detail !== null);
    if (details.length > 0) return `${error} — ${details.join("; ")}`;
  }
  return error;
}

export async function api<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(path, {
    method,
    headers: body !== undefined ? { "content-type": "application/json" } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
    credentials: "same-origin",
  });
  const text = await res.text();
  const json: unknown = text ? JSON.parse(text) : null;
  if (!res.ok) {
    throw new ApiError(res.status, json);
  }
  return json as T;
}

export interface Universe {
  id: string;
  organizationId: string;
  lightspeedUniverseId: string;
  name: string;
  /// Immutable URL segment (the better-auth org slug).
  slug: string;
  gatewayUrl: string | null;
  status: "active" | "archived";
  createdAt: string;
  /// Own membership role; null for platform admins browsing a universe
  /// they are not a member of.
  role?: string | null;
}

/// Engine-side universe inventory entry (operator/universes/list view).
export interface EngineUniverse {
  universeId: string;
  sessions: number;
  workspaces: number;
  profiles: number;
  blobBytes: number;
  createdAtMs: number;
  lastActivityAtMs?: number | null;
}

/// Admin reconciliation: each platform row's engine status, plus engine
/// universes no platform row links to (orphans).
export interface UniverseReconcile {
  platform: {
    id: string;
    lightspeedUniverseId: string;
    engine: "ok" | "missing" | "unchecked";
  }[];
  orphans: EngineUniverse[];
}

export interface Member {
  id: string;
  userId: string;
  role: string;
  email: string;
  name: string;
  createdAt: string;
}

export interface UniverseApiKey {
  keyPrefix: string;
  displayName?: string | null;
  createdAtMs: number;
  revokedAtMs?: number | null;
  lastUsedAtMs?: number | null;
}

export interface UniverseApiKeyCreated {
  apiKey: UniverseApiKey;
  /// Returned once at creation and never recoverable from Lightspeed.
  secret: string;
}

export interface UniverseSetup {
  id: string;
  name: string;
  description: string;
  version: number;
  available: boolean;
  status: "available" | "installing" | "ready" | "failed" | "unavailable";
  installedVersion?: number;
  error?: string;
  resources?: {
    keyPrefix?: string;
    grantId?: string;
    serverId?: string;
    profileId?: string;
  };
}

/// Gateway passthrough shapes (the engine owns the full documents).
export interface ProfileSummary {
  profileId: string;
  displayName?: string | null;
  description?: string | null;
  revision: number;
  updatedAtMs: number;
}

export type ProfileEnvironment = ProfileEnvironmentView;
export type ProfileEnvironmentCredential = ProfileEnvironmentCredentialView;

export type ProfileDocument = {
  profileId: string;
  environment?: ProfileEnvironment | null;
  revision?: number;
  createdAtMs?: number;
  updatedAtMs?: number;
} & Record<string, unknown>;

export type InlineProfile = {
  config?: Record<string, unknown>;
  instructions?:
    | { type: "text"; text: string }
    | { type: "textRef"; blobRef: string };
  environment?: ProfileEnvironment | null;
};

export type ProfileSource =
  | { kind: "named"; profileId: string }
  | { kind: "inline"; profile: InlineProfile };

/// Provider-discovered model route from `models/list`.
export interface ModelOption {
  providerId: string;
  apiKind: string;
  model: string;
  displayName: string;
  createdAtMs?: number | null;
  capabilities: {
    maxInputTokens?: number | null;
    maxOutputTokens?: number | null;
    parallelToolUse?: boolean | null;
    reasoningEfforts?: string[] | null;
  };
  source: "provider";
  fetchedAtMs: number;
}

export interface ModelProviderDiscovery {
  providerId: string;
  apiKinds: string[];
  fetchedAtMs?: number | null;
  error?: string | null;
  /// Stable credential signal from the engine (no string matching on `error`).
  credential: "configured" | "missing" | "invalid" | "notRequired";
  credentialSource: "universe" | "deployment" | "none";
}

export interface ModelListResponse {
  models?: ModelOption[];
  providers?: ModelProviderDiscovery[];
}

export interface AuthGrantOption {
  grantId: string;
  providerId: string;
  providerKind: string;
  displayName?: string | null;
  subjectHint?: string | null;
  status: "active" | "needsReauth" | "revoked" | "failed";
  exposure: "brokered" | "retrievable";
}

export interface SecretGrant extends AuthGrantOption {
  principal: {
    kind?: "user" | "serviceAccount" | "universeDefault" | string;
    id?: string | null;
  };
  scopes?: string[];
  audience?: string | null;
  hasAccessToken: boolean;
  hasRefreshToken: boolean;
  expiresAtMs?: number | null;
  lastLeasedAtMs?: number | null;
  leaseCount: number;
  /// Non-secret provider metadata (GitHub App grants: installation id,
  /// account login, permissions, repository selection).
  metadata?: Record<string, unknown>;
  createdAtMs: number;
  updatedAtMs: number;
}

export interface SecretProvider {
  /// Friendly ModelSelection.providerId, such as `openai`.
  providerId: string;
  /// Namespaced auth-provider row id, such as `model:openai`.
  credentialId: string;
  /// False only for legacy/malformed rows that the model runtime will ignore.
  usableForModels: boolean;
  providerKind:
    | "staticBearer"
    | "mcpOAuth"
    | "gitHubApp"
    | "customOAuth"
    | "modelApiKey"
    | "modelOAuth"
    | "modelEndpoint";
  displayName?: string | null;
  config:
    | { type: "modelApiKey"; endpoint?: ModelEndpointConfig | null }
    | {
        type: "modelOAuth";
        grantId: string;
        audience?: string | null;
        endpoint?: ModelEndpointConfig | null;
      }
    | { type: "modelEndpoint"; endpoint: ModelEndpointConfig }
    | { type: "githubApp"; appId: string; apiBaseUrl: string };
  hasCredential: boolean;
  status: "active" | "needsConfiguration" | "disabled";
  createdAtMs: number;
  updatedAtMs: number;
}

export interface ModelEndpointConfig {
  baseUrl: string;
  headers?: Record<string, string>;
  apiKinds: Array<"openai:responses" | "openai:completions">;
}

export interface SecretsInventory {
  providers: SecretProvider[];
  grants: SecretGrant[];
}

/// Universe-owned GitHub App (BYO provider). The private key is stored by the
/// engine and never returned; `hasCredential` is the only trace of it.
export interface GitHubApp {
  providerId: string;
  providerKind: SecretProvider["providerKind"];
  displayName?: string | null;
  config: { type: "githubApp"; appId: string; apiBaseUrl: string };
  hasCredential: boolean;
  status: "active" | "needsConfiguration" | "disabled";
  createdAtMs: number;
  updatedAtMs: number;
}

/// One installation of a GitHub App, live from GitHub.
export interface GitHubInstallation {
  installationId: number;
  accountLogin?: string | null;
  repositorySelection?: string | null;
  permissions?: Record<string, unknown>;
}

/// Result of the Platform subscription import: the grant plus how to bind it.
export interface SubscriptionImportResult {
  grant: SecretGrant;
  shape: "token" | "codexTokenSet";
}

export interface GitHubIntegration {
  apps: GitHubApp[];
  /// Installation grants (`gitHubApp` kind); `metadata.installation_id` links
  /// each grant to its installation.
  grants: SecretGrant[];
}

/// Engine MCP server record (mcp/servers/list view). The optional credential
/// is a non-secret universe-owned grant reference; token material is never returned.
export interface McpServer {
  serverId: string;
  displayName?: string | null;
  serverUrl: string;
  transport: "streamableHttp" | "sse" | "auto";
  defaultServerLabel: string;
  description?: string | null;
  allowedTools?: string[] | null;
  approvalDefault: "providerDefault" | "always" | "never";
  deferLoadingDefault?: boolean | null;
  authPolicy: { type: string } & Record<string, unknown>;
  credential?: { type: "authGrant"; grantId: string } | null;
  status: "active" | "needsAuthConfig" | "unverified" | "disabled";
  revision: number;
  createdAtMs: number;
  updatedAtMs: number;
}

/// Exact projections of the generated Lightspeed environment contract.
export type Environment = EnvironmentView;
export type EnvironmentProviderBinding = EnvironmentProviderBindingView;
export type EnvironmentTemplate = EnvironmentTemplateView;
export type EnvironmentCredentialSource = EnvironmentCredentialSourceView;
export type EnvironmentCredential = EnvironmentCredentialView;

/// Sub-agent lineage (P134): who delegated the session, under which root,
/// at what depth, from which pinned profile revision. Provenance only.
export interface SessionOrigin {
  kind: "subagent";
  parentSessionId: string;
  parentRunId: string;
  rootSessionId: string;
  depth: number;
  invocationId: string;
  agent: { profileId: string; revision: number };
  limits: { maxDepth: number; maxDescendants: number; maxConcurrent: number; deadlineMs: number };
}

export interface SessionSummary {
  id: string;
  displayName?: string | null;
  createdAtMs: number;
  updatedAtMs: number;
  lifecycleStatus: "new" | "open" | "closed";
  managed: boolean;
  origin?: SessionOrigin | null;
}

export interface SessionManagement {
  version: number;
  lifecycleController?: WorkflowEndpoint | null;
  tools?: ManagedWorkflowTool[];
}

export interface WorkflowEndpoint {
  workflowId: string;
  workflowKind: string;
}

export interface ManagedWorkflowTool {
  toolId: string;
  name: string;
  semanticType: string;
  target: "bound" | "start";
  completion: "accepted" | "promises";
}

export interface SessionView {
  id: string;
  displayName?: string | null;
  createdAtMs: number;
  updatedAtMs: number;
  status: "notLoaded" | "idle" | "active" | "closed" | "error";
  managed: boolean;
  activeEnvironmentId?: string | null;
  config?: Record<string, unknown> | null;
  configRevision: number;
  management?: SessionManagement | null;
  origin?: SessionOrigin | null;
  /// Every run of the session — completed, the active one, and runs queued
  /// behind it — straight from the engine. Authoritative for run state; the
  /// event tail is the live, incremental view.
  runs?: SessionRunView[];
}

export type SessionRunView = RunView;
export type SessionRunStatus = RunStatus;

export type WorkspaceLinkTarget =
  | { type: "workspace"; workspaceId: string }
  | { type: "snapshot"; snapshotRef: string };

export interface WorkspaceLink {
  path: string;
  access: "readOnly" | "readWrite";
  target: WorkspaceLinkTarget;
}

export type WorkspaceLinkDraft = {
  path?: string;
  access?: string;
  target?: {
    type?: string;
    workspaceId?: string;
    snapshotRef?: string;
  } & Record<string, unknown>;
} & Record<string, unknown>;

export interface SessionInstructionState {
  text: string | null;
  contextRevision: number;
  active: Array<{
    key: string | null;
    contentRef: string;
    preview: string | null;
  }>;
}

export interface SessionListPage {
  sessions: SessionSummary[];
  nextCursor: string | null;
}

/// Transcript wire shapes come directly from Lightspeed so new event fields
/// and statuses cannot silently drift from the browser reducer.
export type SessionItem = ContextEntryView;
export type ToolCallDisplay = ToolCallDisplayView;
export type SessionEvent = SessionEventView;
export type SessionEventsPage = SessionEventsReadResponse;

/// POST …/sessions/:id/messages — the run was accepted (`running`, or
/// `queued` behind an active run); the reply arrives through the event tail.
export interface SessionRunAccepted {
  run: { id: string; status: SessionRunStatus };
}

/// POST …/runs/:runId/steer — the steering was admitted into the active run
/// and reaches the model at its next turn boundary.
export interface SessionRunSteered {
  steeringId: string;
  run: { id: string; status: SessionRunStatus };
}

/// POST …/runs/:runId/cancel — the cancel was admitted; `cancelling` for an
/// active run (terminal `cancelled` follows on the tail), `cancelled` for a
/// queued one.
export interface SessionRunCancelled {
  run: { id: string; status: SessionRunStatus };
}

/// Engine workspace view, straight from `vfs/workspaces/list`.
export interface WorkspaceRow {
  workspaceId: string;
  displayName?: string | null;
  headSnapshotRef: string;
  revision: number;
  files: number;
  bytes: number;
  createdAtMs: number;
  updatedAtMs: number;
}

/// VFS manifest tree (engine contract: snake_case, kind-tagged).
export interface VfsFileEntry {
  kind: "file";
  blob_ref: string;
  size_bytes: number;
  media_type?: string;
  executable: boolean;
}
export interface VfsDirEntry {
  kind: "directory";
  entries: Record<string, VfsTreeEntry>;
}
export type VfsTreeEntry = VfsFileEntry | VfsDirEntry;

export interface WorkspaceTree {
  workspace: WorkspaceRow;
  manifest: {
    schema_version: string;
    root: { entries: Record<string, VfsTreeEntry> };
    totals: { files: number; bytes: number };
  };
}

export interface BlobContent {
  blobRef: string;
  bytes: number;
  bytesBase64: string;
}

export interface Binding {
  id: string;
  universeId: string;
  channelAccountId: string;
  name: string;
  matchScope: "direct" | "group" | null;
  profileId: string | null;
  sessionKey: string;
  pairingCode: string | null;
  priority: number;
  enabled: boolean;
  createdAt: string;
  channelAccount: Pick<
    ChannelAccount,
    "id" | "provider" | "accountId" | "displayName" | "enabled"
  >;
}

export interface ChannelAccount {
  id: string;
  provider: "telegram" | "whatsapp";
  accountId: string;
  displayName: string;
  settings: { printQr?: boolean };
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ChannelConnectorHealth {
  version: 1;
  provider: "telegram" | "whatsapp";
  accountId: string;
  state: "starting" | "ready" | "disconnected" | "stopping" | "stopped";
  ingressConnected: boolean;
  activityWorkerReady: boolean;
  reconnectAttempts: number;
  detail?: string;
  lastError?: string;
  lastErrorAtMs?: number;
  changedAtMs: number;
}

export interface ChannelConnectorStatus {
  url: string;
  reachable: boolean;
  httpStatus: number | null;
  health?: ChannelConnectorHealth;
  error?: string;
}

export interface ChannelsStatus {
  connectors: ChannelConnectorStatus[];
}

/// Bots: durable event routers that own managed sessions.
export interface Bot {
  /** Authored, immutable, universe-unique id: what models say and URLs use. */
  botId: string;
  universeId: string;
  /** Mutable label; falls back to the id. */
  displayName: string | null;
  /** One line other bots read about this bot. */
  description: string | null;
  profileId: string;
  brief: string | null;
  runsPerDay: number | null;
  breaker: { fires: number; windowMs: number } | null;
  routedSessionTtlMs: number | null;
  /** Whether this bot's sessions get the mutating self-configuration tools. */
  selfConfig: boolean;
  /** Whether this bot's sessions get bot_emit: events to itself or to other bots' inboxes (rate-capped). */
  emit: boolean;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface BotListItem extends Bot {
  triggerCount: number;
}

export function botLabel(bot: Pick<Bot, "botId" | "displayName">): string {
  return bot.displayName ?? bot.botId;
}

export interface BotScheduleSpec {
  cron?: string | null;
  at?: string | null;
  timezone: string;
  summary: string;
}

export type BotWebhookVerification =
  | { scheme: "token" }
  | { scheme: "hmac-sha256"; grantId: string; header: string; prefix?: string; audience?: string };

export interface BotWebhookSpec {
  token: string;
  verification: BotWebhookVerification;
  preset?: "github" | null;
}

export type BotPollSource =
  | {
      kind: "http";
      url: string;
      method?: "GET" | "POST";
      headers?: Record<string, string>;
      auth?: { grantId: string; header?: string; scheme?: string; audience?: string };
      body?: string;
    }
  | { kind: "exec"; environmentId: string; argv: string[]; cwd?: string | null; timeoutMs?: number | null };

export interface BotPollSpec {
  source: BotPollSource;
  intervalMs: number;
  items: string | null;
  cursor: { kind: "idSet"; id: string } | { kind: "watermark"; field: string };
}

export interface BotPollCursorState {
  ids?: string[];
  watermark?: string | number;
  consecutiveFailures: number;
  baselinedAt?: string;
  lastPolledAt?: string;
}

export type BotRoute =
  | { policy: "bot" }
  | { policy: "perKey"; key?: string | null }
  | { policy: "perEvent" };

export interface BotCoalesce {
  debounceMs: number;
  maxWaitMs: number;
  maxCount: number;
}

/** Inbox: which bots may address this one; absent = any bot in the universe. */
export interface BotInboxSpec {
  from?: string[];
}

export interface BotTrigger {
  /** Authored id; the API addresses triggers by it. */
  name: string;
  /** `bot` is the inbox for events other bots address here; at most one per bot. */
  kind: "schedule" | "webhook" | "poll" | "bot";
  spec: BotScheduleSpec | BotWebhookSpec | BotPollSpec | BotInboxSpec;
  filter: string | null;
  route: BotRoute | null;
  coalesce: BotCoalesce | null;
  deliver: { whenBusy: "queue" | "steer" | "append" } | null;
  /** Poll kind only: the advancing cursor; null until the baseline poll. */
  cursor?: BotPollCursorState | null;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
  /** Webhook kind: the ingest path (capability URL) senders post to. */
  ingestPath?: string;
}

export interface BotEventRef {
  version: 1;
  id: string;
  ref: string;
}

export interface BotRecentEvent {
  id: string;
  ref: string;
  /** Event sequence numbers (#N) in this delivery, when known. */
  seqs?: number[];
  /** Prompt tokens the run consumed and how many the provider served from its cache. */
  usage?: { inputTokens: number; cachedInputTokens: number };
  status:
    | "handled"
    | "deferred"
    | "ignored"
    | "blocked"
    | "unresolved"
    | "run_failed"
    | "appended"
    | "steered";
  eventCount?: number;
  runId?: string;
  summary?: string;
  failure?: string;
}

export interface BotManagedSession {
  sessionId: string;
  label: string;
  kind: "main" | "keyed" | "event";
  lastActiveAtMs?: number;
}

/// Sub-agent descendants of one bot session (P134 lineage), read from core by
/// the platform server beside the controller state.
export interface BotSessionLineage {
  open: number;
  total: number;
  children: Array<{
    id: string;
    displayName: string | null;
    lifecycleStatus: "new" | "open" | "closed";
    profileId: string | null;
    depth: number;
    updatedAtMs: number;
  }>;
}

export type BotLineage = Record<string, BotSessionLineage>;

export interface BotState {
  botName: string;
  displayName: string | null;
  profileId: string;
  sessionId: string;
  sessions: BotManagedSession[];
  controllerStatus:
    | "initializing"
    | "idle"
    | "session_busy"
    | "delivering_event"
    | "budget_exhausted"
    | "degraded";
  activeDeliveries: { id: string; eventCount: number; sessionId: string; runId: string | null }[];
  sessionReady: boolean;
  pendingEventCount: number;
  pendingDeliveryCount: number;
  buffers: { key: string; count: number; flushAtMs: number }[];
  recentEvents: BotRecentEvent[];
  eventsProcessed: number;
  duplicateEventCount: number;
  duplicateEmissionCount: number;
  appliedProfileRevision: number | null;
  runsPerDay: number | null;
  runsToday: number;
  /** Sub-agent sessions delegated under the bot's sessions today; counted against `runsPerDay`. */
  descendantsToday: number;
  lastError: string | null;
}

export interface BotEventEnvelope {
  id: string;
  eventId: string;
  /** Per-bot sequence number (#N); null only for pre-numbering rows. */
  seq: number | null;
  promptRef: string | null;
  kind: string;
  source: string;
  occurredAt: string;
  ref: string;
  session: { sessionId: string; label: string } | null;
  /** Sending bot id for bot-originated events; null for world events. */
  sender: string | null;
  /** Federation hop count; 0 for world events. */
  hops: number;
  /** Receipts: the asked event's #N at the answering bot. */
  inReplyTo: { bot: string; seq: number } | null;
  receivedAt: string;
}

export interface BotEventPage {
  events: BotEventEnvelope[];
  nextCursor: string | null;
}

export interface BotActivityEntry {
  id: string;
  kind: string;
  eventId: string | null;
  runId: string | null;
  detail: string | null;
  createdAt: string;
}

export interface BotActivityPage {
  activity: BotActivityEntry[];
  nextCursor: string | null;
}
