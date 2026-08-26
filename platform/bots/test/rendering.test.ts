import { describe, expect, it } from "vitest";
import {
  largestBranches,
  renderEventPrompt,
  renderValue,
  resolvePath,
} from "../src/rendering.js";

describe("renderValue", () => {
  it("drops plumbing by shape: urls, nulls, and empty containers", () => {
    const rendered = renderValue({
      title: "Fix rate limiter",
      html_url: "https://example.com/pr/877",
      _links: { self: { href: "x" } },
      node_id: "MDEx",
      labels: [],
      assignee: null,
      draft: false,
      number: 877,
    });
    expect(rendered.text).toContain("title: Fix rate limiter");
    expect(rendered.text).toContain("draft: false");
    expect(rendered.text).toContain("number: 877");
    expect(rendered.text).not.toContain("html_url");
    expect(rendered.text).not.toContain("node_id");
    expect(rendered.text).not.toContain("labels");
    expect(rendered.elided).toBe(true);
  });

  it("collapses identity objects to their name", () => {
    const rendered = renderValue({
      user: { login: "lukas", id: 7, avatar_url: "https://a", type: "User", site_admin: false },
      base: { ref: "main", sha: "abc" },
    });
    expect(rendered.text).toContain("user: lukas");
    // Objects with substantive extra fields keep their structure.
    expect(rendered.text).toContain("base:");
    expect(rendered.text).toContain("ref: main");
  });

  it("truncates long strings and caps arrays with visible marks", () => {
    const rendered = renderValue({
      body: "x".repeat(1_000),
      commits: Array.from({ length: 10 }, (_, index) => `c${index}`),
    });
    expect(rendered.text).toMatch(/x{400}… \(\+/);
    expect(rendered.text).toContain("… and 4 more");
    expect(rendered.elided).toBe(true);
  });

  it("stops at the byte budget with an explicit truncation mark", () => {
    const wide = Object.fromEntries(
      Array.from({ length: 200 }, (_, index) => [`key_${index}`, `value ${index}`]),
    );
    const rendered = renderValue(wide, { maxBytes: 300 });
    expect(rendered.text.length).toBeLessThan(400);
    expect(rendered.text).toContain("(truncated)");
    expect(rendered.elided).toBe(true);
  });
});

describe("renderEventPrompt", () => {
  it("renders a compact schedule event with a seq header and no footer", () => {
    const prompt = renderEventPrompt({
      seq: 142,
      kind: "schedule",
      source: "schedule:daily-report",
      occurredAt: "2026-08-23T09:00:00.000Z",
      summary: "Daily report",
      data: { trigger: "daily-report", cron: "0 9 * * *", timezone: "UTC" },
    });
    expect(prompt).toContain("── event #142 · schedule · schedule:daily-report · 2026-08-23 09:00Z");
    expect(prompt).toContain("Daily report");
    expect(prompt).toContain("cron: 0 9 * * *");
    expect(prompt).not.toContain("pruned");
  });

  it("points pruned events at bot_event_read by number", () => {
    const prompt = renderEventPrompt({
      seq: 9,
      kind: "pull_request.opened",
      source: "webhook:gh",
      occurredAt: "2026-08-23T09:14:00Z",
      summary: "GitHub pull_request.opened in acme/api",
      data: { pull_request: { title: "t", html_url: "https://x" } },
    });
    expect(prompt).toContain("full payload: bot_event_read #9");
  });

  it("renders without a seq for legacy events", () => {
    const prompt = renderEventPrompt({
      kind: "manual",
      source: "manual",
      occurredAt: "2026-08-23T00:00:00Z",
      summary: "hello",
    });
    expect(prompt.startsWith("── event · manual")).toBe(true);
  });
});

describe("resolvePath", () => {
  const value = { data: { commits: [{ message: "fix" }, { message: "feat" }] }, headers: { a: "1" } };

  it("walks objects and array indices", () => {
    expect(resolvePath(value, "data.commits.1.message")).toBe("feat");
    expect(resolvePath(value, "headers")).toEqual({ a: "1" });
    expect(resolvePath(value, "data.missing")).toBeUndefined();
    expect(resolvePath(value, "data.commits.x")).toBeUndefined();
  });
});

describe("largestBranches", () => {
  it("reports the biggest children with sizes", () => {
    const branches = largestBranches({
      commits: Array.from({ length: 50 }, () => ({ message: "a commit message" })),
      ref: "refs/heads/main",
    });
    expect(branches[0]).toMatchObject({ path: "commits", items: 50 });
    expect(branches[0]!.bytes).toBeGreaterThan(branches[1]!.bytes);
  });
});
