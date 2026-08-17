import type { SecretGrant } from "@/api";

/// Coding-agent subscription grants (P127) and how they bind into
/// environments. Kept free of React so pages and tests can share it.

export type SubscriptionProvider = "anthropic" | "openAi";

type GrantLike = Pick<SecretGrant, "providerKind" | "metadata">;

/// A Claude Code `setup-token` is stored as an ordinary bearer grant tagged
/// `metadata.subscription = "claudeCode"`; Codex credentials have their own
/// `openAiChatGpt` kind because their injected value differs.
export function isClaudeCodeGrant(grant: GrantLike): boolean {
  return grant.providerKind === "staticBearer" && grant.metadata?.subscription === "claudeCode";
}

export function isSubscriptionGrant(grant: GrantLike): boolean {
  return subscriptionProviderOf(grant) !== null;
}

export function subscriptionProviderOf(grant: GrantLike): SubscriptionProvider | null {
  if (isClaudeCodeGrant(grant)) return "anthropic";
  if (grant.providerKind === "openAiChatGpt") return "openAi";
  return null;
}

/// True for an OpenAI grant that holds a full ChatGPT token set (pasted
/// auth.json) rather than a single Enterprise access token.
export function isCodexTokenSet(grant: Pick<SecretGrant, "providerKind" | "metadata">): boolean {
  return grant.providerKind === "openAiChatGpt" && grant.metadata?.credential === "tokenSet";
}

export interface SubscriptionBinding {
  /// Suggested environment variable name.
  envName: string;
  /// Human label for the credential source option.
  label: string;
  /// True when the injected value is Codex `auth.json` content (a token
  /// set), which the environment writes to `$CODEX_HOME/auth.json`.
  authJson: boolean;
}

/// How a subscription grant is expected to be bound into an environment.
export function subscriptionBinding(grant: GrantLike): SubscriptionBinding | null {
  if (isClaudeCodeGrant(grant)) {
    return { envName: "CLAUDE_CODE_OAUTH_TOKEN", label: "Claude Code subscription", authJson: false };
  }
  if (grant.providerKind === "openAiChatGpt") {
    return isCodexTokenSet(grant)
      ? { envName: "CODEX_AUTH_JSON", label: "Codex auth.json (ChatGPT subscription)", authJson: true }
      : { envName: "CODEX_ACCESS_TOKEN", label: "Codex access token (ChatGPT Enterprise)", authJson: false };
  }
  return null;
}

/// Shell snippet the environment runs before Codex to materialize the
/// injected auth.json content (P127 D4).
export const CODEX_AUTH_JSON_BOOTSTRAP = [
  'install -d -m 700 "${CODEX_HOME:-$HOME/.codex}" \\',
  '  && printf \'%s\' "$CODEX_AUTH_JSON" > "${CODEX_HOME:-$HOME/.codex}/auth.json" \\',
  '  && chmod 600 "${CODEX_HOME:-$HOME/.codex}/auth.json" && unset CODEX_AUTH_JSON',
].join("\n");

/// Best-effort plan/account label from grant metadata.
export function subscriptionAccountLabel(grant: SecretGrant): string {
  const parts: string[] = [];
  const email = typeof grant.metadata?.email === "string" ? grant.metadata.email : grant.subjectHint;
  if (email) parts.push(email);
  if (typeof grant.metadata?.planType === "string") parts.push(grant.metadata.planType);
  if (grant.providerKind === "openAiChatGpt" && !isCodexTokenSet(grant)) parts.push("access token");
  return parts.join(" · ");
}
