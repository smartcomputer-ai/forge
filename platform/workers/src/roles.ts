export const PLATFORM_WORKER_LEAF_ROLES = [
  "channels-workflows",
  "channels-activities",
  "bots-workflows",
  "bots-activities",
  "telegram",
  "whatsapp",
] as const;

export const PLATFORM_WORKER_COMPOSITE_ROLES = ["channels", "bots", "all"] as const;

export type PlatformWorkerLeafRole = (typeof PLATFORM_WORKER_LEAF_ROLES)[number];
export type PlatformWorkerRole =
  | PlatformWorkerLeafRole
  | (typeof PLATFORM_WORKER_COMPOSITE_ROLES)[number];

const CHANNELS_CORE: readonly PlatformWorkerLeafRole[] = [
  "channels-workflows",
  "channels-activities",
];
const BOTS_CORE: readonly PlatformWorkerLeafRole[] = ["bots-workflows", "bots-activities"];

export function resolvePlatformWorkerRoles(
  command: string | undefined,
  configuredConnectors: string | undefined,
): PlatformWorkerLeafRole[] {
  const resolvedCommand = command ?? "all";
  if (isLeafRole(resolvedCommand)) return [resolvedCommand];

  switch (resolvedCommand) {
    case "channels":
      return [...CHANNELS_CORE];
    case "bots":
      return [...BOTS_CORE];
    case "all":
      return [...CHANNELS_CORE, ...BOTS_CORE, ...parseConnectors(configuredConnectors)];
    default:
      throw new TypeError(
        `unknown Platform workers role ${JSON.stringify(resolvedCommand)}; expected ${[
          ...PLATFORM_WORKER_COMPOSITE_ROLES,
          ...PLATFORM_WORKER_LEAF_ROLES,
        ].join(", ")}`,
      );
  }
}

function parseConnectors(value: string | undefined): PlatformWorkerLeafRole[] {
  if (value === undefined || value.trim().length === 0) return [];

  const connectors: PlatformWorkerLeafRole[] = [];
  for (const entry of value.split(",")) {
    const connector = entry.trim();
    if (connector !== "telegram" && connector !== "whatsapp") {
      throw new TypeError(
        `invalid LIGHTSPEED_CHANNELS_CONNECTORS entry ${JSON.stringify(connector)}; expected telegram or whatsapp`,
      );
    }
    if (!connectors.includes(connector)) connectors.push(connector);
  }
  return connectors;
}

function isLeafRole(value: string): value is PlatformWorkerLeafRole {
  return (PLATFORM_WORKER_LEAF_ROLES as readonly string[]).includes(value);
}
