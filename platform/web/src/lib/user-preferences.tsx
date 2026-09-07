import { createContext, useContext, useEffect, useState, type ReactNode } from "react";

const KEY_PREFIX = "lightspeed:user-preferences:";
const Context = createContext({
  showRunStatistics: true,
  setShowRunStatistics: (_show: boolean) => {},
});

function readShowRunStatistics(userId: string): boolean {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(`${KEY_PREFIX}${userId}`) ?? "null");
    return stored && typeof stored === "object" && "showRunStatistics" in stored && typeof stored.showRunStatistics === "boolean"
      ? stored.showRunStatistics : true;
  } catch {
    return true;
  }
}

/** Account-scoped browser preference, shared by all session and bot views. */
export function UserPreferencesProvider({ userId, children }: { userId: string; children: ReactNode }) {
  return <AccountPreferences key={userId} userId={userId}>{children}</AccountPreferences>;
}

function AccountPreferences({ userId, children }: { userId: string; children: ReactNode }) {
  const [showRunStatistics, setShow] = useState(() => readShowRunStatistics(userId));
  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === null || event.key === `${KEY_PREFIX}${userId}`) setShow(readShowRunStatistics(userId));
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, [userId]);
  const setShowRunStatistics = (show: boolean) => {
    setShow(show);
    try {
      window.localStorage.setItem(`${KEY_PREFIX}${userId}`, JSON.stringify({ showRunStatistics: show }));
    } catch {
      // Keep the current view usable when browser storage is blocked or full.
    }
  };
  return <Context.Provider value={{ showRunStatistics, setShowRunStatistics }}>{children}</Context.Provider>;
}

export function useUserPreferences() { return useContext(Context); }
