import { Runtime } from "@temporalio/worker";

let installed = false;

/**
 * Install the process-wide Temporal Prometheus exporter before the first
 * NativeConnection is created. One host process installs it once; every
 * per-account worker shares it.
 */
export function installTemporalMetrics(
  service: string,
  bind: { host: string; port: number },
): void {
  if (installed) return;
  Runtime.install({
    telemetryOptions: {
      metrics: {
        prometheus: {
          bindAddress: `${bind.host}:${bind.port}`,
          countersTotalSuffix: true,
          unitSuffix: true,
          useSecondsForDurations: true,
        },
        globalTags: { service },
      },
    },
  });
  installed = true;
}
