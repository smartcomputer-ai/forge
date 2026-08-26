import type {
  WorkflowEndpointInput,
  WorkflowToolDeclarationInput,
} from "@lightspeed/agent-client";

export const CHANNEL_TOOL_DEADLINE_MS = 120_000;
/**
 * Declaration revision of the `message_*` tools. Declarations are immutable
 * per session; the bot controller rotates a routed session whose carried
 * declarations no longer match, so a bump here rolls out on the next message.
 */
export const CHANNEL_TOOLS_REVISION = 2;

export const CHANNEL_SEND_TOOL_ID = "channels.message_send.v1";
export const CHANNEL_EDIT_TOOL_ID = "channels.message_edit.v1";
export const CHANNEL_REACT_TOOL_ID = "channels.message_react.v1";
export const CHANNEL_NOOP_TOOL_ID = "channels.message_noop.v1";
export const CHANNEL_TOOL_IDS: readonly string[] = [
  CHANNEL_SEND_TOOL_ID,
  CHANNEL_EDIT_TOOL_ID,
  CHANNEL_REACT_TOOL_ID,
  CHANNEL_NOOP_TOOL_ID,
];

/**
 * Messages are named to the model by the bot's event number: `#17` in an
 * event header is the message to reply to, and a send returns the number of
 * the message it created. Provider message ids never reach the model.
 */
export const CHANNEL_TOOL_DESCRIPTIONS = {
  send:
    "Send a message to this conversation. replyTo is the number of the message to reply to (the #N in its event header, or the number a previous send returned), or null. Completes after the provider acknowledges delivery and returns the new message's number.",
  edit:
    "Edit a message you sent earlier in this conversation, by the number your send returned. Completes after the provider acknowledges the edit.",
  react:
    "React to a message in this conversation by its number (the #N in its event header, or the number a send returned). Completes after the provider acknowledges the reaction.",
  noop: "Deliberately send no reply to the conversation.",
} as const;

export const CHANNEL_TOOL_SCHEMAS = {
  sendInput: {
    type: "object",
    description: "Send a message to this conversation.",
    properties: {
      text: { type: "string", minLength: 1, description: "Message text in Markdown." },
      replyTo: {
        type: ["integer", "null"],
        minimum: 1,
        description: "Number of the message to reply to, or null.",
      },
    },
    // Strict function schemas must require every declared property. Optional
    // values are represented as required nullable properties for providers
    // (such as OpenAI Responses) that enforce that invariant.
    required: ["text", "replyTo"],
    additionalProperties: false,
  },
  editInput: {
    type: "object",
    description: "Edit a message you sent earlier.",
    properties: {
      message: { type: "integer", minimum: 1, description: "Number returned by the send." },
      text: { type: "string", minLength: 1, description: "Replacement text in Markdown." },
    },
    required: ["message", "text"],
    additionalProperties: false,
  },
  reactInput: {
    type: "object",
    description: "React to a message.",
    properties: {
      message: { type: "integer", minimum: 1, description: "Number of the message." },
      emoji: { type: "string", minLength: 1 },
    },
    required: ["message", "emoji"],
    additionalProperties: false,
  },
  noopInput: {
    type: "object",
    description: "Deliberately send no reply to the conversation.",
    properties: {
      reason: { type: "string" },
    },
    required: ["reason"],
    additionalProperties: false,
  },
  sendReceipt: {
    type: "object",
    properties: {
      sent: { type: "integer", minimum: 1, description: "Number of the message just sent." },
    },
    required: ["sent"],
    additionalProperties: false,
  },
  messageReceipt: {
    type: "object",
    properties: {
      message: { type: "integer", minimum: 1 },
    },
    required: ["message"],
    additionalProperties: false,
  },
} as const;

export type ChannelToolSchemaName = keyof typeof CHANNEL_TOOL_SCHEMAS;
export type ChannelToolSchemaRefs = Record<ChannelToolSchemaName, string>;
export type ChannelToolDescriptionName = keyof typeof CHANNEL_TOOL_DESCRIPTIONS;
export type ChannelToolDescriptionRefs = Record<ChannelToolDescriptionName, string>;

/**
 * The declarations a conversation carries into the bot's routed session:
 * bound to this conversation workflow as receiver, so `message_send` needs
 * no route argument. The array is stored in CAS and referenced by the event;
 * the bot controller merges it verbatim after its own tools.
 */
export function channelWorkflowTools(
  receiver: WorkflowEndpointInput,
  schemas: ChannelToolSchemaRefs,
  descriptions: ChannelToolDescriptionRefs,
): WorkflowToolDeclarationInput[] {
  const joined = (
    toolId: string,
    semanticType: string,
    name: string,
    inputSchemaRef: string,
    descriptionRef: string,
    replySchemaRef: string,
  ): WorkflowToolDeclarationInput => ({
    definition: {
      toolId,
      revision: CHANNEL_TOOLS_REVISION,
      semanticType,
      tool: {
        name,
        parallelism: "exclusive",
        kind: {
          type: "function",
          inputSchemaRef,
          descriptionRef,
          strict: true,
        },
      },
    },
    target: { type: "bound", receiver, dispatch: "push" },
    completion: {
      type: "joined",
      deadlineAfterMs: CHANNEL_TOOL_DEADLINE_MS,
      replySchemaRef,
    },
  });

  return [
    joined(
      CHANNEL_SEND_TOOL_ID,
      "channels.message.send.v1",
      "message_send",
      schemas.sendInput,
      descriptions.send,
      schemas.sendReceipt,
    ),
    joined(
      CHANNEL_EDIT_TOOL_ID,
      "channels.message.edit.v1",
      "message_edit",
      schemas.editInput,
      descriptions.edit,
      schemas.messageReceipt,
    ),
    joined(
      CHANNEL_REACT_TOOL_ID,
      "channels.message.react.v1",
      "message_react",
      schemas.reactInput,
      descriptions.react,
      schemas.messageReceipt,
    ),
    {
      definition: {
        toolId: CHANNEL_NOOP_TOOL_ID,
        revision: CHANNEL_TOOLS_REVISION,
        semanticType: "channels.message.noop.v1",
        tool: {
          name: "message_noop",
          parallelism: "exclusive",
          kind: {
            type: "function",
            inputSchemaRef: schemas.noopInput,
            descriptionRef: descriptions.noop,
            strict: true,
          },
        },
      },
      target: { type: "bound", receiver, dispatch: "pull" },
      completion: { type: "accepted" },
    },
  ];
}
