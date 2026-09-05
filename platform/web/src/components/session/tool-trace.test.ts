import { createElement } from "react";
import { renderToString } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ToolGroupTrace } from "./tool-trace";

describe("tool trace names", () => {
  it.each(["exec_command", "Bash", "run_process"])("renders the recorded name %s for the same builtin", (toolName) => {
    const html = renderToString(createElement(ToolGroupTrace, {
      group: {
        kind: "tool-group",
        key: "batch",
        status: "succeeded",
        calls: [{
          callId: "call",
          toolId: "env.run_process",
          toolName,
          status: "succeeded",
          isError: false,
        }],
      },
    }));
    expect(html).toContain(toolName);
    expect(html).not.toContain("env.run_process");
  });
});
