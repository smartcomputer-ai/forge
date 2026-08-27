import { createHash } from "node:crypto";
import { and, count, eq, gte } from "drizzle-orm";
import { LightspeedClient, LightspeedRpcError } from "@lightspeed/agent-client";
import { schema, type Db } from "@lightspeed/platform-db";
import type { Client } from "@temporalio/client";
import { deleteBotSchedule } from "../schedules.js";
import {
  botPollEventId,
  type BotEvent,
  type BotEventDocumentV1,
  type BotStartV1,
} from "../contracts/bots.js";
import { storeBotEvent } from "../admission.js";
import {
  MAX_POLL_CONSECUTIVE_FAILURES,
  MAX_POLL_ITEMS_PER_FIRE,
  diffPollItems,
  extractPollItems,
  parsePollPayload,
  pollItemSummary,
  type PollCursorState,
} from "../poll.js";
import { computeRouteSession, evaluateFilter, type FilterContext } from "../webhooks.js";
import { GrantLeaseCache, type GrantLeaseRequest } from "../credentials.js";

const HTTP_TIMEOUT_MS = 30_000;
const MAX_PAYLOAD_BYTES = 1024 * 1024;
const EXEC_DEFAULT_TIMEOUT_MS = 60_000;
const EXEC_READ_INTERVAL_MS = 2_000;
const NON_TERMINAL_JOB_STATUS = new Set(["accepted", "queued", "running", "cancelRequested"]);

export interface BotPollActivitiesConfig {
  db: Db;
  endpoint: string;
  temporal: Client;
  fetch?: typeof fetch;
}

export interface PollBotTriggerInput {
  botId: string;
  triggerId: string;
  scheduledAt: string;
}

export type PollBotTriggerResult =
  | { polled: true; baselined: boolean; admitted: number; filtered: number }
  | {
      polled: false;
      reason:
        | "trigger_missing"
        | "trigger_disabled"
        | "bot_disabled"
        | "breaker_tripped"
        | "poll_failed";
    };

export interface BotPollActivities {
  pollBotTrigger(input: PollBotTriggerInput): Promise<PollBotTriggerResult>;
}

interface PollSpecRow {
  source:
    | {
        kind: "http";
        url: string;
        method?: "GET" | "POST";
        headers?: Record<string, string>;
        auth?: { grantId: string; header?: string; scheme?: string; audience?: string };
        body?: string;
      }
    | {
        kind: "exec";
        /** Null: the bot's own environment (the profile's `existing` one), resolved at fire time. */
        environmentId?: string | null;
        argv: string[];
        cwd?: string | null;
        timeoutMs?: number | null;
      };
  intervalMs: number;
  items: string | null;
  cursor: { kind: "idSet"; id: string } | { kind: "watermark"; field: string };
}

/**
 * The environment an exec poll without an explicit `environmentId` runs in:
 * the `existing` environment of the bot's profile. A profile with another
 * intent (none, per-session provision, inherit) cannot run such a poll —
 * that is a configuration error, not a transient failure.
 */
export async function resolveBotProfileEnvironment(
  client: Pick<LightspeedClient, "call">,
  profileId: string,
): Promise<string> {
  const profile = (await client.call("profiles/read", { profileId })).result.profile;
  const environment = profile.environment;
  if (environment?.type !== "existing") {
    throw new Error(
      `the poll names no environment and profile ${profileId} does not activate an existing one: set environmentId on the trigger, or point the profile at an existing environment`,
    );
  }
  return environment.environmentId;
}

type PollHttpSource = Extract<PollSpecRow["source"], { kind: "http" }>;
const FORBIDDEN_POLL_HEADERS = new Set([
  "authorization",
  "proxy-authorization",
  "cookie",
  "set-cookie",
  "x-api-key",
  "api-key",
]);

export async function fetchHttpPollPayload(input: {
  universeId: string;
  source: PollHttpSource;
  client: Pick<LightspeedClient, "call">;
  leaseCache: GrantLeaseCache;
  fetch: typeof fetch;
}): Promise<unknown> {
  const { universeId, source, client, leaseCache, fetch: doFetch } = input;
  for (const name of Object.keys(source.headers ?? {})) {
    if (FORBIDDEN_POLL_HEADERS.has(name.toLowerCase())) {
      throw new Error(`poll credential header ${name} must use auth.grantId`);
    }
  }
  const leaseRequest: GrantLeaseRequest | null = source.auth
    ? {
        cacheScope: universeId,
        grantId: source.auth.grantId,
        ...(source.auth.audience === undefined ? {} : { audience: source.auth.audience }),
      }
    : null;
  const request = async (): Promise<Response> => {
    const headers = new Headers(source.headers);
    if (source.auth && leaseRequest) {
      const token = await leaseCache.lease(client, leaseRequest);
      headers.set(
        source.auth.header ?? "authorization",
        credentialHeaderValue(token, source.auth.scheme),
      );
    }
    return doFetch(source.url, {
      method: source.method ?? "GET",
      headers,
      ...(source.body === undefined ? {} : { body: source.body }),
      signal: AbortSignal.timeout(HTTP_TIMEOUT_MS),
    });
  };
  let response = await request();
  if (leaseRequest && (response.status === 401 || response.status === 403)) {
    leaseCache.invalidate(leaseRequest);
    response = await request();
  }
  if (!response.ok) throw new Error(`poll source responded ${response.status}`);
  const text = await response.text();
  if (text.length > MAX_PAYLOAD_BYTES) {
    throw new Error(`poll payload exceeds ${MAX_PAYLOAD_BYTES} bytes`);
  }
  return parsePollPayload(text, "response body");
}

export function createBotPollActivities(config: BotPollActivitiesConfig): BotPollActivities {
  const leaseCache = new GrantLeaseCache();
  const clientFor = (universeId: string) =>
    new LightspeedClient({
      endpoint: config.endpoint,
      ...(config.fetch === undefined ? {} : { fetch: config.fetch }),
      headers: {
        "x-lightspeed-universe": universeId,
        "x-lightspeed-principal": "service_account:lightspeed-bots",
      },
    });



  /** Fetch the HTTP source with a wall-clock timeout and a size cap. */
  async function fetchHttpPayload(
    universeId: string,
    source: Extract<PollSpecRow["source"], { kind: "http" }>,
  ): Promise<unknown> {
    return fetchHttpPollPayload({
      universeId,
      source,
      client: clientFor(universeId),
      leaseCache,
      fetch: config.fetch ?? fetch,
    });
  }

  /**
   * Run the command as a one-shot environment job and parse its stdout as
   * JSON. A sleeping environment surfaces as the typed
   * `environment_not_ready` error, which is rethrown for Temporal's activity
   * retry to absorb while the wake completes.
   */
  async function fetchExecPayload(
    universeId: string,
    triggerId: string,
    scheduledAt: string,
    source: Extract<PollSpecRow["source"], { kind: "exec" }>,
    profileId: string,
  ): Promise<unknown> {
    const client = clientFor(universeId);
    const environmentId =
      source.environmentId ?? (await resolveBotProfileEnvironment(client, profileId));
    const budgetMs = source.timeoutMs ?? EXEC_DEFAULT_TIMEOUT_MS;
    const requestId = `poll-${digestHex(`${triggerId}:${scheduledAt}`).slice(0, 24)}`;
    const created = await client.call("environments/jobs/create", {
      environmentId,
      requestId,
      jobs: [
        {
          name: "poll",
          argv: source.argv,
          ...(source.cwd == null ? {} : { cwd: source.cwd }),
          timeoutMs: budgetMs,
        },
      ],
    });
    const started = created.result.jobs?.[0];
    if (!started) throw new Error("environment job start returned no job");
    const handle = { environmentId, jobId: started.jobId };
    const deadline = Date.now() + budgetMs + 30_000;
    let afterSeq: number | undefined;
    let stdout = "";
    let stderrTail = "";
    for (;;) {
      const read = await client.call("environments/jobs/read", {
        jobs: [handle],
        ...(afterSeq === undefined ? {} : { afterSeq }),
        includeArtifacts: false,
      });
      const entry = read.result.jobs?.[0];
      if (!entry) throw new Error("environment job read returned no entry");
      if (entry.error !== undefined) throw new Error(`environment job read failed: ${entry.error}`);
      for (const chunk of entry.outputChunks ?? []) {
        const data = Buffer.from(chunk.dataBase64, "base64").toString("utf8");
        if (chunk.stream === "stdout") stdout += data;
        else stderrTail = (stderrTail + data).slice(-2_000);
      }
      if (stdout.length > MAX_PAYLOAD_BYTES) {
        throw new Error(`poll payload exceeds ${MAX_PAYLOAD_BYTES} bytes`);
      }
      afterSeq = entry.outputNextSeq;
      const status = entry.summary?.status;
      if (status !== undefined && !NON_TERMINAL_JOB_STATUS.has(status)) {
        if (status !== "succeeded") {
          throw new Error(
            `poll command ended ${status}${stderrTail ? `: ${stderrTail.slice(-500)}` : ""}`,
          );
        }
        return parsePollPayload(stdout, "stdout");
      }
      if (Date.now() > deadline) {
        await client
          .call("environments/jobs/cancel", { jobs: [handle], scope: "job", force: false })
          .catch(() => undefined);
        throw new Error(`poll command did not finish within ${budgetMs}ms`);
      }
      await new Promise((resolve) => setTimeout(resolve, EXEC_READ_INTERVAL_MS));
    }
  }

  return {
    async pollBotTrigger(input) {
      const [row] = await config.db
        .select({
          trigger: schema.botTriggers,
          bot: schema.bots,
          lightspeedUniverseId: schema.universes.lightspeedUniverseId,
        })
        .from(schema.botTriggers)
        .innerJoin(schema.bots, eq(schema.botTriggers.botId, schema.bots.id))
        .innerJoin(schema.universes, eq(schema.bots.universeId, schema.universes.id))
        .where(eq(schema.botTriggers.id, input.triggerId))
        .limit(1);
      if (!row || row.bot.id !== input.botId || row.trigger.kind !== "poll") {
        return { polled: false, reason: "trigger_missing" };
      }
      if (!row.trigger.enabled) return { polled: false, reason: "trigger_disabled" };
      if (!row.bot.enabled) return { polled: false, reason: "bot_disabled" };
      const spec = row.trigger.spec as PollSpecRow;
      const state = (row.trigger.cursor as PollCursorState | null) ?? null;
      const nowIso = new Date().toISOString();

      // The flood breaker applies to polls exactly as to webhooks and
      // schedules: a source that suddenly floods new items disables the
      // trigger for a human to look at.
      const breaker = row.bot.breaker;
      if (breaker) {
        const since = new Date(Date.now() - breaker.windowMs);
        const [recent] = await config.db
          .select({ value: count() })
          .from(schema.botEvents)
          .where(
            and(
              eq(schema.botEvents.triggerId, row.trigger.id),
              gte(schema.botEvents.receivedAt, since),
            ),
          );
        if (Number(recent?.value ?? 0) >= breaker.fires) {
          await disableTrigger(row, "breaker_tripped",
            `poll trigger ${row.trigger.name} exceeded ${breaker.fires} events in ${Math.round(breaker.windowMs / 1000)}s and was disabled`);
          return { polled: false, reason: "breaker_tripped" };
        }
      }

      let payload: unknown;
      try {
        payload =
          spec.source.kind === "http"
            ? await fetchHttpPayload(row.lightspeedUniverseId, spec.source)
            : await fetchExecPayload(
                row.lightspeedUniverseId,
                row.trigger.id,
                input.scheduledAt,
                spec.source,
                row.bot.profileId,
              );
      } catch (error) {
        // A sleeping environment is not a failure: the resolver has begun
        // the wake; let Temporal's activity retry absorb the latency.
        if (error instanceof LightspeedRpcError && error.kind === "environment_not_ready") {
          throw error;
        }
        const failures = (state?.consecutiveFailures ?? 0) + 1;
        await config.db
          .update(schema.botTriggers)
          .set({
            cursor: { ...(state ?? {}), consecutiveFailures: failures, lastPolledAt: nowIso },
          })
          .where(eq(schema.botTriggers.id, row.trigger.id));
        if (failures >= MAX_POLL_CONSECUTIVE_FAILURES) {
          await disableTrigger(row, "poll_disabled",
            `poll trigger ${row.trigger.name} failed ${failures} times in a row and was disabled`);
        }
        return { polled: false, reason: "poll_failed" };
      }

      const items = extractPollItems(payload, spec.items);
      const diff = diffPollItems(state, items, spec.cursor, nowIso);
      if (diff.baselined) {
        await config.db
          .update(schema.botTriggers)
          .set({ cursor: diff.nextState })
          .where(eq(schema.botTriggers.id, row.trigger.id));
        return { polled: true, baselined: true, admitted: 0, filtered: 0 };
      }

      const fresh = diff.newItems.slice(0, MAX_POLL_ITEMS_PER_FIRE);
      if (diff.newItems.length > fresh.length) {
      }

      const client = clientFor(row.lightspeedUniverseId);
      const source = `poll:${row.trigger.name}`;
      let admitted = 0;
      let filtered = 0;
      for (const entry of fresh) {
        const eventId = botPollEventId(row.trigger.id, entry.key);
        const occurredAt = itemOccurredAt(entry.item, spec.cursor, input.scheduledAt);
        const filterContext: FilterContext = {
          event: { id: eventId, kind: "poll", source, occurredAt },
          data: entry.item,
          headers: {},
        };
        if (row.trigger.filter !== null) {
          const outcome = evaluateFilter(row.trigger.filter, filterContext);
          if (!outcome.matched) {
            // Filtered poll items advance the cursor but are deliberately
            // not archived: feeds where most items filter out would bury
            // the envelope store. The per-fire count keeps it observable.
            filtered += 1;
            continue;
          }
        }
        const routed = computeRouteSession(
          row.bot.name,
          row.trigger.route,
          null,
          { eventId, data: entry.item },
          filterContext,
        );
        if (routed.error) {
        }
        const document: BotEventDocumentV1 = {
          version: 1,
          kind: "poll",
          source,
          occurredAt,
          summary: pollItemSummary(row.trigger.name, entry.item, entry.key),
          data: entry.item,
        };
        await storeBotEvent(
          { db: config.db, temporal: config.temporal, engine: client },
          {
            bot: row.bot,
            universeId: row.lightspeedUniverseId,
            eventId,
            document,
            triggerId: row.trigger.id,
            ...(routed.session === undefined ? {} : { session: routed.session }),
            ...(row.trigger.coalesce === null
              ? {}
              : {
                  coalesce: {
                    key: `${row.trigger.id}|${routed.session?.sessionId ?? "main"}`,
                    ...row.trigger.coalesce,
                  },
                }),
            ...(row.trigger.deliver === null ? {} : { whenBusy: row.trigger.deliver.whenBusy }),
          },
        );
        admitted += 1;
      }

      await config.db
        .update(schema.botTriggers)
        .set({ cursor: diff.nextState })
        .where(eq(schema.botTriggers.id, row.trigger.id));
      if (admitted > 0 || filtered > 0) {
      }
      return { polled: true, baselined: false, admitted, filtered };
    },
  };

  async function disableTrigger(
    row: { trigger: { id: string; name: string }; bot: { id: string; name: string }; lightspeedUniverseId: string },
    kind: string,
    _detail: string,
  ): Promise<void> {
    await config.db
      .update(schema.botTriggers)
      .set({
        enabled: false,
        disabledReason: kind === "breaker_tripped" ? "breaker" : "poll_failed",
        disabledAt: new Date(),
      })
      .where(eq(schema.botTriggers.id, row.trigger.id));
    await deleteBotSchedule(
      config.temporal,
      row.lightspeedUniverseId,
      row.bot.name,
      row.trigger.name,
    ).catch(() => undefined);
  }
}

export function credentialHeaderValue(token: string, scheme: string | undefined): string {
  const resolved = scheme === undefined ? "Bearer" : scheme.trim();
  return resolved ? `${resolved} ${token}` : token;
}

function itemOccurredAt(
  item: unknown,
  cursor: PollSpecRow["cursor"],
  fallback: string,
): string {
  if (cursor.kind === "watermark" && typeof item === "object" && item !== null) {
    const value = (item as Record<string, unknown>)[cursor.field];
    if (typeof value === "string") {
      const parsed = new Date(value);
      if (!Number.isNaN(parsed.getTime())) return parsed.toISOString();
    }
  }
  return fallback;
}

function digestHex(value: string): string {
  return createHash("sha256").update(value).digest("hex");
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
