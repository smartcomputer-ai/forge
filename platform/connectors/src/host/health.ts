import { createServer, type Server } from "node:http";
import type { AccountHealth } from "./lifecycle.js";

export type ConnectorHostState = "starting" | "ready" | "degraded" | "stopping" | "stopped";

export interface ConnectorHostHealth {
  version: 1;
  state: ConnectorHostState;
  startedAtMs: number;
  discovery: {
    passes: number;
    lastSuccessAtMs?: number;
    lastError?: string;
    lastErrorAtMs?: number;
  };
  accounts: AccountHealth[];
}

export interface HostHealthInputs {
  startedAtMs: number;
  stopping: boolean;
  stopped: boolean;
  discovery: ConnectorHostHealth["discovery"];
  accounts: AccountHealth[];
}

/**
 * The host is ready once discovery succeeded and every served account is
 * ready; with accounts in any other state it is degraded (liveness stays
 * fine, readiness fails).
 */
export function hostHealth(inputs: HostHealthInputs): ConnectorHostHealth {
  const state: ConnectorHostState = inputs.stopped
    ? "stopped"
    : inputs.stopping
      ? "stopping"
      : inputs.discovery.lastSuccessAtMs === undefined
        ? "starting"
        : inputs.accounts.every((account) => account.state === "ready")
          ? "ready"
          : "degraded";
  return {
    version: 1,
    state,
    startedAtMs: inputs.startedAtMs,
    discovery: { ...inputs.discovery },
    accounts: inputs.accounts,
  };
}

export function hostHealthResponse(
  snapshot: ConnectorHostHealth,
  path: string,
): { status: number; body: ConnectorHostHealth | { error: "not found" } } {
  if (path !== "/healthz" && path !== "/readyz") {
    return { status: 404, body: { error: "not found" } };
  }
  return {
    status: path === "/readyz" && snapshot.state !== "ready" ? 503 : 200,
    body: snapshot,
  };
}

export interface HostHealthServer {
  port: number;
  close(): Promise<void>;
}

export async function startHostHealthServer(options: {
  host: string;
  port: number;
  snapshot: () => ConnectorHostHealth;
  metrics: () => string;
}): Promise<HostHealthServer> {
  const server = createServer((request, response) => {
    const path = (request.url ?? "").split("?", 1)[0] ?? "";
    if (path === "/metrics") {
      response.writeHead(200, { "content-type": "text/plain; version=0.0.4; charset=utf-8" });
      response.end(options.metrics());
      return;
    }
    const result = hostHealthResponse(options.snapshot(), path);
    response.writeHead(result.status, { "content-type": "application/json; charset=utf-8" });
    response.end(JSON.stringify(result.body));
  });
  await listen(server, options);
  const address = server.address();
  if (address === null || typeof address === "string") {
    await closeServer(server);
    throw new Error("connector health server did not bind a TCP address");
  }
  return { port: address.port, close: () => closeServer(server) };
}

function listen(server: Server, options: { host: string; port: number }): Promise<void> {
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(options.port, options.host, () => {
      server.off("error", reject);
      resolve();
    });
  });
}

function closeServer(server: Server): Promise<void> {
  return new Promise((resolve, reject) => {
    server.closeAllConnections();
    server.close((error) => (error === undefined ? resolve() : reject(error)));
  });
}
