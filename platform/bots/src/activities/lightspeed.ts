import { ApplicationFailure } from "@temporalio/common";
import {
  LightspeedClient,
  LightspeedRpcError,
  type AgentProfile,
  type SessionStatus,
  type WorkflowToolDeclarationInput,
} from "@lightspeed/agent-client";

import {
  BOT_SESSION_DECLARATION_MISMATCH,
  BOT_SESSION_PROFILE_UNAPPLICABLE,
  BOT_TOOL_DESCRIPTIONS,
  BOT_TOOL_NAMES,
  BOT_TOOL_SCHEMAS,
  botWorkflowTools,
  resolveBotProfile,
  type BotEvent,
  type BotToolDescriptionRefs,
  type BotToolSchemaRefs,
} from "../contracts/bots.js";

/** ApplicationFailure type: a carried declaration cannot be merged into the bot's toolset. */
export const BOT_CARRIED_TOOLS_INVALID = "bot_carried_tools_invalid";

export interface BotLightspeedConfig {
  endpoint: string;
  fetch?: typeof fetch;
}

export interface EnsureBotSessionInput {
  universeId: string;
  sessionId: string;
  displayName: string;
  profileId: string;
  botName: string;
  brief: string | null;
  /** Declare the mutating self-configuration tools (default false). */
  selfConfig?: boolean;
  /** Declare `bot_emit` (default false). */
  emit?: boolean;
  appliedProfileRevision?: number | null;
  controller: { workflowId: string; workflowKind: string };
  /**
   * CAS ref of receiver-bound declarations carried by the event that opens
   * this routed session (a chat conversation's `message_*` tools). Merged
   * verbatim after the bot's own tools; opaque here.
   */
  toolsRef?: string | null;
}

export interface EnsureBotSessionResult {
  profileRevision: number;
  /** Tool ids of the carried declarations; a run that used one counts as handled. */
  carriedToolIds: string[];
}

export interface StartBotRunInput {
  universeId: string;
  sessionId: string;
  deliveryId: string;
  events: BotEvent[];
  submissionId: string;
  terminalToken: string;
}

export interface SteerBotRunInput {
  universeId: string;
  sessionId: string;
  deliveryId: string;
  events: BotEvent[];
}

export interface AppendBotContextInput {
  universeId: string;
  sessionId: string;
  deliveryId: string;
  events: BotEvent[];
}

export interface ReadSessionInput {
  universeId: string;
  sessionId: string;
}

export interface PulledWorkflowToolInvocation {
  invocationId: string;
  toolId: string;
  runId: string;
  argumentsRef: string;
}

export interface ReadWorkflowToolInvocationsInput extends ReadSessionInput {
  afterSeq: number;
}

export interface ReadWorkflowToolInvocationsResult {
  nextSeq: number;
  invocations: PulledWorkflowToolInvocation[];
}

export interface ReadJsonBlobInput {
  universeId: string;
  blobRef: string;
}

export interface CountBotDescendantSessionsInput {
  universeId: string;
  /** Bot sessions whose sub-agent trees are counted (lineage roots). */
  sessionIds: string[];
  /** Only descendants created at or after this instant count. */
  sinceMs: number;
}

export interface ReadRunUsageInput {
  universeId: string;
  sessionId: string;
  runId: string;
}

/** Prompt tokens a run consumed and how many the provider served from its cache. */
export interface BotRunUsage {
  inputTokens: number;
  cachedInputTokens: number;
}

export interface RenameBotSessionInput {
  universeId: string;
  sessionId: string;
  displayName: string;
}

export interface BotLightspeedActivities {
  ensureBotSession(input: EnsureBotSessionInput): Promise<EnsureBotSessionResult>;
  /** Label-only: sets a managed session's display name; identity never moves. */
  renameBotSession(input: RenameBotSessionInput): Promise<void>;
  readBotSessionStatus(input: ReadSessionInput): Promise<{ status: SessionStatus }>;
  /** Usage of one finished run, or null when the provider reported none. */
  readBotRunUsage(input: ReadRunUsageInput): Promise<BotRunUsage | null>;
  startBotRun(input: StartBotRunInput): Promise<{ runId: string }>;
  steerBotRun(input: SteerBotRunInput): Promise<{ steered: boolean; runId?: string }>;
  appendBotContext(input: AppendBotContextInput): Promise<void>;
  /**
   * Close a managed session together with its open sub-agent descendants
   * (P134 lineage). Non-force (the default) leaves a busy session alone and
   * the sweep retries later; `force` cancels the active run and drops queued
   * ones — what a bot close does, matching `session/delete`.
   */
  closeBotSession(
    input: ReadSessionInput & { force?: boolean },
  ): Promise<{ closed: boolean; descendantsClosed?: number }>;
  /**
   * Sub-agent sessions delegated under the bot's sessions since `sinceMs`
   * (P134 lineage). Every descendant counts against the bot's daily run
   * budget like a run the controller started itself.
   */
  countBotDescendantSessions(input: CountBotDescendantSessionsInput): Promise<{ count: number }>;
  readWorkflowToolInvocations(
    input: ReadWorkflowToolInvocationsInput,
  ): Promise<ReadWorkflowToolInvocationsResult>;
  readJsonBlob(input: ReadJsonBlobInput): Promise<unknown>;
}

/** Bound on lineage pages read per root when counting descendants. */
const DESCENDANT_COUNT_MAX_PAGES = 10;

async function isSessionClosed(
  client: Pick<LightspeedClient, "call">,
  sessionId: string,
): Promise<boolean> {
  try {
    const read = await client.call("session/read", { sessionId });
    return read.result.session.status === "closed";
  } catch {
    return false;
  }
}

/**
 * The engine refused the profile's config for this session in a way no
 * retry fixes: an invalid document, or a command rejection of the
 * provider-compatibility kind (the rejection kind leads the message).
 */
export function isProfileUnapplicable(error: LightspeedRpcError): boolean {
  return (
    error.kind === "invalid_request" ||
    (error.kind === "rejected" && /^ProviderCompatibility\b/.test(error.message))
  );
}

export function isBotSessionDeclarationMismatch(error: unknown): boolean {
  return (
    error instanceof LightspeedRpcError &&
    error.kind === "conflict" &&
    (/fingerprint/i.test(error.message) ||
      /managed-session controller, receiver, or tool declaration conflicts/i.test(error.message))
  );
}

export function createBotLightspeedActivities(
  config: BotLightspeedConfig,
): BotLightspeedActivities {
  return {
    async ensureBotSession(input) {
      const client = clientForUniverse(config, input.universeId);
      const profile = (await client.call("profiles/read", { profileId: input.profileId })).result
        .profile;
      const baseInstructions = await readProfileInstructions(client, profile);
      const resolvedProfile = resolveBotProfile(profile, baseInstructions, input);
      const refs = await putToolAssets(client);
      const carried =
        input.toolsRef == null ? [] : await readCarriedDeclarations(client, input.toolsRef);
      try {
        await client.call("session/managed/start", {
          sessionId: input.sessionId,
          displayName: input.displayName,
          profile: { kind: "inline", profile: resolvedProfile },
          workflowTools: {
            version: 1,
            lifecycleController: input.controller,
            tools: [
              ...botWorkflowTools(input.controller, refs.schemas, refs.descriptions, {
                selfConfig: input.selfConfig === true,
                emit: input.emit === true,
              }),
              ...carried,
            ],
          },
        });
      } catch (error) {
        // Declarations are immutable per session: an existing session created
        // under an older tool revision cannot be upgraded in place. Report it
        // as a typed, non-retryable failure so the controller rotates to a
        // successor session instead of retrying forever.
        if (isBotSessionDeclarationMismatch(error)) {
          throw ApplicationFailure.nonRetryable(
            `session ${input.sessionId} was created under another tool declaration`,
            BOT_SESSION_DECLARATION_MISMATCH,
          );
        }
        throw error;
      }
      if (input.appliedProfileRevision !== profile.revision) {
        // A session's provider api kind is pinned for its lifetime. A profile
        // that moved to another kind is valid for a fresh session but not for
        // this one: report that as unapplicable so the controller rotates,
        // rather than retrying into a degraded bot. Checked structurally
        // first; the engine's rejection is the backstop.
        const proposedKind = resolvedProfile.config?.model?.apiKind;
        if (proposedKind !== undefined) {
          const current = (await client.call("session/read", { sessionId: input.sessionId })).result.session;
          const pinnedKind = current.config?.model?.apiKind;
          if (pinnedKind !== undefined && pinnedKind !== proposedKind) {
            throw ApplicationFailure.nonRetryable(
              `session ${input.sessionId} is pinned to provider api kind ${pinnedKind}; profile revision ${profile.revision} needs ${proposedKind}`,
              BOT_SESSION_PROFILE_UNAPPLICABLE,
            );
          }
        }
        try {
          await client.call("session/profiles/apply", {
            sessionId: input.sessionId,
            profile: { kind: "inline", profile: resolvedProfile },
          });
        } catch (error) {
          if (error instanceof LightspeedRpcError && isProfileUnapplicable(error)) {
            throw ApplicationFailure.nonRetryable(
              `session ${input.sessionId} cannot take profile revision ${profile.revision}: ${error.message}`,
              BOT_SESSION_PROFILE_UNAPPLICABLE,
            );
          }
          // A busy session (`rejected`, no run may be active) is transient
          // and stays retryable.
          throw error;
        }
      }
      return {
        profileRevision: profile.revision,
        carriedToolIds: carried.map((declaration) => declaration.definition.toolId),
      };
    },

    async renameBotSession(input) {
      await clientForUniverse(config, input.universeId).call("session/rename", {
        sessionId: input.sessionId,
        displayName: input.displayName,
      });
    },

    async readBotSessionStatus(input) {
      const response = await clientForUniverse(config, input.universeId).call("session/read", {
        sessionId: input.sessionId,
      });
      return { status: response.result.session.status };
    },

    async readBotRunUsage(input) {
      const response = await clientForUniverse(config, input.universeId).call("session/read", {
        sessionId: input.sessionId,
      });
      const usage = (response.result.session.runs ?? []).find((run) => run.id === input.runId)?.usage;
      if (!usage?.inputTokens) return null;
      return { inputTokens: usage.inputTokens, cachedInputTokens: usage.cachedInputTokens ?? 0 };
    },

    async startBotRun(input) {
      const client = clientForUniverse(config, input.universeId);
      const response = await client.call("session/runs/start", {
        sessionId: input.sessionId,
        source: { type: "input", items: deliveryInputItems(input.events) },
        submissionId: input.submissionId,
        notifyOnTerminal: { token: input.terminalToken },
      });
      return { runId: response.result.run.id };
    },

    async steerBotRun(input) {
      const client = clientForUniverse(config, input.universeId);
      const session = (await client.call("session/read", { sessionId: input.sessionId })).result
        .session;
      const active = (session.runs ?? []).find((run) => run.status === "running");
      if (active === undefined) return { steered: false };
      try {
        await client.call("session/runs/steer", {
          sessionId: input.sessionId,
          runId: active.id,
          items: steerInputItems(input.events),
        });
      } catch {
        // The run reached terminal between read and steer; the caller falls
        // back to an ordinary run.
        return { steered: false };
      }
      return { steered: true, runId: active.id };
    },

    async closeBotSession(input) {
      const client = clientForUniverse(config, input.universeId);
      // Descendants first: the bot cannot see below its own sessions except
      // through lineage, and a routed session's sub-agents have no other
      // owner once it goes.
      let descendantsClosed = 0;
      const descendants = await client.call("session/list", {
        rootSessionId: input.sessionId,
        limit: 200,
      });
      const force = input.force === true;
      for (const child of descendants.result.sessions ?? []) {
        if (child.lifecycleStatus === "closed") continue;
        try {
          await client.call("session/close", { sessionId: child.id, force });
          descendantsClosed += 1;
        } catch (error) {
          if (error instanceof LightspeedRpcError) return { closed: false, descendantsClosed };
          throw error;
        }
      }
      try {
        await client.call("session/close", { sessionId: input.sessionId, force });
      } catch (error) {
        // Active work or an already-closed session: report and let the
        // retention sweep try again later. A forced close treats an
        // already-closed session as done — teardown must converge.
        if (error instanceof LightspeedRpcError) {
          if (force && (await isSessionClosed(client, input.sessionId))) {
            return { closed: true, descendantsClosed };
          }
          return { closed: false, descendantsClosed };
        }
        throw error;
      }
      return { closed: true, descendantsClosed };
    },

    async countBotDescendantSessions(input) {
      const client = clientForUniverse(config, input.universeId);
      let count = 0;
      for (const rootSessionId of input.sessionIds) {
        let cursor: string | null = null;
        for (let page = 0; page < DESCENDANT_COUNT_MAX_PAGES; page += 1) {
          const params: { rootSessionId: string; limit: number; cursor?: string } = {
            rootSessionId,
            limit: 200,
          };
          if (cursor !== null) params.cursor = cursor;
          const response = await client.call("session/list", params);
          for (const child of response.result.sessions ?? []) {
            if (child.createdAtMs >= input.sinceMs) count += 1;
          }
          cursor = response.result.nextCursor ?? null;
          if (cursor === null) break;
        }
      }
      return { count };
    },

    async appendBotContext(input) {
      const client = clientForUniverse(config, input.universeId);
      await client.call("session/context/append", {
        sessionId: input.sessionId,
        entries: input.events.map((event) => ({
          key: `bot:event:${event.id}`,
          item: { type: "textRef", blobRef: event.promptRef ?? event.ref },
        })),
      });
    },

    async readWorkflowToolInvocations(input) {
      const client = clientForUniverse(config, input.universeId);
      const invocations: PulledWorkflowToolInvocation[] = [];
      let cursor = input.afterSeq;
      for (;;) {
        const response = await client.call("session/events/read", {
          sessionId: input.sessionId,
          after: { seq: cursor },
          limit: 500,
        });
        for (const event of response.result.events ?? []) {
          cursor = Math.max(cursor, event.cursor.seq);
          // Every bound-tool invocation, not only the controller's own: the
          // caller correlates resolves and recognizes carried tools by id.
          if (event.kind.type === "workflowToolEmitted") {
            invocations.push({
              invocationId: event.kind.invocationId,
              toolId: event.kind.toolId,
              runId: event.kind.runId,
              argumentsRef: event.kind.argumentsRef,
            });
          }
        }
        cursor = response.result.nextCursor?.seq ?? cursor;
        if (response.result.complete || response.result.nextCursor == null) break;
      }
      return { nextSeq: cursor, invocations };
    },

    async readJsonBlob(input) {
      const client = clientForUniverse(config, input.universeId);
      const response = await client.call("blobs/read", { blobRef: input.blobRef });
      const bytes = Buffer.from(response.result.bytesBase64, "base64");
      if (bytes.byteLength !== response.result.bytes) {
        throw new TypeError(
          `blobs/read returned ${bytes.byteLength} bytes, expected ${response.result.bytes}`,
        );
      }
      return JSON.parse(bytes.toString("utf8")) as unknown;
    },
  };
}

type RpcClient = Pick<LightspeedClient, "call">;

type RunInputItem =
  | { type: "text"; text: string }
  | { type: "textRef"; blobRef: string }
  | {
      type: "media";
      blobRef: string;
      kind: "image" | "audio" | "document";
      mime: string;
      name?: string | null;
    };

/** One event as run input: its rendering, then any attachments it carried. */
function eventInputItems(event: Pick<BotEvent, "ref" | "promptRef" | "media">): RunInputItem[] {
  return [
    { type: "textRef", blobRef: event.promptRef ?? event.ref },
    ...(event.media ?? []).map((item) => ({
      type: "media" as const,
      blobRef: item.blobRef,
      kind: item.kind,
      mime: item.mime,
      ...(item.name == null ? {} : { name: item.name }),
    })),
  ];
}

/**
 * A delivery is the event renderings themselves — the standing protocol
 * (untrusted content, resolve semantics) lives in the session instructions,
 * so a single event needs no framing item at all. Only a batch gets a
 * one-line header binding it to one decision.
 */
export function deliveryInputItems(
  events: Pick<BotEvent, "ref" | "promptRef" | "media">[],
): RunInputItem[] {
  const items: RunInputItem[] = events.flatMap(eventInputItems);
  if (events.length > 1) {
    items.unshift({
      type: "text",
      text: `${events.length} events delivered as one batch — handle them together and resolve the delivery once.`,
    });
  }
  return items;
}

export function steerInputItems(
  events: Pick<BotEvent, "ref" | "promptRef" | "media">[],
): RunInputItem[] {
  return [
    {
      type: "text",
      text: `${events.length} more event(s) arrived while you were working — fold them into your current work where relevant.`,
    },
    ...events.flatMap(eventInputItems),
  ];
}

/**
 * Carried declarations are opaque data authored by the admitting source.
 * The only checks here are the ones that would otherwise fail inside the
 * core with a worse message: the shape, and a name collision with `bot_*`.
 */
async function readCarriedDeclarations(
  client: RpcClient,
  toolsRef: string,
): Promise<WorkflowToolDeclarationInput[]> {
  const response = await client.call("blobs/read", { blobRef: toolsRef });
  let parsed: unknown;
  try {
    parsed = JSON.parse(Buffer.from(response.result.bytesBase64, "base64").toString("utf8"));
  } catch {
    throw ApplicationFailure.nonRetryable("carried tool declarations are not JSON", BOT_CARRIED_TOOLS_INVALID);
  }
  return validateCarriedDeclarations(parsed);
}

export function validateCarriedDeclarations(value: unknown): WorkflowToolDeclarationInput[] {
  if (!Array.isArray(value)) {
    throw ApplicationFailure.nonRetryable("carried tool declarations must be an array", BOT_CARRIED_TOOLS_INVALID);
  }
  const seen = new Set<string>();
  for (const entry of value) {
    const declaration = entry as Partial<WorkflowToolDeclarationInput> | null;
    const name = declaration?.definition?.tool?.name;
    const toolId = declaration?.definition?.toolId;
    if (typeof name !== "string" || typeof toolId !== "string" || declaration?.target === undefined) {
      throw ApplicationFailure.nonRetryable("carried tool declaration is malformed", BOT_CARRIED_TOOLS_INVALID);
    }
    if (BOT_TOOL_NAMES.has(name) || seen.has(name)) {
      throw ApplicationFailure.nonRetryable(`carried tool ${name} collides with a declared tool`, BOT_CARRIED_TOOLS_INVALID);
    }
    seen.add(name);
  }
  return value as WorkflowToolDeclarationInput[];
}

async function readProfileInstructions(client: RpcClient, profile: AgentProfile): Promise<string> {
  const instructions = profile.instructions;
  if (instructions == null) return "";
  if (instructions.type === "text") return instructions.text;
  const response = await client.call("blobs/read", { blobRef: instructions.blobRef });
  return Buffer.from(response.result.bytesBase64, "base64").toString("utf8");
}

async function putToolAssets(client: RpcClient): Promise<{
  schemas: BotToolSchemaRefs;
  descriptions: BotToolDescriptionRefs;
}> {
  const schemaNames = Object.keys(BOT_TOOL_SCHEMAS) as (keyof typeof BOT_TOOL_SCHEMAS)[];
  const descriptionNames = Object.keys(
    BOT_TOOL_DESCRIPTIONS,
  ) as (keyof typeof BOT_TOOL_DESCRIPTIONS)[];
  const response = await client.call("blobs/put", {
    blobs: [
      ...schemaNames.map((name) => ({
        bytesBase64: Buffer.from(JSON.stringify(BOT_TOOL_SCHEMAS[name]), "utf8").toString("base64"),
      })),
      ...descriptionNames.map((name) => ({
        bytesBase64: Buffer.from(BOT_TOOL_DESCRIPTIONS[name], "utf8").toString("base64"),
      })),
    ],
  });
  const blobs = response.result.blobs ?? [];
  if (blobs.length !== schemaNames.length + descriptionNames.length) {
    throw new Error(`blobs/put returned ${blobs.length} tool assets`);
  }
  return {
    schemas: Object.fromEntries(
      schemaNames.map((name, index) => [name, requireRef(blobs[index]?.blobRef)]),
    ) as BotToolSchemaRefs,
    descriptions: Object.fromEntries(
      descriptionNames.map((name, index) => [
        name,
        requireRef(blobs[schemaNames.length + index]?.blobRef),
      ]),
    ) as BotToolDescriptionRefs,
  };
}

function requireRef(ref: string | undefined): string {
  if (ref === undefined) throw new Error("blobs/put omitted a blob ref");
  return ref;
}

function clientForUniverse(config: BotLightspeedConfig, universeId: string): LightspeedClient {
  return new LightspeedClient({
    endpoint: config.endpoint,
    ...(config.fetch === undefined ? {} : { fetch: config.fetch }),
    headers: { "x-lightspeed-universe": universeId },
  });
}
