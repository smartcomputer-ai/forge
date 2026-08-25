import { describe, expect, it, vi } from "vitest";
import type { LightspeedClient } from "@lightspeed/agent-client";
import {
  GrantLeaseCache,
  GrantReferenceError,
  validateRetrievableGrant,
} from "../src/credentials.js";
import { credentialHeaderValue, fetchHttpPollPayload } from "../src/activities/poll.js";

type RpcClient = Pick<LightspeedClient, "call">;

function clientWith(call: (method: string, params: unknown) => Promise<unknown>): RpcClient {
  return { call } as unknown as RpcClient;
}

describe("grant credential leases", () => {
  it("caches static credentials in memory and invalidates explicitly", async () => {
    const call = vi.fn(async () => ({
      result: { grantId: "grant-1", providerKind: "staticBearer", token: "secret-token" },
    }));
    const client = clientWith(call);
    const cache = new GrantLeaseCache();
    const request = { cacheScope: "universe-1", grantId: "grant-1" };

    await expect(cache.lease(client, request)).resolves.toBe("secret-token");
    await expect(cache.lease(client, request)).resolves.toBe("secret-token");
    expect(call).toHaveBeenCalledTimes(1);

    cache.invalidate(request);
    await expect(cache.lease(client, request)).resolves.toBe("secret-token");
    expect(call).toHaveBeenCalledTimes(2);
  });

  it("single-flights concurrent leases", async () => {
    let release!: () => void;
    const wait = new Promise<void>((resolve) => {
      release = resolve;
    });
    const call = vi.fn(async () => {
      await wait;
      return { result: { grantId: "grant-1", providerKind: "staticBearer", token: "token" } };
    });
    const cache = new GrantLeaseCache();
    const request = { cacheScope: "universe-1", grantId: "grant-1" };
    const first = cache.lease(clientWith(call), request);
    const second = cache.lease(clientWith(call), request);
    release();
    await expect(Promise.all([first, second])).resolves.toEqual(["token", "token"]);
    expect(call).toHaveBeenCalledTimes(1);
  });

  it("validates retrievable active metadata without leasing", async () => {
    const grant = {
      grantId: "grant-1",
      providerId: "static",
      providerKind: "staticBearer",
      principal: {},
      status: "active",
      exposure: "retrievable",
      scopes: [],
      hasAccessToken: true,
      hasRefreshToken: false,
      leaseCount: 0,
      createdAtMs: 1,
      updatedAtMs: 1,
    } as const;
    const call = vi.fn(async () => ({ result: { grant } }));
    await expect(validateRetrievableGrant(clientWith(call), grant.grantId)).resolves.toEqual(grant);
    expect(call).toHaveBeenCalledWith("auth/grants/read", { grantId: "grant-1" });
  });

  it("rejects brokered metadata", async () => {
    const call = vi.fn(async () => ({
      result: {
        grant: {
          grantId: "grant-1",
          status: "active",
          exposure: "brokered",
        },
      },
    }));
    await expect(validateRetrievableGrant(clientWith(call), "grant-1")).rejects.toBeInstanceOf(
      GrantReferenceError,
    );
  });

  it("formats default, custom, and raw credential headers", () => {
    expect(credentialHeaderValue("token", undefined)).toBe("Bearer token");
    expect(credentialHeaderValue("token", "Token")).toBe("Token token");
    expect(credentialHeaderValue("token", "")).toBe("token");
  });

  it("injects a leased poll credential and refreshes it once after a 401", async () => {
    const lease = vi
      .fn()
      .mockResolvedValueOnce({
        result: { grantId: "grant-1", providerKind: "staticBearer", token: "stale" },
      })
      .mockResolvedValueOnce({
        result: { grantId: "grant-1", providerKind: "staticBearer", token: "fresh" },
      });
    const observed: string[] = [];
    const targetFetch = vi.fn(async (_url: string | URL | Request, init?: RequestInit) => {
      observed.push(new Headers(init?.headers).get("x-api-key") ?? "");
      return observed.length === 1
        ? new Response("unauthorized", { status: 401 })
        : new Response('{"items":[]}', { status: 200 });
    }) as unknown as typeof fetch;

    await expect(
      fetchHttpPollPayload({
        universeId: "universe-1",
        source: {
          kind: "http",
          url: "https://api.example.com/items",
          auth: { grantId: "grant-1", header: "x-api-key", scheme: "" },
        },
        client: clientWith(lease),
        leaseCache: new GrantLeaseCache(),
        fetch: targetFetch,
      }),
    ).resolves.toEqual({ items: [] });
    expect(observed).toEqual(["stale", "fresh"]);
    expect(lease).toHaveBeenCalledTimes(2);
  });

  it("fails closed on legacy plaintext credential headers", async () => {
    const targetFetch = vi.fn() as unknown as typeof fetch;
    await expect(
      fetchHttpPollPayload({
        universeId: "universe-1",
        source: {
          kind: "http",
          url: "https://api.example.com/items",
          headers: { Authorization: "Bearer legacy-plaintext" },
        },
        client: clientWith(vi.fn()),
        leaseCache: new GrantLeaseCache(),
        fetch: targetFetch,
      }),
    ).rejects.toThrow(/must use auth\.grantId/);
    expect(targetFetch).not.toHaveBeenCalled();
  });
});
