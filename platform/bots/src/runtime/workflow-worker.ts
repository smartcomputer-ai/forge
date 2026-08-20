import { fileURLToPath } from "node:url";
import { NativeConnection, Worker } from "@temporalio/worker";
import { BOTS_WORKFLOW_TASK_QUEUE } from "../contracts/bots.js";

const address = process.env.TEMPORAL_ADDRESS ?? "localhost:7233";
const namespace = process.env.TEMPORAL_NAMESPACE ?? "default";
const taskQueue = process.env.LIGHTSPEED_BOTS_WORKFLOW_TASK_QUEUE ?? BOTS_WORKFLOW_TASK_QUEUE;

const connection = await NativeConnection.connect({ address });
const worker = await Worker.create({
  connection,
  namespace,
  taskQueue,
  workflowsPath: fileURLToPath(new URL("../workflows/index.ts", import.meta.url)),
});

console.log(`bots: workflow worker polling ${namespace}/${taskQueue} at ${address}`);
await worker.run();
