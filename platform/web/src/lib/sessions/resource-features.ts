export type ResourceFeature = "vfs" | "environments";

export function resourceFeatureDisableReasons(
  setup: unknown,
): Partial<Record<ResourceFeature, string>> {
  const document = record(setup);
  const workspaceLinkCount = arrayLength(
    record(record(record(document.config).features).vfs).workspaceLinks,
  );
  const hasEnvironmentIntent = hasProfileEnvironment(document);
  return {
    ...(workspaceLinkCount > 0
      ? { vfs: removeFirstMessage(workspaceLinkCount, "workspace link", "VFS") }
      : {}),
    ...(hasEnvironmentIntent
      ? { environments: "Clear the profile environment before disabling the Environments feature." }
      : {}),
  };
}

/// True when the profile document names an environment intent (`existing`
/// or `provision`); absence leaves a session's selection unchanged.
export function hasProfileEnvironment(document: Record<string, unknown>): boolean {
  const environment = record(document.environment);
  return environment.type === "existing" || environment.type === "provision";
}

export function hasSessionFeature(config: unknown, name: ResourceFeature): boolean {
  return name in record(record(config).features);
}

export function setupResourceFeatureError(setup: unknown): string | null {
  const document = record(setup);
  if (hasProfileEnvironment(document) && !hasSessionFeature(document.config, "environments")) {
    return "A profile environment requires the Environments feature to be enabled.";
  }
  const environment = record(document.environment);
  if (environment.type === "existing" && !environment.environmentId) {
    return "Select an existing environment or clear the environment mode.";
  }
  if (environment.type === "provision" && (!environment.providerId || !environment.templateId)) {
    return "Provisioning needs a provider and a template.";
  }
  return null;
}

function arrayLength(value: unknown): number {
  return Array.isArray(value) ? value.length : 0;
}

function removeFirstMessage(count: number, resource: string, feature: string): string {
  const resources = count === 1 ? `the ${resource}` : `all ${count} ${resource}s`;
  return `Remove ${resources} before disabling ${feature}.`;
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}
