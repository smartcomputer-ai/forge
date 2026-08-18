import type { ComponentType, SVGProps } from "react";
import { AnthropicLogo, GitHubLogo, OpenAiLogo } from "./logos";

/// Everything a universe can connect from the Integrations page. Adding an
/// integration = one entry here plus its form/details components; the page
/// itself stays generic.
export type IntegrationKind =
  | "githubApp"
  | "openAiApiKey"
  | "anthropicApiKey"
  | "anthropicSubscription"
  | "openAiSubscription";

export interface IntegrationDefinition {
  kind: IntegrationKind;
  name: string;
  /// One line for the picker card.
  tagline: string;
  Logo: ComponentType<SVGProps<SVGSVGElement> & { size?: number }>;
  /// Whether more than one instance can be connected per universe.
  multiple: boolean;
}

export const INTEGRATION_CATALOG: IntegrationDefinition[] = [
  {
    kind: "githubApp",
    name: "GitHub App",
    tagline: "Bring your own GitHub App and grant its installations to this universe.",
    Logo: GitHubLogo,
    multiple: true,
  },
  {
    kind: "openAiApiKey",
    name: "OpenAI (API key)",
    tagline: "API key that Lightspeed sessions use for OpenAI models — discovery and inference.",
    Logo: OpenAiLogo,
    multiple: false,
  },
  {
    kind: "anthropicApiKey",
    name: "Anthropic (API key)",
    tagline: "API key that Lightspeed sessions use for Anthropic models — discovery and inference.",
    Logo: AnthropicLogo,
    multiple: false,
  },
  {
    kind: "anthropicSubscription",
    name: "Claude Code (subscription)",
    tagline:
      "Lets the Claude Code agent run inside environments on your Claude Pro/Max/Team plan. Not used by Lightspeed's own sessions.",
    Logo: AnthropicLogo,
    multiple: true,
  },
  {
    kind: "openAiSubscription",
    name: "Codex (ChatGPT subscription)",
    tagline:
      "Lets the Codex agent run inside environments on your ChatGPT Plus/Pro/Team/Enterprise plan. Not used by Lightspeed's own sessions.",
    Logo: OpenAiLogo,
    multiple: true,
  },
];

export function integrationDefinition(kind: IntegrationKind): IntegrationDefinition {
  const found = INTEGRATION_CATALOG.find((entry) => entry.kind === kind);
  if (!found) throw new Error(`unknown integration kind ${kind}`);
  return found;
}
