import { describe, expect, it } from "vitest";
import type { BotTrigger } from "@/api";
import { deliverySentence, deliveryShapeOf, describeCron, triggerSummary } from "./trigger-summary";

function trigger(partial: Partial<BotTrigger> & Pick<BotTrigger, "kind" | "spec">): BotTrigger {
  return {
    name: "t",
    filter: null,
    route: null,
    coalesce: null,
    deliver: null,
    sessionTtlMs: null,
    enabled: true,
    disabledReason: null,
    disabledAt: null,
    lastFilterError: null,
    lastFilterErrorAt: null,
    createdAt: "2026-08-27T00:00:00Z",
    updatedAt: "2026-08-27T00:00:00Z",
    ...partial,
  };
}

describe("describeCron", () => {
  it("speaks the builder's shapes", () => {
    expect(describeCron("0 9 * * 1-5", "Europe/Berlin")).toBe("Weekdays at 09:00 Europe/Berlin");
    expect(describeCron("30 8 * * *", "UTC")).toBe("Every day at 08:30 UTC");
    expect(describeCron("*/15 * * * *")).toBe("Every 15 minutes");
    expect(describeCron("0 10 * * 1")).toBe("Mondays at 10:00");
  });
  it("falls back to the expression it cannot express", () => {
    expect(describeCron("5 4 1,15 * *", "UTC")).toBe("5 4 1,15 * * UTC");
  });
});

describe("triggerSummary", () => {
  it("names the source and how it is verified or scoped", () => {
    expect(
      triggerSummary(
        trigger({ kind: "webhook", spec: { token: "x", verification: { scheme: "token" }, preset: "github" } }),
      ),
    ).toBe("GitHub webhook · URL token");
    expect(
      triggerSummary(
        trigger({
          kind: "poll",
          spec: {
            source: { kind: "http", url: "https://api.example.com/items" },
            intervalMs: 300_000,
            items: null,
            cursor: { kind: "idSet", id: "id" },
          },
        }),
      ),
    ).toBe("Checks api.example.com every 5 min");
    expect(triggerSummary(trigger({ kind: "bot", spec: { from: ["release-shepherd"] } }))).toBe(
      "Messages from release-shepherd",
    );
  });
});

describe("deliverySentence", () => {
  it("reads routing, batching, busy handling, and retention as one line", () => {
    const shape = deliveryShapeOf(
      trigger({
        kind: "webhook",
        spec: { token: "x", verification: { scheme: "token" } },
        route: { policy: "perKey", key: "data.pull_request.number" },
        coalesce: { debounceMs: 30_000, maxWaitMs: 120_000, maxCount: 20 },
        deliver: { whenBusy: "queue" },
        sessionTtlMs: 7 * 24 * 3_600_000,
      }),
    );
    expect(deliverySentence(shape)).toBe(
      "one thread per data.pull_request.number · batches for up to 120s · queues when busy · threads close after 168h idle",
    );
  });
  it("keeps chat vocabulary", () => {
    expect(
      deliverySentence(
        {
          routePolicy: "perKey",
          routeKey: "",
          filter: "",
          whenBusy: "steer",
          debounceSeconds: "0.4",
          maxWaitSeconds: "1.5",
          ttlMode: "forever",
          ttlHours: "",
        },
        true,
      ),
    ).toBe("one thread per conversation · batches for up to 1.5s · steers a busy run · threads kept");
  });
});
