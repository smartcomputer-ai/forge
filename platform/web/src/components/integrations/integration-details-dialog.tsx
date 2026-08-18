import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { integrationDefinition } from "./catalog";
import { GitHubAppDetails } from "./github-app";
import { ModelApiKeyDetails } from "./model-api-key";
import { SubscriptionDetails } from "./subscription";
import type { ConnectedIntegration } from "./use-integrations";

/// Details/configuration for one connected integration; content is
/// dispatched on the integration kind.
export function IntegrationDetailsDialog({
  universeId,
  integration,
  onOpenChange,
  onChanged,
}: {
  universeId: string;
  integration: ConnectedIntegration | null;
  onOpenChange: (open: boolean) => void;
  onChanged: () => void;
}) {
  const definition = integration ? integrationDefinition(integration.kind) : null;
  return (
    <Dialog open={integration !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        {integration && definition && (
          <>
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2">
                <definition.Logo size={18} />
                {integration.title}
              </DialogTitle>
            </DialogHeader>
            <IntegrationDetails
              universeId={universeId}
              integration={integration}
              onChanged={onChanged}
              onClose={() => onOpenChange(false)}
            />
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

function IntegrationDetails({
  universeId,
  integration,
  onChanged,
  onClose,
}: {
  universeId: string;
  integration: ConnectedIntegration;
  onChanged: () => void;
  onClose: () => void;
}) {
  const removed = () => {
    onChanged();
    onClose();
  };
  switch (integration.kind) {
    case "openAiApiKey":
    case "anthropicApiKey":
      return (
        <ModelApiKeyDetails
          universeId={universeId}
          provider={integration.provider}
          onChanged={onChanged}
          onRemoved={removed}
        />
      );
    case "githubApp":
      return (
        <GitHubAppDetails
          universeId={universeId}
          app={integration.app}
          grants={integration.grants}
          onChanged={onChanged}
          onRemoved={removed}
        />
      );
    case "anthropicSubscription":
    case "openAiSubscription":
      return (
        <SubscriptionDetails
          universeId={universeId}
          grant={integration.grant}
          onDisconnected={removed}
        />
      );
  }
}
