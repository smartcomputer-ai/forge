import { createHmac, createHash, timingSafeEqual } from "node:crypto";
import { evaluate } from "cel-js";
import type { BotTriggerRoute, BotWebhookTriggerSpec } from "@lightspeed/platform-db/schema";
import {
  botKeyedSessionId,
  botPerEventSessionId,
  type BotEventSession,
} from "@lightspeed/bots/contracts";

/** Headers that must never be persisted into an event document. */
const REDACTED_HEADERS = new Set(["authorization", "cookie", "set-cookie", "proxy-authorization"]);
const HEADER_VALUE_CAP = 500;
const HEADER_COUNT_CAP = 40;

export interface WebhookExtraction {
  eventId: string;
  kind: string;
  summary: string;
  data?: unknown;
  headers: Record<string, string>;
}

export function constantTimeEquals(a: string, b: string): boolean {
  const left = Buffer.from(a, "utf8");
  const right = Buffer.from(b, "utf8");
  if (left.byteLength !== right.byteLength) return false;
  return timingSafeEqual(left, right);
}

/** Verify the URL token plus the spec's signature scheme against the raw body. */
export function verifyWebhook(
  spec: BotWebhookTriggerSpec,
  urlToken: string,
  rawBody: Buffer,
  headers: Record<string, string>,
): { ok: true } | { ok: false; reason: string } {
  if (!constantTimeEquals(spec.token, urlToken)) return { ok: false, reason: "unknown endpoint" };
  const verification = spec.verification;
  if (verification.scheme === "token") return { ok: true };
  const provided = headers[verification.header.toLowerCase()];
  if (!provided) return { ok: false, reason: `missing ${verification.header} header` };
  const prefix = verification.prefix ?? "";
  if (prefix && !provided.startsWith(prefix)) {
    return { ok: false, reason: "signature prefix mismatch" };
  }
  const expected = createHmac("sha256", verification.secret).update(rawBody).digest("hex");
  if (!constantTimeEquals(provided.slice(prefix.length).toLowerCase(), expected)) {
    return { ok: false, reason: "signature mismatch" };
  }
  return { ok: true };
}

/** Normalize incoming headers: lowercase names, redact credentials, cap sizes. */
export function sanitizeHeaders(raw: Record<string, string>): Record<string, string> {
  const result: Record<string, string> = {};
  let count = 0;
  for (const [name, value] of Object.entries(raw)) {
    const lower = name.toLowerCase();
    if (REDACTED_HEADERS.has(lower)) continue;
    if (count >= HEADER_COUNT_CAP) break;
    result[lower] = value.slice(0, HEADER_VALUE_CAP);
    count += 1;
  }
  return result;
}

/**
 * Turn a verified delivery into event identity and description. Presets know
 * the provider's envelope; the generic path stays deliberately dumb: dedupe
 * by body digest, name events from a `kind` field when one exists.
 */
export function extractWebhookEvent(
  trigger: { name: string; spec: BotWebhookTriggerSpec },
  rawBody: Buffer,
  rawHeaders: Record<string, string>,
): WebhookExtraction {
  const headers = sanitizeHeaders(rawHeaders);
  let data: unknown;
  try {
    data = JSON.parse(rawBody.toString("utf8"));
  } catch {
    data = undefined;
  }

  if (trigger.spec.preset === "github") {
    const body = asRecord(data);
    const ghEvent = headers["x-github-event"] ?? "unknown";
    const action = typeof body?.action === "string" ? body.action : null;
    const kind = action ? `${ghEvent}.${action}` : ghEvent;
    const repository = asRecord(body?.repository);
    const repoName = typeof repository?.full_name === "string" ? repository.full_name : null;
    return {
      eventId: headers["x-github-delivery"] ?? bodyDigestId(rawBody),
      kind,
      summary: `GitHub ${kind}${repoName ? ` in ${repoName}` : ""}`,
      ...(data === undefined ? {} : { data }),
      headers,
    };
  }

  const body = asRecord(data);
  const kind = typeof body?.kind === "string" && body.kind ? body.kind.slice(0, 200) : "webhook";
  return {
    eventId: bodyDigestId(rawBody),
    kind,
    summary: `Webhook ${kind} received on trigger ${trigger.name}`,
    ...(data === undefined ? {} : { data }),
    headers,
  };
}

export interface FilterContext {
  event: { id: string; kind: string; source: string; occurredAt: string };
  data: unknown;
  headers: Record<string, string>;
}

/**
 * CEL filter over the admission context. Fail closed: an evaluation error
 * archives the event rather than delivering it, and the error is reported so
 * the activity feed can explain the skip.
 */
export function evaluateFilter(
  filter: string,
  context: FilterContext,
): { matched: boolean; error?: string } {
  try {
    const result = evaluate(filter, {
      event: context.event,
      data: context.data ?? {},
      headers: context.headers,
    });
    return { matched: result === true };
  } catch (error) {
    return { matched: false, error: error instanceof Error ? error.message : String(error) };
  }
}

/**
 * Compute the routing target at admission, where the payload is available.
 * Returns undefined for the main session. Key errors fall back to a shared
 * "default" key so events are never dropped by a broken expression.
 */
export function computeRouteSession(
  botName: string,
  route: BotTriggerRoute | null,
  preset: "github" | null | undefined,
  extraction: { eventId: string; data?: unknown },
  context: FilterContext,
): { session?: BotEventSession; error?: string } {
  if (route === null || route.policy === "bot") return {};
  if (route.policy === "perEvent") {
    return {
      session: {
        sessionId: botPerEventSessionId(botName, extraction.eventId),
        label: `event ${extraction.eventId.slice(0, 24)}`,
      },
    };
  }
  let key: string | undefined;
  let error: string | undefined;
  if (route.key) {
    try {
      const value = evaluate(route.key, {
        event: context.event,
        data: context.data ?? {},
        headers: context.headers,
      });
      if (typeof value === "string" && value) key = value.slice(0, 200);
      else if (typeof value === "number") key = String(value);
      else error = `route key evaluated to ${typeof value}`;
    } catch (evalError) {
      error = evalError instanceof Error ? evalError.message : String(evalError);
    }
  }
  key ??= presetRouteKey(preset, extraction.data) ?? "default";
  const result: { session: BotEventSession; error?: string } = {
    session: { sessionId: botKeyedSessionId(botName, key), label: key },
  };
  if (error !== undefined) result.error = error;
  return result;
}

function presetRouteKey(preset: "github" | null | undefined, data: unknown): string | undefined {
  if (preset !== "github") return undefined;
  const body = asRecord(data);
  const pullRequest = asRecord(body?.pull_request);
  if (typeof pullRequest?.number === "number") return `pr-${pullRequest.number}`;
  const issue = asRecord(body?.issue);
  if (typeof issue?.number === "number") return `issue-${issue.number}`;
  const repository = asRecord(body?.repository);
  if (typeof repository?.full_name === "string") return repository.full_name;
  return undefined;
}

function bodyDigestId(rawBody: Buffer): string {
  return `whk-${createHash("sha256").update(rawBody).digest("hex")}`;
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return undefined;
  return value as Record<string, unknown>;
}
