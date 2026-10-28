//! Saving one message as the file it arrived as.
//!
//! The bytes the server sent, unchanged: no rendering, no sanitizing, no re-encoding. That is the
//! point of an `.eml` — it is the message, and it opens in whatever the person wants to open it in.
//! Anything this build did to make the message safe to *look at* would be a lie if it were written
//! into the file someone keeps.
//!
//! The interface asks for this by naming a message. It never names a place: where the file goes is
//! decided by the person, in the system's own dialog, behind a port (invariant 12).

use crate::application::error::UseCaseError;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential::Credential;
use crate::ports::credential_store::{CredentialPurpose, CredentialRef, CredentialStore};
use crate::ports::file_export::{FileExport, FileToSave, SavedFile};
use crate::ports::mail_provider::ProviderFactory;
use lyonoyl_domain::attachment::message_file_name;
use lyonoyl_domain::message::{Uid, UidValidity};

pub struct SaveMessage<R, C, F, E>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    E: FileExport,
{
    accounts: R,
    credentials: C,
    providers: F,
    files: E,
}

impl<R, C, F, E> SaveMessage<R, C, F, E>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    E: FileExport,
{
    pub fn new(accounts: R, credentials: C, providers: F, files: E) -> Self {
        Self {
            accounts,
            credentials,
            providers,
            files,
        }
    }

    /// `mailbox`, `uid_validity` and `uid` come from the index, `subject` from what a pass
    /// already stored. A number with no generation beside it cannot be checked against the
    /// mailbox, and is refused rather than read by.
    ///
    /// `Ok(None)` is the person closing the dialog.
    pub fn execute(
        &self,
        account_id: &str,
        mailbox: &str,
        uid_validity: Option<UidValidity>,
        uid: Uid,
        subject: Option<&str>,
    ) -> Result<Option<SavedFile>, UseCaseError> {
        let uid_validity = uid_validity.ok_or(UseCaseError::StorageCorrupted)?;

        let account = self
            .accounts
            .find(account_id)?
            .ok_or(UseCaseError::UnknownAccount)?;

        let password = self.credentials.read(&CredentialRef::new(
            account_id.to_owned(),
            CredentialPurpose::ImapPassword,
        ))?;

        let raw = self
            .providers
            .reading(&account, &Credential::password(password))?
            .whole_message(mailbox, uid_validity, uid)?;

        Ok(self.files.save(FileToSave {
            suggested_name: &message_file_name(subject),
            bytes: &raw,
        })?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use secrecy::SecretString;

    use super::*;
    use crate::ports::account_repository::AccountRepositoryError;
    use crate::ports::credential_store::CredentialError;
    use crate::ports::file_export::FileExportError;
    use crate::ports::mail_provider::{
        MailFetch, MailProviderError, MailSync, MailWatch, MailboxDiscovery,
    };
    use lyonoyl_domain::account::{Encryption, MailAccount, MailAccountSummary, ServerSettings};

    fn account() -> MailAccount {
        MailAccount::new(
            MailAccountSummary::new("acc-1", "user@example.com", "User", "imap")
                .expect("valid summary"),
            ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid server"),
            "user@example.com",
        )
        .expect("valid account")
    }

    fn uid(value: u32) -> Uid {
        Uid::new(value).expect("non-zero")
    }

    #[derive(Default)]
    struct FakeAccounts {
        held: Option<MailAccount>,
    }

    impl FakeAccounts {
        fn holding() -> Self {
            Self {
                held: Some(account()),
            }
        }
    }

    impl AccountRepository for FakeAccounts {
        fn find(&self, _account_id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
            Ok(self.held.clone())
        }

        fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
            unreachable!("nothing here asks who can send")
        }

        fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
            unreachable!("saving a message lists no accounts")
        }

        fn find_by_email(&self, _email: &str) -> Result<Option<String>, AccountRepositoryError> {
            unreachable!("saving a message looks nothing up by address")
        }

        fn save(&self, _account: &MailAccount) -> Result<(), AccountRepositoryError> {
            unreachable!("saving a message stores no account")
        }

        fn delete(&self, _account_id: &str) -> Result<(), AccountRepositoryError> {
            unreachable!("saving a message removes nothing")
        }

        fn set_collapsed(
            &self,
            _account_id: &str,
            _collapsed: bool,
        ) -> Result<(), AccountRepositoryError> {
            unreachable!("saving a message does not touch the sidebar")
        }

        fn number_of(&self, _: &str) -> Result<Option<i64>, AccountRepositoryError> {
            unreachable!("this fake is not asked which file the mail is in")
        }
    }

    struct FakeCredentials {
        stored: bool,
    }

    impl CredentialStore for FakeCredentials {
        fn read(&self, _reference: &CredentialRef) -> Result<SecretString, CredentialError> {
            match self.stored {
                true => Ok(SecretString::from("correct horse".to_owned())),
                false => Err(CredentialError::NotFound),
            }
        }

        fn save(
            &self,
            _reference: &CredentialRef,
            _secret: &SecretString,
        ) -> Result<(), CredentialError> {
            unreachable!("saving a message stores no secret")
        }

        fn delete(&self, _reference: &CredentialRef) -> Result<(), CredentialError> {
            unreachable!("saving a message removes no secret")
        }
    }

    struct FakeServer {
        answer: Result<Vec<u8>, MailProviderError>,
    }

    impl MailFetch for FakeServer {
        fn whole_message(
            &self,
            _mailbox: &str,
            _uid_validity: UidValidity,
            _uid: Uid,
        ) -> Result<Vec<u8>, MailProviderError> {
            self.answer.clone()
        }
    }

    /// The generation the index holds the numbers in these tests under.
    fn stored_generation() -> Option<UidValidity> {
        UidValidity::new(7).ok()
    }

    struct FakeFactory {
        answer: Result<Vec<u8>, MailProviderError>,
    }

    impl FakeFactory {
        fn holding(raw: &[u8]) -> Self {
            Self {
                answer: Ok(raw.to_vec()),
            }
        }
    }

    impl ProviderFactory for FakeFactory {
        fn reading(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailFetch>, MailProviderError> {
            Ok(Box::new(FakeServer {
                answer: self.answer.clone(),
            }))
        }

        fn discovery(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
            unreachable!("saving a message lists no mailboxes")
        }

        fn sync(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailSync>, MailProviderError> {
            unreachable!("saving a message runs no pass")
        }

        fn watch(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailWatch>, MailProviderError> {
            unreachable!("saving a message watches nothing")
        }
    }

    /// A place to put a file that keeps what it was handed, so the use case can be watched rather
    /// than trusted to hand over the message unchanged.
    #[derive(Default)]
    struct FakeDialog {
        given: Mutex<Vec<(String, Vec<u8>)>>,
        answer: Option<FileExportError>,
        cancelled: bool,
    }

    impl FakeDialog {
        fn closed_by_the_person() -> Self {
            Self {
                cancelled: true,
                ..Self::default()
            }
        }

        fn failing(error: FileExportError) -> Self {
            Self {
                answer: Some(error),
                ..Self::default()
            }
        }
    }

    impl FileExport for FakeDialog {
        fn save(&self, file: FileToSave<'_>) -> Result<Option<SavedFile>, FileExportError> {
            self.given
                .lock()
                .expect("test lock")
                .push((file.suggested_name.to_owned(), file.bytes.to_vec()));

            if let Some(error) = self.answer {
                return Err(error);
            }

            match self.cancelled {
                true => Ok(None),
                false => Ok(Some(SavedFile {
                    name: file.suggested_name.to_owned(),
                })),
            }
        }
    }

    fn saving<'a>(
        raw: &[u8],
        dialog: &'a FakeDialog,
    ) -> SaveMessage<FakeAccounts, FakeCredentials, FakeFactory, &'a FakeDialog> {
        SaveMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            FakeFactory::holding(raw),
            dialog,
        )
    }

    #[test]
    fn what_the_server_sent_is_what_is_written() {
        // Not the rendered body, not the sanitized one: an .eml that is not the message is a file
        // that opens somewhere else and shows something else.
        let raw = b"From: someone\r\nSubject: Hello\r\n\r\n<p onclick=\"alert(1)\">Hi.</p>";
        let dialog = FakeDialog::default();

        let saved = saving(raw, &dialog)
            .execute("acc-1", "INBOX", stored_generation(), uid(7), Some("Hello"))
            .expect("the message is read")
            .expect("the person chose a place");

        assert_eq!(saved.name, "Hello.eml");
        assert_eq!(dialog.given.lock().expect("test lock")[0].1, raw.to_vec());
    }

    #[test]
    fn closing_the_dialog_is_an_answer_rather_than_a_failure() {
        let dialog = FakeDialog::closed_by_the_person();

        let saved = saving(b"anything", &dialog)
            .execute("acc-1", "INBOX", stored_generation(), uid(1), Some("Hello"))
            .expect("closing a dialog is not an error");

        assert_eq!(saved, None);
    }

    #[test]
    fn a_system_with_nowhere_to_ask_says_so_rather_than_choosing_for_the_person() {
        let dialog = FakeDialog::failing(FileExportError::CannotAsk);

        let outcome = saving(b"anything", &dialog).execute(
            "acc-1",
            "INBOX",
            stored_generation(),
            uid(1),
            Some("Hello"),
        );

        assert!(matches!(outcome, Err(UseCaseError::NoPlaceToSave)));
    }

    #[test]
    fn a_write_that_failed_keeps_its_own_meaning() {
        let dialog = FakeDialog::failing(FileExportError::NotWritten);

        let outcome = saving(b"anything", &dialog).execute(
            "acc-1",
            "INBOX",
            stored_generation(),
            uid(1),
            Some("Hello"),
        );

        assert!(matches!(outcome, Err(UseCaseError::FileNotWritten)));
    }

    #[test]
    fn an_account_that_is_not_on_this_device_is_refused_before_anything_is_read() {
        let dialog = FakeDialog::default();

        let outcome = SaveMessage::new(
            FakeAccounts::default(),
            FakeCredentials { stored: true },
            FakeFactory::holding(b"anything"),
            &dialog,
        )
        .execute("acc-1", "INBOX", stored_generation(), uid(1), Some("Hello"));

        assert!(matches!(outcome, Err(UseCaseError::UnknownAccount)));
        assert!(
            dialog.given.lock().expect("test lock").is_empty(),
            "nobody is asked where to put a message that was never read"
        );
    }

    #[test]
    fn a_missing_password_is_reported_as_itself() {
        let dialog = FakeDialog::default();

        let outcome = SaveMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: false },
            FakeFactory::holding(b"anything"),
            &dialog,
        )
        .execute("acc-1", "INBOX", stored_generation(), uid(1), Some("Hello"));

        assert!(matches!(outcome, Err(UseCaseError::CredentialsMissing)));
    }

    #[test]
    fn a_number_with_no_generation_beside_it_is_not_read_by() {
        // Nothing can check such a number against the mailbox, and a mailbox that renumbered has
        // given it to somebody else's message — which would be written into this person's file.
        let dialog = FakeDialog::default();

        let outcome = saving(b"Not this message.", &dialog).execute(
            "acc-1",
            "INBOX",
            None,
            uid(1),
            Some("Hello"),
        );

        assert!(matches!(outcome, Err(UseCaseError::StorageCorrupted)));
        assert!(
            dialog.given.lock().expect("test lock").is_empty(),
            "something was written from a number nothing could check"
        );
    }

    #[test]
    fn a_server_that_cannot_be_reached_keeps_its_meaning() {
        let dialog = FakeDialog::default();

        let outcome = SaveMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            FakeFactory {
                answer: Err(MailProviderError::Offline),
            },
            &dialog,
        )
        .execute("acc-1", "INBOX", stored_generation(), uid(1), Some("Hello"));

        assert!(matches!(outcome, Err(UseCaseError::Offline)));
    }
}
