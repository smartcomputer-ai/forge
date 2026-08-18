import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  api,
  type GitHubApp,
  type GitHubIntegration,
  type SecretGrant,
  type SecretProvider,
  type SecretsInventory,
} from "@/api";
import { subscriptionAccountLabel, subscriptionProviderOf } from "@/lib/subscriptions";
import type { IntegrationKind } from "./catalog";

/// One connected integration as the list shows it. Each variant keeps its
/// source record so the details dialog can render kind-specific content.
export type ConnectedIntegration =
  | {
      kind: "githubApp";
      id: string;
      title: string;
      subtitle: string;
      status: IntegrationStatus;
      app: GitHubApp;
      grants: SecretGrant[];
    }
  | {
      kind: "anthropicSubscription" | "openAiSubscription";
      id: string;
      title: string;
      subtitle: string;
      status: IntegrationStatus;
      grant: SecretGrant;
    }
  | {
      kind: "openAiApiKey" | "anthropicApiKey";
      id: string;
      title: string;
      subtitle: string;
      status: IntegrationStatus;
      provider: SecretProvider;
    };

export type IntegrationStatus = "active" | "attention" | "disabled";

export function useIntegrations(universeId: string) {
  const github = useQuery({
    queryKey: ["integrations", "github", universeId],
    queryFn: () =>
      api<GitHubIntegration>("GET", `/api/v1/universes/${universeId}/integrations/github`),
  });
  const subscriptions = useQuery({
    queryKey: ["integrations", "subscriptions", universeId],
    queryFn: () =>
      api<SecretGrant[]>("GET", `/api/v1/universes/${universeId}/integrations/subscriptions`),
  });
  const secrets = useQuery({
    queryKey: ["secrets", universeId],
    queryFn: () => api<SecretsInventory>("GET", `/api/v1/universes/${universeId}/secrets`),
  });

  const connected: ConnectedIntegration[] = [
    ...(secrets.data?.providers ?? [])
      .filter(
        (provider) =>
          (provider.providerId === "openai" || provider.providerId === "anthropic")
          && provider.config.type === "modelApiKey",
      )
      .map((provider): ConnectedIntegration => ({
        kind: provider.providerId === "openai" ? "openAiApiKey" : "anthropicApiKey",
        id: `model-key:${provider.credentialId}`,
        title: provider.providerId === "openai" ? "OpenAI (API key)" : "Anthropic (API key)",
        subtitle: provider.displayName ?? provider.credentialId,
        status:
          provider.status === "active" && provider.hasCredential && provider.usableForModels
            ? "active"
            : provider.status === "disabled"
              ? "disabled"
              : "attention",
        provider,
      })),
    ...(github.data?.apps ?? []).map((app): ConnectedIntegration => {
      const grants = (github.data?.grants ?? []).filter((g) => g.providerId === app.providerId);
      const active = grants.filter((g) => g.status === "active").length;
      return {
        kind: "githubApp",
        id: `github:${app.providerId}`,
        title: app.displayName ?? `GitHub App ${app.config.appId}`,
        subtitle: `App ID ${app.config.appId} · ${active} installation${active === 1 ? "" : "s"} granted`,
        status: app.status === "active" && app.hasCredential ? "active" : app.status === "disabled" ? "disabled" : "attention",
        app,
        grants,
      };
    }),
    ...(subscriptions.data ?? [])
      .filter((grant) => grant.status !== "revoked")
      .map((grant): ConnectedIntegration | null => {
        const provider = subscriptionProviderOf(grant);
        if (!provider) return null;
        const kind: IntegrationKind =
          provider === "anthropic" ? "anthropicSubscription" : "openAiSubscription";
        return {
          kind,
          id: `subscription:${grant.grantId}`,
          title:
            grant.displayName ??
            (provider === "anthropic" ? "Claude Code (subscription)" : "Codex (ChatGPT subscription)"),
          subtitle: subscriptionAccountLabel(grant) || grant.grantId,
          status: grant.status === "active" ? "active" : "attention",
          grant,
        };
      })
      .filter((entry): entry is ConnectedIntegration => entry !== null),
  ];

  return {
    connected,
    isLoading: github.isLoading || subscriptions.isLoading || secrets.isLoading,
    error: github.error ?? subscriptions.error ?? secrets.error ?? null,
  };
}

export function useInvalidateIntegrations(universeId: string) {
  const queryClient = useQueryClient();
  return () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: ["integrations"] }),
      queryClient.invalidateQueries({ queryKey: ["secrets", universeId] }),
    ]);
}
