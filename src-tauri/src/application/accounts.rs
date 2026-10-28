use secrecy::SecretString;

use crate::application::error::UseCaseError;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential::Credential;
use crate::ports::credential_store::{
    CredentialError, CredentialPurpose, CredentialRef, CredentialStore,
};
use crate::ports::mail_provider::ProviderFactory;
use lyonoyl_domain::account::MailAccount;

/// Checks that the settings a person typed actually reach their mail.
///
/// Nothing is stored: this is the button next to the form, and a person may press it as often as
/// they like before deciding to add the account.
pub struct TestConnection<F: ProviderFactory> {
    providers: F,
}

impl<F: ProviderFactory> TestConnection<F> {
    pub fn new(providers: F) -> Self {
        Self { providers }
    }

    /// Returns how many mailboxes the server offered, which is what makes the answer convincing:
    /// "connected" is a claim, "found 7 mailboxes" is evidence.
    pub fn execute(
        &self,
        account: &MailAccount,
        password: &SecretString,
    ) -> Result<usize, UseCaseError> {
        let provider = self
            .providers
            .discovery(account, &Credential::password(password.clone()))?;
        Ok(provider.list_mailboxes()?.len())
    }
}

/// Adds an account.
pub struct AddAccount<R: AccountRepository, C: CredentialStore, F: ProviderFactory> {
    accounts: R,
    credentials: C,
    providers: F,
}

impl<R: AccountRepository, C: CredentialStore, F: ProviderFactory> AddAccount<R, C, F> {
    pub fn new(accounts: R, credentials: C, providers: F) -> Self {
        Self {
            accounts,
            credentials,
            providers,
        }
    }

    /// The order is the point of this use case.
    ///
    /// The server is asked first, so a mistyped password never becomes a stored account that fails
    /// quietly afterwards. The secret is stored second. The account row is stored last, and if that
    /// fails the secret is taken back out: a password in the store that belongs to no account is
    /// something a person cannot find and cannot remove.
    pub fn execute(
        &self,
        account: MailAccount,
        password: SecretString,
    ) -> Result<(), UseCaseError> {
        // Two accounts on one address would sync the same mailbox twice, into folders the person
        // cannot tell apart. But an address already here whose password is not is a different
        // situation: someone is supplying it again, and telling them it is already there would
        // leave an account that can never be read.
        let account = match self.existing_with_no_password(account.summary().email())? {
            Some(existing) => account
                .renamed_to(existing)
                // The identifier came out of the store, so it is one the domain already accepted.
                .map_err(|_| UseCaseError::StorageCorrupted)?,
            None => account,
        };

        let provider = self
            .providers
            .discovery(&account, &Credential::password(password.clone()))?;
        provider.list_mailboxes()?;

        let reference = CredentialRef::new(
            account.summary().id().to_owned(),
            CredentialPurpose::ImapPassword,
        );
        self.credentials.save(&reference, &password)?;

        if let Err(error) = self.accounts.save(&account) {
            let _ = self.credentials.delete(&reference);
            return Err(error.into());
        }

        Ok(())
    }

    /// The identifier of an account already stored under this address that has no password.
    ///
    /// Answers `None` when the address is new. Refuses when the address is here *with* a password:
    /// that one really is a duplicate.
    fn existing_with_no_password(&self, email: &str) -> Result<Option<String>, UseCaseError> {
        let Some(existing) = self.accounts.find_by_email(email)? else {
            return Ok(None);
        };

        let reference = CredentialRef::new(existing.clone(), CredentialPurpose::ImapPassword);
        match self.credentials.read(&reference) {
            Ok(_) => Err(UseCaseError::AccountAlreadyAdded),
            // Keeping the identifier matters: it names the account's folders, its mail and its
            // place in the operation log. A new one would orphan all of them.
            Err(CredentialError::NotFound) => Ok(Some(existing)),
            Err(error) => Err(error.into()),
        }
    }
}

/// Changing where an account's mail is fetched from, and the password it is fetched with.
///
/// Its own use case rather than a flag on [`AddAccount`], because the two answer different
/// questions about the same shape: adding refuses an address that is already here, and changing
/// refuses one that is not.
///
/// **This is what was missing rather than broken.** There was no command at all, and the screen
/// that looks like account settings submitted through *add* — which answers `AccountAlreadyAdded`
/// the moment the account has a working password. So a provider that moved a port, or a host typed
/// wrong once, could only be corrected by removing the account, and removing an account takes its
/// mail and everything waiting to be sent with it.
pub struct ChangeAccount<R: AccountRepository, C: CredentialStore, F: ProviderFactory> {
    accounts: R,
    credentials: C,
    providers: F,
}

impl<R: AccountRepository, C: CredentialStore, F: ProviderFactory> ChangeAccount<R, C, F> {
    pub fn new(accounts: R, credentials: C, providers: F) -> Self {
        Self {
            accounts,
            credentials,
            providers,
        }
    }

    /// The order, and why it is not the order [`AddAccount`] uses.
    ///
    /// Adding can undo itself by deleting the secret it wrote, because before it there was nothing.
    /// Changing cannot: the secret it overwrites is the one the account is working with, and losing
    /// it would leave somebody unable to read mail they could read a moment ago. So the old one is
    /// read first and put back if the row will not take the new settings.
    ///
    /// The server is still asked before anything is written. A password the server refuses never
    /// replaces one it accepts.
    pub fn execute(
        &self,
        account_id: &str,
        settings: MailAccount,
        password: SecretString,
    ) -> Result<(), UseCaseError> {
        if self.accounts.find(account_id)?.is_none() {
            return Err(UseCaseError::UnknownAccount);
        }

        // The identifier is what names this account's folders, its mail and its place in the
        // operation log. What the form supplies is settings; the identity is not the form's to
        // change.
        let account = settings
            .renamed_to(account_id.to_owned())
            .map_err(|_| UseCaseError::StorageCorrupted)?;

        let reference = CredentialRef::new(account_id.to_owned(), CredentialPurpose::ImapPassword);
        let before = self.credentials.read(&reference).ok();

        let provider = self
            .providers
            .discovery(&account, &Credential::password(password.clone()))?;
        provider.list_mailboxes()?;

        self.credentials.save(&reference, &password)?;

        if let Err(error) = self.accounts.save(&account) {
            match before {
                Some(old) => {
                    let _ = self.credentials.save(&reference, &old);
                }
                None => {
                    let _ = self.credentials.delete(&reference);
                }
            }
            return Err(error.into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::ports::account_repository::AccountRepositoryError;
    use crate::ports::credential_store::CredentialError;
    use crate::ports::mail_provider::{DiscoveredMailbox, MailProviderError, MailboxDiscovery};
    use lyonoyl_domain::account::{Encryption, MailAccountSummary, ServerSettings};
    use lyonoyl_domain::folder::FolderRole;

    fn account() -> MailAccount {
        MailAccount::new(
            MailAccountSummary::new("acc-1", "user@example.com", "User", "imap")
                .expect("valid summary"),
            ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid server"),
            "user@example.com",
        )
        .expect("valid account")
    }

    fn password() -> SecretString {
        SecretString::from("correct horse".to_owned())
    }

    struct FakeProvider {
        answer: Option<MailProviderError>,
    }

    impl MailboxDiscovery for FakeProvider {
        fn list_mailboxes(&self) -> Result<Vec<DiscoveredMailbox>, MailProviderError> {
            match self.answer {
                Some(error) => Err(error),
                None => Ok(vec![DiscoveredMailbox {
                    name: "INBOX".to_owned(),
                    role: FolderRole::Custom,
                    uid_validity: None,
                    uid_next: None,
                }]),
            }
        }
    }

    #[derive(Default)]
    struct FakeFactory {
        answer: Option<MailProviderError>,
    }

    impl ProviderFactory for FakeFactory {
        fn discovery(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
            Ok(Box::new(FakeProvider {
                answer: self.answer,
            }))
        }

        fn sync(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn crate::ports::mail_provider::MailSync>, MailProviderError> {
            unreachable!("adding an account reads no mail")
        }

        fn reading(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn crate::ports::mail_provider::MailFetch>, MailProviderError> {
            unreachable!("adding an account reads no message")
        }

        fn watch(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn crate::ports::mail_provider::MailWatch>, MailProviderError> {
            unreachable!("adding an account watches nothing")
        }
    }

    #[derive(Default, Clone)]
    struct FakeAccounts {
        stored: std::sync::Arc<Mutex<Vec<MailAccountSummary>>>,
        fail: bool,
    }

    impl FakeAccounts {
        fn holding(summary: MailAccountSummary) -> Self {
            Self {
                stored: std::sync::Arc::new(Mutex::new(vec![summary])),
                fail: false,
            }
        }

        fn count(&self) -> usize {
            self.stored.lock().expect("test lock").len()
        }
    }

    impl AccountRepository for FakeAccounts {
        fn find(&self, account_id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
            if self.fail {
                return Err(AccountRepositoryError::Unavailable);
            }
            // Adding reads none back; changing has to know the account is there before it replaces
            // anything, so this answers with the account itself where the identifier matches.
            Ok(self
                .stored
                .lock()
                .expect("test lock")
                .iter()
                .any(|stored| stored.id() == account_id)
                .then(account))
        }

        fn find_by_email(&self, email: &str) -> Result<Option<String>, AccountRepositoryError> {
            if self.fail {
                return Err(AccountRepositoryError::Unavailable);
            }
            Ok(self
                .stored
                .lock()
                .expect("test lock")
                .iter()
                .find(|stored| stored.email().eq_ignore_ascii_case(email))
                .map(|stored| stored.id().to_owned()))
        }

        fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
            unreachable!("nothing here asks who can send")
        }

        fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
            if self.fail {
                return Err(AccountRepositoryError::Unavailable);
            }
            Ok(self.stored.lock().expect("test lock").clone())
        }

        fn save(&self, account: &MailAccount) -> Result<(), AccountRepositoryError> {
            if self.fail {
                return Err(AccountRepositoryError::Unavailable);
            }
            // Replaced by identifier, as the real store does: saving an account it already has is
            // an update, not a second copy of it.
            let mut stored = self.stored.lock().expect("test lock");
            stored.retain(|kept| kept.id() != account.summary().id());
            stored.push(account.summary().clone());
            Ok(())
        }

        fn delete(&self, account_id: &str) -> Result<(), AccountRepositoryError> {
            self.stored
                .lock()
                .expect("test lock")
                .retain(|stored| stored.id() != account_id);
            Ok(())
        }

        fn set_collapsed(
            &self,
            _account_id: &str,
            _collapsed: bool,
        ) -> Result<(), AccountRepositoryError> {
            Ok(())
        }

        fn number_of(&self, _: &str) -> Result<Option<i64>, AccountRepositoryError> {
            unreachable!("this fake is not asked which file the mail is in")
        }
    }

    #[derive(Default, Clone)]
    struct FakeCredentials {
        /// Which accounts have a password, and how many times one has been written for each.
        stored: std::sync::Arc<Mutex<Vec<(String, u32)>>>,
        fail: bool,
    }

    /// The identifiers the store holds, for a test that cares which account has a password rather
    /// than what the password is.
    fn named(credentials: &FakeCredentials) -> Vec<String> {
        credentials
            .stored
            .lock()
            .expect("test lock")
            .iter()
            .map(|(id, _)| id.clone())
            .collect()
    }

    impl CredentialStore for FakeCredentials {
        fn save(
            &self,
            reference: &CredentialRef,
            _secret: &SecretString,
        ) -> Result<(), CredentialError> {
            if self.fail {
                return Err(CredentialError::Unavailable);
            }
            // The name and a count of how many times it was written, never the value: a secret is
            // unwrapped only where it leaves for its destination, and this is a test double for the
            // place it leaves to rather than that place.
            let mut stored = self.stored.lock().expect("test lock");
            match stored
                .iter_mut()
                .find(|(id, _)| id == &reference.account_id)
            {
                Some((_, times)) => *times += 1,
                None => stored.push((reference.account_id.clone(), 1)),
            }
            Ok(())
        }

        fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
            self.stored
                .lock()
                .expect("test lock")
                .iter()
                .any(|(id, _)| id == &reference.account_id)
                .then(|| SecretString::from("stored".to_owned()))
                .ok_or(CredentialError::NotFound)
        }

        fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
            self.stored
                .lock()
                .expect("test lock")
                .retain(|(id, _)| id != &reference.account_id);
            Ok(())
        }
    }

    #[test]
    fn a_working_connection_is_reported_with_what_was_found() {
        let found = TestConnection::new(FakeFactory::default())
            .execute(&account(), &password())
            .expect("the server answers");

        assert_eq!(found, 1);
    }

    #[test]
    fn a_refused_password_is_reported_as_such_by_the_test_button() {
        let error = TestConnection::new(FakeFactory {
            answer: Some(MailProviderError::Unauthorized),
        })
        .execute(&account(), &password())
        .expect_err("the server refuses");

        assert!(matches!(error, UseCaseError::Unauthorized));
    }

    #[test]
    fn adding_an_account_stores_the_secret_and_the_account() {
        let accounts = FakeAccounts::default();
        let credentials = FakeCredentials::default();

        AddAccount::new(
            accounts.clone(),
            credentials.clone(),
            FakeFactory::default(),
        )
        .execute(account(), password())
        .expect("the account is added");

        assert_eq!(accounts.count(), 1);
        assert_eq!(credentials.stored.lock().expect("test lock").len(), 1);
    }

    #[test]
    fn a_mistyped_password_stores_nothing_at_all() {
        let accounts = FakeAccounts::default();
        let credentials = FakeCredentials::default();

        let error = AddAccount::new(
            accounts.clone(),
            credentials.clone(),
            FakeFactory {
                answer: Some(MailProviderError::Unauthorized),
            },
        )
        .execute(account(), password())
        .expect_err("the server refuses");

        assert!(matches!(error, UseCaseError::Unauthorized));
        assert_eq!(accounts.count(), 0);
        assert!(
            credentials.stored.lock().expect("test lock").is_empty(),
            "a password that does not work must not be kept"
        );
    }

    #[test]
    fn an_unreachable_server_stores_nothing_either() {
        let accounts = FakeAccounts::default();
        let credentials = FakeCredentials::default();

        let error = AddAccount::new(
            accounts.clone(),
            credentials.clone(),
            FakeFactory {
                answer: Some(MailProviderError::Offline),
            },
        )
        .execute(account(), password())
        .expect_err("the server cannot be reached");

        assert!(matches!(error, UseCaseError::Offline));
        assert_eq!(accounts.count(), 0);
        assert!(credentials.stored.lock().expect("test lock").is_empty());
    }

    #[test]
    fn a_failed_account_row_takes_the_secret_back_out() {
        let accounts = FakeAccounts {
            fail: true,
            ..Default::default()
        };
        let credentials = FakeCredentials::default();

        let error = AddAccount::new(
            accounts.clone(),
            credentials.clone(),
            FakeFactory::default(),
        )
        .execute(account(), password())
        .expect_err("the account cannot be stored");

        assert!(matches!(error, UseCaseError::StorageUnavailable));
        assert!(
            credentials.stored.lock().expect("test lock").is_empty(),
            "a password belonging to no account cannot be found or removed by its owner"
        );
    }

    #[test]
    fn the_same_address_is_not_added_twice() {
        let existing = MailAccountSummary::new("acc-0", "USER@example.com", "User", "imap")
            .expect("valid summary");
        let accounts = FakeAccounts::holding(existing);
        let credentials = FakeCredentials::default();
        // It already has a password, which is what makes it a duplicate rather than a repair.
        credentials
            .stored
            .lock()
            .expect("test lock")
            .push(("acc-0".to_owned(), 1));

        let error = AddAccount::new(
            accounts.clone(),
            credentials.clone(),
            FakeFactory::default(),
        )
        .execute(account(), password())
        .expect_err("the address is already here");

        assert!(matches!(error, UseCaseError::AccountAlreadyAdded));
        assert_eq!(
            accounts.count(),
            1,
            "the account that was there is untouched"
        );
        assert_eq!(
            named(&credentials).as_slice(),
            ["acc-0"],
            "and its stored password was neither replaced nor joined by a second one"
        );
    }

    #[test]
    fn an_account_whose_password_is_gone_takes_a_new_one_under_the_identifier_it_had() {
        // A password removed from the store leaves an account that can never be read.
        // Refusing the address as a duplicate would leave it that way for good; a new identifier
        // would orphan its folders and its mail.
        let existing = MailAccountSummary::new("acc-0", "user@example.com", "User", "imap")
            .expect("valid summary");
        let accounts = FakeAccounts::holding(existing);
        let credentials = FakeCredentials::default();

        AddAccount::new(
            accounts.clone(),
            credentials.clone(),
            FakeFactory::default(),
        )
        .execute(account(), password())
        .expect("the password is supplied again");

        assert_eq!(accounts.count(), 1, "no second account appears");
        assert_eq!(
            named(&credentials).as_slice(),
            ["acc-0"],
            "and the password is stored under the identifier the account already had"
        );
    }

    #[test]
    fn a_second_address_is_added_beside_the_first() {
        let existing = MailAccountSummary::new("acc-0", "other@example.com", "Other", "imap")
            .expect("valid summary");
        let accounts = FakeAccounts::holding(existing);

        AddAccount::new(
            accounts.clone(),
            FakeCredentials::default(),
            FakeFactory::default(),
        )
        .execute(account(), password())
        .expect("a different address is a different account");

        assert_eq!(accounts.count(), 2);
    }

    #[test]
    fn a_locked_keychain_leaves_no_half_added_account() {
        let accounts = FakeAccounts::default();
        let credentials = FakeCredentials {
            fail: true,
            ..Default::default()
        };

        let error = AddAccount::new(
            accounts.clone(),
            credentials.clone(),
            FakeFactory::default(),
        )
        .execute(account(), password())
        .expect_err("the secret cannot be stored");

        assert!(matches!(error, UseCaseError::CredentialsUnavailable));
        assert_eq!(
            accounts.count(),
            0,
            "an account whose password was never stored would fail on every sync"
        );
    }

    /// The account form can correct an account that already works.
    ///
    /// Before this there was no command at all: the screen submitted through *add*, which answers
    /// `AccountAlreadyAdded` the moment the account has a password. A mistyped host could only be
    /// corrected by removing the account, and removing an account takes its mail with it.
    #[test]
    fn an_account_that_already_works_can_have_its_settings_corrected() {
        let accounts = FakeAccounts::holding(
            MailAccountSummary::new("acc-1", "user@example.com", "User", "imap")
                .expect("valid summary"),
        );
        let credentials = FakeCredentials::default();
        credentials
            .stored
            .lock()
            .expect("test lock")
            .push(("acc-1".to_owned(), 1));

        ChangeAccount::new(
            accounts.clone(),
            credentials.clone(),
            FakeFactory::default(),
        )
        .execute("acc-1", account(), password())
        .expect("the settings change");

        let stored = credentials.stored.lock().expect("test lock");
        assert_eq!(stored.len(), 1, "a second password was left behind");
        assert_eq!(stored[0].0, "acc-1", "the identifier did not survive");
        assert_eq!(stored[0].1, 2, "the password was not written again");
    }

    /// An account that is not here is not a thing to change.
    #[test]
    fn changing_an_account_that_is_not_here_is_refused() {
        let accounts = FakeAccounts::default();
        let credentials = FakeCredentials::default();

        let outcome = ChangeAccount::new(accounts, credentials, FakeFactory::default()).execute(
            "acc-nowhere",
            account(),
            password(),
        );

        assert!(
            matches!(outcome, Err(UseCaseError::UnknownAccount)),
            "an account that is not here was changed"
        );
    }

    /// A password the server refuses never replaces one it accepts.
    ///
    /// The order is the whole of this: ask first, write second. Without it a typo in the password
    /// field leaves somebody unable to read mail they could read a moment before, and the only
    /// sign is that syncing stops.
    #[test]
    fn a_password_the_server_refuses_leaves_the_working_one_alone() {
        let accounts = FakeAccounts::holding(
            MailAccountSummary::new("acc-1", "user@example.com", "User", "imap")
                .expect("valid summary"),
        );
        let credentials = FakeCredentials::default();
        credentials
            .stored
            .lock()
            .expect("test lock")
            .push(("acc-1".to_owned(), 1));

        let outcome = ChangeAccount::new(
            accounts,
            credentials.clone(),
            FakeFactory {
                answer: Some(MailProviderError::Unauthorized),
            },
        )
        .execute("acc-1", account(), password());

        assert!(outcome.is_err(), "a refused password was accepted");
        assert_eq!(
            credentials.stored.lock().expect("test lock")[0].1,
            1,
            "the store was written to for a password the server had already refused"
        );
    }
}
