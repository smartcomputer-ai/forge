//! In-memory channel registry for tests: the account and pairing stores of
//! [`crate::records`] over `BTreeMap`s behind one `RwLock`. The semantics
//! pinned by the tests at the bottom are the contract the PostgreSQL
//! adapter in `store-pg` is checked against.

use std::{
    collections::BTreeMap,
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use api::{ChannelAccountDocument, ChannelAccountId, ChannelProvider};
use async_trait::async_trait;

use crate::{
    ChannelError,
    records::{
        ChannelAccountRecord, ChannelAccountStore, ChannelPairingFilter, ChannelPairingRecord,
        ChannelPairingStore, validate_account_document,
    },
};

#[derive(Default)]
struct State {
    accounts: BTreeMap<ChannelAccountId, ChannelAccountRecord>,
    pairings: BTreeMap<String, ChannelPairingRecord>,
}

pub struct InMemoryChannelStore {
    state: RwLock<State>,
}

impl Default for InMemoryChannelStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryChannelStore {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(State::default()),
        }
    }

    fn read_state(&self) -> Result<RwLockReadGuard<'_, State>, ChannelError> {
        self.state
            .read()
            .map_err(|_| ChannelError::store("channel store read lock poisoned"))
    }

    fn write_state(&self) -> Result<RwLockWriteGuard<'_, State>, ChannelError> {
        self.state
            .write()
            .map_err(|_| ChannelError::store("channel store write lock poisoned"))
    }
}

fn account_not_found(account_id: &ChannelAccountId) -> ChannelError {
    ChannelError::AccountNotFound {
        account_id: account_id.clone(),
    }
}

#[async_trait]
impl ChannelAccountStore for InMemoryChannelStore {
    async fn create_channel_account(
        &self,
        account_id: ChannelAccountId,
        document: ChannelAccountDocument,
        now_ms: i64,
    ) -> Result<ChannelAccountRecord, ChannelError> {
        validate_account_document(&document)?;
        let mut state = self.write_state()?;
        if state.accounts.contains_key(&account_id) {
            return Err(ChannelError::AccountAlreadyExists { account_id });
        }
        let record = ChannelAccountRecord {
            account_id: account_id.clone(),
            revision: 1,
            document,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        state.accounts.insert(account_id, record.clone());
        Ok(record)
    }

    async fn put_channel_account(
        &self,
        account_id: ChannelAccountId,
        document: ChannelAccountDocument,
        expected_revision: Option<u64>,
        now_ms: i64,
    ) -> Result<ChannelAccountRecord, ChannelError> {
        validate_account_document(&document)?;
        let mut state = self.write_state()?;
        let record = match state.accounts.get(&account_id) {
            Some(existing) => {
                if let Some(expected) = expected_revision
                    && expected != existing.revision
                {
                    return Err(ChannelError::AccountRevisionConflict {
                        account_id,
                        expected,
                        actual: existing.revision,
                    });
                }
                ChannelAccountRecord {
                    account_id: account_id.clone(),
                    revision: existing.revision + 1,
                    document,
                    created_at_ms: existing.created_at_ms,
                    updated_at_ms: now_ms,
                }
            }
            None => ChannelAccountRecord {
                account_id: account_id.clone(),
                revision: 1,
                document,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            },
        };
        state.accounts.insert(account_id, record.clone());
        Ok(record)
    }

    async fn read_channel_account(
        &self,
        account_id: &ChannelAccountId,
    ) -> Result<ChannelAccountRecord, ChannelError> {
        self.read_state()?
            .accounts
            .get(account_id)
            .cloned()
            .ok_or_else(|| account_not_found(account_id))
    }

    async fn list_channel_accounts(
        &self,
        provider: Option<ChannelProvider>,
    ) -> Result<Vec<ChannelAccountRecord>, ChannelError> {
        Ok(self
            .read_state()?
            .accounts
            .values()
            .filter(|record| provider.is_none_or(|provider| record.provider() == provider))
            .cloned()
            .collect())
    }

    async fn delete_channel_account(
        &self,
        account_id: &ChannelAccountId,
    ) -> Result<ChannelAccountRecord, ChannelError> {
        let mut state = self.write_state()?;
        let record = state
            .accounts
            .remove(account_id)
            .ok_or_else(|| account_not_found(account_id))?;
        state
            .pairings
            .retain(|_, pairing| &pairing.account_id != account_id);
        Ok(record)
    }
}

#[async_trait]
impl ChannelPairingStore for InMemoryChannelStore {
    async fn upsert_channel_pairing(
        &self,
        record: ChannelPairingRecord,
    ) -> Result<ChannelPairingRecord, ChannelError> {
        let mut state = self.write_state()?;
        if !state.accounts.contains_key(&record.account_id) {
            return Err(account_not_found(&record.account_id));
        }
        state
            .pairings
            .insert(record.pairing_key.clone(), record.clone());
        Ok(record)
    }

    async fn read_channel_pairing(
        &self,
        pairing_key: &str,
    ) -> Result<Option<ChannelPairingRecord>, ChannelError> {
        Ok(self.read_state()?.pairings.get(pairing_key).cloned())
    }

    async fn list_channel_pairings(
        &self,
        filter: ChannelPairingFilter,
    ) -> Result<Vec<ChannelPairingRecord>, ChannelError> {
        let state = self.read_state()?;
        let mut pairings: Vec<&ChannelPairingRecord> = state
            .pairings
            .values()
            .filter(|record| {
                filter
                    .account_id
                    .as_ref()
                    .is_none_or(|account_id| &record.account_id == account_id)
                    && filter
                        .bot_id
                        .as_ref()
                        .is_none_or(|bot_id| &record.bot_id == bot_id)
                    && filter
                        .trigger_id
                        .as_ref()
                        .is_none_or(|trigger_id| &record.trigger_id == trigger_id)
                    && filter
                        .chat_id
                        .as_ref()
                        .is_none_or(|chat_id| &record.chat_id == chat_id)
            })
            .collect();
        // Newest first; the key breaks ties so a page is deterministic.
        pairings.sort_by(|a, b| {
            b.paired_at_ms
                .cmp(&a.paired_at_ms)
                .then_with(|| a.pairing_key.cmp(&b.pairing_key))
        });
        Ok(pairings.into_iter().cloned().collect())
    }

    async fn delete_channel_pairing(
        &self,
        pairing_key: &str,
    ) -> Result<ChannelPairingRecord, ChannelError> {
        self.write_state()?
            .pairings
            .remove(pairing_key)
            .ok_or_else(|| ChannelError::PairingNotFound {
                pairing_key: pairing_key.to_owned(),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };

    use api::{BotId, BotTriggerId, ChannelAccountSettings};

    use super::*;

    /// The store never suspends, so a no-op waker drives it to completion
    /// without an async runtime in this crate's dependencies.
    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
        }
    }

    const T0: i64 = 1_700_000_000_000;

    fn account(value: &str) -> ChannelAccountId {
        ChannelAccountId::new(value)
    }

    fn document(provider: ChannelProvider, provider_account_id: &str) -> ChannelAccountDocument {
        ChannelAccountDocument {
            provider,
            provider_account_id: provider_account_id.to_owned(),
            display_name: format!("{provider} {provider_account_id}"),
            credential_grant_id: None,
            settings: ChannelAccountSettings::default(),
            enabled: true,
        }
    }

    fn pairing(
        key: &str,
        account_id: &str,
        trigger_id: &str,
        chat_id: &str,
        paired_at_ms: i64,
    ) -> ChannelPairingRecord {
        ChannelPairingRecord {
            pairing_key: key.to_owned(),
            bot_id: BotId::new("triage"),
            trigger_id: BotTriggerId::new(trigger_id),
            account_id: account(account_id),
            chat_id: chat_id.to_owned(),
            paired_at_ms,
        }
    }

    fn store_with(accounts: &[(&str, ChannelProvider)]) -> InMemoryChannelStore {
        let store = InMemoryChannelStore::new();
        for (name, provider) in accounts {
            block_on(store.create_channel_account(account(name), document(*provider, name), T0))
                .unwrap();
        }
        store
    }

    fn keys(records: &[ChannelPairingRecord]) -> Vec<&str> {
        records
            .iter()
            .map(|record| record.pairing_key.as_str())
            .collect()
    }

    // ── Accounts ────────────────────────────────────────────────────────

    #[test]
    fn create_account_starts_at_revision_one_and_refuses_duplicates() {
        let store = InMemoryChannelStore::new();
        let record = block_on(store.create_channel_account(
            account("tg-main"),
            document(ChannelProvider::Telegram, "@triage_bot"),
            T0,
        ))
        .unwrap();
        assert_eq!(record.revision, 1);
        assert_eq!(record.provider(), ChannelProvider::Telegram);
        assert!(record.enabled());
        assert_eq!(record.created_at_ms, T0);
        assert_eq!(record.updated_at_ms, T0);
        assert_eq!(
            block_on(store.read_channel_account(&account("tg-main"))).unwrap(),
            record
        );

        let error = block_on(store.create_channel_account(
            account("tg-main"),
            document(ChannelProvider::Whatsapp, "+15550100"),
            T0 + 1,
        ))
        .unwrap_err();
        assert_eq!(
            error,
            ChannelError::AccountAlreadyExists {
                account_id: account("tg-main")
            }
        );
        assert_eq!(
            block_on(store.read_channel_account(&account("missing"))).unwrap_err(),
            ChannelError::AccountNotFound {
                account_id: account("missing")
            }
        );
    }

    #[test]
    fn create_and_put_validate_the_document() {
        let store = store_with(&[("tg-main", ChannelProvider::Telegram)]);
        let mut invalid = document(ChannelProvider::Telegram, "@triage_bot");
        invalid.display_name = "   ".to_owned();
        assert!(matches!(
            block_on(store.create_channel_account(account("other"), invalid.clone(), T0))
                .unwrap_err(),
            ChannelError::InvalidInput { .. }
        ));
        assert!(block_on(store.read_channel_account(&account("other"))).is_err());
        assert!(matches!(
            block_on(store.put_channel_account(account("tg-main"), invalid, None, T0 + 1))
                .unwrap_err(),
            ChannelError::InvalidInput { .. }
        ));
        assert_eq!(
            block_on(store.read_channel_account(&account("tg-main")))
                .unwrap()
                .revision,
            1
        );
    }

    #[test]
    fn put_account_creates_when_absent_and_replaces_with_revision_check() {
        let store = InMemoryChannelStore::new();
        // Absent: created at revision 1, whatever the expectation.
        let created = block_on(store.put_channel_account(
            account("tg-main"),
            document(ChannelProvider::Telegram, "@triage_bot"),
            Some(7),
            T0,
        ))
        .unwrap();
        assert_eq!(created.revision, 1);
        assert_eq!(created.created_at_ms, T0);

        let mut next = document(ChannelProvider::Telegram, "@triage_bot");
        next.credential_grant_id = Some("grant-1".to_owned());
        next.settings.print_qr = Some(true);
        let replaced =
            block_on(store.put_channel_account(account("tg-main"), next.clone(), Some(1), T0 + 10))
                .unwrap();
        assert_eq!(replaced.revision, 2);
        assert_eq!(replaced.document, next);
        assert_eq!(replaced.created_at_ms, T0);
        assert_eq!(replaced.updated_at_ms, T0 + 10);

        let error =
            block_on(store.put_channel_account(account("tg-main"), next.clone(), Some(1), T0 + 20))
                .unwrap_err();
        assert_eq!(
            error,
            ChannelError::AccountRevisionConflict {
                account_id: account("tg-main"),
                expected: 1,
                actual: 2,
            }
        );
        assert_eq!(
            block_on(store.read_channel_account(&account("tg-main"))).unwrap(),
            replaced
        );

        // No expectation: unconditional replace.
        let unconditional =
            block_on(store.put_channel_account(account("tg-main"), next, None, T0 + 30)).unwrap();
        assert_eq!(unconditional.revision, 3);
        assert_eq!(unconditional.updated_at_ms, T0 + 30);
    }

    #[test]
    fn list_accounts_orders_by_id_and_filters_by_provider() {
        let store = store_with(&[
            ("wa-shop", ChannelProvider::Whatsapp),
            ("tg-main", ChannelProvider::Telegram),
            ("tg-alerts", ChannelProvider::Telegram),
        ]);
        let names = |provider| -> Vec<String> {
            block_on(store.list_channel_accounts(provider))
                .unwrap()
                .into_iter()
                .map(|record| record.account_id.to_string())
                .collect()
        };
        assert_eq!(names(None), vec!["tg-alerts", "tg-main", "wa-shop"]);
        assert_eq!(
            names(Some(ChannelProvider::Telegram)),
            vec!["tg-alerts", "tg-main"]
        );
        assert_eq!(names(Some(ChannelProvider::Whatsapp)), vec!["wa-shop"]);
        assert!(
            block_on(InMemoryChannelStore::new().list_channel_accounts(None))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn delete_account_removes_its_pairings_only() {
        let store = store_with(&[
            ("tg-main", ChannelProvider::Telegram),
            ("wa-shop", ChannelProvider::Whatsapp),
        ]);
        block_on(store.upsert_channel_pairing(pairing("p-1", "tg-main", "chat", "c-1", T0)))
            .unwrap();
        block_on(store.upsert_channel_pairing(pairing("p-2", "tg-main", "chat", "c-2", T0)))
            .unwrap();
        block_on(store.upsert_channel_pairing(pairing("p-3", "wa-shop", "chat", "c-3", T0)))
            .unwrap();

        let removed = block_on(store.delete_channel_account(&account("tg-main"))).unwrap();
        assert_eq!(removed.account_id, account("tg-main"));
        assert!(matches!(
            block_on(store.read_channel_account(&account("tg-main"))).unwrap_err(),
            ChannelError::AccountNotFound { .. }
        ));
        assert_eq!(block_on(store.read_channel_pairing("p-1")).unwrap(), None);
        assert_eq!(block_on(store.read_channel_pairing("p-2")).unwrap(), None);
        assert!(
            block_on(store.read_channel_pairing("p-3"))
                .unwrap()
                .is_some()
        );
        assert_eq!(
            keys(&block_on(store.list_channel_pairings(ChannelPairingFilter::default())).unwrap()),
            vec!["p-3"]
        );
        assert_eq!(
            block_on(store.delete_channel_account(&account("tg-main"))).unwrap_err(),
            ChannelError::AccountNotFound {
                account_id: account("tg-main")
            }
        );
    }

    // ── Pairings ────────────────────────────────────────────────────────

    #[test]
    fn upsert_pairing_replaces_by_key_and_needs_the_account() {
        let store = store_with(&[("tg-main", ChannelProvider::Telegram)]);
        let first = pairing("p-1", "tg-main", "support", "c-1", T0);
        assert_eq!(
            block_on(store.upsert_channel_pairing(first.clone())).unwrap(),
            first
        );
        assert_eq!(
            block_on(store.read_channel_pairing("p-1")).unwrap(),
            Some(first)
        );
        assert_eq!(block_on(store.read_channel_pairing("p-9")).unwrap(), None);

        // A re-pair moves the chat to another trigger under the same key.
        let moved = pairing("p-1", "tg-main", "sales", "c-1", T0 + 5);
        block_on(store.upsert_channel_pairing(moved.clone())).unwrap();
        assert_eq!(
            block_on(store.read_channel_pairing("p-1")).unwrap(),
            Some(moved)
        );
        assert_eq!(
            block_on(store.list_channel_pairings(ChannelPairingFilter::default()))
                .unwrap()
                .len(),
            1
        );

        assert_eq!(
            block_on(store.upsert_channel_pairing(pairing("p-2", "missing", "support", "c-2", T0)))
                .unwrap_err(),
            ChannelError::AccountNotFound {
                account_id: account("missing")
            }
        );
        assert_eq!(block_on(store.read_channel_pairing("p-2")).unwrap(), None);
    }

    #[test]
    fn list_pairings_newest_first_honoring_every_filter() {
        let store = store_with(&[
            ("tg-main", ChannelProvider::Telegram),
            ("wa-shop", ChannelProvider::Whatsapp),
        ]);
        let mut other_bot = pairing("p-4", "tg-main", "support", "c-1", T0 + 40);
        other_bot.bot_id = BotId::new("other");
        for record in [
            pairing("p-1", "tg-main", "support", "c-1", T0 + 10),
            pairing("p-2", "tg-main", "sales", "c-2", T0 + 30),
            pairing("p-3", "wa-shop", "support", "c-1", T0 + 20),
            other_bot,
            pairing("p-5", "tg-main", "support", "c-5", T0 + 30),
        ] {
            block_on(store.upsert_channel_pairing(record)).unwrap();
        }
        let list = |filter| keys_owned(block_on(store.list_channel_pairings(filter)).unwrap());
        fn keys_owned(records: Vec<ChannelPairingRecord>) -> Vec<String> {
            records
                .into_iter()
                .map(|record| record.pairing_key)
                .collect()
        }

        assert_eq!(
            list(ChannelPairingFilter::default()),
            vec!["p-4", "p-2", "p-5", "p-3", "p-1"],
            "newest first, key breaks the tie"
        );
        assert_eq!(
            list(ChannelPairingFilter {
                account_id: Some(account("tg-main")),
                ..Default::default()
            }),
            vec!["p-4", "p-2", "p-5", "p-1"]
        );
        assert_eq!(
            list(ChannelPairingFilter {
                bot_id: Some(BotId::new("other")),
                ..Default::default()
            }),
            vec!["p-4"]
        );
        assert_eq!(
            list(ChannelPairingFilter {
                trigger_id: Some(BotTriggerId::new("support")),
                ..Default::default()
            }),
            vec!["p-4", "p-5", "p-3", "p-1"]
        );
        assert_eq!(
            list(ChannelPairingFilter {
                chat_id: Some("c-1".to_owned()),
                ..Default::default()
            }),
            vec!["p-4", "p-3", "p-1"]
        );
        assert_eq!(
            list(ChannelPairingFilter {
                account_id: Some(account("tg-main")),
                bot_id: Some(BotId::new("triage")),
                trigger_id: Some(BotTriggerId::new("support")),
                chat_id: Some("c-1".to_owned()),
            }),
            vec!["p-1"]
        );
        assert!(
            list(ChannelPairingFilter {
                account_id: Some(account("wa-shop")),
                chat_id: Some("c-2".to_owned()),
                ..Default::default()
            })
            .is_empty()
        );
    }

    #[test]
    fn delete_pairing_returns_the_row_then_not_found() {
        let store = store_with(&[("tg-main", ChannelProvider::Telegram)]);
        let record = pairing("p-1", "tg-main", "support", "c-1", T0);
        block_on(store.upsert_channel_pairing(record.clone())).unwrap();
        assert_eq!(
            block_on(store.delete_channel_pairing("p-1")).unwrap(),
            record
        );
        assert_eq!(block_on(store.read_channel_pairing("p-1")).unwrap(), None);
        assert_eq!(
            block_on(store.delete_channel_pairing("p-1")).unwrap_err(),
            ChannelError::PairingNotFound {
                pairing_key: "p-1".to_owned()
            }
        );
        // The account is untouched.
        assert!(block_on(store.read_channel_account(&account("tg-main"))).is_ok());
    }
}
