import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { TranscriptEntryView } from "./transcript-view";

describe("TranscriptEntryView", () => {
  it.each(["user", "assistant"] as const)("renders a full %s message without expansion", (role) => {
    const text = "Complete message 🦀. ".repeat(700) + "The final sentence.";
    const loadFullText = vi.fn();
    const html = renderToString(createElement(TranscriptEntryView, {
      entry: { kind: "message", key: "message", role, text },
      loadFullText,
    }));
    expect(html).toContain(text);
    expect(html).not.toContain("Expand full entry");
    expect(loadFullText).not.toHaveBeenCalled();
  });
});
