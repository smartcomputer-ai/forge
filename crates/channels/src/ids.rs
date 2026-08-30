//! Conversation identities: keys, workflow ids, delivery task queues, and
//! pairing keys. Every derivation is length-prefixed and domain-separated
//! so provider chat ids never appear in a Temporal id or a row key.

use api::{ChannelAccountId, ChannelProvider};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const CONVERSATION_KEY_DOMAIN: &str = "lightspeed.channels.conversation.v1";
const PAIRING_KEY_DOMAIN: &str = "lightspeed.channels.pairing.v1";
const DELIVERY_QUEUE_DOMAIN: &str = "lightspeed.channels.delivery-queue.v1";

fn framed_digest(domain: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update((domain.len() as u64).to_be_bytes());
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// The conversation a message belongs to: one account's chat, optionally
/// a thread inside it.
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRef {
    pub account_id: ChannelAccountId,
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

impl ConversationRef {
    /// The routing key of the conversation (`data.conversation.key` for CEL
    /// route keys): readable, but never shown to the model.
    pub fn key(&self) -> String {
        match &self.thread_id {
            Some(thread_id) => format!("{}/{}/{}", self.account_id, self.chat_id, thread_id),
            None => format!("{}/{}", self.account_id, self.chat_id),
        }
    }

    /// Stable digest of the conversation for ids that must not carry the
    /// chat id.
    pub fn digest(&self) -> String {
        framed_digest(
            CONVERSATION_KEY_DOMAIN,
            &[
                self.account_id.as_str(),
                self.chat_id.as_str(),
                self.thread_id.as_deref().unwrap_or(""),
            ],
        )
    }
}

/// Conversation workflow id: `{universe}/chat-{provider}-{digest}`.
pub fn conversation_workflow_id(
    universe_id: Uuid,
    provider: ChannelProvider,
    conversation: &ConversationRef,
) -> String {
    let mut digest = conversation.digest();
    digest.truncate(48);
    format!("{universe_id}/chat-{provider}-{digest}")
}

/// Task queue the connector host serves for one account:
/// `lightspeed-connector-{provider}-{24 hex}` derived from the universe and
/// account id.
pub fn connector_task_queue(
    universe_id: Uuid,
    provider: ChannelProvider,
    account_id: &ChannelAccountId,
) -> String {
    let universe = universe_id.hyphenated().to_string();
    let mut digest = framed_digest(
        DELIVERY_QUEUE_DOMAIN,
        &[universe.as_str(), provider.as_str(), account_id.as_str()],
    );
    digest.truncate(24);
    format!("lightspeed-connector-{provider}-{digest}")
}

/// Pairing row key: derived from account and chat, never message data.
pub fn pairing_key(account_id: &ChannelAccountId, chat_id: &str) -> String {
    let mut digest = framed_digest(PAIRING_KEY_DOMAIN, &[account_id.as_str(), chat_id]);
    digest.truncate(48);
    format!("pair-{digest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(thread: Option<&str>) -> ConversationRef {
        ConversationRef {
            account_id: ChannelAccountId::new("tg-main"),
            chat_id: "12345".to_owned(),
            thread_id: thread.map(str::to_owned),
        }
    }

    #[test]
    fn workflow_ids_hide_chat_ids_and_split() {
        let universe = Uuid::parse_str("6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f").unwrap();
        let id = conversation_workflow_id(universe, ChannelProvider::Telegram, &conversation(None));
        assert!(id.starts_with("6f3a1a52-58c1-4f0e-9c2d-1a2b3c4d5e6f/chat-telegram-"));
        assert!(!id.contains("12345"));
        assert_ne!(
            id,
            conversation_workflow_id(
                universe,
                ChannelProvider::Telegram,
                &conversation(Some("7"))
            )
        );
    }

    #[test]
    fn keys_and_queues_are_deterministic() {
        assert_eq!(conversation(None).key(), "tg-main/12345");
        assert_eq!(conversation(Some("7")).key(), "tg-main/12345/7");
        let universe = Uuid::nil();
        let queue = connector_task_queue(
            universe,
            ChannelProvider::Whatsapp,
            &ChannelAccountId::new("wa"),
        );
        assert!(queue.starts_with("lightspeed-connector-whatsapp-"));
        assert_eq!(queue.len(), "lightspeed-connector-whatsapp-".len() + 24);
        assert_eq!(
            pairing_key(&ChannelAccountId::new("tg-main"), "12345"),
            pairing_key(&ChannelAccountId::new("tg-main"), "12345")
        );
    }
}
