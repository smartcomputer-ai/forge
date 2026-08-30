//! Delivery: what the model asked for (messages by `#N`), what a connector
//! executes (provider message ids), and the chunked plan between them.
//!
//! The conversation workflow resolves numbers to provider ids before
//! anything reaches a connector; sends are split before scheduling so every
//! completed chunk is its own durable activity result.

use api::{ChannelAccountId, ChannelProvider};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::ConversationRef;
use crate::tools::{CHANNEL_EDIT_TOOL_ID, CHANNEL_REACT_TOOL_ID, CHANNEL_SEND_TOOL_ID};

/// Wire version of delivery commands and results.
pub const CHANNEL_DELIVERY_VERSION: u32 = 1;
/// Characters per send chunk; below every supported provider's text limit.
pub const DELIVERY_CHUNK_CHARS: usize = 3_500;
/// Chunks one send may split into.
pub const MAX_DELIVERY_CHUNKS: usize = 32;
/// Provider message ids one delivery result may carry.
pub const MAX_DELIVERY_MESSAGE_IDS: usize = 32;

const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// Where a delivery goes: one account's chat, optionally a thread.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelRoute {
    pub provider: ChannelProvider,
    pub account_id: ChannelAccountId,
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

impl ChannelRoute {
    pub fn new(provider: ChannelProvider, conversation: &ConversationRef) -> Self {
        Self {
            provider,
            account_id: conversation.account_id.clone(),
            chat_id: conversation.chat_id.clone(),
            thread_id: conversation.thread_id.clone(),
        }
    }

    pub fn conversation(&self) -> ConversationRef {
        ConversationRef {
            account_id: self.account_id.clone(),
            chat_id: self.chat_id.clone(),
            thread_id: self.thread_id.clone(),
        }
    }
}

/// What the model asked for: messages by number (`#N`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ChannelToolOperation {
    Send {
        text: String,
        /// `null` when the send is not a reply.
        #[serde(default)]
        reply_to: Option<u64>,
    },
    Edit {
        message: u64,
        text: String,
    },
    React {
        message: u64,
        emoji: String,
    },
}

/// The message a send replies to, for providers that quote it themselves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplyContext {
    pub sender_id: String,
    pub text: String,
}

/// What a connector executes: provider message ids and their direction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ChannelDeliveryOperation {
    Send {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_context: Option<ReplyContext>,
    },
    Edit {
        message_id: String,
        text: String,
    },
    React {
        message_id: String,
        emoji: String,
        /// Whether the reacted-to message is the bot's own.
        from_me: bool,
    },
}

/// One connector activity call. `idempotency_key` is the invocation id, or
/// `{invocation}:chunk:{i}/{n}` for a chunk of a split send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDeliveryCommand {
    pub version: u32,
    pub invocation_id: String,
    pub idempotency_key: String,
    pub route: ChannelRoute,
    pub operation: ChannelDeliveryOperation,
}

impl ChannelDeliveryCommand {
    pub fn new(
        invocation_id: impl Into<String>,
        idempotency_key: impl Into<String>,
        route: ChannelRoute,
        operation: ChannelDeliveryOperation,
    ) -> Self {
        Self {
            version: CHANNEL_DELIVERY_VERSION,
            invocation_id: invocation_id.into(),
            idempotency_key: idempotency_key.into(),
            route,
            operation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDeliveryResult {
    pub version: u32,
    pub provider: ChannelProvider,
    /// Every provider id the delivery produced (a chunked send has several;
    /// the first is the anchor).
    pub message_ids: Vec<String>,
}

/// Decode a pushed `message_*` invocation's arguments into an operation.
pub fn parse_tool_operation(tool_id: &str, value: &Value) -> Result<ChannelToolOperation, String> {
    let args = value
        .as_object()
        .ok_or_else(|| "tool arguments must be an object".to_owned())?;
    match tool_id {
        CHANNEL_SEND_TOOL_ID => {
            let text = non_empty_string(args.get("text"), "text")?;
            let reply_to = match args.get("replyTo") {
                None | Some(Value::Null) => None,
                Some(value) => Some(handle(value, "replyTo")?),
            };
            Ok(ChannelToolOperation::Send { text, reply_to })
        }
        CHANNEL_EDIT_TOOL_ID => Ok(ChannelToolOperation::Edit {
            message: handle(args.get("message").unwrap_or(&Value::Null), "message")?,
            text: non_empty_string(args.get("text"), "text")?,
        }),
        CHANNEL_REACT_TOOL_ID => Ok(ChannelToolOperation::React {
            message: handle(args.get("message").unwrap_or(&Value::Null), "message")?,
            emoji: non_empty_string(args.get("emoji"), "emoji")?,
        }),
        other => Err(format!("unsupported pushed channel tool: {other}")),
    }
}

fn non_empty_string(value: Option<&Value>, name: &str) -> Result<String, String> {
    match value.and_then(Value::as_str) {
        Some(text) if !text.is_empty() => Ok(text.to_owned()),
        _ => Err(format!("{name} must be a non-empty string")),
    }
}

/// A message number: a positive safe integer.
fn handle(value: &Value, name: &str) -> Result<u64, String> {
    let number = match value {
        Value::Number(number) => number.as_u64().or_else(|| {
            number
                .as_f64()
                .filter(|float| float.is_finite() && float.fract() == 0.0 && *float >= 0.0)
                .map(|float| float as u64)
        }),
        _ => None,
    };
    number
        .filter(|number| (1..=MAX_SAFE_INTEGER).contains(number))
        .ok_or_else(|| format!("{name} must be a message number (the #N of a message)"))
}

pub fn validate_delivery_result(
    result: &ChannelDeliveryResult,
    expected_provider: ChannelProvider,
) -> Result<(), String> {
    if result.version != CHANNEL_DELIVERY_VERSION || result.provider != expected_provider {
        return Err("delivery result does not match the command provider".to_owned());
    }
    if result.message_ids.is_empty()
        || result.message_ids.len() > MAX_DELIVERY_MESSAGE_IDS
        || result.message_ids.iter().any(String::is_empty)
    {
        return Err(format!(
            "delivery result must contain 1 to {MAX_DELIVERY_MESSAGE_IDS} message ids"
        ));
    }
    Ok(())
}

/// Whether the command's idempotency key was derived from its invocation.
pub fn is_delivery_idempotency_key(command: &ChannelDeliveryCommand) -> bool {
    command.idempotency_key == command.invocation_id
        || command
            .idempotency_key
            .strip_prefix(&command.invocation_id)
            .is_some_and(|rest| rest.starts_with(":chunk:"))
}

/// Split a message into chunks of at most `max_chars` characters, cutting at
/// the last newline (or space) in the second half of the window.
pub fn split_message_text(text: &str, max_chars: usize) -> Result<Vec<String>, String> {
    split_message_text_bounded(text, max_chars, MAX_DELIVERY_CHUNKS)
}

pub fn split_message_text_bounded(
    text: &str,
    max_chars: usize,
    max_chunks: usize,
) -> Result<Vec<String>, String> {
    let limit = max_chars.max(1);
    let mut chunks = Vec::new();
    let mut remaining: Vec<char> = text.chars().collect();
    while remaining.len() > limit {
        let cut = last_index_at_or_before(&remaining, '\n', limit)
            .filter(|cut| cut * 2 >= limit)
            .or_else(|| {
                last_index_at_or_before(&remaining, ' ', limit).filter(|cut| cut * 2 >= limit)
            })
            .unwrap_or(limit);
        let head: String = remaining[..cut].iter().collect();
        chunks.push(head.trim_end().to_owned());
        let tail_start = remaining[cut..]
            .iter()
            .position(|ch| !ch.is_whitespace())
            .map_or(remaining.len(), |offset| cut + offset);
        remaining.drain(..tail_start);
        if chunks.len() >= max_chunks {
            return Err(format!(
                "message exceeds the {max_chunks}-chunk provider delivery limit"
            ));
        }
    }
    if !remaining.is_empty() {
        chunks.push(remaining.into_iter().collect());
    }
    Ok(chunks)
}

fn last_index_at_or_before(chars: &[char], needle: char, position: usize) -> Option<usize> {
    let end = position.min(chars.len().saturating_sub(1));
    chars[..=end].iter().rposition(|ch| *ch == needle)
}

/// Plan the connector calls for one operation with the default chunk size.
pub fn plan_delivery_commands(
    invocation_id: &str,
    route: &ChannelRoute,
    operation: &ChannelDeliveryOperation,
) -> Result<Vec<ChannelDeliveryCommand>, String> {
    plan_delivery_commands_with(invocation_id, route, operation, DELIVERY_CHUNK_CHARS)
}

/// Split sends into chunk commands (reply fields only on the first); edits
/// and reactions stay one atomic command keyed by the invocation id.
pub fn plan_delivery_commands_with(
    invocation_id: &str,
    route: &ChannelRoute,
    operation: &ChannelDeliveryOperation,
    chunk_chars: usize,
) -> Result<Vec<ChannelDeliveryCommand>, String> {
    let ChannelDeliveryOperation::Send {
        text,
        reply_to,
        reply_context,
    } = operation
    else {
        return Ok(vec![ChannelDeliveryCommand::new(
            invocation_id,
            invocation_id,
            route.clone(),
            operation.clone(),
        )]);
    };
    let chunks = split_message_text_bounded(text, chunk_chars, MAX_DELIVERY_CHUNKS)?;
    if chunks.is_empty() {
        return Err("send text must not be empty".to_owned());
    }
    let count = chunks.len();
    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            let key = if count == 1 {
                invocation_id.to_owned()
            } else {
                format!("{invocation_id}:chunk:{}/{count}", index + 1)
            };
            let first = index == 0;
            ChannelDeliveryCommand::new(
                invocation_id,
                key,
                route.clone(),
                ChannelDeliveryOperation::Send {
                    text,
                    reply_to: first.then(|| reply_to.clone()).flatten(),
                    reply_context: first.then(|| reply_context.clone()).flatten(),
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn route() -> ChannelRoute {
        ChannelRoute {
            provider: ChannelProvider::Telegram,
            account_id: ChannelAccountId::new("primary"),
            chat_id: "123".to_owned(),
            thread_id: None,
        }
    }

    #[test]
    fn decodes_tool_arguments_as_message_numbers() {
        assert_eq!(
            parse_tool_operation(
                CHANNEL_SEND_TOOL_ID,
                &json!({ "text": "hello", "replyTo": 41 })
            ),
            Ok(ChannelToolOperation::Send {
                text: "hello".to_owned(),
                reply_to: Some(41),
            })
        );
        assert_eq!(
            parse_tool_operation(
                CHANNEL_SEND_TOOL_ID,
                &json!({ "text": "hello", "replyTo": null })
            ),
            Ok(ChannelToolOperation::Send {
                text: "hello".to_owned(),
                reply_to: None,
            })
        );
        assert_eq!(
            parse_tool_operation(
                CHANNEL_EDIT_TOOL_ID,
                &json!({ "message": 42, "text": "fixed" })
            ),
            Ok(ChannelToolOperation::Edit {
                message: 42,
                text: "fixed".to_owned(),
            })
        );
        assert_eq!(
            parse_tool_operation(
                CHANNEL_REACT_TOOL_ID,
                &json!({ "message": 42, "emoji": "👍" })
            ),
            Ok(ChannelToolOperation::React {
                message: 42,
                emoji: "👍".to_owned(),
            })
        );
        let error = parse_tool_operation(
            CHANNEL_SEND_TOOL_ID,
            &json!({ "text": "hello", "replyTo": "41" }),
        )
        .unwrap_err();
        assert!(error.contains("message number"), "{error}");
        let error = parse_tool_operation(
            CHANNEL_REACT_TOOL_ID,
            &json!({ "message": 0, "emoji": "x" }),
        )
        .unwrap_err();
        assert!(error.contains("message number"), "{error}");
        let error = parse_tool_operation(
            CHANNEL_REACT_TOOL_ID,
            &json!({ "message": 1.5, "emoji": "x" }),
        )
        .unwrap_err();
        assert!(error.contains("message number"), "{error}");
        let error = parse_tool_operation(CHANNEL_SEND_TOOL_ID, &json!({ "text": "" })).unwrap_err();
        assert!(error.contains("text must be a non-empty string"), "{error}");
        let error = parse_tool_operation("unknown", &json!({})).unwrap_err();
        assert!(error.contains("unsupported pushed channel tool"), "{error}");
        let error = parse_tool_operation(CHANNEL_SEND_TOOL_ID, &json!([])).unwrap_err();
        assert!(error.contains("must be an object"), "{error}");
    }

    #[test]
    fn tool_operations_keep_null_reply_to_on_the_wire() {
        let json = serde_json::to_value(ChannelToolOperation::Send {
            text: "hi".to_owned(),
            reply_to: None,
        })
        .unwrap();
        assert_eq!(
            json,
            json!({ "type": "send", "text": "hi", "replyTo": null })
        );
    }

    #[test]
    fn rejects_empty_or_cross_provider_receipts() {
        let result = ChannelDeliveryResult {
            version: 1,
            provider: ChannelProvider::Whatsapp,
            message_ids: vec!["42".to_owned()],
        };
        assert!(
            validate_delivery_result(&result, ChannelProvider::Telegram)
                .unwrap_err()
                .contains("does not match")
        );
        assert_eq!(
            validate_delivery_result(&result, ChannelProvider::Whatsapp),
            Ok(())
        );
        let empty = ChannelDeliveryResult {
            version: 1,
            provider: ChannelProvider::Telegram,
            message_ids: Vec::new(),
        };
        assert!(
            validate_delivery_result(&empty, ChannelProvider::Telegram)
                .unwrap_err()
                .contains("1 to 32")
        );
        let too_many = ChannelDeliveryResult {
            version: 1,
            provider: ChannelProvider::Telegram,
            message_ids: (0..33).map(|index| index.to_string()).collect(),
        };
        assert!(validate_delivery_result(&too_many, ChannelProvider::Telegram).is_err());
        let wrong_version = ChannelDeliveryResult {
            version: 2,
            provider: ChannelProvider::Telegram,
            message_ids: vec!["42".to_owned()],
        };
        assert!(validate_delivery_result(&wrong_version, ChannelProvider::Telegram).is_err());
    }

    #[test]
    fn idempotency_keys_derive_from_the_invocation() {
        let command = ChannelDeliveryCommand::new(
            "inv",
            "inv",
            route(),
            ChannelDeliveryOperation::Edit {
                message_id: "1".to_owned(),
                text: "x".to_owned(),
            },
        );
        assert!(is_delivery_idempotency_key(&command));
        let chunk = ChannelDeliveryCommand {
            idempotency_key: "inv:chunk:1/2".to_owned(),
            ..command.clone()
        };
        assert!(is_delivery_idempotency_key(&chunk));
        let foreign = ChannelDeliveryCommand {
            idempotency_key: "other".to_owned(),
            ..command
        };
        assert!(!is_delivery_idempotency_key(&foreign));
    }

    #[test]
    fn splits_on_useful_boundaries_and_enforces_receipt_bounds() {
        assert_eq!(
            split_message_text("alpha beta gamma", 11).unwrap(),
            vec!["alpha beta", "gamma"]
        );
        assert_eq!(
            split_message_text("line one\nline two\nline three", 20).unwrap(),
            vec!["line one\nline two", "line three"]
        );
        assert_eq!(
            split_message_text("abcdefgh", 3).unwrap(),
            vec!["abc", "def", "gh"]
        );
        assert_eq!(split_message_text("", 10).unwrap(), Vec::<String>::new());
        assert_eq!(
            split_message_text("héllo wörld", 6).unwrap(),
            vec!["héllo", "wörld"]
        );
        let error = split_message_text_bounded("abcdefgh", 2, 2).unwrap_err();
        assert!(error.contains("2-chunk"), "{error}");
    }

    #[test]
    fn records_long_sends_as_independently_retryable_commands() {
        let operation = ChannelDeliveryOperation::Send {
            text: format!("first {}", "x".repeat(3_600)),
            reply_to: Some("41".to_owned()),
            reply_context: Some(ReplyContext {
                sender_id: "7".to_owned(),
                text: "question".to_owned(),
            }),
        };
        let commands = plan_delivery_commands("invocation", &route(), &operation).unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands
                .iter()
                .map(|command| command.idempotency_key.as_str())
                .collect::<Vec<_>>(),
            vec!["invocation:chunk:1/2", "invocation:chunk:2/2"]
        );
        for command in &commands {
            assert_eq!(command.version, 1);
            assert_eq!(command.invocation_id, "invocation");
            assert!(is_delivery_idempotency_key(command));
        }
        let ChannelDeliveryOperation::Send {
            reply_to,
            reply_context,
            ..
        } = &commands[0].operation
        else {
            panic!("first command must be a send");
        };
        assert_eq!(reply_to.as_deref(), Some("41"));
        assert_eq!(
            reply_context.as_ref().map(|context| context.text.as_str()),
            Some("question")
        );
        let second = serde_json::to_value(&commands[1].operation).unwrap();
        assert!(second.get("replyTo").is_none());
        assert!(second.get("replyContext").is_none());
        assert_eq!(second["type"], "send");
    }

    #[test]
    fn keeps_a_single_send_edit_or_reaction_on_the_invocation_id() {
        let send = plan_delivery_commands(
            "invocation",
            &route(),
            &ChannelDeliveryOperation::Send {
                text: "short".to_owned(),
                reply_to: None,
                reply_context: None,
            },
        )
        .unwrap();
        assert_eq!(send.len(), 1);
        assert_eq!(send[0].idempotency_key, "invocation");
        let edit = plan_delivery_commands(
            "invocation",
            &route(),
            &ChannelDeliveryOperation::Edit {
                message_id: "42".to_owned(),
                text: "fixed".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(edit.len(), 1);
        assert_eq!(edit[0].idempotency_key, "invocation");
        let react = plan_delivery_commands(
            "invocation",
            &route(),
            &ChannelDeliveryOperation::React {
                message_id: "42".to_owned(),
                emoji: "👍".to_owned(),
                from_me: true,
            },
        )
        .unwrap();
        let json = serde_json::to_value(&react[0]).unwrap();
        assert_eq!(json["operation"]["messageId"], "42");
        assert_eq!(json["operation"]["fromMe"], true);
        assert_eq!(json["idempotencyKey"], "invocation");
        let back: ChannelDeliveryCommand = serde_json::from_value(json).unwrap();
        assert_eq!(back, react[0]);
        let empty = plan_delivery_commands(
            "invocation",
            &route(),
            &ChannelDeliveryOperation::Send {
                text: String::new(),
                reply_to: None,
                reply_context: None,
            },
        );
        assert!(empty.is_err());
    }

    #[test]
    fn routes_round_trip_through_conversation_refs() {
        let conversation = ConversationRef {
            account_id: ChannelAccountId::new("primary"),
            chat_id: "-100".to_owned(),
            thread_id: Some("3".to_owned()),
        };
        let route = ChannelRoute::new(ChannelProvider::Whatsapp, &conversation);
        assert_eq!(route.conversation(), conversation);
        assert_eq!(route.provider, ChannelProvider::Whatsapp);
    }
}
