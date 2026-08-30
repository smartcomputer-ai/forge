//! Channel account and pairing records with their store contracts.

use api::{
    BotId, BotTriggerId, ChannelAccountDocument, ChannelAccountId, ChannelAccountView,
    ChannelPairedVia, ChannelPairingView, ChannelProvider,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::ChannelError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelAccountRecord {
    pub account_id: ChannelAccountId,
    pub revision: u64,
    pub document: ChannelAccountDocument,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl ChannelAccountRecord {
    pub fn provider(&self) -> &ChannelProvider {
        &self.document.provider
    }

    pub fn enabled(&self) -> bool {
        self.document.enabled
    }

    pub fn view(&self) -> ChannelAccountView {
        ChannelAccountView {
            account_id: self.account_id.clone(),
            revision: self.revision,
            document: self.document.clone(),
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        }
    }
}

pub const MAX_ACCOUNT_DISPLAY_NAME_LEN: usize = 200;
pub const MAX_PROVIDER_ACCOUNT_ID_LEN: usize = 200;

pub fn validate_account_document(document: &ChannelAccountDocument) -> Result<(), ChannelError> {
    if document.provider_account_id.trim().is_empty()
        || document.provider_account_id.len() > MAX_PROVIDER_ACCOUNT_ID_LEN
    {
        return Err(ChannelError::invalid(format!(
            "providerAccountId must be 1..={MAX_PROVIDER_ACCOUNT_ID_LEN} bytes"
        )));
    }
    if document.display_name.trim().is_empty()
        || document.display_name.len() > MAX_ACCOUNT_DISPLAY_NAME_LEN
    {
        return Err(ChannelError::invalid(format!(
            "displayName must be 1..={MAX_ACCOUNT_DISPLAY_NAME_LEN} bytes"
        )));
    }
    if let Some(grant_id) = &document.credential_grant_id
        && (grant_id.trim().is_empty() || grant_id.len() > 300)
    {
        return Err(ChannelError::invalid(
            "credentialGrantId must be 1..=300 bytes",
        ));
    }
    Ok(())
}

#[async_trait]
pub trait ChannelAccountStore: Send + Sync {
    async fn create_channel_account(
        &self,
        account_id: ChannelAccountId,
        document: ChannelAccountDocument,
        now_ms: i64,
    ) -> Result<ChannelAccountRecord, ChannelError>;

    /// Create when absent, otherwise replace and bump the revision;
    /// `expected_revision` is checked only when the account exists.
    async fn put_channel_account(
        &self,
        account_id: ChannelAccountId,
        document: ChannelAccountDocument,
        expected_revision: Option<u64>,
        now_ms: i64,
    ) -> Result<ChannelAccountRecord, ChannelError>;

    async fn read_channel_account(
        &self,
        account_id: &ChannelAccountId,
    ) -> Result<ChannelAccountRecord, ChannelError>;

    /// Ordered by account id.
    async fn list_channel_accounts(
        &self,
        provider: Option<ChannelProvider>,
    ) -> Result<Vec<ChannelAccountRecord>, ChannelError>;

    async fn delete_channel_account(
        &self,
        account_id: &ChannelAccountId,
    ) -> Result<ChannelAccountRecord, ChannelError>;
}

/// A conversation's route, keyed by `(account_id, chat_id)`: claimed by an
/// open trigger's first contact or paired by code, and owned by the paired
/// trigger while the row exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelPairingRecord {
    pub account_id: ChannelAccountId,
    pub chat_id: String,
    pub bot_id: BotId,
    pub trigger_id: BotTriggerId,
    pub paired_via: ChannelPairedVia,
    pub paired_at_ms: i64,
}

impl ChannelPairingRecord {
    pub fn view(&self) -> ChannelPairingView {
        ChannelPairingView {
            paired_via: self.paired_via,
            bot_id: self.bot_id.clone(),
            trigger_id: self.trigger_id.clone(),
            account_id: self.account_id.clone(),
            chat_id: self.chat_id.clone(),
            paired_at_ms: self.paired_at_ms,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChannelPairingFilter {
    pub account_id: Option<ChannelAccountId>,
    pub bot_id: Option<BotId>,
    pub trigger_id: Option<BotTriggerId>,
    pub chat_id: Option<String>,
}

#[async_trait]
pub trait ChannelPairingStore: Send + Sync {
    /// Insert or replace the chat's pairing (a re-pair moves the chat to
    /// another trigger).
    async fn upsert_channel_pairing(
        &self,
        record: ChannelPairingRecord,
    ) -> Result<ChannelPairingRecord, ChannelError>;

    async fn read_channel_pairing(
        &self,
        account_id: &ChannelAccountId,
        chat_id: &str,
    ) -> Result<Option<ChannelPairingRecord>, ChannelError>;

    /// Ordered by paired-at time, newest first.
    async fn list_channel_pairings(
        &self,
        filter: ChannelPairingFilter,
    ) -> Result<Vec<ChannelPairingRecord>, ChannelError>;

    async fn delete_channel_pairing(
        &self,
        account_id: &ChannelAccountId,
        chat_id: &str,
    ) -> Result<ChannelPairingRecord, ChannelError>;
}

pub trait ChannelRegistryStore: ChannelAccountStore + ChannelPairingStore {}

impl<T: ChannelAccountStore + ChannelPairingStore> ChannelRegistryStore for T {}
