import { ApplicationFailure } from "@temporalio/common";
import {
  LightspeedClient,
  LightspeedRpcError,
  type AgentProfile,
  type SessionStatus,
} from "@lightspeed/agent-client";

import {
  BOT_EVENT_RESOLVE_TOOL_ID,
  BOT_SESSION_DECLARATION_MISMATCH,
  BOT_TOOL_DESCRIPTIONS,
  BOT_TOOL_SCHEMAS,
  botWorkflowTools,
  resolveBotProfile,
  type BotEvent,
  type BotToolDescriptionRefs,
  type BotToolSchemaRefs,
} from "../contracts/bots.js";

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
  appliedProfileRevision?: number | null;
  controller: { workflowId: string; workflowKind: string };
}

export interface EnsureBotSessionResult {
  profileRevision: number;
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

export interface BotLightspeedActivities {
  ensureBotSession(input: EnsureBotSessionInput): Promise<EnsureBotSessionResult>;
  readBotSessionStatus(input: ReadSessionInput): Promise<{ status: SessionStatus }>;
  startBotRun(input: StartBotRunInput): Promise<{ runId: string }>;
  steerBotRun(input: SteerBotRunInput): Promise<{ steered: boolean; runId?: string }>;
  appendBotContext(input: AppendBotContextInput): Promise<void>;
  /** Close an idle routed session (non-force: a busy session is left alone). */
  closeBotSession(input: ReadSessionInput): Promise<{ closed: boolean }>;
  readWorkflowToolInvocations(
    input: ReadWorkflowToolInvocationsInput,
  ): Promise<ReadWorkflowToolInvocationsResult>;
  readJsonBlob(input: ReadJsonBlobInput): Promise<unknown>;
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
      try {
        await client.call("session/managed/start", {
          sessionId: input.sessionId,
          displayName: input.displayName,
          profile: { kind: "inline", profile: resolvedProfile },
          workflowTools: {
            version: 1,
            lifecycleController: input.controller,
            tools: botWorkflowTools(input.controller, refs.schemas, refs.descriptions),
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
        await client.call("session/profiles/apply", {
          sessionId: input.sessionId,
          profile: { kind: "inline", profile: resolvedProfile },
        });
      }
      return { profileRevision: profile.revision };
    },

    async readBotSessionStatus(input) {
      const response = await clientForUniverse(config, input.universeId).call("session/read", {
        sessionId: input.sessionId,
      });
      return { status: response.result.session.status };
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
      try {
        await client.call("session/close", { sessionId: input.sessionId });
      } catch (error) {
        // Active work or an already-closed session: report and let the
        // retention sweep try again later.
        if (error instanceof LightspeedRpcError) return { closed: false };
        throw error;
      }
      return { closed: true };
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
          if (
            event.kind.type === "workflowToolEmitted" &&
            event.kind.toolId === BOT_EVENT_RESOLVE_TOOL_ID
          ) {
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

type RunInputItem = { type: "text"; text: string } | { type: "textRef"; blobRef: string };

/**
 * A delivery is the event renderings themselves — the standing protocol
 * (untrusted content, resolve semantics) lives in the session instructions,
 * so a single event needs no framing item at all. Only a batch gets a
 * one-line header binding it to one decision.
 */
export function deliveryInputItems(events: Pick<BotEvent, "ref" | "promptRef">[]): RunInputItem[] {
  const items: RunInputItem[] = events.map((event) => ({
    type: "textRef",
    blobRef: event.promptRef ?? event.ref,
  }));
  if (events.length > 1) {
    items.unshift({
      type: "text",
      text: `${events.length} events delivered as one batch — handle them together and resolve the delivery once.`,
    });
  }
  return items;
}

export function steerInputItems(events: Pick<BotEvent, "ref" | "promptRef">[]): RunInputItem[] {
  return [
    {
      type: "text",
      text: `${events.length} more event(s) arrived while you were working — fold them into your current work where relevant.`,
    },
    ...events.map((event) => ({
      type: "textRef" as const,
      blobRef: event.promptRef ?? event.ref,
    })),
  ];
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
