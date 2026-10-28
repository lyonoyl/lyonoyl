//! Removing an account, and everything of it this device holds.

use crate::application::error::UseCaseError;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential_store::{
    CredentialError, CredentialPurpose, CredentialRef, CredentialStore,
};
use crate::ports::mail_files::MailFiles;

/// What an account keeps in the store this device keeps its secrets in.
///
/// Not the key the mail is sealed under: that is one per installation and shared by every account,
/// so removing one account must not touch it — doing so would make every other account's mail
/// unreadable.
const ITS_OWN_SECRETS: [CredentialPurpose; 3] = [
    CredentialPurpose::ImapPassword,
    CredentialPurpose::SmtpPassword,
    CredentialPurpose::OAuthRefreshToken,
];

pub struct RemoveAccount<R: AccountRepository, S: CredentialStore, F: MailFiles> {
    accounts: R,
    secrets: S,
    files: F,
}

impl<R: AccountRepository, S: CredentialStore, F: MailFiles> RemoveAccount<R, S, F> {
    pub fn new(accounts: R, secrets: S, files: F) -> Self {
        Self {
            accounts,
            secrets,
            files,
        }
    }

    /// Takes the account off this device: its mail, its secrets, and what it was asked to do.
    ///
    /// **The order is the whole design.** The row goes last, because it is what says the mail and
    /// the secrets exist: remove it and fail on either, and somebody's mail is left on the disk, or
    /// a working password to it in the store, with nothing left to say whose it was or that it is
    /// there. Fail before it and the account is still listed — which the list already says out
    /// loud, and which trying again puts right.
    ///
    /// The secrets go first, because the store is the one step that can be refused — a locked
    /// keyring, a keychain answered with Deny, a file that will not be written — and a refusal then
    /// leaves everything as it was. Refused after the mail file had gone, it would leave the account
    /// listed with that file about to be made again from nothing, and the actions still waiting to
    /// reach the server dropped with the old one.
    ///
    /// Removing the row takes the queue, the outbox, the drafts and the record of activity with it:
    /// they are the account's by a foreign key that cascades. Its number is not freed, because the
    /// counter that hands numbers out never goes back — an identifier that reached the interface or
    /// the queue would otherwise start naming a different account's message.
    pub fn execute(&self, account_id: &str) -> Result<(), UseCaseError> {
        let number = self
            .accounts
            .number_of(account_id)?
            .ok_or(UseCaseError::UnknownAccount)?;

        // A secret that is not there is a secret that is gone, which is what was asked for.
        for purpose in ITS_OWN_SECRETS {
            match self
                .secrets
                .delete(&CredentialRef::new(account_id, purpose))
            {
                Ok(()) | Err(CredentialError::NotFound) => {}
                Err(refused) => return Err(refused.into()),
            }
        }

        self.files
            .remove(number)
            .map_err(|_| UseCaseError::StorageUnavailable)?;

        self.accounts.delete(account_id)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::account_repository::AccountRepositoryError;
    use crate::ports::credential_store::CredentialError;
    use crate::ports::mail_files::MailFilesError;
    use lyonoyl_domain::account::{MailAccount, MailAccountSummary};
    use secrecy::SecretString;
    use std::sync::Mutex;

    /// What happened, in the order it happened.
    ///
    /// The order is the design here, so it is what the fakes record: a set of things done would
    /// pass whichever way round they were done, and one of those ways leaves somebody's mail on
    /// the disk with nothing to say whose it is.
    #[derive(Default)]
    struct Written(Mutex<Vec<String>>);

    impl Written {
        fn note(&self, what: &str) {
            self.0.lock().expect("not poisoned").push(what.to_owned());
        }

        fn so_far(&self) -> Vec<String> {
            self.0.lock().expect("not poisoned").clone()
        }
    }

    struct FakeAccounts<'a> {
        written: &'a Written,
        number: Option<i64>,
    }

    impl AccountRepository for FakeAccounts<'_> {
        fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
            unreachable!("removing an account lists none")
        }
        fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
            unreachable!("removing an account sends nothing")
        }
        fn find(&self, _: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
            unreachable!("removing an account reads none back")
        }
        fn find_by_email(&self, _: &str) -> Result<Option<String>, AccountRepositoryError> {
            unreachable!("removing an account looks up no address")
        }
        fn save(&self, _: &MailAccount) -> Result<(), AccountRepositoryError> {
            unreachable!("removing an account writes none")
        }
        fn set_collapsed(&self, _: &str, _: bool) -> Result<(), AccountRepositoryError> {
            unreachable!("removing an account folds nothing away")
        }
        fn number_of(&self, _: &str) -> Result<Option<i64>, AccountRepositoryError> {
            Ok(self.number)
        }
        fn delete(&self, account_id: &str) -> Result<(), AccountRepositoryError> {
            self.written.note(&format!("row of {account_id}"));
            Ok(())
        }
    }

    struct FakeSecrets<'a> {
        written: &'a Written,
        /// What the store answers when asked to delete.
        answers: fn() -> Result<(), CredentialError>,
    }

    impl CredentialStore for FakeSecrets<'_> {
        fn save(&self, _: &CredentialRef, _: &SecretString) -> Result<(), CredentialError> {
            unreachable!("removing an account saves nothing")
        }
        fn read(&self, _: &CredentialRef) -> Result<SecretString, CredentialError> {
            unreachable!("removing an account reads nothing")
        }
        fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
            (self.answers)()?;
            self.written
                .note(&format!("secret {}", reference.purpose.as_str()));
            Ok(())
        }
    }

    struct FakeFiles<'a> {
        written: &'a Written,
        refuses: bool,
    }

    impl MailFiles for FakeFiles<'_> {
        fn held_by(&self, _account: i64) -> Result<u64, MailFilesError> {
            Ok(0)
        }

        fn remove(&self, account: i64) -> Result<(), MailFilesError> {
            if self.refuses {
                return Err(MailFilesError::Unavailable);
            }
            self.written.note(&format!("mail of {account}"));
            Ok(())
        }
    }

    fn removing(written: &Written, number: Option<i64>, refuses: bool) -> Result<(), UseCaseError> {
        removing_with(written, number, refuses, || Ok(()))
    }

    fn removing_with(
        written: &Written,
        number: Option<i64>,
        refuses: bool,
        secrets_answer: fn() -> Result<(), CredentialError>,
    ) -> Result<(), UseCaseError> {
        RemoveAccount::new(
            FakeAccounts { written, number },
            FakeSecrets {
                written,
                answers: secrets_answer,
            },
            FakeFiles { written, refuses },
        )
        .execute("acc-1")
    }

    #[test]
    fn the_mail_goes_before_the_row_that_says_whose_it_is() {
        let written = Written::default();
        removing(&written, Some(7), false).expect("the account is removed");

        let done = written.so_far();
        let mail = done.iter().position(|what| what == "mail of 7");
        let row = done.iter().position(|what| what == "row of acc-1");

        assert!(
            mail < row,
            "the row went first, so the mail was orphaned: {done:?}"
        );
    }

    #[test]
    fn its_own_secrets_go_and_the_key_the_others_share_stays() {
        let written = Written::default();
        removing(&written, Some(7), false).expect("the account is removed");

        let done = written.so_far();
        assert!(done.iter().any(|what| what == "secret imap-password"));
        assert!(
            !done.iter().any(|what| what.contains("message-key")),
            "the key every account's mail is sealed under was removed with one of them: {done:?}"
        );
    }

    #[test]
    fn a_password_the_store_will_not_let_go_of_leaves_the_account_where_it_was() {
        // Removed over the top of it, the password would stay in the keychain or the file as
        // working access to the mailbox, with nothing left on this device to say it is there. And
        // a mail file removed first would come back empty, without the actions still waiting.
        let written = Written::default();
        let refused = removing_with(&written, Some(7), false, || {
            Err(CredentialError::Unavailable)
        })
        .expect_err("the account stays");

        assert!(matches!(refused, UseCaseError::CredentialsUnavailable));
        assert!(
            written.so_far().is_empty(),
            "something was removed although a password of it could not be: {:?}",
            written.so_far()
        );
    }

    #[test]
    fn a_password_that_is_not_there_is_already_gone() {
        let written = Written::default();
        removing_with(&written, Some(7), false, || Err(CredentialError::NotFound))
            .expect("nothing stored is nothing left behind");

        assert!(written.so_far().iter().any(|what| what == "row of acc-1"));
    }

    #[test]
    fn mail_that_will_not_go_keeps_the_row_that_says_whose_it_is() {
        let written = Written::default();
        let refused = removing(&written, Some(7), true).expect_err("it cannot be removed");

        assert!(matches!(refused, UseCaseError::StorageUnavailable));
        assert!(
            !written.so_far().iter().any(|what| what == "row of acc-1"),
            "the row went although the mail could not: {:?}",
            written.so_far()
        );
    }

    #[test]
    fn an_account_this_device_does_not_have_is_said_so_rather_than_removed() {
        let written = Written::default();
        let refused = removing(&written, None, false).expect_err("there is nothing to remove");

        assert!(matches!(refused, UseCaseError::UnknownAccount));
        assert!(written.so_far().is_empty());
    }
}
