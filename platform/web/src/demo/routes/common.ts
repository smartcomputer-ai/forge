/// Helpers shared by the demo route modules.
import type { Context } from "hono";
import type { DemoStore, UniverseState } from "../store";

/// The universe addressed by the `:id` path param (platform universe id).
export function universeFor(store: DemoStore, c: Context): UniverseState | null {
  return store.universe(c.req.param("id"));
}

export function notFound(c: Context, error = "not found") {
  return c.json({ error }, 404);
}

export function conflict(c: Context, error: string) {
  return c.json({ error }, 409);
}

export function badRequest(c: Context, error: string) {
  return c.json({ error }, 400);
}

/// Parses the JSON body, tolerating an empty one.
export async function readBody<T = Record<string, unknown>>(c: Context): Promise<T> {
  try {
    return (await c.req.json()) as T;
  } catch {
    return {} as T;
  }
}

export function nowIso(): string {
  return new Date().toISOString();
}

export function intQuery(c: Context, name: string, fallback: number): number {
  const raw = Number(c.req.query(name));
  return Number.isFinite(raw) && raw > 0 ? Math.floor(raw) : fallback;
}

export const HOUR_MS = 3_600_000;
export const DAY_MS = 86_400_000;
