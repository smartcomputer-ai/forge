import { describe, expect, it } from "vitest";
import { planReconciliation, selectAccounts } from "../src/host/discovery.js";
import { UNIVERSE_A, UNIVERSE_B, account } from "./fixtures.js";

describe("account discovery", () => {
  it("narrows discovered accounts to the host's providers and selectors", () => {
    const listed = [
      account({ accountId: "tg-main" }),
      account({ accountId: "wa-main", provider: "whatsapp", credentialGrantId: null }),
      account({ accountId: "tg-other", universeId: UNIVERSE_B }),
      account({ accountId: "tg-off", enabled: false }),
    ];
    expect(selectAccounts(listed, { providers: ["telegram", "whatsapp"], accounts: null }).map(key)).toEqual([
      `${UNIVERSE_A}/tg-main`,
      `${UNIVERSE_A}/wa-main`,
      `${UNIVERSE_B}/tg-other`,
    ]);
    expect(selectAccounts(listed, { providers: ["whatsapp"], accounts: null }).map(key)).toEqual([
      `${UNIVERSE_A}/wa-main`,
    ]);
    expect(
      selectAccounts(listed, {
        providers: ["telegram", "whatsapp"],
        accounts: [{ universeId: UNIVERSE_B, accountId: "tg-other" }],
      }).map(key),
    ).toEqual([`${UNIVERSE_B}/tg-other`]);
  });

  it("plans starts, stops, and restarts against the running set", () => {
    const running = [
      { key: `${UNIVERSE_A}/tg-main`, revision: 1, failed: false },
      { key: `${UNIVERSE_A}/tg-gone`, revision: 1, failed: false },
      { key: `${UNIVERSE_A}/tg-bumped`, revision: 1, failed: false },
      { key: `${UNIVERSE_A}/tg-dead`, revision: 4, failed: true },
    ];
    const desired = [
      account({ accountId: "tg-main" }),
      account({ accountId: "tg-bumped", revision: 2 }),
      account({ accountId: "tg-dead", revision: 4 }),
      account({ accountId: "tg-new" }),
    ];
    const plan = planReconciliation(running, desired);
    expect(plan.stop).toEqual([`${UNIVERSE_A}/tg-gone`]);
    expect(plan.restart.map(key)).toEqual([`${UNIVERSE_A}/tg-bumped`, `${UNIVERSE_A}/tg-dead`]);
    expect(plan.start.map(key)).toEqual([`${UNIVERSE_A}/tg-new`]);
    expect(plan.unchanged).toEqual([`${UNIVERSE_A}/tg-main`]);
  });

  it("is a no-op when nothing changed", () => {
    const desired = [account({ accountId: "tg-main", revision: 3 })];
    expect(
      planReconciliation([{ key: `${UNIVERSE_A}/tg-main`, revision: 3, failed: false }], desired),
    ).toEqual({ start: [], stop: [], restart: [], unchanged: [`${UNIVERSE_A}/tg-main`] });
  });
});

function key(entry: { universeId: string; accountId: string }): string {
  return `${entry.universeId}/${entry.accountId}`;
}
