import { Plus, Trash2 } from "lucide-react";
import { SetupEditorSection } from "@/components/session/setup-editor-section";
import { Button } from "@/components/ui/button";
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { ProfileEnvironment, ProfileEnvironmentCredential, SecretsInventory } from "@/api";
import {
  environmentCredentialOptions,
  environmentCredentialSourceFromValue,
  environmentCredentialSourceLabel,
  environmentCredentialSourceValue,
} from "@/lib/environment-credentials";
import { isTerminalEnvironmentStatus, selectableEnvironments } from "@/lib/sessions/resource-features";
import { IdlePolicyFields } from "@/components/environment/idle-policy-fields";

export type EnvironmentOption = {
  environmentId: string;
  displayName?: string | null;
  incarnation: {
    providerTargetId?: string | null;
    templateId?: string | null;
  };
  status?: string;
  /// Present on core environment views; registered environments show their
  /// identity mode so an ephemeral pick can be flagged.
  source?: { type: string; identityMode?: string };
};

function isEphemeralRegistered(environment: EnvironmentOption | undefined): boolean {
  return environment?.source?.type === "registered" && environment.source.identityMode === "ephemeral";
}

export type ProviderBindingOption = {
  bindingId: string;
  providerId: string;
  status: "enabled" | "disabled";
};

export type TemplateOption = {
  templateId: string;
  providerId: string;
  bindingId: string;
  displayName: string;
  deprecated: boolean;
};

type Mode = "none" | "existing" | "provision" | "inherit";

const NONE = "__no_profile_environment__";

/// Profile environment intent: leave the session's selection alone,
/// activate an existing universe environment, or provision a fresh one for the
/// session from a provider template.
export function ProfileEnvironmentEditor({
  value,
  environments: allEnvironments = [],
  bindings = [],
  templates = [],
  secrets,
  disabled = false,
  embedded = false,
  title = "Environment",
  description = "How the session obtains its active environment when this profile is applied.",
  onChange,
}: {
  value?: ProfileEnvironment | null;
  environments?: EnvironmentOption[];
  bindings?: ProviderBindingOption[];
  templates?: TemplateOption[];
  /// Universe secrets inventory for the provision credentials picker.
  secrets?: SecretsInventory;
  disabled?: boolean;
  /** Render inside the Environments capability panel instead of as its own section. */
  embedded?: boolean;
  title?: string;
  description?: string;
  onChange: (environment: ProfileEnvironment | undefined) => void;
}) {
  const mode: Mode = value?.type ?? "none";
  const environments = selectableEnvironments(
    allEnvironments,
    value?.type === "existing" ? value.environmentId : undefined,
  );
  const providerIds = [...new Set([
    ...bindings.map((binding) => binding.providerId),
    ...(value?.type === "provision" ? [value.providerId] : []),
  ])];

  const content = (
    <>
      {disabled ? (
        <p className="rounded-lg border border-dashed p-3 text-sm text-muted-foreground">
          Enable Environment access above to select or provision an environment.
        </p>
      ) : (
        <div className="space-y-3">
          <Field>
            <FieldLabel>Mode</FieldLabel>
            <Select
              value={mode}
              onValueChange={(next) => {
                const nextMode = next as Mode;
                if (nextMode === "none") onChange(undefined);
                else if (nextMode === "inherit") onChange({ type: "inherit" });
                else if (nextMode === "existing") {
                  onChange({ type: "existing", environmentId: environments[0]?.environmentId ?? "" });
                } else {
                  const provider = providerIds[0] ?? "";
                  const template = templates.find((candidate) => candidate.providerId === provider);
                  onChange({
                    type: "provision",
                    providerId: provider,
                    templateId: template?.templateId ?? "",
                    retention: "closeWithSession",
                  });
                }
              }}
            >
              <SelectTrigger className="w-full">
                <SelectValue>
                  {(current: string) => modeLabel(current as Mode)}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="none">{modeLabel("none")}</SelectItem>
                <SelectItem value="existing">{modeLabel("existing")}</SelectItem>
                <SelectItem value="provision">{modeLabel("provision")}</SelectItem>
                <SelectItem value="inherit">{modeLabel("inherit")}</SelectItem>
              </SelectContent>
            </Select>
          </Field>

          {value?.type === "existing" && (
            <ExistingEnvironmentField
              value={value.environmentId}
              environments={environments}
              onChange={(environmentId) =>
                onChange(environmentId ? { type: "existing", environmentId } : undefined)
              }
            />
          )}

          {value?.type === "inherit" && (
            <FieldDescription className="text-xs">
              Applied only when this profile runs as a sub-agent: the child activates the delegating
              parent's environment (shared, never copied, never closed by the child).
            </FieldDescription>
          )}

          {value?.type === "provision" && (
            <ProvisionFields
              value={value}
              providerIds={providerIds}
              bindings={bindings}
              templates={templates}
              secrets={secrets}
              onChange={onChange}
            />
          )}
        </div>
      )}
    </>
  );

  if (embedded) {
    return (
      <div className="grid min-w-0 max-w-full gap-3">
        <div className="grid min-w-0 gap-0.5">
          <p className="text-sm font-medium">Session environment</p>
          <p className="text-xs text-muted-foreground">{description}</p>
        </div>
        {content}
      </div>
    );
  }

  return (
    <SetupEditorSection title={title} description={description}>
      {content}
    </SetupEditorSection>
  );
}

function modeLabel(mode: Mode): string {
  switch (mode) {
    case "none":
      return "Do not change the active environment";
    case "existing":
      return "Activate an existing environment";
    case "provision":
      return "Provision a new environment for the session";
    case "inherit":
      return "Inherit the parent's active environment (sub-agents only)";
  }
}

function ExistingEnvironmentField({
  value,
  environments,
  onChange,
}: {
  value: string;
  environments: EnvironmentOption[];
  onChange: (environmentId: string | undefined) => void;
}) {
  const ids = [...new Set([
    ...environments.map((environment) => environment.environmentId),
    ...(value ? [value] : []),
  ])];
  const selected = value
    ? environments.find((environment) => environment.environmentId === value)
    : undefined;
  const unavailable = Boolean(value) && !selected;
  const closed = isTerminalEnvironmentStatus(selected?.status);
  return (
    <Field>
      <FieldLabel>Environment</FieldLabel>
      <Select
        value={value || NONE}
        onValueChange={(environmentId) =>
          onChange(environmentId === NONE ? undefined : environmentId as string)
        }
      >
        <SelectTrigger className="w-full">
          <SelectValue>
            {(environmentId: string) => {
              if (environmentId === NONE) return "Select an environment";
              const environment = environments.find(
                (candidate) => candidate.environmentId === environmentId,
              );
              return environment
                ? environmentLabel(environment)
                : `${environmentId} (unavailable)`;
            }}
          </SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={NONE}>Select an environment</SelectItem>
          {ids.map((environmentId) => {
            const environment = environments.find(
              (candidate) => candidate.environmentId === environmentId,
            );
            return (
              <SelectItem key={environmentId} value={environmentId}>
                {environment
                  ? environmentLabel(environment)
                  : `${environmentId} (unavailable)`}
              </SelectItem>
            );
          })}
        </SelectContent>
      </Select>
      <FieldDescription className={unavailable || closed ? "text-xs text-destructive" : "text-xs"}>
        {unavailable
          ? "This saved environment is no longer available."
          : closed
            ? "This saved environment is closed and can no longer be activated."
            : isEphemeralRegistered(selected)
              ? "This is an ephemeral registered environment: it closes on its own once its daemon has been away longer than its key's disconnect grace, and sessions that name it will then fail to start. Prefer a persistent key for anything a profile or bot points at."
              : "The profile activates this environment and never closes it; a bot's sessions share it this way. Whether it sleeps while idle is the environment's own idle policy, set on the Environments page. Closed environments are not offered."}
      </FieldDescription>
    </Field>
  );
}

function ProvisionFields({
  value,
  providerIds,
  bindings,
  templates,
  secrets,
  onChange,
}: {
  value: Extract<ProfileEnvironment, { type: "provision" }>;
  providerIds: string[];
  bindings: ProviderBindingOption[];
  templates: TemplateOption[];
  secrets?: SecretsInventory;
  onChange: (environment: ProfileEnvironment) => void;
}) {
  const binding = bindings.find((candidate) => candidate.providerId === value.providerId);
  const providerTemplates = templates.filter((template) => template.providerId === value.providerId);
  const templateIds = [...new Set([
    ...providerTemplates.map((template) => template.templateId),
    ...(value.templateId ? [value.templateId] : []),
  ])];
  const templateKnown = providerTemplates.some((template) => template.templateId === value.templateId);
  const retention = value.retention ?? "closeWithSession";
  return (
    <>
      <Field>
        <FieldLabel>Provider</FieldLabel>
        <Select
          value={value.providerId || NONE}
          onValueChange={(providerId) => {
            const next = providerId === NONE ? "" : providerId as string;
            const template = templates.find((candidate) => candidate.providerId === next);
            onChange({ ...value, providerId: next, templateId: template?.templateId ?? "" });
          }}
        >
          <SelectTrigger className="w-full">
            <SelectValue>
              {(providerId: string) => (providerId === NONE ? "Select a provider" : providerId)}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NONE}>Select a provider</SelectItem>
            {providerIds.map((providerId) => (
              <SelectItem key={providerId} value={providerId}>
                {providerId}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <FieldDescription className={binding && binding.status !== "enabled" ? "text-xs text-destructive" : "text-xs"}>
          {!value.providerId
            ? "Providers this universe is bound to."
            : !binding
              ? "This universe has no binding for the provider; provisioning will be rejected."
              : binding.status !== "enabled"
                ? `Binding ${binding.bindingId} is disabled; provisioning will be rejected.`
                : `Binding ${binding.bindingId}.`}
        </FieldDescription>
      </Field>
      <Field>
        <FieldLabel>Template</FieldLabel>
        <Select
          value={value.templateId || NONE}
          onValueChange={(templateId) =>
            onChange({ ...value, templateId: templateId === NONE ? "" : templateId as string })
          }
        >
          <SelectTrigger className="w-full">
            <SelectValue>
              {(templateId: string) => {
                if (templateId === NONE) return "Select a template";
                const template = providerTemplates.find((candidate) => candidate.templateId === templateId);
                return template ? templateLabel(template) : `${templateId} (unavailable)`;
              }}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NONE}>Select a template</SelectItem>
            {templateIds.map((templateId) => {
              const template = providerTemplates.find((candidate) => candidate.templateId === templateId);
              return (
                <SelectItem key={templateId} value={templateId}>
                  {template ? templateLabel(template) : `${templateId} (unavailable)`}
                </SelectItem>
              );
            })}
          </SelectContent>
        </Select>
        <FieldDescription className={value.templateId && !templateKnown ? "text-xs text-destructive" : "text-xs"}>
          {value.templateId && !templateKnown
            ? "This template is not offered by the selected provider."
            : "Provider-owned immutable template version."}
        </FieldDescription>
      </Field>
      <Field>
        <FieldLabel>Retention</FieldLabel>
        <Select
          value={retention}
          onValueChange={(next) =>
            onChange({ ...value, retention: next as "closeWithSession" | "retain" })
          }
        >
          <SelectTrigger className="w-full">
            <SelectValue>
              {(current: string) => retentionLabel(current)}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="closeWithSession">{retentionLabel("closeWithSession")}</SelectItem>
            <SelectItem value="retain">{retentionLabel("retain")}</SelectItem>
          </SelectContent>
        </Select>
        <FieldDescription className="text-xs">
          One environment is provisioned per session and activated while it boots; environment tools wait until it is ready.
        </FieldDescription>
      </Field>
      <Field>
        <FieldLabel>Display name (optional)</FieldLabel>
        <Input
          value={value.displayName ?? ""}
          placeholder="Defaults to the profile and session id"
          onChange={(event) => {
            const displayName = event.target.value;
            const next = { ...value };
            if (displayName) next.displayName = displayName;
            else delete next.displayName;
            onChange(next);
          }}
        />
      </Field>
      <ProvisionCredentialsField
        credentials={value.credentials ?? []}
        secrets={secrets}
        onChange={(credentials) => {
          const next = { ...value };
          if (credentials.length) next.credentials = credentials;
          else delete next.credentials;
          onChange(next);
        }}
      />
      <IdlePolicyFields
        value={value.idlePolicy ?? undefined}
        onChange={(idlePolicy) => {
          const next = { ...value };
          if (idlePolicy) next.idlePolicy = idlePolicy;
          else delete next.idlePolicy;
          onChange(next);
        }}
      />
    </>
  );
}

function retentionLabel(retention: string): string {
  return retention === "retain"
    ? "Retain after the session closes"
    : "Close with the session";
}

function templateLabel(template: TemplateOption): string {
  return `${template.displayName} (${template.templateId})${template.deprecated ? " · deprecated" : ""}`;
}

function environmentLabel(environment: EnvironmentOption): string {
  const status = environment.status && environment.status !== "ready" ? ` — ${environment.status}` : "";
  return `${environment.displayName
    ?? environment.incarnation.templateId
    ?? environment.incarnation.providerTargetId
    ?? environment.environmentId} (${environment.environmentId})${status}`;
}

const NO_SOURCE = "__no_credential_source__";

/// Credentials bound to the provisioned environment right after creation:
/// references to universe secrets, never values. Suggested env
/// names come from the credential (e.g. CLAUDE_CODE_OAUTH_TOKEN).
function ProvisionCredentialsField({
  credentials,
  secrets,
  onChange,
}: {
  credentials: ProfileEnvironmentCredential[];
  secrets?: SecretsInventory;
  onChange: (credentials: ProfileEnvironmentCredential[]) => void;
}) {
  const options = environmentCredentialOptions(secrets);
  const update = (index: number, patch: Partial<ProfileEnvironmentCredential>) =>
    onChange(credentials.map((c, i) => (i === index ? { ...c, ...patch } : c)));
  const remove = (index: number) => onChange(credentials.filter((_, i) => i !== index));
  const duplicates = new Set(
    credentials
      .map((c) => c.envName)
      .filter((name, i, all) => name && all.indexOf(name) !== i),
  );
  return (
    <Field>
      <FieldLabel>Environment credentials</FieldLabel>
      <div className="grid gap-2">
        {credentials.map((credential, index) => {
          const currentValue = environmentCredentialSourceValue(credential.source);
          const known = options.some((option) => option.value === currentValue);
          const invalidName =
            credential.envName !== "" && !/^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(credential.envName);
          return (
            <div key={index} className="grid min-w-0 grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] items-start gap-2">
              <div className="grid min-w-0 gap-1">
                <Input
                  value={credential.envName}
                  onChange={(event) => update(index, { envName: event.target.value })}
                  placeholder="ENV_VAR_NAME"
                  spellCheck={false}
                  className="font-mono text-xs"
                  aria-label="Environment variable name"
                />
                {(invalidName || duplicates.has(credential.envName)) && (
                  <span className="text-xs text-destructive">
                    {invalidName ? "Invalid variable name." : "Bound more than once."}
                  </span>
                )}
              </div>
              <Select
                value={known ? currentValue : NO_SOURCE}
                onValueChange={(next) => {
                  if (next === NO_SOURCE) return;
                  const option = options.find((candidate) => candidate.value === next);
                  update(index, {
                    source: environmentCredentialSourceFromValue(next as string),
                    ...(option?.suggestedEnvName && !credential.envName
                      ? { envName: option.suggestedEnvName }
                      : {}),
                  });
                }}
              >
                <SelectTrigger className="w-full" aria-label="Credential source">
                  <SelectValue>
                    {(current: string) =>
                      current === NO_SOURCE
                        ? `${environmentCredentialSourceLabel(credential.source, secrets)} (unavailable)`
                        : options.find((option) => option.value === current)?.label ?? current}
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  {!known && (
                    <SelectItem value={NO_SOURCE}>
                      {environmentCredentialSourceLabel(credential.source, secrets)} (unavailable)
                    </SelectItem>
                  )}
                  {options.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label="Remove credential"
                onClick={() => remove(index)}
              >
                <Trash2 />
              </Button>
            </div>
          );
        })}
        <div>
          <Button
            type="button"
            variant="outline"
            size="xs"
            disabled={options.length === 0}
            onClick={() => {
              const first = options[0];
              if (!first) return;
              onChange([
                ...credentials,
                {
                  envName: first.suggestedEnvName ?? "",
                  source: environmentCredentialSourceFromValue(first.value),
                },
              ]);
            }}
          >
            <Plus data-icon="inline-start" />
            Add credential
          </Button>
        </div>
      </div>
      <FieldDescription className="text-xs">
        Bound to the environment right after it is provisioned, before activation. References
        universe secrets and integrations (never values); they become ordinary environment
        credential bindings you can change later under Environments.
        {options.length === 0 && " No secrets or integrations are available in this universe yet."}
      </FieldDescription>
    </Field>
  );
}
