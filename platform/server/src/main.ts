import { serve } from "@hono/node-server";
import { createDb, migrateDb } from "@lightspeed/platform-db";
import { buildApp } from "./api.js";
import { createAuth } from "./auth.js";
import { bootstrapAdmin } from "./bootstrap.js";
import { reconcileAllBotSchedules } from "./bot-schedules.js";
import { loadEnv } from "./env.js";

const env = loadEnv();
const { db, pool } = createDb(env.databaseUrl);

console.log("[main] applying migrations…");
await migrateDb({ db, pool });
await bootstrapAdmin(db, env);

const auth = createAuth(db, env);
const app = buildApp({ db, pool, auth, env });

// Background: converge Temporal Schedules to the trigger rows; the server
// must come up even when Temporal is still starting.
void reconcileAllBotSchedules({ db }).catch((error) => {
  console.error(`[main] bot schedule reconcile failed: ${String(error)}`);
});

const server = serve({ fetch: app.fetch, port: env.port }, (info) => {
  console.log(`[main] Lightspeed platform listening on :${info.port}`);
});

for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => {
    console.log(`[main] ${signal} — shutting down`);
    server.close(() => {
      void pool.end().finally(() => process.exit(0));
    });
  });
}
