export interface SessionListPreferences {
  showClosed: boolean;
  showSubagents: boolean;
}

export const DEFAULT_SESSION_LIST_PREFERENCES: SessionListPreferences = {
  showClosed: true,
  showSubagents: true,
};

const STORAGE_KEY = "lightspeed:sessions:list-preferences";
const METADATA_FILTER_KEY_PREFIX = "lightspeed:sessions:metadata-filter:";

interface StorageReader {
  getItem(key: string): string | null;
}

interface StorageWriter {
  setItem(key: string, value: string): void;
}

export function readSessionListPreferences(
  storage: StorageReader | undefined = typeof window === "undefined"
    ? undefined
    : window.localStorage,
): SessionListPreferences {
  if (!storage) return DEFAULT_SESSION_LIST_PREFERENCES;
  try {
    const parsed: unknown = JSON.parse(storage.getItem(STORAGE_KEY) ?? "null");
    if (!parsed || typeof parsed !== "object") return DEFAULT_SESSION_LIST_PREFERENCES;
    const record = parsed as Record<string, unknown>;
    return {
      showClosed: typeof record.showClosed === "boolean"
        ? record.showClosed
        : DEFAULT_SESSION_LIST_PREFERENCES.showClosed,
      showSubagents: typeof record.showSubagents === "boolean"
        ? record.showSubagents
        : DEFAULT_SESSION_LIST_PREFERENCES.showSubagents,
    };
  } catch {
    return DEFAULT_SESSION_LIST_PREFERENCES;
  }
}

export function writeSessionListPreferences(
  preferences: SessionListPreferences,
  storage: StorageWriter | undefined = typeof window === "undefined"
    ? undefined
    : window.localStorage,
) {
  try {
    storage?.setItem(STORAGE_KEY, JSON.stringify(preferences));
  } catch {
    // Browsing remains usable when storage is unavailable or full.
  }
}

export function readSessionMetadataFilter(
  universeId: string,
  storage: StorageReader | undefined = typeof window === "undefined"
    ? undefined
    : window.localStorage,
): Record<string, string> {
  if (!storage) return {};
  try {
    const raw = storage.getItem(`${METADATA_FILTER_KEY_PREFIX}${universeId}`);
    const parsed: unknown = JSON.parse(raw ?? "null");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    return Object.fromEntries(
      Object.entries(parsed).filter(
        ([key, value]) => key.length > 0 && typeof value === "string" && value.length > 0,
      ),
    ) as Record<string, string>;
  } catch {
    return {};
  }
}

export function writeSessionMetadataFilter(
  universeId: string,
  filter: Record<string, string>,
  storage: StorageWriter | undefined = typeof window === "undefined"
    ? undefined
    : window.localStorage,
) {
  try {
    storage?.setItem(`${METADATA_FILTER_KEY_PREFIX}${universeId}`, JSON.stringify(filter));
  } catch {
    // URL-backed filtering still works when browser storage does not.
  }
}

export function metadataFilterFromSearchParams(params: URLSearchParams): Record<string, string> {
  const filter: Record<string, string> = {};
  for (const raw of params.getAll("metadata")) {
    const pair = parseMetadataPair(raw);
    if (pair) filter[pair.key] = pair.value;
  }
  return filter;
}

export function searchParamsWithMetadataFilter(
  current: URLSearchParams,
  filter: Record<string, string>,
): URLSearchParams {
  const next = new URLSearchParams(current);
  next.delete("metadata");
  for (const [key, value] of Object.entries(filter)) {
    next.append("metadata", `${key}=${value}`);
  }
  return next;
}

/** The value may itself contain `=`; only the first separator is structural. */
export function parseMetadataPair(raw: string): { key: string; value: string } | null {
  const at = raw.indexOf("=");
  if (at <= 0) return null;
  const key = raw.slice(0, at).trim();
  const value = raw.slice(at + 1).trim();
  return key && value ? { key, value } : null;
}
