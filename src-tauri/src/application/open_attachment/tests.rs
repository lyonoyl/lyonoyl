use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use secrecy::SecretString;

use super::*;
use crate::ports::account_repository::AccountRepositoryError;
use crate::ports::credential::Credential;
use crate::ports::credential_store::{CredentialError, CredentialRef};
use crate::ports::file_opening::FileOpeningError;
use crate::ports::mail_provider::{
    MailFetch, MailProviderError, MailSync, MailWatch, MailboxDiscovery,
};
use crate::ports::message_bodies::MessageBodyError;
use crate::ports::message_parts::{FileTakenOut, NotReadNow, PassedOn};
use lyonoyl_domain::account::{Encryption, MailAccount, MailAccountSummary, ServerSettings};

const A_PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<< >>\nendobj\n";
const A_PROGRAM: &[u8] = b"MZ\x90\0\x03\0\0\0\x04\0\0\0";
const THE_MESSAGE: &[u8] = b"Subject: statement\r\n\r\nattached\r\n";

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

/// A server holding the message, which counts how often it was asked for the whole of it.
#[derive(Default)]
struct Server {
    asked: Arc<Mutex<u32>>,
}

struct Reading {
    asked: Arc<Mutex<u32>>,
}

impl MailFetch for Reading {
    fn whole_message(
        &self,
        _mailbox: &str,
        _uid_validity: UidValidity,
        _uid: Uid,
    ) -> Result<Vec<u8>, MailProviderError> {
        *self.asked.lock().expect("not poisoned") += 1;
        Ok(THE_MESSAGE.to_vec())
    }
}

impl ProviderFactory for &Server {
    fn reading(
        &self,
        _account: &MailAccount,
        _credential: &Credential,
    ) -> Result<Box<dyn MailFetch>, MailProviderError> {
        Ok(Box::new(Reading {
            asked: Arc::clone(&self.asked),
        }))
    }
    fn discovery(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
        unreachable!("opening a file does not list mailboxes")
    }
    fn sync(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailSync>, MailProviderError> {
        unreachable!("opening a file does not synchronise")
    }
    fn watch(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailWatch>, MailProviderError> {
        unreachable!("opening a file does not watch")
    }
}

/// The copies this device keeps, by message.
#[derive(Default)]
struct Kept {
    held: Mutex<HashMap<u64, Vec<u8>>>,
}

impl Kept {
    fn holding_the_message() -> Self {
        Self {
            held: Mutex::new(HashMap::from([(7, THE_MESSAGE.to_vec())])),
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

/// One file, at the third place in the message, under the name and with the bytes given, and a
/// message carried inside this one where it says so; or, busy, the message not read this time.
#[derive(Default)]
struct Parts {
    name: &'static str,
    bytes: &'static [u8],
    a_message: bool,
    busy: bool,
}

impl MessageParts for Parts {
    fn file_at(&self, raw: &[u8], at: u32) -> Result<Option<FileTakenOut>, NotReadNow> {
        assert_eq!(
            raw, THE_MESSAGE,
            "the file is taken out of the message read"
        );
        if self.busy {
            return Err(NotReadNow);
        }
        Ok((at == 2).then(|| FileTakenOut {
            name: self.name.to_owned(),
            bytes: self.bytes.to_vec(),
            a_message: self.a_message,
        }))
    }
    fn passed_on(&self, _raw: &[u8]) -> Option<PassedOn> {
        unreachable!("opening a file passes nothing on")
    }
}

/// What the system was asked to open, and what it answers.
#[derive(Default)]
struct System {
    opened: Mutex<Vec<(String, Vec<u8>)>>,
    answers: Option<FileOpeningError>,
}

impl FileOpening for System {
    fn open(&self, file: FileToOpen<'_>) -> Result<(), FileOpeningError> {
        self.opened
            .lock()
            .expect("not poisoned")
            .push((file.name.to_owned(), file.bytes.to_vec()));
        self.answers.map_or(Ok(()), Err)
    }
}

fn the_message() -> MessageId {
    MessageId::new(7).expect("non-zero")
}

fn opening(
    server: &Server,
    kept: &Kept,
    parts: Parts,
    system: &System,
    at: u32,
) -> Result<(), UseCaseError> {
    OpenAttachment::new(Accounts, Secrets, server, kept, parts, system).execute(
        "acc-1",
        the_message(),
        "INBOX",
        Some(UidValidity::new(1).expect("non-zero")),
        Uid::new(42).expect("non-zero"),
        at,
    )
}

#[test]
fn a_pdf_is_handed_to_the_system_under_its_name_with_its_bytes() {
    let system = System::default();
    let parts = Parts {
        name: "Statement.pdf",
        bytes: A_PDF,
        ..Parts::default()
    };

    opening(
        &Server::default(),
        &Kept::holding_the_message(),
        parts,
        &system,
        2,
    )
    .expect("opened");

    assert_eq!(
        *system.opened.lock().expect("not poisoned"),
        vec![("Statement.pdf".to_owned(), A_PDF.to_vec())]
    );
}

#[test]
fn a_file_is_named_for_what_its_bytes_are_whatever_the_message_called_it() {
    // The system picks the program by the ending: a PDF called a program opens as a PDF.
    let system = System::default();
    let parts = Parts {
        name: "invoice.exe",
        bytes: A_PDF,
        ..Parts::default()
    };

    opening(
        &Server::default(),
        &Kept::holding_the_message(),
        parts,
        &system,
        2,
    )
    .expect("opened");

    assert_eq!(
        system.opened.lock().expect("not poisoned")[0].0,
        "invoice.exe.pdf"
    );
}

#[test]
fn a_program_called_a_pdf_is_not_opened_and_nothing_is_written() {
    let system = System::default();
    let parts = Parts {
        name: "Statement.pdf",
        bytes: A_PROGRAM,
        ..Parts::default()
    };

    let answer = opening(
        &Server::default(),
        &Kept::holding_the_message(),
        parts,
        &system,
        2,
    );

    assert!(
        matches!(answer, Err(UseCaseError::FileNotOpened)),
        "{answer:?}"
    );
    assert!(
        system.opened.lock().expect("not poisoned").is_empty(),
        "the system was asked about a file that does not open"
    );
}

#[test]
fn a_message_carried_inside_is_not_opened_whatever_its_bytes_begin_with() {
    // Named `.eml`, as every message carried inside another is, it is saved as one.
    let system = System::default();
    let parts = Parts {
        name: "About Tuesday.eml",
        bytes: A_PDF,
        ..Parts::default()
    };

    let answer = opening(
        &Server::default(),
        &Kept::holding_the_message(),
        parts,
        &system,
        2,
    );

    assert!(
        matches!(answer, Err(UseCaseError::FileNotOpened)),
        "{answer:?}"
    );
    assert!(system.opened.lock().expect("not poisoned").is_empty());
}

#[test]
fn a_place_the_message_holds_no_file_in_is_said_and_nothing_is_opened() {
    let system = System::default();
    let parts = Parts {
        name: "Statement.pdf",
        bytes: A_PDF,
        ..Parts::default()
    };

    let answer = opening(
        &Server::default(),
        &Kept::holding_the_message(),
        parts,
        &system,
        5,
    );

    assert!(
        matches!(answer, Err(UseCaseError::NoSuchPart)),
        "{answer:?}"
    );
    assert!(system.opened.lock().expect("not poisoned").is_empty());
}

#[test]
fn a_message_this_device_keeps_opens_its_file_without_its_server() {
    let server = Server::default();
    let parts = Parts {
        name: "Statement.pdf",
        bytes: A_PDF,
        ..Parts::default()
    };

    opening(
        &server,
        &Kept::holding_the_message(),
        parts,
        &System::default(),
        2,
    )
    .expect("opened");

    assert_eq!(*server.asked.lock().expect("not poisoned"), 0);
}

#[test]
fn a_message_this_device_does_not_keep_is_read_from_its_server_and_kept() {
    let server = Server::default();
    let kept = Kept::default();
    let parts = Parts {
        name: "Statement.pdf",
        bytes: A_PDF,
        ..Parts::default()
    };

    opening(&server, &kept, parts, &System::default(), 2).expect("opened");

    assert_eq!(*server.asked.lock().expect("not poisoned"), 1);
    assert_eq!(
        kept.held
            .lock()
            .expect("not poisoned")
            .get(&7)
            .map(Vec::as_slice),
        Some(THE_MESSAGE),
        "kept, so the next file opens from here"
    );
}

#[test]
fn a_copy_not_written_or_not_opened_is_said_as_a_file_not_opened() {
    for refusal in [FileOpeningError::NotWritten, FileOpeningError::NotOpened] {
        let system = System {
            answers: Some(refusal),
            ..System::default()
        };
        let parts = Parts {
            name: "Statement.pdf",
            bytes: A_PDF,
            ..Parts::default()
        };

        let answer = opening(
            &Server::default(),
            &Kept::holding_the_message(),
            parts,
            &system,
            2,
        );

        assert!(
            matches!(answer, Err(UseCaseError::FileNotOpened)),
            "{refusal:?}: {answer:?}"
        );
    }
}

#[test]
fn a_moment_when_every_place_a_message_is_read_in_is_taken_is_said_as_such() {
    let system = System::default();
    let parts = Parts {
        busy: true,
        ..Parts::default()
    };

    let answer = opening(
        &Server::default(),
        &Kept::holding_the_message(),
        parts,
        &system,
        2,
    );

    assert!(
        matches!(answer, Err(UseCaseError::TooBusyToRead)),
        "{answer:?}"
    );
    assert!(system.opened.lock().expect("not poisoned").is_empty());
}

#[test]
fn a_message_carried_inside_is_not_opened_whatever_it_is_called() {
    // Told by what it is, not by its name: a message named `scan.pdf` is saved as a message.
    let system = System::default();
    let parts = Parts {
        name: "scan.pdf",
        bytes: A_PDF,
        a_message: true,
        ..Parts::default()
    };

    let answer = opening(
        &Server::default(),
        &Kept::holding_the_message(),
        parts,
        &system,
        2,
    );

    assert!(
        matches!(answer, Err(UseCaseError::FileNotOpened)),
        "{answer:?}"
    );
    assert!(system.opened.lock().expect("not poisoned").is_empty());
}
