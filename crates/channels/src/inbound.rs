//! The normalized inbound envelope and the conversation workflow's durable
//! start input.
//!
//! A connector normalizes a provider message into [`api::ChannelInbound`];
//! admission stamps the provider and account onto it
//! ([`NormalizedInbound`]), authorizes the sender
//! ([`ChannelAuthorization`]), and hands the result to the conversation
//! workflow, which is started (once) with a secret-free
//! [`ConversationStart`].

use api::{
    BotId, BotTriggerId, ChannelAccountId, ChannelInbound, ChannelProvider, ChatAccess,
    ChatActivation, ChatScope,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ChannelError;
use crate::delivery::ChannelRoute;
use crate::ids::{ConversationRef, conversation_workflow_id};
use crate::media::{MAX_CHANNEL_MEDIA_PER_MESSAGE, validate_inbound_media};

/// One provider message with the identity of the account that received it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedInbound {
    pub provider: ChannelProvider,
    pub account_id: ChannelAccountId,
    #[serde(flatten)]
    pub inbound: ChannelInbound,
}

impl NormalizedInbound {
    pub fn new(
        provider: ChannelProvider,
        account_id: ChannelAccountId,
        inbound: ChannelInbound,
    ) -> Self {
        Self {
            provider,
            account_id,
            inbound,
        }
    }

    pub fn conversation(&self) -> ConversationRef {
        ConversationRef {
            account_id: self.account_id.clone(),
            chat_id: self.inbound.chat_id.clone(),
            thread_id: self.inbound.thread_id.clone(),
        }
    }

    pub fn scope(&self) -> ChatScope {
        if self.inbound.is_direct {
            ChatScope::Direct
        } else {
            ChatScope::Group
        }
    }

    pub fn route(&self) -> ChannelRoute {
        ChannelRoute::new(self.provider, &self.conversation())
    }
}

/// The label a conversation gets from its first message: who it is with,
/// never an id the model copies.
pub fn conversation_label(inbound: &NormalizedInbound) -> String {
    let provider = inbound.provider;
    if inbound.inbound.is_direct {
        return format!("{provider} dm · {}", inbound.inbound.sender_name);
    }
    match &inbound.inbound.thread_id {
        Some(thread_id) => format!(
            "{provider} group · {} · thread {thread_id}",
            inbound.inbound.chat_id
        ),
        None => format!("{provider} group · {}", inbound.inbound.chat_id),
    }
}

/// Validate a connector's inbound and return it with every attachment's
/// admitted MIME type.
pub fn normalize_inbound(inbound: &ChannelInbound) -> Result<ChannelInbound, ChannelError> {
    for (name, value) in [
        ("messageId", &inbound.message_id),
        ("chatId", &inbound.chat_id),
        ("senderId", &inbound.sender_id),
        ("senderName", &inbound.sender_name),
    ] {
        if value.is_empty() {
            return Err(ChannelError::invalid(format!(
                "{name} must be a non-empty string"
            )));
        }
    }
    if inbound.thread_id.as_deref().is_some_and(str::is_empty) {
        return Err(ChannelError::invalid("threadId must be a non-empty string"));
    }
    if inbound.timestamp_ms < 0 {
        return Err(ChannelError::invalid("timestampMs must not be negative"));
    }
    if inbound.media.len() > MAX_CHANNEL_MEDIA_PER_MESSAGE {
        return Err(ChannelError::invalid(format!(
            "channel inbound media must contain at most {MAX_CHANNEL_MEDIA_PER_MESSAGE} items"
        )));
    }
    let media = inbound
        .media
        .iter()
        .map(|media| validate_inbound_media(media).map_err(ChannelError::invalid))
        .collect::<Result<Vec<_>, _>>()?;
    if inbound.text.is_empty() && media.is_empty() {
        return Err(ChannelError::invalid(
            "channel inbound payload must contain text or media",
        ));
    }
    Ok(ChannelInbound {
        media,
        ..inbound.clone()
    })
}

pub fn validate_inbound(inbound: &ChannelInbound) -> Result<(), ChannelError> {
    normalize_inbound(inbound).map(drop)
}

/// What the sender may do, decided at admission from the trigger's access
/// policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelAuthorization {
    pub turn_allowed: bool,
    pub control_allowed: bool,
}

/// The conversation workflow's inbound signal: an authorized message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdmittedInbound {
    #[serde(flatten)]
    pub inbound: NormalizedInbound,
    pub authorization: ChannelAuthorization,
}

/// Secret-free durable input of one conversation workflow: the chat trigger
/// it serves and the conversation it fronts. The workflow owns nothing on
/// the core side: the bot controller creates and controls the session; the
/// workflow is the source of the conversation's events and the receiver of
/// its `message_*` tools.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConversationStart {
    #[schemars(with = "String")]
    pub universe_id: Uuid,
    pub bot_id: BotId,
    pub trigger_id: BotTriggerId,
    pub account_id: ChannelAccountId,
    pub provider: ChannelProvider,
    pub conversation: ConversationRef,
    pub scope: ChatScope,
    pub activation: ChatActivation,
    pub access: ChatAccess,
    /// Human label of the conversation (routed session label, display name).
    pub label: String,
    /// Task queue of the connector host serving the account.
    pub connector_task_queue: String,
}

impl ConversationStart {
    pub fn workflow_id(&self) -> String {
        conversation_workflow_id(self.universe_id, self.provider, &self.conversation)
    }

    pub fn route(&self) -> ChannelRoute {
        ChannelRoute::new(self.provider, &self.conversation)
    }

    /// An inbound signal must belong to this conversation: same provider,
    /// account, chat, thread, and scope.
    pub fn check_inbound(&self, inbound: &NormalizedInbound) -> Result<(), ChannelError> {
        if inbound.scope() != self.scope {
            return Err(ChannelError::invalid(
                "conversation scope must match the inbound scope",
            ));
        }
        if inbound.provider != self.provider
            || inbound.account_id != self.account_id
            || inbound.conversation() != self.conversation
        {
            return Err(ChannelError::invalid(
                "inbound route must match the conversation route",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::{ChannelInboundMedia, ChannelMediaKind};

    fn inbound() -> ChannelInbound {
        ChannelInbound {
            message_id: "42".to_owned(),
            chat_id: "123".to_owned(),
            thread_id: None,
            sender_id: "7".to_owned(),
            sender_name: "Lukas".to_owned(),
            timestamp_ms: 1_700_000_000_000,
            text: "hello".to_owned(),
            media: Vec::new(),
            is_direct: true,
            mentioned_bot: false,
            is_reply_to_bot: false,
        }
    }

    fn normalized(inbound: ChannelInbound) -> NormalizedInbound {
        NormalizedInbound::new(
            ChannelProvider::Telegram,
            ChannelAccountId::new("primary"),
            inbound,
        )
    }

    fn image() -> ChannelInboundMedia {
        ChannelInboundMedia {
            file_id: "file-1".to_owned(),
            kind: ChannelMediaKind::Image,
            mime: "image/jpeg".to_owned(),
            name: None,
            byte_size: None,
        }
    }

    fn start() -> ConversationStart {
        ConversationStart {
            universe_id: Uuid::parse_str("6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f").unwrap(),
            bot_id: BotId::new("concierge"),
            trigger_id: BotTriggerId::new("tg"),
            account_id: ChannelAccountId::new("primary"),
            provider: ChannelProvider::Telegram,
            conversation: ConversationRef {
                account_id: ChannelAccountId::new("primary"),
                chat_id: "123".to_owned(),
                thread_id: None,
            },
            scope: ChatScope::Direct,
            activation: ChatActivation::default(),
            access: ChatAccess::default(),
            label: "telegram dm · Lukas".to_owned(),
            connector_task_queue: "lightspeed-connector-telegram-test".to_owned(),
        }
    }

    #[test]
    fn labels_conversations_by_counterpart_never_by_message_id() {
        assert_eq!(
            conversation_label(&normalized(inbound())),
            "telegram dm · Lukas"
        );
        let group = normalized(ChannelInbound {
            is_direct: false,
            chat_id: "-100".to_owned(),
            thread_id: Some("3".to_owned()),
            ..inbound()
        });
        assert_eq!(
            conversation_label(&group),
            "telegram group · -100 · thread 3"
        );
        let plain_group = normalized(ChannelInbound {
            is_direct: false,
            chat_id: "-100".to_owned(),
            ..inbound()
        });
        assert_eq!(conversation_label(&plain_group), "telegram group · -100");
        assert!(!conversation_label(&group).contains("42"));
    }

    #[test]
    fn derives_conversation_and_scope() {
        let message = normalized(ChannelInbound {
            thread_id: Some("9".to_owned()),
            ..inbound()
        });
        assert_eq!(message.conversation().key(), "primary/123/9");
        assert_eq!(message.scope(), ChatScope::Direct);
        assert_eq!(message.route().thread_id.as_deref(), Some("9"));
        let group = normalized(ChannelInbound {
            is_direct: false,
            ..inbound()
        });
        assert_eq!(group.scope(), ChatScope::Group);
    }

    #[test]
    fn validates_inbound_shape() {
        assert_eq!(validate_inbound(&inbound()), Ok(()));
        let no_sender = ChannelInbound {
            sender_id: String::new(),
            ..inbound()
        };
        assert!(matches!(
            validate_inbound(&no_sender),
            Err(ChannelError::InvalidInput { message }) if message.contains("senderId")
        ));
        let empty_thread = ChannelInbound {
            thread_id: Some(String::new()),
            ..inbound()
        };
        assert!(validate_inbound(&empty_thread).is_err());
        let negative = ChannelInbound {
            timestamp_ms: -1,
            ..inbound()
        };
        assert!(validate_inbound(&negative).is_err());
        let nothing = ChannelInbound {
            text: String::new(),
            ..inbound()
        };
        assert!(matches!(
            validate_inbound(&nothing),
            Err(ChannelError::InvalidInput { message }) if message.contains("text or media")
        ));
        let media_only = ChannelInbound {
            text: String::new(),
            media: vec![image()],
            ..inbound()
        };
        assert_eq!(validate_inbound(&media_only), Ok(()));
        let too_many = ChannelInbound {
            media: vec![image(); MAX_CHANNEL_MEDIA_PER_MESSAGE + 1],
            ..inbound()
        };
        assert!(validate_inbound(&too_many).is_err());
        let unsupported = ChannelInbound {
            media: vec![ChannelInboundMedia {
                mime: "image/svg+xml".to_owned(),
                ..image()
            }],
            ..inbound()
        };
        assert!(matches!(
            validate_inbound(&unsupported),
            Err(ChannelError::InvalidInput { message }) if message.contains("unsupported image MIME")
        ));
    }

    #[test]
    fn normalizes_media_mime_types() {
        let voice = ChannelInbound {
            media: vec![ChannelInboundMedia {
                kind: ChannelMediaKind::Audio,
                mime: "application/octet-stream".to_owned(),
                name: Some("voice.opus".to_owned()),
                ..image()
            }],
            ..inbound()
        };
        let normalized = normalize_inbound(&voice).unwrap();
        assert_eq!(normalized.media[0].mime, "audio/ogg");
        assert_eq!(normalized.text, "hello");
    }

    #[test]
    fn inbound_wire_shape_flattens_the_provider_message() {
        let message = normalized(inbound());
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["provider"], "telegram");
        assert_eq!(json["accountId"], "primary");
        assert_eq!(json["messageId"], "42");
        assert_eq!(json["isReplyToBot"], false);
        assert!(json.get("inbound").is_none());
        let back: NormalizedInbound = serde_json::from_value(json).unwrap();
        assert_eq!(back, message);

        let admitted = AdmittedInbound {
            inbound: message,
            authorization: ChannelAuthorization {
                turn_allowed: true,
                control_allowed: false,
            },
        };
        let json = serde_json::to_value(&admitted).unwrap();
        assert_eq!(json["authorization"]["turnAllowed"], true);
        assert_eq!(json["senderName"], "Lukas");
        let back: AdmittedInbound = serde_json::from_value(json).unwrap();
        assert_eq!(back, admitted);
    }

    #[test]
    fn conversation_start_round_trips_and_checks_identity() {
        let start = start();
        let json = serde_json::to_value(&start).unwrap();
        assert_eq!(json["universeId"], "6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f");
        assert_eq!(
            json["connectorTaskQueue"],
            "lightspeed-connector-telegram-test"
        );
        assert_eq!(json["conversation"]["chatId"], "123");
        assert_eq!(json["scope"], "direct");
        let back: ConversationStart = serde_json::from_value(json).unwrap();
        assert_eq!(back, start);
        assert!(
            start
                .workflow_id()
                .starts_with("6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f/chat-telegram-")
        );
        assert_eq!(start.route().chat_id, "123");

        assert_eq!(start.check_inbound(&normalized(inbound())), Ok(()));
        let other_chat = normalized(ChannelInbound {
            chat_id: "999".to_owned(),
            ..inbound()
        });
        assert!(matches!(
            start.check_inbound(&other_chat),
            Err(ChannelError::InvalidInput { message }) if message.contains("conversation route")
        ));
        let group = normalized(ChannelInbound {
            is_direct: false,
            ..inbound()
        });
        assert!(matches!(
            start.check_inbound(&group),
            Err(ChannelError::InvalidInput { message }) if message.contains("scope")
        ));
        let other_account = NormalizedInbound::new(
            ChannelProvider::Telegram,
            ChannelAccountId::new("secondary"),
            inbound(),
        );
        assert!(start.check_inbound(&other_account).is_err());
    }
}
