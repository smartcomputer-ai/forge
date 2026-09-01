import type { ChannelProvider, OperatorChannelAccountView } from "@lightspeed-ai/agent-client";
import { accountKey, type AccountSelector } from "../core/identity.js";

export interface AccountFilter {
  providers: readonly ChannelProvider[];
  /** Serve only these accounts; null serves every discovered account. */
  accounts: readonly AccountSelector[] | null;
}

/** What the host knows about one runner when a discovery pass arrives. */
export interface RunningAccount {
  key: string;
  revision: number;
  /** The runner's run loop died; the next pass restarts it. */
  failed: boolean;
}

export interface ReconciliationPlan {
  start: OperatorChannelAccountView[];
  /** Keys of runners whose account disappeared or was disabled. */
  stop: string[];
  /** Accounts whose document revision changed or whose runner failed. */
  restart: OperatorChannelAccountView[];
  unchanged: string[];
}

/** The accounts this host serves out of a discovery result. */
export function selectAccounts(
  accounts: readonly OperatorChannelAccountView[],
  filter: AccountFilter,
): OperatorChannelAccountView[] {
  const selected = new Map<string, OperatorChannelAccountView>();
  for (const account of accounts) {
    if (account.enabled === false) continue;
    if (!filter.providers.includes(account.provider)) continue;
    const key = accountKey(account.universeId, account.accountId);
    if (
      filter.accounts !== null &&
      !filter.accounts.some((selector) => accountKey(selector.universeId, selector.accountId) === key)
    ) {
      continue;
    }
    selected.set(key, account);
  }
  return [...selected.values()];
}

/**
 * Pure reconciliation: which runners to start, stop, and restart so the
 * running set matches the desired set. Order is stop, restart, start; the
 * host applies it in that order.
 */
export function planReconciliation(
  running: readonly RunningAccount[],
  desired: readonly OperatorChannelAccountView[],
): ReconciliationPlan {
  const plan: ReconciliationPlan = { start: [], stop: [], restart: [], unchanged: [] };
  const desiredByKey = new Map(
    desired.map((account) => [accountKey(account.universeId, account.accountId), account] as const),
  );
  const runningByKey = new Map(running.map((entry) => [entry.key, entry] as const));
  for (const entry of running) {
    if (!desiredByKey.has(entry.key)) plan.stop.push(entry.key);
  }
  for (const [key, account] of desiredByKey) {
    const current = runningByKey.get(key);
    if (current === undefined) {
      plan.start.push(account);
    } else if (current.failed || current.revision !== account.revision) {
      plan.restart.push(account);
    } else {
      plan.unchanged.push(key);
    }
  }
  return plan;
}
