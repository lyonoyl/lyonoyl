use std::collections::HashMap;
use std::sync::Mutex;

use secrecy::SecretString;

use super::*;
use crate::ports::account_repository::AccountRepositoryError;
use crate::ports::credential::Credential;
use crate::ports::credential_store::{CredentialError, CredentialRef};
use crate::ports::mail_provider::{
    MailFetch, MailProviderError, MailSync, MailWatch, MailboxDiscovery,
};
use crate::ports::message_bodies::MessageBodyError;
use lyonoyl_domain::account::{Encryption, MailAccount, MailAccountSummary, ServerSettings};

fn account() -> MailAccount {
    MailAccount::new(
        MailAccountSummary::new("acc-1", "zeta@example.com", "Zeta", "imap").expect("well-formed"),
        ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid"),
        "zeta@example.com",
    )
    .expect("well-formed")
}

/// The accounts on this device: the one the message is in, or none at all.
struct Accounts {
    here: bool,
}

impl AccountRepository for Accounts {
    fn find(&self, _id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
        Ok(self.here.then(account))
    }
    fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
        unreachable!("passing a message on lists no accounts")
    }
    fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
        unreachable!("passing a message on does not ask who can send")
    }
    fn find_by_email(&self, _email: &str) -> Result<Option<String>, AccountRepositoryError> {
        unreachable!("passing a message on looks no account up by address")
    }
    fn save(&self, _account: &MailAccount) -> Result<(), AccountRepositoryError> {
        unreachable!("passing a message on stores no account")
    }
    fn delete(&self, _id: &str) -> Result<(), AccountRepositoryError> {
        unreachable!("passing a message on removes no account")
    }
    fn number_of(&self, _id: &str) -> Result<Option<i64>, AccountRepositoryError> {
        unreachable!("passing a message on does not ask which file the mail is in")
    }
    fn set_collapsed(&self, _id: &str, _collapsed: bool) -> Result<(), AccountRepositoryError> {
        unreachable!("passing a message on does not touch the sidebar")
    }
}

struct Secrets;

impl CredentialStore for Secrets {
    fn read(&self, _r: &CredentialRef) -> Result<SecretString, CredentialError> {
        Ok(SecretString::from("correct horse"))
    }
    fn save(&self, _r: &CredentialRef, _s: &SecretString) -> Result<(), CredentialError> {
        unreachable!("passing a message on stores no secret")
    }
    fn delete(&self, _r: &CredentialRef) -> Result<(), CredentialError> {
        unreachable!("passing a message on removes no secret")
    }
}

/// What the server was asked for: the mailbox, the generation and the number.
type Asked = std::sync::Arc<Mutex<Vec<(String, u32, u32)>>>;

/// A server holding one message, or out of reach, which remembers what it was asked for.
struct Server {
    answer: Result<Vec<u8>, MailProviderError>,
    asked: Asked,
}

impl Server {
    fn holding(raw: &[u8]) -> Self {
        Self {
            answer: Ok(raw.to_vec()),
            asked: Asked::default(),
        }
    }

    fn out_of_reach() -> Self {
        Self {
            answer: Err(MailProviderError::Offline),
            asked: Asked::default(),
        }
    }

    fn asked(&self) -> Vec<(String, u32, u32)> {
        self.asked.lock().expect("not poisoned").clone()
    }
}

struct Reading {
    answer: Result<Vec<u8>, MailProviderError>,
    asked: Asked,
}

impl MailFetch for Reading {
    fn whole_message(
        &self,
        mailbox: &str,
        uid_validity: UidValidity,
        uid: Uid,
    ) -> Result<Vec<u8>, MailProviderError> {
        self.asked.lock().expect("not poisoned").push((
            mailbox.to_owned(),
            uid_validity.get(),
            uid.get(),
        ));
        self.answer.clone()
    }
}

impl ProviderFactory for &Server {
    fn reading(
        &self,
        _account: &MailAccount,
        _credential: &Credential,
    ) -> Result<Box<dyn MailFetch>, MailProviderError> {
        Ok(Box::new(Reading {
            answer: self.answer.clone(),
            asked: self.asked.clone(),
        }))
    }
    fn discovery(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
        unreachable!("passing a message on does not list mailboxes")
    }
    fn sync(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailSync>, MailProviderError> {
        unreachable!("passing a message on does not synchronise")
    }
    fn watch(
        &self,
        _a: &MailAccount,
        _c: &Credential,
    ) -> Result<Box<dyn MailWatch>, MailProviderError> {
        unreachable!("passing a message on does not watch")
    }
}

/// The messages this device keeps, and what it was asked to keep.
#[derive(Default)]
struct Kept {
    held: Mutex<HashMap<u64, Vec<u8>>>,
    keeps: Mutex<Vec<u64>>,
    /// A store that answers nothing at all, for reading or for keeping.
    refuses: bool,
}

impl Kept {
    fn holding(raw: &[u8]) -> Self {
        Self {
            held: Mutex::new(HashMap::from([(message().get(), raw.to_vec())])),
            ..Self::default()
        }
    }

    fn keeps(&self) -> Vec<u64> {
        self.keeps.lock().expect("not poisoned").clone()
    }
}

impl MessageBodies for &Kept {
    fn held(&self, message: MessageId) -> Result<Option<Vec<u8>>, MessageBodyError> {
        if self.refuses {
            return Err(MessageBodyError::Unavailable);
        }
        Ok(self
            .held
            .lock()
            .expect("not poisoned")
            .get(&message.get())
            .cloned())
    }
    fn keep(&self, message: MessageId, raw: &[u8]) -> Result<(), MessageBodyError> {
        if self.refuses {
            return Err(MessageBodyError::Unavailable);
        }
        self.keeps.lock().expect("not poisoned").push(message.get());
        self.held
            .lock()
            .expect("not poisoned")
            .insert(message.get(), raw.to_vec());
        Ok(())
    }
    fn fill_in_missing(&self, _message: MessageId, _raw: &[u8]) -> Result<(), MessageBodyError> {
        unreachable!("passing a message on repairs nothing in the index")
    }
}

/// Reads a message for passing on as its bytes say, and remembers which bytes it was handed.
#[derive(Default)]
struct Parts {
    /// Every place a message is read in taken.
    busy: bool,
    read: Mutex<Vec<Vec<u8>>>,
}

impl Parts {
    fn read(&self) -> Vec<Vec<u8>> {
        self.read.lock().expect("not poisoned").clone()
    }
}

impl MessageParts for &Parts {
    fn file_at(&self, _raw: &[u8], _at: u32) -> Option<(String, Vec<u8>)> {
        unreachable!("passing a message on takes every file, not one")
    }
    fn passed_on(&self, raw: &[u8]) -> Option<PassedOn> {
        self.read.lock().expect("not poisoned").push(raw.to_vec());
        (!self.busy).then(|| PassedOn {
            text: String::from_utf8_lossy(raw).into_owned(),
            files: vec![
                ("invoice.pdf".to_owned(), b"%PDF".to_vec()),
                ("photo.jpg".to_owned(), b"\xff\xd8".to_vec()),
            ],
        })
    }
}

fn message() -> MessageId {
    MessageId::new(7).expect("non-zero")
}

/// The generation the index holds the message's number under.
fn generation() -> Option<UidValidity> {
    UidValidity::new(3).ok()
}

fn forwarding(
    server: &Server,
    kept: &Kept,
    parts: &Parts,
    uid_validity: Option<UidValidity>,
) -> Result<PassedOn, UseCaseError> {
    ForwardMessage::new(Accounts { here: true }, Secrets, server, kept, parts).execute(
        "acc-1",
        message(),
        "INBOX",
        uid_validity,
        Uid::new(42).expect("non-zero"),
    )
}

#[test]
fn a_message_held_here_is_passed_on_from_the_copy_and_no_server_is_asked() {
    // Somebody passing a message on has just read it, so it is here; asking its server again would
    // put a wait, and a failure, between the press and the window.
    let server = Server::holding(b"the server's copy");
    let kept = Kept::holding(b"the copy kept here");
    let parts = Parts::default();

    let passed = forwarding(&server, &kept, &parts, generation()).expect("it is here");

    assert_eq!(passed.text, "the copy kept here");
    assert_eq!(parts.read(), vec![b"the copy kept here".to_vec()]);
    assert!(
        server.asked().is_empty(),
        "the server was asked all the same"
    );
    assert!(
        kept.keeps().is_empty(),
        "a copy already here was kept again"
    );
}

#[test]
fn a_message_not_held_is_read_from_its_server_and_kept() {
    // The body had not arrived when Forward was pressed, or keeping it failed. Passed on from
    // nothing, the message would go without what it says and every file it carries.
    let server = Server::holding(b"the server's copy");
    let kept = Kept::default();
    let parts = Parts::default();

    let passed = forwarding(&server, &kept, &parts, generation()).expect("the server has it");

    assert_eq!(passed.text, "the server's copy");
    assert_eq!(server.asked(), vec![("INBOX".to_owned(), 3, 42)]);
    assert_eq!(
        kept.keeps(),
        vec![7],
        "what the server handed over was not kept"
    );
}

#[test]
fn a_store_that_will_not_answer_is_read_past_to_the_server() {
    // An unreadable store means the same as an empty one: passing a message on must not depend on
    // being able to keep a copy of it.
    let server = Server::holding(b"the server's copy");
    let kept = Kept {
        refuses: true,
        ..Kept::default()
    };
    let parts = Parts::default();

    let passed = forwarding(&server, &kept, &parts, generation()).expect("the server has it");

    assert_eq!(passed.text, "the server's copy");
    assert_eq!(server.asked().len(), 1);
}

#[test]
fn what_is_passed_on_is_what_the_message_carries_in_its_order() {
    let server = Server::holding(b"the server's copy");
    let kept = Kept::holding(b"the copy kept here");
    let parts = Parts::default();

    let passed = forwarding(&server, &kept, &parts, generation()).expect("it is here");

    assert_eq!(
        passed.files,
        vec![
            ("invoice.pdf".to_owned(), b"%PDF".to_vec()),
            ("photo.jpg".to_owned(), b"\xff\xd8".to_vec()),
        ]
    );
}

#[test]
fn a_number_with_no_generation_beside_it_is_not_read_by() {
    // Nothing can check such a number against the mailbox, and a mailbox that renumbered has given
    // it to somebody else's message — which would go out as this one.
    let server = Server::holding(b"somebody else's message");
    let kept = Kept::default();
    let parts = Parts::default();

    assert!(matches!(
        forwarding(&server, &kept, &parts, None),
        Err(UseCaseError::StorageCorrupted)
    ));
    assert!(server.asked().is_empty());
    assert!(
        parts.read().is_empty(),
        "something was passed on all the same"
    );
}

#[test]
fn a_server_out_of_reach_is_said_rather_than_passed_on_as_nothing() {
    let server = Server::out_of_reach();
    let kept = Kept::default();
    let parts = Parts::default();

    assert!(matches!(
        forwarding(&server, &kept, &parts, generation()),
        Err(UseCaseError::Offline)
    ));
    assert!(parts.read().is_empty());
}

#[test]
fn an_account_no_longer_here_is_said() {
    let server = Server::holding(b"the server's copy");
    let kept = Kept::default();
    let parts = Parts::default();

    let refused = ForwardMessage::new(Accounts { here: false }, Secrets, &server, &kept, &parts)
        .execute(
            "acc-1",
            message(),
            "INBOX",
            generation(),
            Uid::new(42).expect("non-zero"),
        );

    assert!(matches!(refused, Err(UseCaseError::UnknownAccount)));
    assert!(server.asked().is_empty());
}

#[test]
fn a_message_nowhere_could_read_is_said_rather_than_passed_on_empty() {
    // Passed on as nothing, it would go to somebody as a message saying nothing, under a subject
    // that says it is somebody else's.
    let server = Server::holding(b"the server's copy");
    let kept = Kept::holding(b"the copy kept here");
    let parts = Parts {
        busy: true,
        ..Parts::default()
    };

    assert!(matches!(
        forwarding(&server, &kept, &parts, generation()),
        Err(UseCaseError::TooBusyToRead)
    ));
}

fn passing_whole(
    server: &Server,
    kept: &Kept,
    parts: &Parts,
    subject: Option<&str>,
) -> Result<(String, Vec<u8>), UseCaseError> {
    ForwardMessage::new(Accounts { here: true }, Secrets, server, kept, parts).whole(
        "acc-1",
        message(),
        "INBOX",
        generation(),
        Uid::new(42).expect("non-zero"),
        subject,
    )
}

#[test]
fn a_message_passed_on_whole_is_the_copy_held_here_under_its_subject() {
    // The bytes as they were handed over, which nothing reads on the way: an exact copy is the
    // point of sending it as a file.
    let server = Server::holding(b"the server's copy");
    let kept = Kept::holding(b"the copy kept here");
    let parts = Parts::default();

    let (name, bytes) =
        passing_whole(&server, &kept, &parts, Some("About Tuesday")).expect("it is here");

    assert_eq!(name, "About Tuesday.eml");
    assert_eq!(bytes, b"the copy kept here".to_vec());
    assert!(
        server.asked().is_empty(),
        "the server was asked all the same"
    );
    assert!(
        parts.read().is_empty(),
        "the message was read for its parts"
    );
}

#[test]
fn a_message_passed_on_whole_is_read_from_its_server_when_it_is_not_here() {
    let server = Server::holding(b"the server's copy");
    let kept = Kept::default();
    let parts = Parts::default();

    let (_, bytes) =
        passing_whole(&server, &kept, &parts, Some("About Tuesday")).expect("the server has it");

    assert_eq!(bytes, b"the server's copy".to_vec());
    assert_eq!(server.asked(), vec![("INBOX".to_owned(), 3, 42)]);
    assert_eq!(kept.keeps(), vec![7]);
}

#[test]
fn a_message_with_no_subject_goes_whole_under_the_name_a_saved_one_gets() {
    let server = Server::holding(b"the server's copy");
    let kept = Kept::holding(b"the copy kept here");
    let parts = Parts::default();

    let (name, _) = passing_whole(&server, &kept, &parts, None).expect("it is here");

    assert_eq!(name, "message.eml");
}

#[test]
fn a_message_passed_on_whole_from_a_server_out_of_reach_is_said() {
    let server = Server::out_of_reach();
    let kept = Kept::default();
    let parts = Parts::default();

    assert!(matches!(
        passing_whole(&server, &kept, &parts, Some("About Tuesday")),
        Err(UseCaseError::Offline)
    ));
}
