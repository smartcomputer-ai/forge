import type { NormalizedInboundV1 } from "../contracts/channel.js";

export type GroupActivation = "mention" | "always";
export type ActivationPolicy = "dm" | GroupActivation;

/**
 * When a conversation's messages become bot events. Batching is the chat
 * trigger's coalescing; silent rooms (ambient context without runs) are not
 * a v1 mode — a group message that does not activate is dropped.
 */
export interface ChannelActivationSettings {
  mode: ActivationPolicy;
  triggerPrefixes: string[];
  mentionNames: string[];
}

export interface StoredChannelActivationSettings {
  group?: GroupActivation;
  triggerPrefixes?: string[];
  mentionNames?: string[];
}

export type InboundClassification =
  | { kind: "drop"; reason: "empty" | "empty-trigger" | "ambient" }
  | { kind: "userTurn"; text: string };

const DEFAULT_TRIGGER_PREFIXES = ["/ask", "/lightspeed"];

export function resolveActivationSettings(
  scope: "direct" | "group",
  value: unknown,
): ChannelActivationSettings {
  const stored = parseStoredActivationSettings(value);
  return {
    mode: scope === "direct" ? "dm" : (stored.group ?? "mention"),
    triggerPrefixes: uniqueNonEmpty(stored.triggerPrefixes ?? DEFAULT_TRIGGER_PREFIXES),
    mentionNames: uniqueNonEmpty(stored.mentionNames ?? []),
  };
}

export function classifyInbound(
  inbound: NormalizedInboundV1,
  settings: ChannelActivationSettings,
): InboundClassification {
  const text = inbound.text.trim();
  if (text.length === 0 && inbound.media === undefined) {
    return { kind: "drop", reason: "empty" };
  }

  const triggered = text.length === 0 ? null : extractTriggeredText(text, settings);
  if (triggered !== null) {
    return triggered.length === 0 && inbound.media === undefined
      ? { kind: "drop", reason: "empty-trigger" }
      : { kind: "userTurn", text: triggered };
  }
  if (inbound.isDirect || settings.mode === "dm" || settings.mode === "always") {
    return { kind: "userTurn", text };
  }
  if (inbound.mentionedBot || inbound.isReplyToBot) {
    return { kind: "userTurn", text: stripNamedMention(text, settings.mentionNames) };
  }
  return { kind: "drop", reason: "ambient" };
}

/** The one-line rendering of a message for the model: who, when, what. Never a provider id. */
export function formatMessageLine(
  inbound: Pick<NormalizedInboundV1, "senderName" | "timestampMs">,
  text: string,
): string {
  return `${inbound.senderName} (${formatTimestamp(inbound.timestampMs)}): ${text}`;
}

function parseStoredActivationSettings(value: unknown): StoredChannelActivationSettings {
  if (value === null || value === undefined) {
    return {};
  }
  if (typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("chat activation must be an object");
  }
  const input = value as Record<string, unknown>;
  if (input.group !== undefined && input.group !== "mention" && input.group !== "always") {
    throw new TypeError("chat activation.group must be mention or always");
  }
  for (const key of ["triggerPrefixes", "mentionNames"] as const) {
    if (
      input[key] !== undefined &&
      (!Array.isArray(input[key]) || input[key].some((entry) => typeof entry !== "string"))
    ) {
      throw new TypeError(`chat activation.${key} must be a string array`);
    }
  }
  return input as StoredChannelActivationSettings;
}

function extractTriggeredText(text: string, settings: ChannelActivationSettings): string | null {
  for (const prefix of settings.triggerPrefixes) {
    const slashMatch = new RegExp(`^${escapeRegExp(prefix)}(?:@[\\w_]+)?(?:\\s+|$)`, "i");
    if (slashMatch.test(text)) {
      return text.replace(slashMatch, "").trim();
    }
  }
  for (const name of settings.mentionNames) {
    const mention = name.replace(/^@/, "");
    const pattern = new RegExp(`^@${escapeRegExp(mention)}(?:[:,]?\\s+|$)`, "i");
    if (pattern.test(text)) {
      return text.replace(pattern, "").trim();
    }
  }
  return null;
}

function stripNamedMention(text: string, names: readonly string[]): string {
  let result = text;
  for (const value of names) {
    const name = value.replace(/^@/, "");
    result = result.replace(new RegExp(`@${escapeRegExp(name)}\\b[:,]?\\s*`, "i"), " ");
  }
  return result.replace(/\s+/g, " ").trim() || text;
}

function uniqueNonEmpty(values: readonly string[]): string[] {
  return [...new Set(values.map((value) => value.trim()).filter(Boolean))];
}

export function formatTimestamp(timestampMs: number): string {
  const date = new Date(timestampMs);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())} ${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}Z`;
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
