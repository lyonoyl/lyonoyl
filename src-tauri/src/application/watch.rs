//! Waiting for one account's server to say that mail arrived.
//!
//! The counterpart of a pass, never a replacement for one: a watch answers only that something
//! happened and whether mail arrived with it, and what follows it is the same pass a timer would
//! have run. Kept apart from
//! `SyncAccount` because the two fail differently — a server that cannot be watched is not a
//! server that cannot be read, and only the first of those is ordinary.

use std::time::Duration;

use crate::application::error::UseCaseError;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential::Credential;
use crate::ports::credential_store::{CredentialPurpose, CredentialRef, CredentialStore};
use crate::ports::mail_index::MailIndex;
use crate::ports::mail_provider::{MailWatch, ProviderFactory, WatchEnded, Watermark};

/// How long one watch is asked to last.
///
/// The account is looked up again between watches, and that lookup is what ends the watch of an
/// account removed from the device — so this is also how long such a connection can outlive it.
/// An adapter shortens it where its protocol demands: IMAP may not hold one idle for longer than
/// half an hour.
const A_WATCH_LASTS: Duration = Duration::from_secs(25 * 60);

/// Holds one connection open until an account's server reports a change in its inbox.
pub struct WatchAccount<R, C, F, I>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    I: MailIndex,
{
    accounts: R,
    credentials: C,
    providers: F,
    index: I,
}

impl<R, C, F, I> WatchAccount<R, C, F, I>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    I: MailIndex,
{
    pub fn new(accounts: R, credentials: C, providers: F, index: I) -> Self {
        Self {
            accounts,
            credentials,
            providers,
            index,
        }
    }

    /// `account_id` is the identifier the rest of the application uses; `number` is what the index
    /// knows the account by. `since` is where the account's last watch ended, handed to the
    /// provider as it came.
    pub fn execute(
        &self,
        account_id: &str,
        number: i64,
        since: Option<&Watermark>,
    ) -> Result<WatchEnded, UseCaseError> {
        let account = self
            .accounts
            .find(account_id)?
            .ok_or(UseCaseError::UnknownAccount)?;

        // The inbox and nothing else. A watch holds a connection and a thread for as long as it
        // lasts, and the mailbox worth that is the one mail arrives in.
        let inbox = self
            .index
            .inbox_of(number)?
            .ok_or(UseCaseError::MailboxesNotKnownYet)?;

        // Asked for after the inbox rather than before it: an account whose mailboxes have not
        // been read yet has nothing to watch, and the store would be opened for nothing.
        let password = self.credentials.read(&CredentialRef::new(
            account_id.to_owned(),
            CredentialPurpose::ImapPassword,
        ))?;

        self.providers
            .watch(&account, &Credential::password(password))?
            .watch(&inbox, since, A_WATCH_LASTS)
            .map_err(UseCaseError::from)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use secrecy::SecretString;

    use super::*;
    use crate::ports::account_repository::AccountRepositoryError;
    use crate::ports::credential_store::CredentialError;
    use crate::ports::mail_index::{
        AppliedSummary, FolderRecord, FolderSyncState, MailIndexError, UidScope,
    };
    use crate::ports::mail_provider::{MailProviderError, MailSync, MailboxDiscovery, Watched};
    use lyonoyl_domain::account::{Encryption, MailAccount, MailAccountSummary, ServerSettings};
    use lyonoyl_domain::message::{FlagScope, FolderId, Uid, UidValidity};
    use lyonoyl_domain::sync::{FolderDelta, SyncCursor};

    /// The account the index knows as number four.
    const NUMBER: i64 = 4;

    fn account() -> MailAccount {
        MailAccount::new(
            MailAccountSummary::new("acc-1", "user@example.com", "User", "imap")
                .expect("valid summary"),
            ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid server"),
            "user@example.com",
        )
        .expect("valid account")
    }

    #[derive(Default)]
    struct FakeAccounts {
        held: Option<MailAccount>,
        unavailable: bool,
    }

    impl FakeAccounts {
        fn holding_the_account() -> Self {
            Self {
                held: Some(account()),
                unavailable: false,
            }
        }
    }

    impl AccountRepository for FakeAccounts {
        fn find(&self, _account_id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
            if self.unavailable {
                return Err(AccountRepositoryError::Unavailable);
            }

            Ok(self.held.clone())
        }

        fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
            unreachable!("nothing here asks who can send")
        }

        fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
            unreachable!("watching one account lists none")
        }

        fn find_by_email(&self, _email: &str) -> Result<Option<String>, AccountRepositoryError> {
            unreachable!("watching an account does not look it up by address")
        }

        fn save(&self, _account: &MailAccount) -> Result<(), AccountRepositoryError> {
            unreachable!("watching an account stores nothing")
        }

        fn delete(&self, _account_id: &str) -> Result<(), AccountRepositoryError> {
            unreachable!("watching an account removes nothing")
        }

        fn set_collapsed(
            &self,
            _account_id: &str,
            _collapsed: bool,
        ) -> Result<(), AccountRepositoryError> {
            unreachable!("watching an account does not touch the sidebar")
        }

        fn number_of(&self, _: &str) -> Result<Option<i64>, AccountRepositoryError> {
            unreachable!("this fake is not asked which file the mail is in")
        }
    }

    /// A credential store that answers from a script and remembers whether it was asked at all.
    struct FakeCredentials {
        stored: bool,
        asked: Mutex<Vec<CredentialRef>>,
    }

    impl FakeCredentials {
        fn holding_a_password() -> Self {
            Self {
                stored: true,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn holding_nothing() -> Self {
            Self {
                stored: false,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn times_asked(&self) -> usize {
            self.asked.lock().expect("test lock").len()
        }
    }

    impl CredentialStore for FakeCredentials {
        fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
            self.asked
                .lock()
                .expect("test lock")
                .push(reference.clone());

            if self.stored {
                return Ok(SecretString::from("correct horse".to_owned()));
            }

            Err(CredentialError::NotFound)
        }

        fn save(
            &self,
            _reference: &CredentialRef,
            _secret: &SecretString,
        ) -> Result<(), CredentialError> {
            unreachable!("watching an account stores no secret")
        }

        fn delete(&self, _reference: &CredentialRef) -> Result<(), CredentialError> {
            unreachable!("watching an account removes no secret")
        }
    }

    /// An index that knows an account's inbox, or does not know it yet.
    #[derive(Default)]
    struct FakeIndex {
        inbox: Option<String>,
        asked_about: Mutex<Vec<i64>>,
    }

    impl FakeIndex {
        fn holding(inbox: &str) -> Self {
            Self {
                inbox: Some(inbox.to_owned()),
                asked_about: Mutex::new(Vec::new()),
            }
        }
    }

    impl MailIndex for FakeIndex {
        fn not_read_whole(
            &self,
            _folder_id: FolderId,
            _at_most: u32,
        ) -> Result<Vec<(lyonoyl_domain::message::MessageId, Uid)>, MailIndexError> {
            Ok(Vec::new())
        }
        fn inbox_of(&self, account: i64) -> Result<Option<String>, MailIndexError> {
            self.asked_about.lock().expect("test lock").push(account);

            Ok(self.inbox.clone())
        }

        fn record_folders(
            &self,
            _account: i64,
            _folders: &[FolderRecord],
        ) -> Result<Vec<(String, FolderId)>, MailIndexError> {
            unreachable!("a watch records no mailboxes")
        }

        fn folder_state(&self, _folder_id: FolderId) -> Result<FolderSyncState, MailIndexError> {
            unreachable!("a watch reads no folder state")
        }

        fn known_uids(
            &self,
            _folder_id: FolderId,
            _scope: &UidScope,
        ) -> Result<Vec<Uid>, MailIndexError> {
            unreachable!("a watch reads no messages")
        }

        fn placement_count(&self, _folder_id: FolderId) -> Result<usize, MailIndexError> {
            unreachable!("a watch counts nothing")
        }

        fn record_window_floor(
            &self,
            _folder_id: FolderId,
            _floor: Uid,
            _complete: bool,
        ) -> Result<(), MailIndexError> {
            unreachable!("a watch reads no window")
        }

        fn apply(
            &self,
            _delta: &FolderDelta,
            _deletions: &[Uid],
            _cursor: &SyncCursor,
            _flag_scope: FlagScope,
        ) -> Result<AppliedSummary, MailIndexError> {
            unreachable!("a watch applies nothing")
        }

        fn reset_folder(
            &self,
            _folder_id: FolderId,
            _uid_validity: UidValidity,
        ) -> Result<(), MailIndexError> {
            unreachable!("a watch resets nothing")
        }

        fn forget_it_stopped_short(&self, _folder_id: FolderId) -> Result<(), MailIndexError> {
            unreachable!("a watch reads no folder")
        }

        fn refused_for_now(
            &self,
            _folder_id: FolderId,
            _reading: crate::ports::mail_index::Reading,
            _refused: bool,
        ) -> Result<(), MailIndexError> {
            unreachable!("a watch reads no folder")
        }

        fn holds(
            &self,
            _folder_id: FolderId,
            _message: lyonoyl_domain::message::MessageId,
        ) -> Result<bool, MailIndexError> {
            unreachable!("a watch asks after no copy")
        }

        fn unplaced(
            &self,
            _account: i64,
        ) -> Result<Vec<lyonoyl_domain::message::MessageId>, MailIndexError> {
            unreachable!("a watch asks after no message")
        }

        fn forget_the_unplaced(
            &self,
            _account: i64,
            _named: &[lyonoyl_domain::message::MessageId],
        ) -> Result<(), MailIndexError> {
            unreachable!("a watch forgets nothing")
        }
    }

    /// What the provider was asked to watch, shared so the test can read it back.
    type Asked = Arc<Mutex<Vec<(String, Option<Watermark>, Duration)>>>;

    struct FakeWatch {
        answer: Result<WatchEnded, MailProviderError>,
        asked: Asked,
    }

    impl MailWatch for FakeWatch {
        fn watch(
            &self,
            mailbox: &str,
            since: Option<&Watermark>,
            at_most: Duration,
        ) -> Result<WatchEnded, MailProviderError> {
            self.asked.lock().expect("test lock").push((
                mailbox.to_owned(),
                since.cloned(),
                at_most,
            ));

            self.answer.clone()
        }
    }

    struct FakeFactory {
        answer: Result<WatchEnded, MailProviderError>,
        asked: Asked,
    }

    impl FakeFactory {
        fn answering(answer: Result<Watched, MailProviderError>) -> Self {
            Self::ending(answer.map(WatchEnded::from))
        }

        fn ending(answer: Result<WatchEnded, MailProviderError>) -> Self {
            Self {
                answer,
                asked: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl ProviderFactory for FakeFactory {
        fn reading(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn crate::ports::mail_provider::MailFetch>, MailProviderError> {
            unreachable!("a watch reads no message")
        }

        fn watch(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailWatch>, MailProviderError> {
            Ok(Box::new(FakeWatch {
                answer: self.answer.clone(),
                asked: Arc::clone(&self.asked),
            }))
        }

        fn discovery(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
            unreachable!("a watch lists no mailboxes")
        }

        fn sync(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailSync>, MailProviderError> {
            unreachable!("a watch reads no mail")
        }
    }

    #[test]
    fn the_inbox_the_index_holds_is_the_mailbox_watched() {
        let factory = FakeFactory::answering(Ok(Watched::Changed));
        let asked = Arc::clone(&factory.asked);
        let index = FakeIndex::holding("INBOX.Mail");

        let watched = WatchAccount::new(
            FakeAccounts::holding_the_account(),
            FakeCredentials::holding_a_password(),
            factory,
            index,
        )
        .execute("acc-1", NUMBER, None);

        assert_eq!(watched.expect("the watch ends").watched, Watched::Changed);
        let asked = asked.lock().expect("test lock");
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].0, "INBOX.Mail");
    }

    #[test]
    fn a_watch_takes_up_where_the_last_one_ended_and_says_where_it_ended() {
        // Where the last watch ended is the scheduler's to keep and the provider's to read: this
        // hands it over untouched, and hands back where this one ended the same way.
        let factory = FakeFactory::ending(Ok(WatchEnded {
            watched: Watched::Quiet,
            seen: Some(Watermark::new("later")),
        }));
        let asked = Arc::clone(&factory.asked);

        let ended = WatchAccount::new(
            FakeAccounts::holding_the_account(),
            FakeCredentials::holding_a_password(),
            factory,
            FakeIndex::holding("INBOX"),
        )
        .execute("acc-1", NUMBER, Some(&Watermark::new("earlier")))
        .expect("the watch ends");

        assert_eq!(ended.seen, Some(Watermark::new("later")));
        assert_eq!(
            asked.lock().expect("test lock")[0].1,
            Some(Watermark::new("earlier"))
        );
    }

    #[test]
    fn a_watch_is_asked_to_last_no_longer_than_the_ceiling() {
        let factory = FakeFactory::answering(Ok(Watched::Quiet));
        let asked = Arc::clone(&factory.asked);

        let _ = WatchAccount::new(
            FakeAccounts::holding_the_account(),
            FakeCredentials::holding_a_password(),
            factory,
            FakeIndex::holding("INBOX"),
        )
        .execute("acc-1", NUMBER, None);

        assert_eq!(asked.lock().expect("test lock")[0].2, A_WATCH_LASTS);
    }

    #[test]
    fn a_server_that_cannot_push_is_reported_as_itself() {
        let watched = WatchAccount::new(
            FakeAccounts::holding_the_account(),
            FakeCredentials::holding_a_password(),
            FakeFactory::answering(Ok(Watched::CannotPush)),
            FakeIndex::holding("INBOX"),
        )
        .execute("acc-1", NUMBER, None);

        // Anything else and the watcher would keep re-opening a connection to a server that has
        // nothing to say on it.
        assert_eq!(
            watched.expect("the watch ends").watched,
            Watched::CannotPush
        );
    }

    #[test]
    fn an_account_whose_mailboxes_are_not_known_yet_is_not_watched() {
        let credentials = FakeCredentials::holding_a_password();
        let factory = FakeFactory::answering(Ok(Watched::Changed));
        let asked = Arc::clone(&factory.asked);

        let watched = WatchAccount::new(
            FakeAccounts::holding_the_account(),
            &credentials,
            factory,
            FakeIndex::default(),
        )
        .execute("acc-1", NUMBER, None);

        assert!(matches!(watched, Err(UseCaseError::MailboxesNotKnownYet)));
        // No connection, and no dialog from the system store either: the first pass has not run.
        assert!(asked.lock().expect("test lock").is_empty());
        assert_eq!(credentials.times_asked(), 0);
    }

    #[test]
    fn an_account_that_is_not_on_this_device_is_refused() {
        let watched = WatchAccount::new(
            FakeAccounts::default(),
            FakeCredentials::holding_a_password(),
            FakeFactory::answering(Ok(Watched::Changed)),
            FakeIndex::holding("INBOX"),
        )
        .execute("acc-1", NUMBER, None);

        assert!(matches!(watched, Err(UseCaseError::UnknownAccount)));
    }

    #[test]
    fn a_missing_password_is_reported_rather_than_read_as_a_quiet_server() {
        let watched = WatchAccount::new(
            FakeAccounts::holding_the_account(),
            FakeCredentials::holding_nothing(),
            FakeFactory::answering(Ok(Watched::Changed)),
            FakeIndex::holding("INBOX"),
        )
        .execute("acc-1", NUMBER, None);

        assert!(matches!(watched, Err(UseCaseError::CredentialsMissing)));
    }

    #[test]
    fn a_provider_failure_keeps_its_meaning() {
        let watched = WatchAccount::new(
            FakeAccounts::holding_the_account(),
            FakeCredentials::holding_a_password(),
            FakeFactory::answering(Err(MailProviderError::Offline)),
            FakeIndex::holding("INBOX"),
        )
        .execute("acc-1", NUMBER, None);

        // Collapsing this into "nothing happened" would have the watcher reconnect at once, in a
        // loop, to a server that is not there.
        assert!(matches!(watched, Err(UseCaseError::Offline)));
    }
}
