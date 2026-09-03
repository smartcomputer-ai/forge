import { describe, expect, it } from "vitest";
import { createDemoStore } from "./fixtures";
import { createDemoRouter } from "./router";
import type {
  Universe,
  BotListItem,
  McpServer,
  McpToolDiscovery,
  SessionEventsPage,
  SessionListPage,
} from "@/api";

/// Walks the demo router the way the UI does: every read path each page
/// opens with must answer, and the live paths (a message, its tail, a bot
/// event) must move state. A stub the UI needs but the router lacks fails
/// here before it fails in a browser.
async function boot() {
  const store = createDemoStore();
  const app = createDemoRouter(store);
  const call = async (method: string, path: string, body?: unknown) => {
    const response = await app.fetch(
      new Request(`http://demo.local${path}`, {
        method,
        headers: body !== undefined ? { "content-type": "application/json" } : undefined,
        body: body !== undefined ? JSON.stringify(body) : undefined,
      }),
    );
    const text = await response.text();
    return { status: response.status, json: text ? (JSON.parse(text) as unknown) : null };
  };
  return { store, call };
}

const universeReads = [
  "sessions",
  "profiles",
  "workspaces",
  "environments",
  "environment-templates",
  "environment-provider-bindings",
  "environment-registration-keys",
  "mcp-servers",
  "secrets",
  "auth-grants",
  "models",
  "setups",
  "api-keys",
  "members",
  "bots",
  "channel-accounts",
  "channel-pairings",
  "integrations/github",
  "integrations/subscriptions",
];

describe("demo router", () => {
  it("signs the visitor in as a platform admin without a login", async () => {
    const { call } = await boot();
    const session = await call("GET", "/api/auth/get-session");
    expect(session.status).toBe(200);
    expect((session.json as { user: { role: string } }).user.role).toBe("admin");
  });

  it("answers every read the universe pages open with, for every universe", async () => {
    const { call } = await boot();
    const universes = await call("GET", "/api/v1/universes");
    expect(universes.status).toBe(200);
    const list = universes.json as Universe[];
    expect(list.length).toBeGreaterThanOrEqual(2);
    for (const universe of list) {
      for (const path of universeReads) {
        const result = await call("GET", `/api/v1/universes/${universe.id}/${path}`);
        expect(result.status, `${universe.slug}: ${path}`).toBe(200);
      }
      const bots = ((await call("GET", `/api/v1/universes/${universe.id}/bots`)).json as { bots: BotListItem[] }).bots;
      expect(bots.length, `${universe.slug}: bots`).toBeGreaterThan(0);
      for (const bot of bots) {
        for (const path of ["", "/triggers", "/events", "/state"]) {
          const result = await call("GET", `/api/v1/universes/${universe.id}/bots/${bot.botId}${path}`);
          expect(result.status, `${universe.slug}: bots/${bot.botId}${path}`).toBe(200);
        }
      }
      const sessions = (await call("GET", `/api/v1/universes/${universe.id}/sessions`)).json as SessionListPage;
      expect(sessions.sessions.length, `${universe.slug}: sessions`).toBeGreaterThan(0);
      for (const session of sessions.sessions) {
        for (const path of ["", "/events?limit=200", "/instructions"]) {
          const result = await call("GET", `/api/v1/universes/${universe.id}/sessions/${session.id}${path}`);
          expect(result.status, `${universe.slug}: sessions/${session.id}${path}`).toBe(200);
        }
      }
    }
  });

  it("answers the admin pages", async () => {
    const { call } = await boot();
    for (const path of [
      "/api/v1/users",
      "/api/v1/universes/reconcile",
      "/api/v1/status/channels",
      "/api/v1/channel-accounts",
      "/api/v1/admin/environment-providers",
      "/api/v1/admin/environment-provider-bindings",
      "/api/auth/admin/list-users",
    ]) {
      expect((await call("GET", path)).status, path).toBe(200);
    }
  });

  it("updates a user's admin-managed account fields and accepts a password reset", async () => {
    const { store, call } = await boot();
    const target = [...store.users.values()].find((user) => user.id !== store.currentUser.id)!;

    const updated = await call("POST", "/api/auth/admin/update-user", {
      userId: target.id,
      data: {
        name: "Updated User",
        email: "UPDATED@EXAMPLE.COM",
        emailVerified: true,
        role: "admin",
      },
    });
    expect(updated.status).toBe(200);
    expect(store.users.get(target.id)).toMatchObject({
      name: "Updated User",
      email: "updated@example.com",
      emailVerified: true,
      role: "admin",
    });

    expect(
      (await call("POST", "/api/auth/admin/set-user-password", {
        userId: target.id,
        newPassword: "replacement-password",
      })).status,
    ).toBe(200);
    expect(
      (await call("POST", "/api/auth/admin/revoke-user-sessions", {
        userId: target.id,
      })).status,
    ).toBe(200);
  });

  it("returns a request-local MCP tool inventory", async () => {
    const { call } = await boot();
    const [universe] = (await call("GET", "/api/v1/universes")).json as Universe[];
    const servers = (await call(
      "GET",
      `/api/v1/universes/${universe!.id}/mcp-servers`,
    )).json as McpServer[];
    const server = servers.find((candidate) => candidate.status === "active");
    expect(server).toBeDefined();
    const discovered = await call(
      "POST",
      `/api/v1/universes/${universe!.id}/mcp-servers/${server!.serverId}/tools/discover`,
    );
    expect(discovered.status).toBe(200);
    const inventory = discovered.json as McpToolDiscovery;
    expect(inventory.status).toBe("success");
    if (inventory.status === "success") {
      expect(inventory.tools[0]).toMatchObject({ name: "search" });
    }
  });

  it("runs a message through the tail and completes it", async () => {
    const { call } = await boot();
    const [universe] = (await call("GET", "/api/v1/universes")).json as Universe[];
    const created = await call("POST", `/api/v1/universes/${universe!.id}/sessions`, {
      displayName: "smoke",
      profile: { kind: "inline", profile: {} },
    });
    expect(created.status).toBe(200);
    const sessionId = (created.json as { id: string }).id;
    const accepted = await call("POST", `/api/v1/universes/${universe!.id}/sessions/${sessionId}/messages`, {
      text: "hello from the smoke test",
      submissionId: "sub-1",
    });
    expect(accepted.status).toBe(200);
    const runId = (accepted.json as { run: { id: string; status: string } }).run.id;
    let after = 0;
    let completed = false;
    let acceptedSubmission: string | null | undefined;
    let userEntrySource: unknown = null;
    for (let i = 0; i < 20 && !completed; i++) {
      const page = (
        await call("GET", `/api/v1/universes/${universe!.id}/sessions/${sessionId}/events?after=${after}&limit=100&waitMs=3000`)
      ).json as SessionEventsPage;
      for (const event of page.events ?? []) {
        after = event.cursor.seq;
        if (event.kind.type === "runAccepted" && event.kind.runId === runId) {
          acceptedSubmission = event.kind.submissionId;
        }
        if (event.kind.type === "contextEntriesApplied") {
          for (const entry of event.kind.entries) {
            if (entry.kind.type === "message" && entry.kind.role === "user") {
              userEntrySource = entry.source ?? null;
            }
          }
        }
        if (event.kind.type === "runCompleted" && event.kind.runId === runId) completed = true;
      }
    }
    expect(completed).toBe(true);
    // The page reconciles its optimistic bubble through these two joins:
    // `runAccepted.submissionId` maps the send to its run, and the user
    // entry's `source.runId` confirms the echo. Losing either shows a
    // double bubble in the demo.
    expect(acceptedSubmission).toBe("sub-1");
    expect(userEntrySource).toMatchObject({ type: "runInput", runId });
    const view = (await call("GET", `/api/v1/universes/${universe!.id}/sessions/${sessionId}`)).json as {
      status: string;
      runs: Array<{ id: string; status: string }>;
    };
    expect(view.status).toBe("idle");
    expect(view.runs.find((run) => run.id === runId)?.status).toBe("completed");
  }, 30_000);

  it("sets and clears session-tree retention through the web routes", async () => {
    const { call } = await boot();
    const [universe] = (await call("GET", "/api/v1/universes")).json as Universe[];
    const created = await call("POST", `/api/v1/universes/${universe!.id}/sessions`, {
      displayName: "retained",
      deleteAfterCloseMs: 86_400_000,
      profile: { kind: "inline", profile: {} },
    });
    expect(created.status).toBe(200);
    const session = created.json as { id: string; retention: { rootSessionId: string; deleteAfterCloseMs: number } };
    expect(session.retention).toMatchObject({
      rootSessionId: session.id,
      deleteAfterCloseMs: 86_400_000,
    });

    const cleared = await call(
      "PUT",
      `/api/v1/universes/${universe!.id}/sessions/${session.id}/retention`,
      { deleteAfterCloseMs: null },
    );
    expect(cleared.status).toBe(200);
    expect((cleared.json as { retention: { deleteAfterCloseMs: null } }).retention.deleteAfterCloseMs).toBeNull();
  });

  it("admits a manual bot event and resolves its outcome", async () => {
    const { call } = await boot();
    const [universe] = (await call("GET", "/api/v1/universes")).json as Universe[];
    const [bot] = ((await call("GET", `/api/v1/universes/${universe!.id}/bots`)).json as { bots: BotListItem[] }).bots;
    const before = (await call("GET", `/api/v1/universes/${universe!.id}/bots/${bot!.botId}/events`)).json as {
      events: Array<{ seq: number }>;
    };
    const admitted = await call("POST", `/api/v1/universes/${universe!.id}/bots/${bot!.botId}/events`, {
      event: { kind: "smoke.test", summary: "smoke test event", data: { hello: "world" } },
    });
    expect(admitted.status).toBe(202);
    expect((admitted.json as { duplicate: boolean }).duplicate).toBe(false);
    await new Promise((resolve) => setTimeout(resolve, 6_000));
    const after = (await call("GET", `/api/v1/universes/${universe!.id}/bots/${bot!.botId}/events`)).json as {
      events: Array<{ seq: number; outcome: string | null }>;
    };
    expect(after.events.length).toBe(before.events.length + 1);
    const newest = after.events.reduce((a, b) => (a.seq > b.seq ? a : b));
    expect(newest.outcome).not.toBeNull();
  }, 20_000);

  it("names the missing stub instead of hanging", async () => {
    const { call } = await boot();
    const result = await call("GET", "/api/v1/nope");
    expect(result.status).toBe(404);
    expect((result.json as { error: string }).error).toMatch(/demo: no stub/);
  });
});
