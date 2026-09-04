import { normalizeSessionConfig } from "@/components/session/session-config-editor";

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

/** Copy only the environment capability from source onto base. */
export function withEnvironmentFeature(
  base: Record<string, unknown> | undefined,
  source: Record<string, unknown> | undefined,
): Record<string, unknown> | undefined {
  const next = structuredClone(base ?? {});
  const features = record(next.features);
  const sourceFeatures = record(source?.features);
  if (Object.hasOwn(sourceFeatures, "environments")) {
    features.environments = structuredClone(sourceFeatures.environments);
  } else {
    delete features.environments;
  }
  if (Object.keys(features).length) next.features = features;
  else delete next.features;
  return normalizeSessionConfig(next);
}

export function environmentFeatureSnapshot(
  config: Record<string, unknown> | undefined,
): unknown {
  const features = record(config?.features);
  return Object.hasOwn(features, "environments") ? features.environments : undefined;
}
