use std::sync::Mutex;

use secrecy::SecretString;

use super::*;
use crate::ports::account_repository::AccountRepositoryError;
use crate::ports::credential::Credential;
use crate::ports::credential_store::CredentialError;
use crate::ports::credential_store::CredentialRef;
use crate::ports::file_export::FileExportError;
use crate::ports::mail_provider::{
    MailFetch, MailProviderError, MailSync, MailWatch, MailboxDiscovery,
};
use crate::ports::message_bodies::MessageBodyError;
use crate::ports::message_parts::{FileTakenOut, NotReadNow};
use lyonoyl_domain::account::{Encryption, MailAccount, MailAccountSummary, ServerSettings};

fn account() -> MailAccount {
    MailAccount::new(
        MailAccountSummary::new("acc-1", "a@example.com", "A", "imap").expect("well-formed"),
        ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid"),
        "a@example.com",
    )
    .expect("well-formed")
}

struct Accounts;

impl AccountRepository for Accounts {
    fn find(&self, _id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
        Ok(Some(account()))
    }
    fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
        Ok(Vec::new())
    }
    fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
        Ok(Vec::new())
    }
    fn find_by_email(&self, _email: &str) -> Result<Option<String>, AccountRepositoryError> {
        Ok(None)
    }
    fn save(&self, _account: &MailAccount) -> Result<(), AccountRepositoryError> {
        Ok(())
    }
    fn delete(&self, _id: &str) -> Result<(), AccountRepositoryError> {
        Ok(())
    }
    fn number_of(&self, _id: &str) -> Result<Option<i64>, AccountRepositoryError> {
        Ok(Some(1))
    }
    fn set_collapsed(&self, _id: &str, _collapsed: bool) -> Result<(), AccountRepositoryError> {
        Ok(())
    }
}

struct Secrets;

impl CredentialStore for Secrets {
    fn read(&self, _r: &CredentialRef) -> Result<SecretString, CredentialError> {
        Ok(SecretString::from("correct horse"))
    }
    fn save(&self, _r: &CredentialRef, _s: &SecretString) -> Result<(), CredentialError> {
        Ok(())
    }
    fn delete(&self, _r: &CredentialRef) -> Result<(), CredentialError> {
        Ok(())
    }
}

/// A provider that hands back one message, and records that it was asked for the whole of it.
struct Provider {
    raw: Vec<u8>,
    asked: Mutex<Vec<(String, u32)>>,
}

impl MailFetch for Provider {
    fn whole_message(
        &self,
        mailbox: &str,
        _uid_validity: UidValidity,
        uid: Uid,
    ) -> Result<Vec<u8>, MailProviderError> {
        self.asked
            .lock()
            .expect("not poisoned")
            .push((mailbox.to_owned(), uid.get()));
        Ok(self.raw.clone())
    }
}

impl ProviderFactory for &Provider {
    fn reading(
        &self,
        _account: &MailAccount,
        _credential: &Credential,
    ) -> Result<Box<dyn MailFetch>, MailProviderError> {
        Ok(Box::new(Provider {
            raw: self.raw.clone(),
            asked: Mutex::new(Vec::new()),
        }))
    }
    fn discovery(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
        unreachable!("saving a file does not list mailboxes")
    }
    fn sync(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailSync>, MailProviderError> {
        unreachable!("saving a file does not synchronise")
    }
    fn watch(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailWatch>, MailProviderError> {
        unreachable!("saving a file does not watch")
    }
}

/// What was offered to the dialog, and what the dialog answered.
#[derive(Default)]
struct Dialog {
    offered: Mutex<Vec<(String, Vec<u8>)>>,
    /// The person closing it without choosing.
    refused: bool,
}

impl FileExport for &Dialog {
    fn save(&self, file: FileToSave<'_>) -> Result<Option<SavedFile>, FileExportError> {
        self.offered
            .lock()
            .expect("not poisoned")
            .push((file.suggested_name.to_owned(), file.bytes.to_vec()));

        if self.refused {
            return Ok(None);
        }

        Ok(Some(SavedFile {
            name: file.suggested_name.to_owned(),
        }))
    }
}

/// Files by their place in the message; or, busy, the message not read this time.
#[derive(Default)]
struct Parts {
    files: Vec<(String, Vec<u8>)>,
    busy: bool,
}

impl Parts {
    fn of(files: Vec<(String, Vec<u8>)>) -> Self {
        Self { files, busy: false }
    }
}

impl MessageParts for &Parts {
    fn file_at(&self, raw: &[u8], at: u32) -> Result<Option<FileTakenOut>, NotReadNow> {
        assert_eq!(
            raw, b"the whole message",
            "the file is taken out of the message read"
        );
        if self.busy {
            return Err(NotReadNow);
        }
        Ok(self
            .files
            .get(at as usize)
            .cloned()
            .map(|(name, bytes)| FileTakenOut {
                name,
                bytes,
                a_message: false,
            }))
    }
    fn passed_on(&self, _raw: &[u8]) -> Option<crate::ports::message_parts::PassedOn> {
        unreachable!("saving one file passes nothing on")
    }
}

/// The copies this device keeps, by message, and every message kept while a test ran.
#[derive(Default)]
struct Kept {
    held: Mutex<std::collections::HashMap<u64, Vec<u8>>>,
    keeps: Mutex<Vec<u64>>,
}

impl Kept {
    fn holding(message: u64, raw: &[u8]) -> Self {
        Self {
            held: Mutex::new(std::collections::HashMap::from([(message, raw.to_vec())])),
            ..Self::default()
        }
    }
}

impl MessageBodies for &Kept {
    fn held(&self, message: MessageId) -> Result<Option<Vec<u8>>, MessageBodyError> {
        Ok(self
            .held
            .lock()
            .expect("not poisoned")
            .get(&message.get())
            .cloned())
    }
    fn keep(&self, message: MessageId, raw: &[u8]) -> Result<(), MessageBodyError> {
        self.keeps.lock().expect("not poisoned").push(message.get());
        self.held
            .lock()
            .expect("not poisoned")
            .insert(message.get(), raw.to_vec());
        Ok(())
    }
    fn fill_in_missing(&self, _message: MessageId, _raw: &[u8]) -> Result<(), MessageBodyError> {
        Ok(())
    }
}

/// The message the tests save from.
fn the_message() -> MessageId {
    MessageId::new(42).expect("non-zero")
}

fn saving(parts: &Parts, dialog: &Dialog, at: u32) -> Result<Option<SavedFile>, UseCaseError> {
    saving_under(UidValidity::new(3).ok(), parts, dialog, at)
}

/// The same, with the generation the index holds the message's number under.
fn saving_under(
    uid_validity: Option<UidValidity>,
    parts: &Parts,
    dialog: &Dialog,
    at: u32,
) -> Result<Option<SavedFile>, UseCaseError> {
    let provider = Provider {
        raw: b"the whole message".to_vec(),
        asked: Mutex::new(Vec::new()),
    };

    SaveAttachment::new(
        Accounts,
        Secrets,
        &provider,
        &Kept::default(),
        parts,
        dialog,
    )
    .execute(
        "acc-1",
        the_message(),
        "INBOX",
        uid_validity,
        Uid::new(7).expect("non-zero"),
        at,
    )
}

/// A server that cannot be reached.
struct Unreachable;

impl ProviderFactory for &Unreachable {
    fn reading(
        &self,
        _account: &MailAccount,
        _credential: &Credential,
    ) -> Result<Box<dyn MailFetch>, MailProviderError> {
        Err(MailProviderError::Offline)
    }
    fn discovery(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
        Err(MailProviderError::Offline)
    }
    fn sync(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailSync>, MailProviderError> {
        Err(MailProviderError::Offline)
    }
    fn watch(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailWatch>, MailProviderError> {
        Err(MailProviderError::Offline)
    }
}

#[test]
fn a_file_of_a_message_this_device_keeps_is_saved_without_its_server() {
    // As the reader reads it and as opening reads it: the line that says a file did not open
    // offers saving it, and that has to hold with no network.
    let kept = Kept::holding(the_message().get(), b"the whole message");
    let parts = Parts::of(vec![("invoice.pdf".to_owned(), b"%PDF-1.4".to_vec())]);
    let dialog = Dialog::default();

    let saved = SaveAttachment::new(Accounts, Secrets, &Unreachable, &kept, &parts, &dialog)
        .execute(
            "acc-1",
            the_message(),
            "INBOX",
            None,
            Uid::new(7).expect("non-zero"),
            0,
        )
        .expect("saved from the copy this device keeps");

    assert_eq!(saved.map(|file| file.name), Some("invoice.pdf".to_owned()));
}

#[test]
fn a_message_this_device_keeps_for_another_is_not_saved_from() {
    // The copy is found by the message asked about: one kept for a different message is not it,
    // and with its server out of reach, nothing is saved.
    let kept = Kept::holding(7, b"another message");
    let parts = Parts::of(vec![("invoice.pdf".to_owned(), b"%PDF-1.4".to_vec())]);
    let dialog = Dialog::default();

    let answer = SaveAttachment::new(Accounts, Secrets, &Unreachable, &kept, &parts, &dialog)
        .execute(
            "acc-1",
            the_message(),
            "INBOX",
            UidValidity::new(3).ok(),
            Uid::new(7).expect("non-zero"),
            0,
        );

    assert!(matches!(answer, Err(UseCaseError::Offline)), "{answer:?}");
    assert!(dialog.offered.lock().expect("not poisoned").is_empty());
}

#[test]
fn a_message_this_device_does_not_keep_is_read_from_its_server_and_kept() {
    let provider = Provider {
        raw: b"the whole message".to_vec(),
        asked: Mutex::new(Vec::new()),
    };
    let kept = Kept::default();
    let parts = Parts::of(vec![("invoice.pdf".to_owned(), b"%PDF-1.4".to_vec())]);
    let dialog = Dialog::default();

    SaveAttachment::new(Accounts, Secrets, &provider, &kept, &parts, &dialog)
        .execute(
            "acc-1",
            the_message(),
            "INBOX",
            UidValidity::new(3).ok(),
            Uid::new(7).expect("non-zero"),
            0,
        )
        .expect("saved");

    assert_eq!(
        *kept.keeps.lock().expect("not poisoned"),
        vec![the_message().get()],
        "kept under the message it is, so the next reading is from here"
    );
}

#[test]
fn a_number_with_no_generation_beside_it_is_not_read_by() {
    // Nothing can check such a number against the mailbox, and a mailbox that renumbered has given
    // it to somebody else's message — whose file would be offered under this one's name.
    let parts = Parts::of(vec![("invoice.pdf".to_owned(), b"%PDF-1.4".to_vec())]);
    let dialog = Dialog::default();

    assert!(matches!(
        saving_under(None, &parts, &dialog, 0),
        Err(UseCaseError::StorageCorrupted)
    ));
    assert!(
        dialog.offered.lock().expect("not poisoned").is_empty(),
        "a file was offered out of a number nothing could check"
    );
}

#[test]
fn the_file_is_offered_under_the_name_the_parser_made_and_never_a_path() {
    // The interface asked by naming a message and a position. It never named a place, and the name
    // offered is the one the parser made safe rather than the one the message carried.
    let parts = Parts::of(vec![("invoice.pdf".to_owned(), b"%PDF-1.4".to_vec())]);
    let dialog = Dialog::default();

    let saved = saving(&parts, &dialog, 0).expect("saved");

    assert_eq!(saved.map(|file| file.name), Some("invoice.pdf".to_owned()));
    let offered = dialog.offered.lock().expect("not poisoned");
    assert_eq!(offered[0].0, "invoice.pdf");
    assert_eq!(offered[0].1, b"%PDF-1.4");
}

#[test]
fn closing_the_dialog_saves_nothing_and_is_not_a_failure() {
    let parts = Parts::of(vec![("invoice.pdf".to_owned(), b"x".to_vec())]);
    let dialog = Dialog {
        refused: true,
        ..Dialog::default()
    };

    assert_eq!(saving(&parts, &dialog, 0).expect("no failure"), None);
}

#[test]
fn a_part_that_is_no_longer_there_is_said_rather_than_saved_as_nothing() {
    // A message can be fetched again between the listing and the asking. Saving an empty file, or
    // saving the wrong part, would both be worse than saying so.
    let parts = Parts::of(vec![("invoice.pdf".to_owned(), b"x".to_vec())]);
    let dialog = Dialog::default();

    assert!(matches!(
        saving(&parts, &dialog, 9),
        Err(UseCaseError::NoSuchPart)
    ));
    assert!(
        dialog.offered.lock().expect("not poisoned").is_empty(),
        "nothing is offered to the dialog"
    );
}

#[test]
fn a_moment_the_message_cannot_be_read_is_said_as_such_and_nothing_is_offered() {
    let parts = Parts {
        busy: true,
        ..Parts::default()
    };
    let dialog = Dialog::default();

    assert!(matches!(
        saving(&parts, &dialog, 0),
        Err(UseCaseError::TooBusyToRead)
    ));
    assert!(dialog.offered.lock().expect("not poisoned").is_empty());
}
