/**
 * Model-facing event rendering. The stored `BotEventDocumentV1` stays the
 * complete machine envelope (filters, UI, replay, bot_event_read); what a
 * session reads is a compact text produced here. Pruning is by shape, never
 * by service knowledge, and every cut is marked so the model knows to pull
 * the full payload with bot_event_read.
 */

/** Keys whose values are API plumbing, dropped at any depth. */
const DROP_KEY = /(^|_)(url|urls|href|link|links)$|^(node_id|gravatar_id|etag|_links)$/;
/** Keys that carry an object's human identity, used to collapse it. */
const NAME_KEYS = ["login", "full_name", "name", "slug", "username", "email"] as const;
/** Keys tolerated (and hidden) when collapsing an identity object. */
const IDENTITY_NOISE = new Set(["id", "type", "site_admin", "user_view_type"]);

const MAX_STRING = 400;
const MAX_ARRAY_ITEMS = 6;
const MAX_DEPTH = 6;
export const DEFAULT_PROMPT_BUDGET = 2_048;
export const DEFAULT_READ_BUDGET = 8_192;

export interface RenderedValue {
  text: string;
  /** True when anything was dropped, truncated, or capped. */
  elided: boolean;
}

interface RenderState {
  lines: string[];
  bytes: number;
  budget: number;
  elided: boolean;
  overflowed: boolean;
}

/** Metadata half of an event document; `data` is rendered separately. */
export interface EventPromptInput {
  seq?: number | null;
  kind: string;
  source: string;
  occurredAt: string;
  summary: string;
  /** Salient payload for the prompt; presets may project the raw body. */
  data?: unknown;
  correlationId?: string | null;
  links?: string[];
}

/**
 * Render the delivered representation of one event. Header, summary, pruned
 * payload, and an honest footer when anything was cut.
 */
export function renderEventPrompt(
  event: EventPromptInput,
  options?: { maxBytes?: number },
): string {
  const handle = event.seq == null ? "event" : `event #${event.seq}`;
  const header = `── ${handle} · ${event.kind} · ${event.source} · ${compactTime(event.occurredAt)}`;
  const parts: string[] = [header, event.summary];
  let elided = false;
  if (event.data !== undefined && event.data !== null) {
    const rendered = renderValue(event.data, {
      maxBytes: options?.maxBytes ?? DEFAULT_PROMPT_BUDGET,
    });
    if (rendered.text.length > 0) parts.push(rendered.text);
    elided = rendered.elided;
  }
  if (event.correlationId != null) parts.push(`correlation: ${event.correlationId}`);
  if (event.links !== undefined && event.links.length > 0) {
    parts.push(`links: ${event.links.slice(0, 5).join(" ")}`);
  }
  if (elided) {
    parts.push(
      event.seq == null
        ? "(… pruned — the full stored payload is available via bot_event_read)"
        : `(… pruned — full payload: bot_event_read #${event.seq})`,
    );
  }
  return parts.join("\n");
}

/** Render arbitrary JSON as compact indented text with shape-based pruning. */
export function renderValue(value: unknown, options?: { maxBytes?: number }): RenderedValue {
  const state: RenderState = {
    lines: [],
    bytes: 0,
    budget: options?.maxBytes ?? DEFAULT_PROMPT_BUDGET,
    elided: false,
    overflowed: false,
  };
  renderNode(value, "", 0, state);
  if (state.overflowed) {
    state.lines.push("(truncated)");
    state.elided = true;
  }
  return { text: state.lines.join("\n"), elided: state.elided };
}

function emit(state: RenderState, line: string): boolean {
  if (state.overflowed) return false;
  if (state.bytes + line.length + 1 > state.budget) {
    state.overflowed = true;
    return false;
  }
  state.lines.push(line);
  state.bytes += line.length + 1;
  return true;
}

function renderNode(value: unknown, indent: string, depth: number, state: RenderState): void {
  const scalar = renderScalar(value, state);
  if (scalar !== undefined) {
    emit(state, `${indent}${scalar}`);
    return;
  }
  if (depth >= MAX_DEPTH) {
    state.elided = true;
    emit(state, `${indent}…`);
    return;
  }
  if (Array.isArray(value)) {
    const shown = value.slice(0, MAX_ARRAY_ITEMS);
    for (const item of shown) {
      const inline = renderScalar(item, state);
      if (inline !== undefined) emit(state, `${indent}- ${inline}`);
      else {
        emit(state, `${indent}-`);
        renderNode(item, `${indent}  `, depth + 1, state);
      }
    }
    if (value.length > MAX_ARRAY_ITEMS) {
      state.elided = true;
      emit(state, `${indent}… and ${value.length - MAX_ARRAY_ITEMS} more`);
    }
    return;
  }
  const record = value as Record<string, unknown>;
  for (const [key, entry] of Object.entries(record)) {
    if (state.overflowed) return;
    if (dropEntry(key, entry)) {
      if (entry !== null && entry !== undefined && !isEmptyContainer(entry)) state.elided = true;
      continue;
    }
    const inline = renderScalar(entry, state);
    if (inline !== undefined) {
      emit(state, `${indent}${key}: ${inline}`);
      continue;
    }
    const identity = collapseIdentity(entry);
    if (identity !== undefined) {
      state.elided = true;
      emit(state, `${indent}${key}: ${identity}`);
      continue;
    }
    emit(state, `${indent}${key}:`);
    renderNode(entry, `${indent}  `, depth + 1, state);
  }
}

/** Scalar rendering, or undefined when the value needs structural layout. */
function renderScalar(value: unknown, state: RenderState): string | undefined {
  if (value === null) return "null";
  if (typeof value === "string") {
    const flattened = value.replace(/\s*\n\s*/g, " ⏎ ");
    if (flattened.length > MAX_STRING) {
      state.elided = true;
      return `${flattened.slice(0, MAX_STRING)}… (+${formatBytes(flattened.length - MAX_STRING)})`;
    }
    return flattened;
  }
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return undefined;
}

function dropEntry(key: string, value: unknown): boolean {
  if (value === null || value === undefined) return true;
  if (DROP_KEY.test(key)) return true;
  if (isEmptyContainer(value)) return true;
  return false;
}

function isEmptyContainer(value: unknown): boolean {
  if (Array.isArray(value)) return value.length === 0;
  if (typeof value === "object" && value !== null) return Object.keys(value).length === 0;
  return false;
}

/**
 * An object that is just an identity (a name key plus ids and urls) renders
 * as its name: `user: lukas` instead of eight lines of avatar plumbing.
 */
function collapseIdentity(value: unknown): string | undefined {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return undefined;
  const record = value as Record<string, unknown>;
  let name: string | undefined;
  for (const key of NAME_KEYS) {
    const candidate = record[key];
    if (typeof candidate === "string" && candidate.length > 0) {
      name = candidate;
      break;
    }
  }
  if (name === undefined) return undefined;
  for (const [key, entry] of Object.entries(record)) {
    if ((NAME_KEYS as readonly string[]).includes(key)) continue;
    if (IDENTITY_NOISE.has(key) || DROP_KEY.test(key)) continue;
    if (entry === null || isEmptyContainer(entry)) continue;
    return undefined;
  }
  return name;
}

/** Walk a dot path (array indices as numbers: `commits.0.message`). */
export function resolvePath(value: unknown, path: string): unknown {
  let current: unknown = value;
  for (const segment of path.split(".")) {
    if (segment.length === 0) return undefined;
    if (Array.isArray(current)) {
      const index = Number(segment);
      if (!Number.isSafeInteger(index)) return undefined;
      current = current[index];
    } else if (typeof current === "object" && current !== null) {
      current = (current as Record<string, unknown>)[segment];
    } else {
      return undefined;
    }
    if (current === undefined) return undefined;
  }
  return current;
}

/** Largest child branches of a value, for honest over-budget reporting. */
export function largestBranches(
  value: unknown,
  limit = 5,
): { path: string; bytes: number; items?: number }[] {
  if (typeof value !== "object" || value === null) return [];
  const entries = Array.isArray(value)
    ? value.map((item, index) => [String(index), item] as const)
    : Object.entries(value as Record<string, unknown>);
  return entries
    .map(([key, entry]) => ({
      path: key,
      bytes: JSON.stringify(entry)?.length ?? 0,
      ...(Array.isArray(entry) ? { items: entry.length } : {}),
    }))
    .sort((a, b) => b.bytes - a.bytes)
    .slice(0, limit);
}

export function formatBytes(bytes: number): string {
  if (bytes < 1_024) return `${bytes} B`;
  return `${(bytes / 1_024).toFixed(1)} KB`;
}

function compactTime(iso: string): string {
  const match = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})/.exec(iso);
  return match ? `${match[1]} ${match[2]}Z` : iso;
}
