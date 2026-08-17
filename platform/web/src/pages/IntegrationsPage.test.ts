import { describe, expect, it } from "vitest";
import {
  installationGrantFor,
  permissionSummary,
  validateGitHubAppForm,
} from "./IntegrationsPage";

describe("GitHub installation grants", () => {
  it("links grants to installations through metadata and prefers the live one", () => {
    const grants = [
      { status: "revoked" as const, metadata: { installation_id: 678 } },
      { status: "active" as const, metadata: { installation_id: 678 } },
      { status: "active" as const, metadata: { installation_id: 999 } },
    ];
    expect(installationGrantFor(grants, 678)?.status).toBe("active");
    expect(installationGrantFor(grants, 42)).toBeUndefined();
  });

  it("falls back to a revoked grant when nothing else exists", () => {
    const grants = [{ status: "revoked" as const, metadata: { installation_id: "678" } }];
    expect(installationGrantFor(grants, 678)?.status).toBe("revoked");
  });

  it("summarises permission maps deterministically", () => {
    expect(permissionSummary({ pull_requests: "write", contents: "read", metadata: 1 }))
      .toBe("contents: read, pull_requests: write");
    expect(permissionSummary(undefined)).toBe("—");
    expect(permissionSummary({})).toBe("—");
  });
});

describe("GitHub App form validation", () => {
  const pem = "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----\n";

  it("requires a numeric App ID", () => {
    expect(validateGitHubAppForm({ appId: "Iv1.abc", privateKey: pem })).toContain("numeric");
    expect(validateGitHubAppForm({ appId: " 123 ", privateKey: pem })).toBeNull();
  });

  it("requires a PEM private key", () => {
    expect(validateGitHubAppForm({ appId: "123", privateKey: "" })).toContain("required");
    expect(validateGitHubAppForm({ appId: "123", privateKey: "ghp_token" })).toContain("PEM");
    expect(
      validateGitHubAppForm({
        appId: "123",
        privateKey: "-----BEGIN PRIVATE KEY-----\nx\n-----END PRIVATE KEY-----",
      }),
    ).toBeNull();
  });
});

describe("subscription expiry formatting", () => {
  it("renders relative and absolute expiries", async () => {
    const { formatExpiry } = await import("./IntegrationsPage");
    const now = Date.UTC(2026, 7, 17);
    expect(formatExpiry(undefined, now)).toBe("—");
    expect(formatExpiry(now - 1, now)).toBe("expired");
    expect(formatExpiry(now + 3 * 86_400_000, now)).toBe("in 3 d");
    expect(formatExpiry(Date.UTC(2027, 7, 17), now)).toBe("2027-08-17");
  });
});
