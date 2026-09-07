import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import type { TranscriptRunSummary } from "@/lib/sessions/transcript";
import { TranscriptEntryView } from "./transcript-view";
import { RunStatsDetails } from "./run-stats";

const summary: TranscriptRunSummary = {
  kind: "run-summary", key: "run", status: "completed", contextTokens: 78512,
  usageComplete: true, durationMs: 84000, toolCalls: 24,
  usage: { inputTokens: 733000, outputTokens: 4200, modelCalls: 10, cachedInputTokens: 718340 },
};

function caption(entry = summary) {
  return renderToStaticMarkup(createElement(TranscriptEntryView, { entry }));
}
function details(entry = summary) {
  return renderToStaticMarkup(createElement(RunStatsDetails, { summary: entry }));
}
function text(html: string) { return html.replace(/<[^>]+>/g, ""); }

it("centers context, usage, and duration in a clickable separator", () => {
  const html = caption();
  expect(text(html)).toBe("Context 78.5k·Usage 737.2k·1m 24s");
  expect(html).toContain('data-variant="separator"');
  expect(html).not.toContain("98%");
  expect(html).toContain('aria-haspopup="dialog"');
  expect(html).toContain('aria-label="Run statistics"');
});

it("hides statistics while preserving failures and cancellations", () => {
  const render = (entry: TranscriptRunSummary) => text(renderToStaticMarkup(createElement(TranscriptEntryView, {
    entry, showRunStatistics: false,
  })));
  expect(render(summary)).toBe("");
  expect(render({ ...summary, status: "failed", error: "Provider unavailable" })).toBe("Run failed: Provider unavailable");
  expect(render({ ...summary, status: "cancelled" })).toBe("Run cancelled");
});

it("separates the last context measurement from cumulative usage in the breakdown", () => {
  const content = text(details());
  expect(content).toContain("Context at last model call78.5k tokens");
  expect(content).toContain("Input733k tokens");
  expect(content).toContain("Output4.2k tokens");
  expect(content).toContain("Model calls10");
  expect(content).toContain("Tool calls24");
  expect(content).toContain("Input served from cache98%");
  expect(content).toContain("Run duration1m 24s");
  expect(content).not.toContain("The next call");
});

it("shows known context but explains why a partial run has no usage total", () => {
  const partial = { ...summary, usage: undefined, usageComplete: false };
  expect(text(caption(partial))).toBe("Context 78.5k·1m 24s");
  expect(text(details(partial))).toContain("Full run usage is unavailable until earlier history is loaded.");
});

it("shows only duration when token totals are missing and preserves explicit zero cache hits", () => {
  const missing = {
    ...summary, contextTokens: undefined,
    usage: { inputTokens: 750, outputTokens: undefined, cachedInputTokens: 0, modelCalls: 1 },
  };
  expect(text(caption(missing))).toBe("1m 24s");
  const content = text(details(missing));
  expect(content).toContain("Input750 tokens");
  expect(content).toContain("OutputUnavailable");
  expect(content).toContain("Input served from cache0%");
});

it("does not invent a duration when the loaded history contains no start time", () => {
  expect(text(caption({ ...summary, durationMs: undefined }))).toBe("Context 78.5k·Usage 737.2k·Duration unavailable");
});

it.each(["failed", "cancelled"] as const)("keeps %s status and duration visible", (status) => {
  const content = text(caption({ ...summary, status, error: status === "failed" ? "Provider unavailable" : undefined }));
  expect(content).toContain(`Run ${status}`);
  if (status === "failed") expect(content).toContain("Provider unavailable");
  expect(content).toContain("1m 24s");
});
