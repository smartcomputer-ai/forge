import { createHmac } from "node:crypto";
import { describe, expect, it } from "vitest";
import type { BotWebhookTriggerSpec } from "@lightspeed/platform-db/schema";
import {
  computeRouteSession,
  evaluateFilter,
  extractWebhookEvent,
  sanitizeHeaders,
  verifyWebhook,
  type FilterContext,
} from "../src/webhooks.js";

const tokenSpec: BotWebhookTriggerSpec = { token: "tok-1", verification: { scheme: "token" } };

function hmacSpec(secret: string): BotWebhookTriggerSpec {
  return {
    token: "tok-1",
    verification: { scheme: "hmac-sha256", secret, header: "X-Hub-Signature-256", prefix: "sha256=" },
    preset: "github",
  };
}

function filterContext(overrides?: Partial<FilterContext>): FilterContext {
  return {
    event: { id: "evt", kind: "issues.opened", source: "webhook:gh", occurredAt: "2026-08-20T00:00:00Z" },
    data: { issue: { number: 7 } },
    headers: {},
    ...overrides,
  };
}

describe("webhook verification", () => {
  it("accepts the URL token and rejects mismatches without timing leaks", () => {
    const body = Buffer.from("{}");
    expect(verifyWebhook(tokenSpec, "tok-1", body, {}).ok).toBe(true);
    expect(verifyWebhook(tokenSpec, "tok-2", body, {})).toEqual({
      ok: false,
      reason: "unknown endpoint",
    });
    expect(verifyWebhook(tokenSpec, "tok-1-longer", body, {}).ok).toBe(false);
  });

  it("verifies hmac-sha256 signatures with prefix", () => {
    const body = Buffer.from(JSON.stringify({ action: "opened" }));
    const signature = `sha256=${createHmac("sha256", "s3cret-key").update(body).digest("hex")}`;
    expect(
      verifyWebhook(hmacSpec("s3cret-key"), "tok-1", body, { "x-hub-signature-256": signature }).ok,
    ).toBe(true);
    expect(
      verifyWebhook(hmacSpec("wrong-secret"), "tok-1", body, { "x-hub-signature-256": signature }),
    ).toEqual({ ok: false, reason: "signature mismatch" });
    expect(verifyWebhook(hmacSpec("s3cret-key"), "tok-1", body, {})).toEqual({
      ok: false,
      reason: "missing X-Hub-Signature-256 header",
    });
    expect(
      verifyWebhook(hmacSpec("s3cret-key"), "tok-1", body, { "x-hub-signature-256": "md5=nope" }),
    ).toEqual({ ok: false, reason: "signature prefix mismatch" });
  });
});

describe("webhook extraction", () => {
  it("uses the github preset for identity, naming, summary, and projection", () => {
    const payload = {
      action: "opened",
      repository: { full_name: "acme/widgets", html_url: "https://github.com/acme/widgets" },
      sender: { login: "lukas", avatar_url: "https://example.com/a.png", id: 7 },
      issues: { number: 5, title: "Broken build" },
    };
    const body = Buffer.from(JSON.stringify(payload));
    const extraction = extractWebhookEvent(
      { name: "gh", spec: hmacSpec("s") },
      body,
      { "X-GitHub-Event": "issues", "X-GitHub-Delivery": "d-123" },
    );
    expect(extraction.eventId).toBe("d-123");
    expect(extraction.kind).toBe("issues.opened");
    expect(extraction.summary).toBe("GitHub issues.opened in acme/widgets");
    // The stored document keeps the full body; only the prompt is projected
    // to the subject object plus envelope identity.
    expect(extraction.data).toEqual(payload);
    expect(extraction.promptData).toEqual({
      action: "opened",
      repository: "acme/widgets",
      sender: payload.sender,
      issues: { number: 5, title: "Broken build" },
    });
  });

  it("falls back to the full body when the payload has no subject object", () => {
    const payload = { ref: "refs/heads/main", commits: [{ message: "fix" }] };
    const body = Buffer.from(JSON.stringify(payload));
    const extraction = extractWebhookEvent(
      { name: "gh", spec: hmacSpec("s") },
      body,
      { "X-GitHub-Event": "push", "X-GitHub-Delivery": "d-9" },
    );
    expect(extraction.kind).toBe("push");
    expect(extraction.promptData).toEqual(payload);
  });

  it("falls back to a body digest and generic naming without a preset", () => {
    const body = Buffer.from(JSON.stringify({ kind: "deploy.finished", ok: true }));
    const extraction = extractWebhookEvent({ name: "ci", spec: tokenSpec }, body, {});
    expect(extraction.eventId).toMatch(/^whk-[0-9a-f]{64}$/);
    expect(extraction.kind).toBe("deploy.finished");
    // Identical retried payloads converge on the same dedupe identity.
    expect(extractWebhookEvent({ name: "ci", spec: tokenSpec }, body, {}).eventId).toBe(
      extraction.eventId,
    );
  });

  it("redacts credential headers and caps values", () => {
    const headers = sanitizeHeaders({
      Authorization: "Bearer secret",
      Cookie: "session=1",
      "X-Long": "a".repeat(1_000),
      "X-Ok": "fine",
    });
    expect(headers.authorization).toBeUndefined();
    expect(headers.cookie).toBeUndefined();
    expect(headers["x-long"]).toHaveLength(500);
    expect(headers["x-ok"]).toBe("fine");
  });
});

describe("filters", () => {
  it("matches, rejects, and fails closed on errors", () => {
    expect(evaluateFilter('event.kind == "issues.opened"', filterContext())).toEqual({
      matched: true,
    });
    expect(evaluateFilter('event.kind == "push"', filterContext()).matched).toBe(false);
    const errored = evaluateFilter("data.missing.deep == 1", filterContext());
    expect(errored.matched).toBe(false);
    expect(errored.error).toBeTruthy();
  });
});

describe("routing", () => {
  it("routes to the main session by default", () => {
    expect(computeRouteSession("triage", null, null, { eventId: "e" }, filterContext())).toEqual({});
    expect(
      computeRouteSession("triage", { policy: "bot" }, null, { eventId: "e" }, filterContext()),
    ).toEqual({});
  });

  it("derives per-event sessions", () => {
    const routed = computeRouteSession(
      "triage",
      { policy: "perEvent" },
      null,
      { eventId: "delivery-1" },
      filterContext(),
    );
    expect(routed.session?.sessionId).toMatch(/^bot:v1:triage:e-[0-9a-f]{12}$/);
  });

  it("derives per-key sessions from a CEL key with digest suffix", () => {
    const routed = computeRouteSession(
      "triage",
      { policy: "perKey", key: "data.issue.number" },
      null,
      { eventId: "e", data: { issue: { number: 7 } } },
      filterContext(),
    );
    expect(routed.session?.label).toBe("7");
    expect(routed.session?.sessionId).toMatch(/^bot:v1:triage:k-7-[0-9a-f]{8}$/);
    expect(routed.error).toBeUndefined();
  });

  it("uses github preset keys when no expression is set", () => {
    const pr = computeRouteSession(
      "triage",
      { policy: "perKey" },
      "github",
      { eventId: "e", data: { pull_request: { number: 12 } } },
      filterContext(),
    );
    expect(pr.session?.label).toBe("pr-12");
    const issue = computeRouteSession(
      "triage",
      { policy: "perKey" },
      "github",
      { eventId: "e", data: { issue: { number: 3 } } },
      filterContext(),
    );
    expect(issue.session?.label).toBe("issue-3");
    const repo = computeRouteSession(
      "triage",
      { policy: "perKey" },
      "github",
      { eventId: "e", data: { repository: { full_name: "acme/widgets" } } },
      filterContext(),
    );
    expect(repo.session?.label).toBe("acme/widgets");
  });

  it("falls back to the shared default key on evaluation errors", () => {
    const routed = computeRouteSession(
      "triage",
      { policy: "perKey", key: "data.missing.deep" },
      null,
      { eventId: "e", data: {} },
      filterContext({ data: {} }),
    );
    expect(routed.session?.label).toBe("default");
    expect(routed.error).toBeTruthy();
  });
});
