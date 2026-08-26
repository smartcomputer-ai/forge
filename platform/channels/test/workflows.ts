import { defineQuery, defineSignal, setHandler, condition } from "@temporalio/workflow";
import type { EmissionEnvelope } from "@lightspeed/agent-client/workflow";

export { channelConversationWorkflowV1 } from "../src/workflows/conversation.js";

const emissionSignal = defineSignal<[EmissionEnvelope]>("deliver_emission");
const holderStateQuery = defineQuery<EmissionEnvelope[]>("holder_state");

export async function testHolderWorkflow(): Promise<never> {
  const emissions: EmissionEnvelope[] = [];
  setHandler(emissionSignal, (emission) => {
    emissions.push(emission);
  });
  setHandler(holderStateQuery, () => [...emissions]);
  for (;;) {
    await condition(() => false);
  }
}
