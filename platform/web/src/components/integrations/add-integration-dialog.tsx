import { useEffect, useState } from "react";
import { ArrowLeft } from "lucide-react";
import type { SubscriptionImportResult } from "@/api";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { subscriptionBinding } from "@/lib/subscriptions";
import { INTEGRATION_CATALOG, integrationDefinition, type IntegrationKind } from "./catalog";
import { GitHubAppForm } from "./github-app";
import { ModelApiKeyForm, OpenAiCompatibleForm } from "./model-api-key";
import { SubscriptionForm } from "./subscription";
import type { ConnectedIntegration } from "./use-integrations";

/// Two-step dialog: pick an integration from the catalog, then configure it.
export function AddIntegrationDialog({
  universeId,
  open,
  connected,
  initialKind = null,
  onOpenChange,
  onAdded,
}: {
  universeId: string;
  open: boolean;
  connected: ConnectedIntegration[];
  /// Pre-select a catalog entry (deep links such as `?add=openAiApiKey`).
  initialKind?: IntegrationKind | null;
  onOpenChange: (open: boolean) => void;
  onAdded: () => void;
}) {
  const [selected, setSelected] = useState<IntegrationKind | null>(initialKind);
  useEffect(() => {
    if (open && initialKind) setSelected(initialKind);
  }, [open, initialKind]);
  const [done, setDone] = useState<SubscriptionImportResult | "github" | "modelKey" | null>(null);
  const alreadyConnected = (kind: IntegrationKind) => connected.some((c) => c.kind === kind);

  const close = () => {
    onOpenChange(false);
    setSelected(null);
    setDone(null);
  };
  const definition = selected ? integrationDefinition(selected) : null;

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close();
        else onOpenChange(true);
      }}
    >
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {definition && !done && (
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Back to catalog"
                onClick={() => setSelected(null)}
              >
                <ArrowLeft />
              </Button>
            )}
            {definition ? (
              <>
                <definition.Logo size={18} /> {definition.name}
              </>
            ) : (
              "Add integration"
            )}
          </DialogTitle>
          {!definition && (
            <DialogDescription>
              Connect a third-party service to this universe. Credentials are encrypted by
              Lightspeed and never returned.
            </DialogDescription>
          )}
        </DialogHeader>

        {!definition && (
          <div className="grid gap-2 sm:grid-cols-2">
            {INTEGRATION_CATALOG.map((entry) => {
              const existing = !entry.multiple && alreadyConnected(entry.kind);
              return (
                <button
                  key={entry.kind}
                  type="button"
                  onClick={() => setSelected(entry.kind)}
                  className="flex items-start gap-3 rounded-xl border p-3 text-left transition-colors hover:bg-muted/40 focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none"
                >
                  <span className="mt-0.5 shrink-0 text-foreground">
                    <entry.Logo size={22} />
                  </span>
                  <span className="grid gap-0.5">
                    <span className="text-sm font-medium">
                      {entry.name}
                      {existing && (
                        <span className="ml-2 rounded-full border px-1.5 py-0.5 text-[10px] font-normal text-muted-foreground">
                          connected · replace
                        </span>
                      )}
                    </span>
                    <span className="text-xs text-muted-foreground">{entry.tagline}</span>
                  </span>
                </button>
              );
            })}
          </div>
        )}

        {definition && done && (
          <div className="grid gap-3 text-sm">
            {done === "github" ? (
              <p>
                GitHub App added. Open it from the list to grant installations once the App is
                installed on your GitHub accounts.
              </p>
            ) : done === "modelKey" ? (
              <p>API key saved. Sessions using this provider pick it up on their next model call.</p>
            ) : (
              <>
                <p>
                  Connected{done.grant.displayName ? ` as ${done.grant.displayName}` : ""}.
                </p>
                <p className="text-muted-foreground">
                  Bind it to environments as{" "}
                  <span className="font-mono">{subscriptionBinding(done.grant)?.envName}</span>
                  {done.shape === "codexTokenSet"
                    ? "; the value is Codex auth.json content — the bootstrap line is shown in the integration's details."
                    : "."}
                </p>
              </>
            )}
            <DialogFooter>
              <Button onClick={close}>Done</Button>
            </DialogFooter>
          </div>
        )}

        {definition && !done && (selected === "openAiApiKey" || selected === "anthropicApiKey") && (
          <ModelApiKeyForm
            universeId={universeId}
            provider={selected === "openAiApiKey" ? "openai" : "anthropic"}
            replace={alreadyConnected(selected)}
            onSaved={() => {
              onAdded();
              setDone("modelKey");
            }}
            onCancel={() => setSelected(null)}
          />
        )}
        {definition && !done && selected === "githubApp" && (
          <GitHubAppForm
            universeId={universeId}
            onCreated={() => {
              onAdded();
              setDone("github");
            }}
            onCancel={() => setSelected(null)}
          />
        )}
        {definition && !done && selected === "openAiCompatible" && (
          <OpenAiCompatibleForm
            universeId={universeId}
            onSaved={() => {
              onAdded();
              setDone("modelKey");
            }}
            onCancel={() => setSelected(null)}
          />
        )}
        {definition && !done && (selected === "anthropicSubscription" || selected === "openAiSubscription") && (
          <SubscriptionForm
            universeId={universeId}
            provider={selected === "anthropicSubscription" ? "anthropic" : "openAi"}
            onConnected={(result) => {
              onAdded();
              setDone(result);
            }}
            onCancel={() => setSelected(null)}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}
