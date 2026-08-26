import { describe, expect, it } from "vitest";

import {
  DELIVER_EMISSION_SIGNAL,
  REPLY_COMPLETION_KEY,
  WORKFLOW_CONTRACT_VECTORS,
  WORKFLOW_TOOL_RECOVERY_QUERY,
  emissionId,
  environmentJobWorkflowId,
  parseEmissionEnvelope,
  recipeFingerprint,
  replyPromiseId,
  sessionWorkflowId,
  sourceResolutionEnvelope,
  splitWorkflowId,
  type EmissionEnvelope,
  type WorkflowToolInvocation,
} from "../src/workflow.js";

describe("generated workflow contract", () => {
  const { inputs } = WORKFLOW_CONTRACT_VECTORS;

  it("matches every Rust derivation vector", () => {
    expect(
      emissionId.runTerminal(
        inputs.universeId,
        inputs.sessionId,
        inputs.runId,
        inputs.token,
      ),
    ).toBe(WORKFLOW_CONTRACT_VECTORS.emissionIds.runTerminal);
    expect(
      emissionId.sourceResolution(
        inputs.universeId,
        inputs.producerWorkflowId,
        inputs.holderWorkflowId,
        inputs.promiseId,
      ),
    ).toBe(WORKFLOW_CONTRACT_VECTORS.emissionIds.sourceResolution);
    expect(emissionId.toolInvocation(inputs.invocationId)).toBe(
      WORKFLOW_CONTRACT_VECTORS.emissionIds.toolInvocation,
    );
    expect(
      emissionId.invocationCancellation(
        inputs.invocationId,
        inputs.completionKey,
      ),
    ).toBe(WORKFLOW_CONTRACT_VECTORS.emissionIds.invocationCancellation);
    expect(recipeFingerprint(inputs.recipeJson)).toBe(
      WORKFLOW_CONTRACT_VECTORS.recipeFingerprint,
    );
    expect(sessionWorkflowId(inputs.universeId, inputs.sessionId)).toBe(
      WORKFLOW_CONTRACT_VECTORS.workflowIds.session,
    );
    expect(
      environmentJobWorkflowId(
        inputs.universeId,
        inputs.environmentId,
        inputs.jobGroupId,
      ),
    ).toBe(WORKFLOW_CONTRACT_VECTORS.workflowIds.environmentJob);
    expect(
      splitWorkflowId(WORKFLOW_CONTRACT_VECTORS.workflowIds.session),
    ).toEqual(WORKFLOW_CONTRACT_VECTORS.workflowIds.split);
  });

  it("exports manifest-owned names and the reply convention", () => {
    expect(DELIVER_EMISSION_SIGNAL).toBe("deliver_emission");
    expect(WORKFLOW_TOOL_RECOVERY_QUERY).toBe("workflow_tool_recovery");
    expect(REPLY_COMPLETION_KEY).toBe("reply");
    const invocation = WORKFLOW_CONTRACT_VECTORS.startArgs
      .invocation as WorkflowToolInvocation;
    expect(replyPromiseId(invocation)).toBe(inputs.promiseId);
  });

  it("parses every Rust envelope vector", () => {
    for (const envelope of WORKFLOW_CONTRACT_VECTORS.envelopes) {
      expect(parseEmissionEnvelope(envelope)).toEqual(envelope);
    }
  });

  it("constructs canonical source resolutions", () => {
    const envelope = sourceResolutionEnvelope({
      universeId: inputs.universeId,
      producerWorkflowId: inputs.producerWorkflowId,
      holderWorkflowId: inputs.holderWorkflowId,
      promiseId: inputs.promiseId,
      resolution: { kind: "resolved", payload_ref: null },
    });
    expect(envelope).toEqual(WORKFLOW_CONTRACT_VECTORS.envelopes[1]);
  });

  it("rejects unknown bodies and non-canonical ids", () => {
    const envelope = WORKFLOW_CONTRACT_VECTORS.envelopes[0] as EmissionEnvelope;
    expect(() =>
      parseEmissionEnvelope({
        ...envelope,
        body: { kind: "future_protocol_shape" },
      }),
    ).toThrow(/unknown emission body kind/);
    expect(() =>
      parseEmissionEnvelope({
        ...envelope,
        emission_id: "emission:sha256:ABC",
      }),
    ).toThrow(/invalid format/);
  });
});
