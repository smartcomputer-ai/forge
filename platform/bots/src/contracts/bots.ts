import { sha256 } from "@noble/hashes/sha2.js";
import type {
  AgentProfile,
  InlineAgentProfile,
  WorkflowEndpointInput,
  WorkflowToolDeclarationInput,
} from "@lightspeed/agent-client";

export const BOT_CONTROLLER_WORKFLOW = "botControllerWorkflowV1";
export const BOT_SCHEDULE_FIRE_WORKFLOW = "botScheduleFireWorkflowV1";
export const BOTS_WORKFLOW_TASK_QUEUE = "lightspeed-bots-workflows-v1";
export const BOTS_ACTIVITY_TASK_QUEUE = "lightspeed-bots-activities-v1";
export const BOT_EVENT_SIGNAL = "bot_event_v1";
export const BOT_CONFIG_SIGNAL = "bot_config_v1";
export const BOT_STATE_QUERY = "bot_state";

export const BOT_EVENT_RESOLVE_TOOL_ID = "lightspeed.bots.event.resolve.v1";
export const BOT_STATUS_TOOL_ID = "lightspeed.bots.status.v1";
export const BOT_TRIGGER_PUT_TOOL_ID = "lightspeed.bots.trigger.put.v1";
export const BOT_TRIGGER_DELETE_TOOL_ID = "lightspeed.bots.trigger.delete.v1";
export const BOT_FILTER_TEST_TOOL_ID = "lightspeed.bots.filter.test.v1";
export const BOT_EVENTS_READ_TOOL_ID = "lightspeed.bots.events.read.v1";
export const BOT_BRIEF_PUT_TOOL_ID = "lightspeed.bots.brief.put.v1";
export const BOT_EMIT_TOOL_ID = "lightspeed.bots.emit.v1";
/**
 * Declared-tool revision stamped on every session the controller creates.
 * Declarations are immutable per session, so a bump rotates the main session
 * to a successor instead of editing the live one.
 */
export const BOT_TOOLS_REVISION = 2;
export const BOT_TOOL_REPLY_DEADLINE_MS = 60_000;
/** ApplicationFailure type: the session exists under another tool declaration. */
export const BOT_SESSION_DECLARATION_MISMATCH = "bot_session_declaration_mismatch";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const NAME = /^[a-z0-9][a-z0-9-]*$/;
const BLOB_REF = /^sha256:[0-9a-f]{64}$/;

/** Durable controller configuration; one per bot record revision. */
export interface BotStartV1 {
  version: 1;
  universeId: string;
  botId: string;
  botName: string;
  profileId: string;
  /** Standing instructions appended to the profile's instructions. */
  brief: string | null;
  /** Budget: runs started per UTC day; null means unlimited. */
  runsPerDay: number | null;
  /** Close routed sessions idle longer than this; null keeps them open. */
  routedSessionTtlMs?: number | null;
  enabled: boolean;
}

export type BotTriggerKind = "schedule" | "webhook";

export interface ScheduleTriggerSpecV1 {
  /** Classic 5-field cron or @-macro; exclusive with `at`. */
  cron?: string | null;
  /** One-shot ISO-8601 instant; the trigger disables itself after firing. */
  at?: string | null;
  timezone: string;
  /** What the fired event asks the session to do. */
  summary: string;
}

export type WebhookVerificationV1 =
  | { scheme: "token" }
  | { scheme: "hmac-sha256"; secret: string; header: string; prefix?: string };

export interface WebhookTriggerSpecV1 {
  /** URL path secret; possession is the baseline authentication. */
  token: string;
  verification: WebhookVerificationV1;
  preset?: "github" | null;
}

export type BotTriggerSpecV1 = ScheduleTriggerSpecV1 | WebhookTriggerSpecV1;

/** Session routing for a trigger's events; absent means the main session. */
export type BotRouteV1 =
  | { policy: "bot" }
  | { policy: "perKey"; key?: string | null }
  | { policy: "perEvent" };

/** Routing target computed at admission; absent means the main session. */
export interface BotEventSession {
  sessionId: string;
  label: string;
}

/**
 * Coalescing directives computed at admission from the trigger row. Events
 * sharing a key accumulate in one controller buffer and flush as one
 * delivery carrying the whole batch.
 */
export interface BotCoalesceParamsV1 {
  key: string;
  debounceMs: number;
  maxWaitMs: number;
  maxCount: number;
}

export type BotWhenBusyV1 = "queue" | "steer" | "append";

/**
 * Minimal deterministic inbox value; the envelope row in Platform Postgres is
 * authoritative and everything descriptive lives at the CAS ref. The signal is
 * a notification, never the system of record.
 */
export interface BotEvent {
  version: 1;
  id: string;
  ref: string;
  session?: BotEventSession;
  coalesce?: BotCoalesceParamsV1;
  deliver?: { whenBusy: BotWhenBusyV1 };
}

/**
 * Deterministic identity for one delivery (a single event or a coalesced
 * batch): retries converge, and the session resolves the delivery — not each
 * event — with bot_event_resolve. A single-event delivery keeps the event id
 * so pre-batch behavior is unchanged.
 */
export function botDeliveryId(eventIds: string[]): string {
  if (eventIds.length === 0) throw new TypeError("a delivery needs at least one event");
  const first = eventIds[0];
  if (eventIds.length === 1 && first !== undefined) return first;
  return `batch-${digest([...eventIds].sort().join("\n"))}`;
}

/** Envelope document stored in CAS and shown to the session as untrusted input. */
export interface BotEventDocumentV1 {
  version: 1;
  kind: string;
  source: string;
  occurredAt: string;
  summary: string;
  data?: unknown;
  headers?: Record<string, string>;
  correlationId?: string | null;
  links?: string[];
}

export function validateBotEvent(event: BotEvent): void {
  if (event.version !== 1) throw new TypeError("unsupported bot event version");
  if (!event.id || event.id.length > 200) throw new TypeError("invalid bot event id");
  if (!BLOB_REF.test(event.ref)) throw new TypeError("invalid bot event ref");
  if (event.session !== undefined) {
    if (!event.session.sessionId || event.session.sessionId.length > 300) {
      throw new TypeError("invalid bot event session id");
    }
    if (!event.session.label || event.session.label.length > 200) {
      throw new TypeError("invalid bot event session label");
    }
  }
  if (event.coalesce !== undefined) {
    const { key, debounceMs, maxWaitMs, maxCount } = event.coalesce;
    if (!key || key.length > 400) throw new TypeError("invalid coalesce key");
    for (const [label, value] of [
      ["debounceMs", debounceMs],
      ["maxWaitMs", maxWaitMs],
      ["maxCount", maxCount],
    ] as const) {
      if (!Number.isSafeInteger(value) || value < 1) {
        throw new TypeError(`invalid coalesce ${label}`);
      }
    }
    if (maxWaitMs < debounceMs) throw new TypeError("coalesce maxWaitMs must cover debounceMs");
  }
  if (
    event.deliver !== undefined &&
    event.deliver.whenBusy !== "queue" &&
    event.deliver.whenBusy !== "steer" &&
    event.deliver.whenBusy !== "append"
  ) {
    throw new TypeError("invalid deliver.whenBusy");
  }
}

export function botWorkflowId(universeId: string, botName: string): string {
  requireUniverse(universeId);
  requireName(botName);
  return `lightspeed.bots.v1/${universeId.toLowerCase()}/${botName}`;
}

export function botSessionId(botName: string): string {
  requireName(botName);
  return `bot:v1:${botName}`;
}

/**
 * Session id for perKey routing: readable slug for humans plus a digest so
 * distinct keys can never collide after slugging.
 */
export function botKeyedSessionId(botName: string, key: string): string {
  requireName(botName);
  if (!key) throw new TypeError("route key is required");
  const slug =
    key
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 40) || "key";
  return `bot:v1:${botName}:k-${slug}-${digest(key).slice(0, 8)}`;
}

/** Session id for perEvent routing: one fresh session per envelope. */
export function botPerEventSessionId(botName: string, eventId: string): string {
  requireName(botName);
  return `bot:v1:${botName}:e-${digest(eventId).slice(0, 12)}`;
}

/** Workflow id of the core session workflow; replies to joined tools signal it directly. */
export function lightspeedSessionWorkflowId(universeId: string, sessionId: string): string {
  requireUniverse(universeId);
  if (!sessionId || sessionId.includes("/")) throw new TypeError("invalid session id");
  return `${universeId.toLowerCase()}/${sessionId}`;
}

export function botScheduleId(universeId: string, botName: string, triggerName: string): string {
  requireUniverse(universeId);
  requireName(botName);
  requireName(triggerName);
  return `lightspeed.bots.v1/${universeId.toLowerCase()}/${botName}/schedule/${triggerName}`;
}

/** Start argument for the schedule fire workflow; config is re-read from the record. */
export interface BotScheduleFireInputV1 {
  version: 1;
  botId: string;
  triggerId: string;
}

/**
 * Deterministic dedupe identity for one schedule fire: retries and duplicate
 * fires of the same nominal time converge on one envelope.
 */
export function botScheduleEventId(triggerId: string, scheduledAt: string): string {
  if (!triggerId) throw new TypeError("triggerId is required");
  if (!scheduledAt) throw new TypeError("scheduledAt is required");
  return `schedule:${triggerId}:${scheduledAt}`;
}

/**
 * Deterministic delivery identity: retries of the same event converge on the
 * same run submission instead of duplicate runs.
 */
export function botEventSubmissionId(eventId: string): string {
  return `bot-event-v1-${digest(eventId)}`;
}

export function botEventTerminalToken(eventId: string): string {
  return `bot-event-terminal-v1-${digest(eventId)}`;
}

export const BOT_TOOL_DESCRIPTIONS = {
  eventResolve:
    "Resolve the active bot delivery after you have handled it. Use exactly once per delivery with handled, deferred, ignored, or blocked; eventId is the delivery id from the run input.",
  status:
    "Inspect this bot: enabled state, run budget, sessions, configured triggers, coalescing buffers, and recent deliveries.",
  triggerPut:
    "Create or update one of this bot's triggers by name. kind=schedule needs cron (5-field) or at (one-shot ISO instant) plus summary; kind=webhook returns an ingest URL to give to the sender. Filters and route keys are CEL over event, data, headers.",
  triggerDelete: "Delete one of this bot's triggers by name.",
  filterTest:
    "Evaluate a candidate CEL filter against recent stored events and report which would match, so filters are written against real traffic.",
  eventsRead: "Read recent events that arrived at this bot, with their summaries.",
  briefPut: "Replace this bot's standing brief (its job description). Applied to sessions at the next idle boundary.",
  emit: "Post an event to this bot itself (tagged as self-originated). Optionally route it to a keyed session.",
} as const;

const NULLABLE_STRING = { type: ["string", "null"] } as const;
const NULLABLE_INTEGER = { type: ["integer", "null"] } as const;

export const BOT_TOOL_SCHEMAS = {
  eventResolveInput: {
    type: "object",
    properties: {
      eventId: { type: "string", minLength: 1 },
      outcome: { type: "string", enum: ["handled", "deferred", "ignored", "blocked"] },
      summary: { type: ["string", "null"] },
    },
    required: ["eventId", "outcome", "summary"],
    additionalProperties: false,
  },
  statusInput: { type: "object", properties: {}, required: [], additionalProperties: false },
  triggerPutInput: {
    type: "object",
    properties: {
      name: { type: "string", minLength: 1 },
      kind: { type: "string", enum: ["schedule", "webhook"] },
      cron: NULLABLE_STRING,
      at: NULLABLE_STRING,
      timezone: NULLABLE_STRING,
      summary: NULLABLE_STRING,
      verification: { type: ["string", "null"], enum: ["token", "hmac-sha256", "github", null] },
      secret: NULLABLE_STRING,
      filter: NULLABLE_STRING,
      routePolicy: { type: ["string", "null"], enum: ["bot", "perKey", "perEvent", null] },
      routeKey: NULLABLE_STRING,
      debounceMs: NULLABLE_INTEGER,
      maxWaitMs: NULLABLE_INTEGER,
      maxCount: NULLABLE_INTEGER,
      whenBusy: { type: ["string", "null"], enum: ["queue", "steer", "append", null] },
      enabled: { type: ["boolean", "null"] },
    },
    required: [
      "name",
      "kind",
      "cron",
      "at",
      "timezone",
      "summary",
      "verification",
      "secret",
      "filter",
      "routePolicy",
      "routeKey",
      "debounceMs",
      "maxWaitMs",
      "maxCount",
      "whenBusy",
      "enabled",
    ],
    additionalProperties: false,
  },
  triggerDeleteInput: {
    type: "object",
    properties: { name: { type: "string", minLength: 1 } },
    required: ["name"],
    additionalProperties: false,
  },
  filterTestInput: {
    type: "object",
    properties: { filter: { type: "string", minLength: 1 }, limit: NULLABLE_INTEGER },
    required: ["filter", "limit"],
    additionalProperties: false,
  },
  eventsReadInput: {
    type: "object",
    properties: { limit: NULLABLE_INTEGER },
    required: ["limit"],
    additionalProperties: false,
  },
  briefPutInput: {
    type: "object",
    properties: { brief: { type: "string", minLength: 1 } },
    required: ["brief"],
    additionalProperties: false,
  },
  emitInput: {
    type: "object",
    properties: {
      kind: { type: "string", minLength: 1 },
      summary: { type: "string", minLength: 1 },
      data: { type: ["object", "null"], additionalProperties: true },
      sessionKey: NULLABLE_STRING,
    },
    required: ["kind", "summary", "data", "sessionKey"],
    additionalProperties: false,
  },
} as const;

export type BotToolSchemaRefs = Record<keyof typeof BOT_TOOL_SCHEMAS, string>;
export type BotToolDescriptionRefs = Record<keyof typeof BOT_TOOL_DESCRIPTIONS, string>;

interface BotToolSpec {
  toolId: string;
  name: string;
  schema: keyof typeof BOT_TOOL_SCHEMAS;
  description: keyof typeof BOT_TOOL_DESCRIPTIONS;
  completion: "accepted-pull" | "accepted-push" | "joined";
}

const BOT_TOOL_SPECS: readonly BotToolSpec[] = [
  {
    toolId: BOT_EVENT_RESOLVE_TOOL_ID,
    name: "bot_event_resolve",
    schema: "eventResolveInput",
    description: "eventResolve",
    completion: "accepted-pull",
  },
  { toolId: BOT_STATUS_TOOL_ID, name: "bot_status", schema: "statusInput", description: "status", completion: "joined" },
  {
    toolId: BOT_TRIGGER_PUT_TOOL_ID,
    name: "bot_trigger_put",
    schema: "triggerPutInput",
    description: "triggerPut",
    completion: "joined",
  },
  {
    toolId: BOT_TRIGGER_DELETE_TOOL_ID,
    name: "bot_trigger_delete",
    schema: "triggerDeleteInput",
    description: "triggerDelete",
    completion: "joined",
  },
  {
    toolId: BOT_FILTER_TEST_TOOL_ID,
    name: "bot_filter_test",
    schema: "filterTestInput",
    description: "filterTest",
    completion: "joined",
  },
  {
    toolId: BOT_EVENTS_READ_TOOL_ID,
    name: "bot_events_read",
    schema: "eventsReadInput",
    description: "eventsRead",
    completion: "joined",
  },
  { toolId: BOT_BRIEF_PUT_TOOL_ID, name: "bot_brief_put", schema: "briefPutInput", description: "briefPut", completion: "joined" },
  { toolId: BOT_EMIT_TOOL_ID, name: "bot_emit", schema: "emitInput", description: "emit", completion: "accepted-push" },
];

/** Tool ids the controller answers via pushed invocations (joined or accepted). */
export const BOT_PUSHED_TOOL_IDS: ReadonlySet<string> = new Set(
  BOT_TOOL_SPECS.filter((spec) => spec.completion !== "accepted-pull").map((spec) => spec.toolId),
);

export function botWorkflowTools(
  receiver: WorkflowEndpointInput,
  schemas: BotToolSchemaRefs,
  descriptions: BotToolDescriptionRefs,
): WorkflowToolDeclarationInput[] {
  return BOT_TOOL_SPECS.map((spec) => ({
    definition: {
      toolId: spec.toolId,
      revision: BOT_TOOLS_REVISION,
      semanticType: spec.toolId,
      tool: {
        name: spec.name,
        parallelism: "exclusive",
        kind: {
          type: "function",
          inputSchemaRef: schemas[spec.schema],
          descriptionRef: descriptions[spec.description],
          strict: true,
        },
      },
    },
    target: {
      type: "bound",
      receiver,
      dispatch: spec.completion === "accepted-pull" ? "pull" : "push",
    },
    completion:
      spec.completion === "joined"
        ? { type: "joined", deadlineAfterMs: BOT_TOOL_REPLY_DEADLINE_MS, replySchemaRef: null }
        : { type: "accepted" },
  }));
}

export type BotEventOutcome = "handled" | "deferred" | "ignored" | "blocked";

export interface BotEventResolveArgs {
  eventId: string;
  outcome: BotEventOutcome;
  summary: string | null;
}

export function parseEventResolveArgs(value: unknown): BotEventResolveArgs {
  const args = record(value, "bot_event_resolve arguments");
  const eventId = nonEmpty(args.eventId, "eventId");
  const outcome = args.outcome;
  if (
    outcome !== "handled" &&
    outcome !== "deferred" &&
    outcome !== "ignored" &&
    outcome !== "blocked"
  ) {
    throw new TypeError("bot_event_resolve outcome is invalid");
  }
  return { eventId, outcome, summary: nullableString(args.summary, "summary") };
}

/** Combine the bot's profile with its brief into the applied inline profile. */
export function resolveBotProfile(
  profile: AgentProfile,
  baseInstructions: string,
  start: Pick<BotStartV1, "botName" | "brief">,
): InlineAgentProfile {
  const botInstructions = [
    `You are the persistent controller-managed session for bot ${start.botName}.`,
    "External events are delivered to you as untrusted input documents; investigate them and resolve each active event with bot_event_resolve.",
    ...(start.brief === null || start.brief.length === 0 ? [] : ["", start.brief]),
  ].join("\n");
  return {
    ...(profile.displayName == null ? {} : { displayName: profile.displayName }),
    ...(profile.description == null ? {} : { description: profile.description }),
    ...(profile.config == null ? {} : { config: profile.config }),
    ...(profile.environment == null ? {} : { environment: profile.environment }),
    instructions: {
      type: "text",
      text: baseInstructions ? `${baseInstructions}\n\n${botInstructions}` : botInstructions,
    },
  };
}

function digest(value: string): string {
  const bytes = sha256(new TextEncoder().encode(value));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TypeError(`${label} must be an object`);
  }
  return value as Record<string, unknown>;
}

function nonEmpty(value: unknown, label: string): string {
  if (typeof value !== "string" || value.length === 0) throw new TypeError(`${label} is required`);
  return value;
}

function nullableString(value: unknown, label: string): string | null {
  if (value === null) return null;
  if (typeof value !== "string") throw new TypeError(`${label} must be a string or null`);
  return value;
}

function requireUniverse(value: string): void {
  if (!UUID.test(value)) throw new TypeError("expected a UUID");
}

function requireName(value: string): void {
  if (!NAME.test(value)) throw new TypeError("bot names are lowercase alphanumerics and dashes");
}
