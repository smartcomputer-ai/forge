import { describe, expect, it } from "vitest";
import { createElement } from "react";
import { renderToString } from "react-dom/server";
import {
  compareModelOptions,
  modelPickerOptions,
  normalizeSessionConfig,
  SessionConfigEditor,
  type ModelOption,
  workspaceLinksError,
  workspaceLinksFromConfig,
} from "./session-config-editor";

describe("model picker ordering", () => {
  const option = (
    model: string,
    createdAtMs?: number,
    apiKind = "openai:responses",
  ): ModelOption => ({
    providerId: "openai",
    apiKind,
    model,
    displayName: model,
    createdAtMs,
    capabilities: {},
  });

  it("puts provider-dated models newest first and unknown dates last", () => {
    expect(
      [
        option("unknown"),
        option("older", 1_700_000_000_000),
        option("newer", 1_800_000_000_000),
      ]
        .sort(compareModelOptions)
        .map((model) => model.model),
    ).toEqual(["newer", "older", "unknown"]);
  });

  it("collapses API-kind variants while preserving an existing or pinned route", () => {
    const responses = option("gpt-5.5", 1_800_000_000_000);
    const completions = option(
      "gpt-5.5",
      1_800_000_000_000,
      "openai:completions",
    );

    expect(modelPickerOptions([completions, responses])).toEqual([responses]);
    expect(modelPickerOptions([responses, completions], completions)).toEqual([
      completions,
    ]);
    expect(
      modelPickerOptions(
        [responses, completions],
        undefined,
        "openai:completions",
      ),
    ).toEqual([completions]);
  });
});

describe("OpenAI processing tier config", () => {
  it("renders the selector for an OpenAI model in the shared config editor", () => {
    const model = {
      providerId: "openai",
      apiKind: "openai:responses",
      model: "gpt-5.6-sol",
      displayName: "GPT-5.6",
      capabilities: {},
    } satisfies ModelOption;
    const html = renderToString(createElement(SessionConfigEditor, {
      value: { model },
      onChange: () => {},
      models: [model],
    }));

    expect(html).toContain("Processing tier");
    expect(html).toContain("Provider default");
  });

  it("persists the tier in generation defaults for built-in OpenAI", () => {
    expect(normalizeSessionConfig({
      model: {
        providerId: "openai",
        apiKind: "openai:responses",
        model: "gpt-5.6-sol",
      },
      generation: { processingTier: "fast" },
    })).toEqual({
      model: {
        providerId: "openai",
        apiKind: "openai:responses",
        model: "gpt-5.6-sol",
      },
      generation: { processingTier: "fast" },
    });
  });

  it("drops the OpenAI-only tier when the configured provider changes", () => {
    expect(normalizeSessionConfig({
      model: {
        providerId: "deepseek",
        apiKind: "openai:completions",
        model: "deepseek-chat",
      },
      generation: { processingTier: "fast" },
    })).toEqual({
      model: {
        providerId: "deepseek",
        apiKind: "openai:completions",
        model: "deepseek-chat",
      },
    });
  });
});

describe("workspace link config", () => {
  it("round-trips links inside the VFS feature", () => {
    const config = normalizeSessionConfig({
      features: {
        vfs: {
          tools: "edit",
          workspaceLinks: [
            {
              path: "/workspace",
              access: "readWrite",
              target: { type: "workspace", workspaceId: "primary" },
            },
            {
              path: "/skills",
              access: "readOnly",
              target: { type: "snapshot", snapshotRef: "sha256:skills" },
            },
          ],
        },
      },
    });

    expect(workspaceLinksFromConfig(config)).toEqual([
      {
        path: "/workspace",
        access: "readWrite",
        target: { type: "workspace", workspaceId: "primary" },
      },
      {
        path: "/skills",
        access: "readOnly",
        target: { type: "snapshot", snapshotRef: "sha256:skills" },
      },
    ]);
  });

  it("omits an empty workspace-link collection from the sparse config", () => {
    expect(normalizeSessionConfig({
      features: { vfs: { tools: "edit", workspaceLinks: [] } },
    })).toEqual({ features: { vfs: { tools: "edit" } } });
  });

  it("rejects overlapping paths and writable snapshots", () => {
    expect(workspaceLinksError([
      {
        path: "/workspace",
        access: "readWrite",
        target: { type: "workspace", workspaceId: "primary" },
      },
      {
        path: "/workspace/docs",
        access: "readOnly",
        target: { type: "snapshot", snapshotRef: "sha256:docs" },
      },
    ])).toContain("cannot overlap");

    expect(workspaceLinksError([{
      path: "/archive",
      access: "readWrite",
      target: { type: "snapshot", snapshotRef: "sha256:archive" },
    }])).toContain("must be read only");
  });
});

describe("environment feature config", () => {
  it("preserves independent selection and jobs grants", () => {
    expect(normalizeSessionConfig({
      features: {
        environments: {
          providers: ["sandbox-a"],
          selectionTools: true,
          jobs: true,
        },
      },
    })).toEqual({
      features: {
        environments: {
          providers: ["sandbox-a"],
          selectionTools: true,
          jobs: true,
        },
      },
    });
  });

  it("renders session environment setup inside the enabled capability panel", () => {
    const marker = "Choose the session environment here";
    const html = renderToString(createElement(SessionConfigEditor, {
      value: { features: { environments: {} } },
      onChange: () => {},
      environmentSetup: createElement("p", null, marker),
    }));

    expect(html).toContain("Environments");
    expect(html).toContain("Allowed providers");
    expect(html).toContain(marker);
    expect(html).toContain('aria-expanded="true"');
    expect(html.indexOf(marker)).toBeGreaterThan(html.indexOf("Allowed providers"));
  });

  it("can move the environment capability out of the general editor", () => {
    const html = renderToString(createElement(SessionConfigEditor, {
      value: { features: { environments: {} } },
      onChange: () => {},
      hideEnvironmentFeature: true,
    }));

    expect(html).not.toContain("Enable Environments");
    expect(html).not.toContain("Allowed providers");
  });

  it("renders features in task-oriented order and keeps Timers non-expandable", () => {
    const html = renderToString(createElement(SessionConfigEditor, {
      value: {
        features: {
          environments: {},
          mcp: { servers: [{ serverId: "demo" }] },
          subagents: {},
          vfs: {},
          web: { search: {} },
          timers: {},
        },
      },
      onChange: () => {},
      metadataSetup: createElement("p", null, "Metadata fields"),
      retentionSetup: createElement("p", null, "Retention fields"),
    }));
    const labels = [
      "Environments",
      "MCP Servers",
      "Sub-agents",
      "Virtual File System: Files, Instructions, Skills",
      "Web",
      "Timers",
      "Session metadata",
      "Automatic deletion",
    ];
    const positions = labels.map((label) => html.indexOf(label));

    expect(positions.every((position) => position >= 0)).toBe(true);
    expect(positions).toEqual([...positions].sort((left, right) => left - right));
    const timersStart = html.indexOf('aria-label="Enable Timers"');
    const timersEnd = html.indexOf("</button></div></div>", timersStart);
    const timersHtml = html.slice(timersStart, timersEnd);
    expect(timersHtml).not.toContain("aria-expanded");
    expect(html).not.toContain("Metadata fields");
    expect(html).not.toContain("Retention fields");
  });
});

describe("sub-agent feature config", () => {
  it("preserves API-shaped profile selections across editor normalization", () => {
    const config = {
      features: {
        subagents: {
          agents: [{ profileId: "primary" }],
          maxDepth: 3,
          maxDescendants: 24,
          maxConcurrent: 6,
          deadlineMs: 7_200_000,
        },
      },
    };

    expect(normalizeSessionConfig(normalizeSessionConfig(config))).toEqual(config);
  });

  it("converts profile ids selected by the editor to the API shape", () => {
    expect(normalizeSessionConfig({
      features: {
        subagents: {
          agents: ["primary"],
        },
      },
    })).toEqual({
      features: {
        subagents: {
          agents: [{ profileId: "primary" }],
        },
      },
    });
  });
});

describe("MCP feature config", () => {
  it("keeps only server selection", () => {
    expect(normalizeSessionConfig({
      features: {
        mcp: {
          servers: [{
            serverId: "github",
            allowedTools: ["search"],
            approval: "never",
            deferLoading: true,
          }],
        },
      },
    })).toEqual({
      features: {
        mcp: {
          servers: [{
            serverId: "github",
          }],
        },
      },
    });
  });
});
