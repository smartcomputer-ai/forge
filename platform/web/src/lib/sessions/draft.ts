/** Shared by normal sessions and every bot conversation showing that session. */
export function sessionDraftKey(universeId: string, sessionId: string): string {
  return `lightspeed:sessions:draft:${JSON.stringify([universeId, sessionId])}`;
}

export function readSessionDraft(key: string): string {
  try {
    return typeof window === "undefined" ? "" : window.localStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}

export function writeSessionDraft(key: string, text: string): void {
  try {
    if (typeof window === "undefined") return;
    if (text === "") {
      window.localStorage.removeItem(key);
    } else {
      window.localStorage.setItem(key, text);
    }
  } catch {
    // The composer remains usable when browser storage is blocked or full.
  }
}
