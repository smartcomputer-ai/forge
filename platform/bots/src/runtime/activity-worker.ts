import { Client, Connection } from "@temporalio/client";
import { NativeConnection, Worker } from "@temporalio/worker";
import { createDb } from "@lightspeed/platform-db";
import {
  createBotControlPlaneActivities,
  createBotLightspeedActivities,
  createBotScheduleActivities,
} from "../activities/index.js";
import { BOTS_ACTIVITY_TASK_QUEUE } from "../contracts/bots.js";

const address = process.env.TEMPORAL_ADDRESS ?? "localhost:7233";
const namespace = process.env.TEMPORAL_NAMESPACE ?? "default";
const taskQueue = process.env.LIGHTSPEED_BOTS_ACTIVITY_TASK_QUEUE ?? BOTS_ACTIVITY_TASK_QUEUE;
const endpoint = process.env.LIGHTSPEED_ENDPOINT;
const databaseUrl = process.env.LIGHTSPEED_PLATFORM_DATABASE_URL;

if (endpoint === undefined || endpoint.length === 0) {
  throw new TypeError("LIGHTSPEED_ENDPOINT is required");
}
if (databaseUrl === undefined || databaseUrl.length === 0) {
  throw new TypeError("LIGHTSPEED_PLATFORM_DATABASE_URL is required");
}

const database = createDb(databaseUrl);
const temporal = new Client({
  connection: await Connection.connect({ address }),
  namespace,
});
const connection = await NativeConnection.connect({ address });
const worker = await Worker.create({
  connection,
  namespace,
  taskQueue,
  activities: {
    ...createBotLightspeedActivities({ endpoint }),
    ...createBotControlPlaneActivities(database.db),
    ...createBotScheduleActivities({ db: database.db, endpoint, temporal }),
  },
});

console.log(`bots: activity worker polling ${namespace}/${taskQueue} at ${address}`);
try {
  await worker.run();
} finally {
  await database.pool.end();
}
