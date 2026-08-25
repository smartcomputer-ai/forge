import {
  LightspeedRpcError,
  type AuthGrantView,
  type LightspeedClient,
} from "@lightspeed/agent-client";

const STATIC_LEASE_TTL_MS = 5 * 60_000;
const EXPIRING_LEASE_MARGIN_MS = 30_000;

type RpcClient = Pick<LightspeedClient, "call">;

export class GrantReferenceError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "GrantReferenceError";
  }
}

/** Validate metadata only; trigger configuration never leases plaintext. */
export async function validateRetrievableGrant(
  client: RpcClient,
  grantId: string,
): Promise<AuthGrantView> {
  let response;
  try {
    response = await client.call("auth/grants/read", { grantId });
  } catch (error) {
    if (error instanceof LightspeedRpcError && (error.kind === "not_found" || error.kind === "rejected")) {
      throw new GrantReferenceError(`credential ${grantId} is not available in this universe`);
    }
    throw error;
  }
  const grant = response.result.grant;
  if (grant.status !== "active") {
    throw new GrantReferenceError(`credential ${grantId} is ${grant.status}, not active`);
  }
  if (grant.exposure !== "retrievable") {
    throw new GrantReferenceError(
      `credential ${grantId} is brokered; recreate it as retrievable for service use`,
    );
  }
  return grant;
}

interface CachedLease {
  token: string;
  validUntilMs: number;
}

export interface GrantLeaseRequest {
  cacheScope: string;
  grantId: string;
  audience?: string | null;
}

/**
 * Process-local credential cache. Tokens never enter workflow inputs, trigger
 * documents, or durable Platform state.
 */
export class GrantLeaseCache {
  private readonly cached = new Map<string, CachedLease>();
  private readonly inFlight = new Map<string, Promise<string>>();

  async lease(client: RpcClient, request: GrantLeaseRequest): Promise<string> {
    const key = cacheKey(request);
    const now = Date.now();
    const cached = this.cached.get(key);
    if (cached && cached.validUntilMs > now) return cached.token;
    this.cached.delete(key);

    const pending = this.inFlight.get(key);
    if (pending) return pending;

    const lease = this.fetchLease(client, request, key, now);
    this.inFlight.set(key, lease);
    try {
      return await lease;
    } finally {
      this.inFlight.delete(key);
    }
  }

  invalidate(request: GrantLeaseRequest): void {
    this.cached.delete(cacheKey(request));
  }

  private async fetchLease(
    client: RpcClient,
    request: GrantLeaseRequest,
    key: string,
    now: number,
  ): Promise<string> {
    const response = await client.call("auth/grants/lease", {
      grantId: request.grantId,
      ...(request.audience == null ? {} : { audience: request.audience }),
    });
    const { token, expiresAtMs } = response.result;
    const validUntilMs =
      expiresAtMs == null
        ? now + STATIC_LEASE_TTL_MS
        : Math.max(now, expiresAtMs - EXPIRING_LEASE_MARGIN_MS);
    if (validUntilMs > now) this.cached.set(key, { token, validUntilMs });
    return token;
  }
}

function cacheKey(request: GrantLeaseRequest): string {
  return `${request.cacheScope}\0${request.grantId}\0${request.audience ?? ""}`;
}
