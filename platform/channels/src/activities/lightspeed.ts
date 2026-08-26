import {
  LightspeedClient,
  type ContextEntryView,
  type SessionView,
} from "@lightspeed/agent-client";
import type { LightspeedActivities } from "../contracts/bridge.js";
import {
  CHANNEL_TOOL_DESCRIPTIONS,
  CHANNEL_TOOL_SCHEMAS,
  channelWorkflowTools,
  type ChannelToolDescriptionName,
  type ChannelToolDescriptionRefs,
  type ChannelToolSchemaName,
  type ChannelToolSchemaRefs,
} from "../contracts/tools.js";

export interface LightspeedActivityConfig {
  endpoint: string;
  fetch?: typeof fetch;
}

export function createLightspeedActivities(config: LightspeedActivityConfig): LightspeedActivities {
  return {
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
    async putJsonBlob(input) {
      const client = clientForUniverse(config, input.universeId);
      const response = await client.call("blobs/put", {
        blobs: [
          {
            bytesBase64: Buffer.from(JSON.stringify(input.value), "utf8").toString("base64"),
          },
        ],
      });
      const blob = response.result.blobs?.[0];
      if (blob === undefined) {
        throw new Error("blobs/put omitted JSON blob result");
      }
      return { blobRef: blob.blobRef };
    },
    async putChatToolDeclarations(input) {
      const client = clientForUniverse(config, input.universeId);
      const refs = await putToolAssets(client);
      const declarations = channelWorkflowTools(input.receiver, refs.schemas, refs.descriptions);
      const response = await client.call("blobs/put", {
        blobs: [
          { bytesBase64: Buffer.from(JSON.stringify(declarations), "utf8").toString("base64") },
        ],
      });
      const blob = response.result.blobs?.[0];
      if (blob === undefined) {
        throw new Error("blobs/put omitted the tool declaration blob");
      }
      return {
        toolsRef: blob.blobRef,
        toolIds: declarations.map((declaration) => declaration.definition.toolId),
      };
    },
    async reconcileDelivery(input) {
      if (input.status === "run_failed") {
        return { action: "deliver", text: "I couldn't complete that request." };
      }
      if (input.runId === null) {
        return { action: "suppress", reason: "no_run" };
      }
      const client = clientForUniverse(config, input.universeId);
      const response = await client.call("session/read", { sessionId: input.sessionId });
      if (runUsedMessagingTool(response.result.session, input.runId)) {
        return { action: "suppress", reason: "messaging_tool" };
      }
      return {
        action: "deliver",
        text:
          extractAssistantText(response.result.session, input.runId) ??
          "Lightspeed completed the run, but no assistant text was available.",
      };
    },
  };
}

export function runUsedMessagingTool(session: SessionView, runId: string): boolean {
  const run = session.runs?.find((candidate) => candidate.id === runId);
  if (run?.entries === undefined) {
    return false;
  }
  const messagingCalls = new Set<string>();
  for (const entry of run.entries) {
    if (entry.kind.type === "toolCall" && entry.kind.name.startsWith("message_")) {
      messagingCalls.add(entry.kind.callId);
    }
  }
  return run.entries.some(
    (entry) =>
      entry.kind.type === "toolResult" &&
      !entry.kind.isError &&
      messagingCalls.has(entry.kind.callId),
  );
}

export function extractAssistantText(session: SessionView, runId: string): string | null {
  const run = session.runs?.find((candidate) => candidate.id === runId);
  const texts = assistantTexts(run?.entries);
  return texts.length === 0 ? null : texts.join("\n\n");
}

function assistantTexts(entries: readonly ContextEntryView[] | undefined): string[] {
  return (entries ?? []).flatMap((entry) => {
    if (entry.kind.type !== "message" || entry.kind.role !== "assistant") {
      return [];
    }
    const text = entry.text?.trim();
    return text ? [text] : [];
  });
}

type RpcClient = Pick<LightspeedClient, "call">;

export async function putToolAssets(client: RpcClient): Promise<{
  schemas: ChannelToolSchemaRefs;
  descriptions: ChannelToolDescriptionRefs;
}> {
  const schemaEntries = Object.entries(CHANNEL_TOOL_SCHEMAS) as [
    ChannelToolSchemaName,
    (typeof CHANNEL_TOOL_SCHEMAS)[ChannelToolSchemaName],
  ][];
  const descriptionEntries = Object.entries(CHANNEL_TOOL_DESCRIPTIONS) as [
    ChannelToolDescriptionName,
    (typeof CHANNEL_TOOL_DESCRIPTIONS)[ChannelToolDescriptionName],
  ][];
  const response = await client.call("blobs/put", {
    blobs: [
      ...schemaEntries.map(([, schema]) => ({
        bytesBase64: Buffer.from(JSON.stringify(schema), "utf8").toString("base64"),
      })),
      ...descriptionEntries.map(([, description]) => ({
        bytesBase64: Buffer.from(description, "utf8").toString("base64"),
      })),
    ],
  });
  const blobs = response.result.blobs ?? [];
  const expectedCount = schemaEntries.length + descriptionEntries.length;
  if (blobs.length !== expectedCount) {
    throw new Error(`blobs/put returned ${blobs.length} tool assets, expected ${expectedCount}`);
  }
  const schemas = Object.fromEntries(
    schemaEntries.map(([name], index) => {
      const blob = blobs[index];
      if (!blob) {
        throw new Error(`blobs/put omitted schema ${name}`);
      }
      return [name, blob.blobRef];
    }),
  ) as ChannelToolSchemaRefs;
  const descriptions = Object.fromEntries(
    descriptionEntries.map(([name], index) => {
      const blob = blobs[schemaEntries.length + index];
      if (!blob) {
        throw new Error(`blobs/put omitted description ${name}`);
      }
      return [name, blob.blobRef];
    }),
  ) as ChannelToolDescriptionRefs;
  return { schemas, descriptions };
}

function clientForUniverse(config: LightspeedActivityConfig, universeId: string): LightspeedClient {
  return new LightspeedClient({
    endpoint: config.endpoint,
    ...(config.fetch === undefined ? {} : { fetch: config.fetch }),
    headers: { "x-lightspeed-universe": universeId },
  });
}
