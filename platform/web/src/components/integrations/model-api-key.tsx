import { useState, type FormEvent } from "react";
import { useMutation } from "@tanstack/react-query";
import { api, type SecretProvider } from "@/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { DialogFooter } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { IdText } from "@/components/ui/table";
import { ConfirmDangerButton } from "./confirm-danger-button";

export type ModelKeyProvider = "openai" | "anthropic";

const COPY: Record<ModelKeyProvider, { name: string; where: string; placeholder: string }> = {
  openai: {
    name: "OpenAI",
    where: "platform.openai.com → API keys",
    placeholder: "sk-…",
  },
  anthropic: {
    name: "Anthropic",
    where: "console.anthropic.com → API keys",
    placeholder: "sk-ant-api…",
  },
};

/// Add or replace the API key Lightspeed sessions use for a model provider
/// (`model:<provider>` row). Rendered in the Add-integration dialog and in
/// the details dialog (replace mode).
export function ModelApiKeyForm({
  universeId,
  provider,
  replace,
  onSaved,
  onCancel,
  cancelLabel = "Back",
}: {
  universeId: string;
  provider: ModelKeyProvider;
  replace: boolean;
  onSaved: (provider: SecretProvider) => void;
  onCancel: () => void;
  cancelLabel?: string;
}) {
  const copy = COPY[provider];
  const [displayName, setDisplayName] = useState("");
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);

  const save = useMutation<SecretProvider, Error, void>({
    mutationFn: () =>
      api<SecretProvider>("POST", `/api/v1/universes/${universeId}/integrations/model-keys`, {
        provider,
        credential: key,
        replace,
        ...(displayName.trim() ? { displayName: displayName.trim() } : {}),
      }),
    onSuccess: (saved) => {
      setKey("");
      onSaved(saved);
    },
    onError: (reason) => setError(reason.message),
  });

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!key.trim()) {
      setError("paste the API key first");
      return;
    }
    save.mutate();
  };

  return (
    <form onSubmit={submit} className="grid gap-4">
      <p className="text-sm text-muted-foreground">
        Sessions using <span className="font-mono">model.providerId = {provider}</span> will use
        this key for model discovery and inference instead of the deployment-wide fallback key.
        {replace && " Saving replaces the current key."} The key is sent once to Lightspeed,
        encrypted, and never returned by an API.
      </p>
      <Field>
        <FieldLabel htmlFor={`model-key-name-${provider}`}>Display name</FieldLabel>
        <Input
          id={`model-key-name-${provider}`}
          value={displayName}
          onChange={(event) => setDisplayName(event.target.value)}
          placeholder={`${copy.name} production`}
        />
      </Field>
      <Field>
        <FieldLabel htmlFor={`model-key-${provider}`}>API key</FieldLabel>
        <Input
          id={`model-key-${provider}`}
          type="password"
          value={key}
          onChange={(event) => {
            setKey(event.target.value);
            setError(null);
          }}
          autoComplete="new-password"
          spellCheck={false}
          className="font-mono"
          placeholder={copy.placeholder}
          autoFocus
        />
        <FieldDescription>Create one at {copy.where}. Whitespace is preserved.</FieldDescription>
      </Field>
      {error && <p className="text-sm text-destructive">{error}</p>}
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onCancel}>
          {cancelLabel}
        </Button>
        <Button type="submit" disabled={save.isPending}>
          {save.isPending ? "Encrypting…" : replace ? "Replace key" : "Save key"}
        </Button>
      </DialogFooter>
    </form>
  );
}

/// Details for a connected model API key: status, replace, remove.
export function ModelApiKeyDetails({
  universeId,
  provider,
  onChanged,
  onRemoved,
}: {
  universeId: string;
  provider: SecretProvider;
  onChanged: () => void;
  onRemoved: () => void;
}) {
  const [replacing, setReplacing] = useState(false);
  const remove = useMutation({
    mutationFn: () =>
      api<SecretProvider>(
        "DELETE",
        `/api/v1/universes/${universeId}/secrets/providers/${encodeURIComponent(provider.credentialId)}`,
      ),
    onSuccess: onRemoved,
  });
  const providerKey: ModelKeyProvider = provider.providerId === "openai" ? "openai" : "anthropic";

  if (replacing) {
    return (
      <ModelApiKeyForm
        universeId={universeId}
        provider={providerKey}
        replace
        onSaved={() => {
          setReplacing(false);
          onChanged();
        }}
        onCancel={() => setReplacing(false)}
        cancelLabel="Cancel"
      />
    );
  }

  return (
    <div className="grid gap-4">
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-muted-foreground">Provider</dt>
        <dd className="font-mono">{provider.providerId}</dd>
        <dt className="text-muted-foreground">Credential ID</dt>
        <dd>
          <IdText>{provider.credentialId}</IdText>
        </dd>
        <dt className="text-muted-foreground">Status</dt>
        <dd>
          {provider.status === "active" && provider.hasCredential && provider.usableForModels ? (
            <Badge variant="secondary">active</Badge>
          ) : !provider.usableForModels ? (
            <Badge variant="outline" className="border-destructive/50 text-destructive">
              legacy id — replace
            </Badge>
          ) : (
            <Badge variant="outline" className="border-destructive/50 text-destructive">
              needs key
            </Badge>
          )}
        </dd>
      </dl>
      <p className="text-sm text-muted-foreground">
        Used by Lightspeed sessions with{" "}
        <span className="font-mono">model.providerId = {provider.providerId}</span>. Not injected
        into environments; coding-agent subscriptions are separate integrations.
      </p>
      {remove.error && <p className="text-sm text-destructive">{remove.error.message}</p>}
      <DialogFooter>
        <ConfirmDangerButton
          label="Remove key"
          title="Remove this API key?"
          description={
            <>
              Sessions using <span className="font-mono text-xs">{provider.providerId}</span> fall
              back to the deployment-wide key, or fail if none is configured.
            </>
          }
          pending={remove.isPending}
          onConfirm={() => remove.mutate()}
        />
        <Button onClick={() => setReplacing(true)}>Replace key</Button>
      </DialogFooter>
    </div>
  );
}
