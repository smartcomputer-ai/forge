import { condition, defineQuery, defineSignal, setHandler } from "@temporalio/workflow";

/**
 * Test stand-in for a core session workflow: records `deliver_emission`
 * signals so a test can observe the controller's tool replies.
 */
export async function fakeSessionWorkflow(): Promise<void> {
  const received: unknown[] = [];
  setHandler(defineSignal<[unknown]>("deliver_emission"), (envelope) => {
    received.push(envelope);
  });
  setHandler(defineQuery<unknown[]>("received"), () => received);
  await condition(() => false);
}
