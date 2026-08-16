import { SetupEditorSection } from "@/components/session/setup-editor-section";
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { ProfileEnvironment } from "@/api";

export type EnvironmentOption = {
  environmentId: string;
  displayName?: string | null;
  incarnation: {
    providerTargetId?: string | null;
    templateId?: string | null;
  };
  status?: string;
};

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

type Mode = "none" | "existing" | "provision";

const NONE = "__no_profile_environment__";

/// Profile environment intent (P125): leave the session's selection alone,
/// activate an existing universe environment, or provision a fresh one for the
/// session from a provider template.
export function ProfileEnvironmentEditor({
  value,
  environments = [],
  bindings = [],
  templates = [],
  disabled = false,
  title = "Environment",
  description = "How the session obtains its active environment when this profile is applied.",
  onChange,
}: {
  value?: ProfileEnvironment | null;
  environments?: EnvironmentOption[];
  bindings?: ProviderBindingOption[];
  templates?: TemplateOption[];
  disabled?: boolean;
  title?: string;
  description?: string;
  onChange: (environment: ProfileEnvironment | undefined) => void;
}) {
  const mode: Mode = value?.type ?? "none";
  const providerIds = [...new Set([
    ...bindings.map((binding) => binding.providerId),
    ...(value?.type === "provision" ? [value.providerId] : []),
  ])];

  return (
    <SetupEditorSection title={title} description={description}>
      {disabled ? (
        <p className="rounded-lg border border-dashed p-3 text-sm text-muted-foreground">
          Enable the Environments feature in Session config to select or provision an environment.
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

          {value?.type === "provision" && (
            <ProvisionFields
              value={value}
              providerIds={providerIds}
              bindings={bindings}
              templates={templates}
              onChange={onChange}
            />
          )}
        </div>
      )}
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
      <FieldDescription className={unavailable ? "text-xs text-destructive" : "text-xs"}>
        {unavailable
          ? "This saved environment is no longer available."
          : "The profile activates this environment and never closes it."}
      </FieldDescription>
    </Field>
  );
}

function ProvisionFields({
  value,
  providerIds,
  bindings,
  templates,
  onChange,
}: {
  value: Extract<ProfileEnvironment, { type: "provision" }>;
  providerIds: string[];
  bindings: ProviderBindingOption[];
  templates: TemplateOption[];
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
  return `${environment.displayName
    ?? environment.incarnation.templateId
    ?? environment.incarnation.providerTargetId
    ?? environment.environmentId} (${environment.environmentId})`;
}
