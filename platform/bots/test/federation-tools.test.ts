import { describe, expect, it } from "vitest";
import { parseTriggerPutArgs } from "../src/activities/tools.js";
import { BOT_TOOL_SCHEMAS, MAX_BOT_HOPS } from "../src/contracts/bots.js";
import { inboxSpecInput, triggerCreateInput } from "../src/config.js";

describe("federation tool surface", () => {
  it("flattens bot_trigger_put arguments for the inbox kind", () => {
    const flat = parseTriggerPutArgs({
      name: "inbox",
      kind: "bot",
      from: ["triage", "", 7, "ops"],
      filter: 'event.kind.startsWith("incident.")',
      whenBusy: "append",
    });
    expect(flat.create).toEqual({
      name: "inbox",
      kind: "bot",
      spec: { from: ["triage", "ops"] },
      filter: 'event.kind.startsWith("incident.")',
      route: null,
      coalesce: null,
      deliver: { whenBusy: "append" },
    });
    const parsed = triggerCreateInput.parse(flat.create);
    expect(parsed).toMatchObject({ kind: "bot", spec: { from: ["triage", "ops"] }, enabled: true });
    // No senders listed reads as "any bot".
    expect(parseTriggerPutArgs({ name: "inbox", kind: "bot" }).create).toMatchObject({ spec: {} });
    expect(() => parseTriggerPutArgs({ name: "x", kind: "mailbox" })).toThrow(/kind must be/);
  });

  it("validates inbox senders as bot ids", () => {
    expect(inboxSpecInput.safeParse({ from: ["triage"] }).success).toBe(true);
    expect(inboxSpecInput.safeParse({ from: ["Not A Bot"] }).success).toBe(false);
    expect(inboxSpecInput.safeParse({}).success).toBe(true);
  });

  it("declares to, reply, and the inbox kind on the model-facing schemas", () => {
    const emit = BOT_TOOL_SCHEMAS.emitInput.properties;
    expect(emit.to.type).toEqual(["string", "null"]);
    expect(emit.reply.type).toEqual(["boolean", "null"]);
    expect(BOT_TOOL_SCHEMAS.triggerPutInput.properties.kind.enum).toContain("bot");
    expect(BOT_TOOL_SCHEMAS.triggerPutInput.properties.from.type).toEqual(["array", "null"]);
    expect(MAX_BOT_HOPS).toBe(8);
  });
});
