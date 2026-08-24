import { resolvePath } from "./rendering.js";
import type { BotPollCursorSpecV1 } from "./contracts/bots.js";

/** Ids kept in an id-set cursor; older ids age out oldest-first. */
export const MAX_POLL_CURSOR_IDS = 500;
/** New items admitted per fire; the rest wait for the next fire. */
export const MAX_POLL_ITEMS_PER_FIRE = 100;
/** Consecutive failed fires before the trigger disables itself. */
export const MAX_POLL_CONSECUTIVE_FAILURES = 10;

export interface PollCursorState {
  ids?: string[];
  watermark?: string | number;
  consecutiveFailures: number;
  baselinedAt?: string;
  lastPolledAt?: string;
}

export interface PollDiff {
  /** First contact: the cursor was initialized and nothing is delivered. */
  baselined: boolean;
  newItems: { item: unknown; key: string }[];
  nextState: PollCursorState;
}

/**
 * The item list of one poll payload. An explicit path must resolve to an
 * array; without a path an array payload is the list and any other payload
 * is a single item.
 */
export function extractPollItems(payload: unknown, itemsPath: string | null | undefined): unknown[] {
  if (itemsPath == null || itemsPath.length === 0) {
    return Array.isArray(payload) ? payload : [payload];
  }
  const found = resolvePath(payload, itemsPath);
  if (found === undefined) {
    throw new Error(`poll items path "${itemsPath}" not found in the payload`);
  }
  if (!Array.isArray(found)) {
    throw new Error(`poll items path "${itemsPath}" is not an array`);
  }
  return found;
}

/** Stable identity for one item under the trigger's cursor discipline. */
export function pollItemKey(item: unknown, cursor: BotPollCursorSpecV1): string | null {
  if (cursor.kind === "idSet") {
    const value = resolvePath(item, cursor.id);
    if (value === undefined || value === null) return null;
    if (typeof value === "object") return null;
    return String(value);
  }
  const value = resolvePath(item, cursor.field);
  if (value === undefined || value === null) return null;
  if (typeof value !== "string" && typeof value !== "number") return null;
  return String(value);
}

function watermarkValue(item: unknown, field: string): string | number | null {
  const value = resolvePath(item, field);
  return typeof value === "string" || typeof value === "number" ? value : null;
}

/** Watermarks compare numerically when both sides are numbers, else lexically
 * (ISO-8601 timestamps compare correctly as strings). */
function watermarkAfter(value: string | number, mark: string | number): boolean {
  if (typeof value === "number" && typeof mark === "number") return value > mark;
  return String(value) > String(mark);
}

/**
 * Diff one payload against the cursor. A null state is the baseline poll:
 * the cursor initializes from the current payload and nothing delivers —
 * enabling a poll against a feed with a deep history must not flood the bot.
 */
export function diffPollItems(
  state: PollCursorState | null,
  items: unknown[],
  cursor: BotPollCursorSpecV1,
  nowIso: string,
): PollDiff {
  if (cursor.kind === "idSet") {
    const keyed = items.flatMap((item) => {
      const key = pollItemKey(item, cursor);
      return key === null ? [] : [{ item, key }];
    });
    if (state === null) {
      return {
        baselined: true,
        newItems: [],
        nextState: {
          ids: dedupeTail(keyed.map((entry) => entry.key)),
          consecutiveFailures: 0,
          baselinedAt: nowIso,
          lastPolledAt: nowIso,
        },
      };
    }
    const seen = new Set(state.ids ?? []);
    const fresh: { item: unknown; key: string }[] = [];
    const freshKeys = new Set<string>();
    for (const entry of keyed) {
      if (seen.has(entry.key) || freshKeys.has(entry.key)) continue;
      freshKeys.add(entry.key);
      fresh.push(entry);
    }
    return {
      baselined: false,
      newItems: fresh,
      nextState: {
        ...state,
        ids: dedupeTail([...(state.ids ?? []), ...fresh.map((entry) => entry.key)]),
        consecutiveFailures: 0,
        lastPolledAt: nowIso,
      },
    };
  }

  const marked = items.flatMap((item) => {
    const value = watermarkValue(item, cursor.field);
    return value === null ? [] : [{ item, value, key: String(value) }];
  });
  const highest = marked.reduce<string | number | null>(
    (max, entry) => (max === null || watermarkAfter(entry.value, max) ? entry.value : max),
    null,
  );
  if (state === null || state.watermark === undefined) {
    return {
      baselined: true,
      newItems: [],
      nextState: {
        ...(highest === null ? {} : { watermark: highest }),
        consecutiveFailures: 0,
        baselinedAt: nowIso,
        lastPolledAt: nowIso,
      },
    };
  }
  const mark = state.watermark;
  const fresh = marked.filter((entry) => watermarkAfter(entry.value, mark));
  return {
    baselined: false,
    newItems: fresh.map((entry) => ({ item: entry.item, key: entry.key })),
    nextState: {
      ...state,
      ...(highest !== null && watermarkAfter(highest, mark) ? { watermark: highest } : {}),
      consecutiveFailures: 0,
      lastPolledAt: nowIso,
    },
  };
}

/**
 * Parse a poll payload, failing with a snippet of the offending text: "not
 * JSON" debugging starts with seeing what the source actually produced (an
 * HTML error page, a stray warning line before the JSON).
 */
export function parsePollPayload(text: string, label: string): unknown {
  try {
    return JSON.parse(text) as unknown;
  } catch (error) {
    const preview = text.slice(0, 200).replace(/\s+/g, " ").trim();
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`poll ${label} is not JSON (${message}); ${label} starts: ${preview || "(empty)"}`);
  }
}

/** One line describing an item, preferring its human-ish fields. */
export function pollItemSummary(triggerName: string, item: unknown, key: string): string {
  if (typeof item === "object" && item !== null && !Array.isArray(item)) {
    const record = item as Record<string, unknown>;
    for (const field of ["summary", "title", "name", "subject", "message"]) {
      const value = record[field];
      if (typeof value === "string" && value.trim().length > 0) {
        return `${triggerName}: ${value.trim().slice(0, 300)}`;
      }
    }
  }
  return `${triggerName}: new item ${key.slice(0, 80)}`;
}

function dedupeTail(ids: string[]): string[] {
  const seen = new Set<string>();
  const unique: string[] = [];
  for (let index = ids.length - 1; index >= 0; index -= 1) {
    const id = ids[index] as string;
    if (seen.has(id)) continue;
    seen.add(id);
    unique.push(id);
  }
  unique.reverse();
  return unique.slice(-MAX_POLL_CURSOR_IDS);
}
