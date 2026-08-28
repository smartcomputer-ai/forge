/// better-auth endpoints the web client calls. The demo has one signed-in
/// platform admin and never asks for credentials.
import { Hono } from "hono";
import type { DemoStore, DemoUser } from "../store";
import { nowIso, readBody } from "./common";

export function authRoutes(store: DemoStore): Hono {
  const app = new Hono();

  const sessionOf = (user: DemoUser) => ({
    session: {
      id: "demo-session",
      userId: user.id,
      token: "demo-token",
      expiresAt: new Date(Date.now() + 30 * 86_400_000).toISOString(),
      createdAt: user.createdAt,
      updatedAt: nowIso(),
      ipAddress: "",
      userAgent: "",
      activeOrganizationId: null,
    },
    user,
  });

  app.get("/get-session", (c) => c.json(sessionOf(store.currentUser)));
  app.post("/sign-in/email", (c) =>
    c.json({ redirect: false, token: "demo-token", user: store.currentUser }),
  );
  app.post("/sign-out", (c) => c.json({ success: true }));
  app.post("/update-user", async (c) => {
    const body = await readBody<{ name?: string; image?: string | null }>(c);
    if (typeof body.name === "string" && body.name.trim()) store.currentUser.name = body.name.trim();
    if (body.image !== undefined) store.currentUser.image = body.image;
    store.currentUser.updatedAt = nowIso();
    return c.json({ status: true });
  });
  app.post("/change-password", (c) => c.json({ token: "demo-token", user: store.currentUser }));

  app.get("/admin/list-users", (c) => {
    const users = [...store.users.values()];
    return c.json({ users, total: users.length, limit: users.length, offset: 0 });
  });
  app.post("/admin/create-user", async (c) => {
    const body = await readBody<{ email?: string; name?: string; role?: string }>(c);
    if (!body.email?.trim()) return c.json({ message: "email is required" }, 400);
    const at = nowIso();
    const user: DemoUser = {
      id: store.nextId("user"),
      name: body.name?.trim() || body.email.split("@")[0] || "New user",
      email: body.email.trim(),
      role: body.role ?? "user",
      emailVerified: true,
      image: null,
      banned: false,
      createdAt: at,
      updatedAt: at,
    };
    store.users.set(user.id, user);
    return c.json({ user });
  });

  return app;
}
