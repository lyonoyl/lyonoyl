//! Where secrets are kept: the system credential store — Keychain on macOS, Credential Manager on
//! Windows, Secret Service on Linux — or, on a Linux desktop whose first start found none, a file
//! opened with a master password.
//!
//! Nothing else in the application is allowed to hold a password at rest. What the database keeps
//! is the name of an entry here, which is useless without that store.

// Reached from the composition root once an account can be added.
#![allow(dead_code)]

pub mod cache;
pub mod database_key;
#[cfg(any(test, feature = "screenshots"))]
pub mod in_memory;
pub mod master_password;
pub mod message_key;
pub mod vault;

use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

use crate::infrastructure::secure_storage::master_password::{
    MasterPasswordStore, SecretStoreState,
};
use crate::ports::credential_store::{CredentialError, CredentialRef, CredentialStore};

/// The name the entries are filed under. Visible to the user in Keychain Access and its
/// equivalents, so it says what it is.
const SERVICE: &str = "Lyonoyl Mail";

/// How long a read waits for the system store before treating it as unavailable.
///
/// Long enough for someone who sees a dialog to answer it, short enough that a dialog nobody sees
/// does not stop mail arriving. `LYONOYL_KEYCHAIN_WAIT_SECS` lengthens it for the one-off routine
/// that rewrites stored secrets, where a person is deliberately sitting there answering.
///
/// The start waits this long for the key the databases are written under before the window comes
/// up, and a store that answers after that is still listened to: see
/// [`SecretStore::waiting_for_an_answer`].
pub fn answer_within() -> std::time::Duration {
    std::env::var("LYONOYL_KEYCHAIN_WAIT_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or(DEFAULT_ANSWER_WITHIN)
}

const DEFAULT_ANSWER_WITHIN: std::time::Duration = std::time::Duration::from_secs(20);

/// The file a device with no system store keeps its secrets in, beside the databases.
pub const SECRETS_FILE: &str = "secrets.vault";

/// The entry the probe below asks about. Never written, so a store that is there can only answer
/// that it has no such thing.
const PROBE: &str = "probe:is-there-a-store";

/// Where this device keeps its secrets: a store of the system's own, or — on a Linux desktop with no
/// Secret Service — a file opened with a master password. Which of the two is decided at the start:
/// by what is already beside the databases, and on a device with nothing there yet by whether a
/// keyring answers.
#[derive(Clone)]
pub enum SecretStore {
    System(SystemCredentialStore),
    Master(Arc<MasterPasswordStore>),
    /// The build made for screenshots, and only that build: what it keeps is the made-up password
    /// of a made-up account and the key to databases it makes again on every start, held in memory
    /// for as long as it runs. The system's store is never asked, so a build that plays a scene
    /// leaves no entry behind and raises no dialog.
    #[cfg(feature = "screenshots")]
    Scene(in_memory::InMemory),
}

impl SecretStore {
    pub fn state(&self) -> SecretStoreState {
        match self {
            Self::System(_) => SecretStoreState::System,
            Self::Master(store) => store.state(),
            // A store that is always there and never asks for anything, as the system's own is.
            #[cfg(feature = "screenshots")]
            Self::Scene(_) => SecretStoreState::System,
        }
    }

    /// The file, when this device has one. Nothing to open on a system with a store of its own.
    pub fn master_password(&self) -> Option<&MasterPasswordStore> {
        match self {
            Self::System(_) => None,
            Self::Master(store) => Some(store.as_ref()),
            #[cfg(feature = "screenshots")]
            Self::Scene(_) => None,
        }
    }

    /// The same store, asked in a way that waits for its answer for as long as that takes.
    ///
    /// For the start, which waits on the key the databases are written under in a thread of its
    /// own and stops waiting itself. The keychain asks a person before it hands its item to a build
    /// the item's access list does not name, and their answer can come after the start stopped
    /// waiting: it is still the key, and a read that had already given up would have thrown it
    /// away — leaving the window saying the key could not be had to somebody who had just allowed
    /// it.
    pub fn waiting_for_an_answer(&self) -> Self {
        match self {
            Self::System(_) => Self::System(SystemCredentialStore { patient: true }),
            Self::Master(store) => Self::Master(Arc::clone(store)),
            #[cfg(feature = "screenshots")]
            Self::Scene(store) => Self::Scene(store.clone()),
        }
    }
}

impl CredentialStore for SecretStore {
    fn save(
        &self,
        reference: &CredentialRef,
        secret: &SecretString,
    ) -> Result<(), CredentialError> {
        match self {
            Self::System(store) => store.save(reference, secret),
            Self::Master(store) => store.save(reference, secret),
            #[cfg(feature = "screenshots")]
            Self::Scene(store) => store.save(reference, secret),
        }
    }

    fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        match self {
            Self::System(store) => store.read(reference),
            Self::Master(store) => store.read(reference),
            #[cfg(feature = "screenshots")]
            Self::Scene(store) => store.read(reference),
        }
    }

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        match self {
            Self::System(store) => store.delete(reference),
            Self::Master(store) => store.delete(reference),
            #[cfg(feature = "screenshots")]
            Self::Scene(store) => store.delete(reference),
        }
    }
}

/// Whether the system store is there at all, asked by reading an entry that was never written.
///
/// Asked only where a system may have no store, and only before anything is written: after that,
/// what is beside the databases says where their key is.
pub fn system_store_answers() -> bool {
    answered(&keyring::Entry::new(SERVICE, PROBE).and_then(|entry| entry.get_password()))
}

/// The rule apart from the call, so the three platforms can all run it as a test.
///
/// "There is no such entry" is a store answering. So is a secret coming back. What says there is no
/// store are the two that mean nothing could be reached: no store configured for this build, and a
/// platform that failed to answer at all — on Linux, no Secret Service on the session bus.
///
/// A store that exists and is locked answers `NoStorageAccess`, and that is deliberately **not**
/// counted as absent: asking such a person for a master password would put their secrets in a file
/// while their system store sits there, which is the confusion the decision forbids.
fn answered(answer: &Result<String, keyring::Error>) -> bool {
    !matches!(
        answer,
        Err(keyring::Error::NoDefaultStore) | Err(keyring::Error::PlatformFailure(_))
    )
}

#[derive(Clone)]
pub struct SystemCredentialStore {
    /// Whether a read waits for as long as the store takes to answer, rather than giving up.
    patient: bool,
}

impl SystemCredentialStore {
    pub fn new() -> Self {
        Self { patient: false }
    }

    /// How long a read waits before treating the store as unavailable, or nothing for a read that
    /// waits for as long as it takes.
    fn gives_up_after(&self) -> Option<std::time::Duration> {
        (!self.patient).then(answer_within)
    }
}

impl Default for SystemCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialStore for SystemCredentialStore {
    fn save(
        &self,
        reference: &CredentialRef,
        secret: &SecretString,
    ) -> Result<(), CredentialError> {
        platform::save(SERVICE, &entry_name(reference), secret.expose_secret())
    }

    /// Reads a secret, giving up if the system store does not answer.
    ///
    /// The store can stop answering: a keychain item whose access list does not name this build
    /// puts a dialog on screen and waits for a person, and that dialog can be behind another window
    /// or on another desktop. A read that waits for ever takes the whole synchronisation round with
    /// it — mail stops arriving for every account, with nothing on screen to explain it.
    ///
    /// Giving up leaves that account unread until the next round, which is a state the interface
    /// can already show. The thread stays blocked until the dialog is answered; there is one of
    /// them per account at worst, and answering the dialog releases it.
    ///
    /// The store the start reads the databases' key from does not give up: what it is waiting for
    /// is the only thing that opens them. See [`SecretStore::waiting_for_an_answer`].
    fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        let name = entry_name(reference);

        answered_within(self.gives_up_after(), move || {
            platform::read(SERVICE, &name)
        })
        .map(SecretString::from)
    }

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        platform::delete(SERVICE, &entry_name(reference))
    }
}

/// What a read answers, given up on once `deadline` has passed, or waited for as long as it takes
/// when there is none.
///
/// A read given up on goes on in a thread of its own, blocked until somebody answers the dialog it
/// is waiting on. Nobody is listening by then, and the secret is dropped with the message.
fn answered_within(
    deadline: Option<std::time::Duration>,
    read: impl FnOnce() -> Result<String, CredentialError> + Send + 'static,
) -> Result<String, CredentialError> {
    let Some(deadline) = deadline else {
        return read();
    };
    let (answer, wait) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let _ = answer.send(read());
    });

    wait.recv_timeout(deadline)
        .unwrap_or(Err(CredentialError::Unavailable))
}

/// The keychain on macOS.
///
/// Not through `keyring` here, deliberately. It writes with `SecItemAdd`, which stores the secret
/// with no list of applications allowed to read it — so macOS asks the person every single time,
/// however the application is signed and whether or not the keychain is unlocked. The older
/// `SecKeychainAddGenericPassword` records the application that created the item, and reading back
/// what you wrote yourself is then silent, which is the arrangement a keychain exists for.
///
/// An item written the other way keeps its empty list when it is modified, so a secret stored by an
/// earlier build has to be removed and added again rather than updated.
#[cfg(target_os = "macos")]
mod platform {
    use secrecy::zeroize::Zeroize;
    use security_framework::base::Error as KeychainError;
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};
    use security_framework::os::macos::keychain::SecKeychain;
    use security_framework::os::macos::keychain_item::SecKeychainItem;

    use crate::ports::credential_store::CredentialError;

    /// What macOS answers when there is no such item.
    const NOT_FOUND: i32 = -25300;

    fn keychain() -> Result<SecKeychain, CredentialError> {
        SecKeychain::default().map_err(map_error)
    }

    pub fn read(service: &str, account: &str) -> Result<String, CredentialError> {
        match keychain()?.find_generic_password(service, account) {
            Ok((password, _item)) => String::from_utf8(password.as_ref().to_vec())
                .map_err(|_| CredentialError::Unavailable),
            Err(error) if error.code() == NOT_FOUND => read_and_move(service, account),
            Err(error) => Err(map_error(error)),
        }
    }

    /// Reads a secret an earlier build stored, and puts it where this one keeps them.
    ///
    /// `SecItemAdd` and `SecKeychainAddGenericPassword` write items the other's search does not
    /// find, so without this an upgrade looks exactly like a lost password. Read once through the
    /// old path — the last time that item makes the system ask about it — and written back through
    /// the new one.
    ///
    /// **Until the new copy is written, the old item is the only one**, and for the key the
    /// databases are written under, losing it is losing the mail. So it goes only once the write
    /// has succeeded, and by the reference to the item that was read rather than by its name: a
    /// delete by name after the write could take the copy just made with it. A write that fails
    /// leaves the old item where it was, the secret is returned all the same, and the next read
    /// tries the move again.
    fn read_and_move(service: &str, account: &str) -> Result<String, CredentialError> {
        let found = security_framework::passwords::get_generic_password(service, account)
            .map_err(map_error)?;
        let secret = String::from_utf8(found).map_err(|_| CredentialError::Unavailable)?;
        // Looked for before anything is written, so it can only be the item just read.
        let read_from = the_item_written_the_other_way(service, account);

        Ok(moved(
            secret,
            read_from,
            |secret| save(service, account, secret),
            SecKeychainItem::delete,
        ))
    }

    /// The item an earlier build wrote, by reference, to be taken out once its secret is safe in
    /// the new one. Nothing when the search for it finds nothing: the old item then stays.
    fn the_item_written_the_other_way(service: &str, account: &str) -> Option<SecKeychainItem> {
        ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(service)
            .account(account)
            .load_refs(true)
            .limit(Limit::Max(1))
            .search()
            .ok()?
            .into_iter()
            .find_map(|found| match found {
                SearchResult::Ref(Reference::KeychainItem(item)) => Some(item),
                _ => None,
            })
    }

    /// The order a move keeps, apart from the keychain so a test can hold it: the new copy is
    /// written first, the item that was read is taken out only once that write has succeeded, and
    /// the secret comes back whichever way the write went.
    fn moved<Item>(
        secret: String,
        read_from: Option<Item>,
        write: impl FnOnce(&str) -> Result<(), CredentialError>,
        take_out: impl FnOnce(Item),
    ) -> String {
        if write(&secret).is_ok() {
            if let Some(item) = read_from {
                take_out(item);
            }
        }

        secret
    }

    pub fn save(service: &str, account: &str, secret: &str) -> Result<(), CredentialError> {
        let keychain = keychain()?;

        // Removed and added rather than modified: modifying keeps whatever access list the item
        // already had, and an item written by an earlier build has none.
        if let Ok((_, item)) = keychain.find_generic_password(service, account) {
            item.delete();
        }

        keychain
            .add_generic_password(service, account, secret.as_bytes())
            .map_err(map_error)
    }

    pub fn delete(service: &str, account: &str) -> Result<(), CredentialError> {
        // Both places: an item an earlier build wrote is invisible to the search below, and a
        // secret left behind after its account is gone is one nobody can find or remove.
        let _ = security_framework::passwords::delete_generic_password(service, account);

        let keychain = keychain()?;
        match keychain.find_generic_password(service, account) {
            Ok((_, item)) => item.delete(),
            // Already gone is the state the caller asked for.
            Err(error) if error.code() == NOT_FOUND => {}
            Err(error) => return Err(map_error(error)),
        }

        // Looked for again in both places rather than taken on trust: deleting an item reports
        // nothing back, and a person who answers the system's question with Deny leaves it where
        // it was.
        gone(keychain.find_generic_password(service, account).map(drop))?;
        gone(
            security_framework::passwords::get_generic_password(service, account)
                .map(|mut found| found.zeroize()),
        )
    }

    /// What a search made after a delete says: nothing found is gone, and anything else, the item
    /// or no answer at all, is a secret that may still be there.
    fn gone(found: Result<(), KeychainError>) -> Result<(), CredentialError> {
        match found {
            Err(error) if error.code() == NOT_FOUND => Ok(()),
            _ => Err(CredentialError::Unavailable),
        }
    }

    /// A locked, absent or refused store is one situation for the user: the secret cannot be
    /// reached right now. Which of the three it was belongs in a log, not in a type the interface
    /// branches on.
    fn map_error(error: KeychainError) -> CredentialError {
        if error.code() == NOT_FOUND {
            CredentialError::NotFound
        } else {
            CredentialError::Unavailable
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn only_nothing_found_after_a_delete_counts_as_gone() {
            // Deleting reports nothing back, so the search after it is the whole answer. The item
            // still there is a person who pressed Deny, and a keychain that would not answer says
            // nothing about whether it is there.
            assert_eq!(gone(Err(KeychainError::from_code(NOT_FOUND))), Ok(()));
            assert_eq!(gone(Ok(())), Err(CredentialError::Unavailable));
            assert_eq!(
                gone(Err(KeychainError::from_code(-25293))),
                Err(CredentialError::Unavailable)
            );
        }

        /// What a move did, in the order it did it.
        type Done = std::cell::RefCell<Vec<String>>;

        #[test]
        fn a_moved_secret_is_written_before_the_item_it_was_read_from_goes() {
            let done = Done::default();

            let answered = moved(
                "the key".to_owned(),
                Some("the old item"),
                |secret| {
                    done.borrow_mut().push(format!("wrote {secret}"));
                    Ok(())
                },
                |item| done.borrow_mut().push(format!("took out {item}")),
            );

            assert_eq!(answered, "the key");
            assert_eq!(*done.borrow(), ["wrote the key", "took out the old item"]);
        }

        #[test]
        fn a_move_whose_write_fails_keeps_the_old_item_and_still_answers_the_secret() {
            // The old item is the only copy there is. For the key the databases are written
            // under, taking it out first and then failing to write is losing the mail.
            let done = Done::default();

            let answered = moved(
                "the key".to_owned(),
                Some("the old item"),
                |_| {
                    done.borrow_mut().push("the write was refused".to_owned());
                    Err(CredentialError::Unavailable)
                },
                |item| done.borrow_mut().push(format!("took out {item}")),
            );

            assert_eq!(answered, "the key");
            assert_eq!(*done.borrow(), ["the write was refused"]);
        }
    }
}

/// The Credential Manager on Windows and the Secret Service on Linux, through `keyring`.
///
/// Neither binds an entry to the application that wrote it, so neither has the problem the macOS
/// module above exists for.
#[cfg(not(target_os = "macos"))]
mod platform {

    use keyring::Entry;

    use crate::ports::credential_store::CredentialError;

    pub fn read(service: &str, account: &str) -> Result<String, CredentialError> {
        Entry::new(service, account)
            .map_err(map_error)?
            .get_password()
            .map_err(map_error)
    }

    pub fn save(service: &str, account: &str, secret: &str) -> Result<(), CredentialError> {
        Entry::new(service, account)
            .map_err(map_error)?
            .set_password(secret)
            .map_err(map_error)
    }

    pub fn delete(service: &str, account: &str) -> Result<(), CredentialError> {
        match Entry::new(service, account)
            .map_err(map_error)?
            .delete_credential()
        {
            Ok(()) => Ok(()),
            // Already gone is the state the caller asked for.
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(map_error(error)),
        }
    }

    fn map_error(error: keyring::Error) -> CredentialError {
        match error {
            keyring::Error::NoEntry => CredentialError::NotFound,
            _ => CredentialError::Unavailable,
        }
    }
}

/// The name one entry is filed under.
///
/// Account identifier first so that every secret of one account sits together in the list a person
/// sees in their system keychain.
fn entry_name(reference: &CredentialRef) -> String {
    format!("{}:{}", reference.account_id, reference.purpose.as_str())
}

/// A locked, absent or refused store is one situation for the user: the secret cannot be reached
/// right now. Which of the three it was belongs in a log, not in a type the interface branches on.
fn map_error(error: keyring::Error) -> CredentialError {
    match error {
        keyring::Error::NoEntry => CredentialError::NotFound,
        _ => CredentialError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::credential_store::CredentialPurpose;

    fn reference(account: &str, purpose: CredentialPurpose) -> CredentialRef {
        CredentialRef::new(account, purpose)
    }

    #[test]
    fn one_account_keeps_its_secrets_together_and_apart() {
        let imap = entry_name(&reference("acc-1", CredentialPurpose::ImapPassword));
        let smtp = entry_name(&reference("acc-1", CredentialPurpose::SmtpPassword));
        let other = entry_name(&reference("acc-2", CredentialPurpose::ImapPassword));

        assert_eq!(imap, "acc-1:imap-password");
        assert_ne!(imap, smtp, "two secrets of one account must not collide");
        assert_ne!(imap, other, "two accounts must not share an entry");
    }

    #[test]
    fn purpose_names_are_stored_and_must_not_be_renamed() {
        // The name is part of the entry a user already has in their keychain; renaming it loses
        // the password rather than moving it.
        assert_eq!(CredentialPurpose::ImapPassword.as_str(), "imap-password");
        assert_eq!(CredentialPurpose::SmtpPassword.as_str(), "smtp-password");
        assert_eq!(
            CredentialPurpose::OAuthRefreshToken.as_str(),
            "oauth-refresh-token"
        );
    }

    #[test]
    fn opening_the_file_opens_it_for_everything_that_shares_the_store() {
        // The rounds run with their own handle on the store, and the window opens it. Two separate
        // stores would mean typing the master password changed nothing for the mail: the rounds
        // would go on finding it closed until the next start.
        let directory = std::env::temp_dir().join(format!("lyonoyl-shared-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("a directory to write in");

        let window = SecretStore::Master(Arc::new(
            crate::infrastructure::secure_storage::master_password::MasterPasswordStore::quick(
                directory.join(SECRETS_FILE),
            ),
        ));
        let rounds = window.clone();

        assert_eq!(rounds.state(), SecretStoreState::NeedsChoosing);
        window
            .master_password()
            .expect("this one has a file")
            .choose(&SecretString::from("correct horse battery".to_owned()))
            .expect("the file is made");

        assert_eq!(rounds.state(), SecretStoreState::Unlocked);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_store_that_says_it_has_no_such_entry_is_a_store_that_is_there() {
        // What the Linux probe reads. The entry it asks about was never written, so "no such
        // entry" is the answer a working store gives.
        assert!(answered(&Err(keyring::Error::NoEntry)));
        assert!(answered(&Ok("a secret".to_owned())));
    }

    #[test]
    fn a_store_that_could_not_be_reached_at_all_is_one_that_is_not_there() {
        assert!(!answered(&Err(keyring::Error::NoDefaultStore)));
        assert!(!answered(&Err(keyring::Error::PlatformFailure(Box::new(
            std::io::Error::other("no session bus")
        )))));
    }

    #[test]
    fn a_store_that_is_there_but_locked_is_not_mistaken_for_an_absent_one() {
        // The confusion that is forbidden: a person whose keyring is merely locked must not be
        // asked to choose a master password and end up with their secrets in a file.
        assert!(answered(&Err(keyring::Error::NoStorageAccess(Box::new(
            std::io::Error::other("the collection is locked")
        )))));
        assert!(answered(&Err(keyring::Error::Ambiguous(Vec::new()))));
    }

    #[test]
    fn a_missing_entry_is_told_apart_from_an_unreachable_store() {
        assert_eq!(
            map_error(keyring::Error::NoEntry),
            CredentialError::NotFound
        );
        assert_eq!(
            map_error(keyring::Error::Ambiguous(Vec::new())),
            CredentialError::Unavailable
        );
    }

    #[test]
    #[ignore = "writes to the real system keychain; started explicitly"]
    fn a_secret_this_application_wrote_is_read_back_without_anyone_being_asked() {
        // The arrangement a keychain exists for: what an application stores, it can read. A build
        // that has to ask the person every time has not stored the secret, it has hidden it.
        let store = SystemCredentialStore::new();
        let reference = reference("silent-read-test", CredentialPurpose::ImapPassword);
        let secret = SecretString::from("correct horse battery staple".to_owned());

        store.save(&reference, &secret).expect("stored");

        let started = std::time::Instant::now();
        let read = store.read(&reference).expect("read back");
        let waited = started.elapsed();

        assert_eq!(read.expose_secret(), "correct horse battery staple");
        assert!(
            waited < std::time::Duration::from_secs(2),
            "reading took {waited:?}, which means the system stopped to ask"
        );

        store.delete(&reference).expect("removed");
    }

    #[test]
    #[ignore = "writes to the real system keychain; started explicitly"]
    fn a_secret_survives_a_round_trip_through_the_system_store() {
        let store = SystemCredentialStore::new();
        let reference = reference("round-trip-test", CredentialPurpose::ImapPassword);
        let secret = SecretString::from("correct horse battery staple".to_owned());

        store
            .save(&reference, &secret)
            .expect("the system store accepts a secret");

        let read = store.read(&reference).expect("and gives it back");
        assert_eq!(read.expose_secret(), "correct horse battery staple");

        store.delete(&reference).expect("and forgets it");
        assert_eq!(
            store.read(&reference).expect_err("it is gone"),
            CredentialError::NotFound
        );

        // Deleting what is already gone is not a failure.
        store.delete(&reference).expect("deleting twice is allowed");
    }

    #[test]
    fn a_store_that_does_not_answer_is_unavailable_rather_than_a_wait() {
        // The failure this is about: a keychain dialog behind another window stops mail arriving
        // for every account, with nothing on screen to explain it.
        let (answer, wait) = std::sync::mpsc::channel::<()>();
        let started = std::time::Instant::now();

        let outcome = answered_within(Some(std::time::Duration::from_millis(50)), move || {
            let _ = wait.recv();
            Ok("too late".to_owned())
        });

        assert_eq!(
            outcome,
            Err(CredentialError::Unavailable),
            "a store that says nothing must time out"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(1),
            "and it must give up rather than wait"
        );
        drop(answer);
    }

    #[test]
    fn a_read_with_no_deadline_is_simply_waited_for() {
        // Nothing stands between the caller and the store's answer: no thread to give up on, so
        // no answer to lose, however late a person gives it.
        let caller = std::thread::current().id();

        let answered = answered_within(None, move || {
            assert_eq!(
                std::thread::current().id(),
                caller,
                "asked on a thread of its own, which can be given up on"
            );
            Ok("the key".to_owned())
        });

        assert_eq!(answered, Ok("the key".to_owned()));
    }

    #[test]
    fn the_store_the_start_reads_the_key_from_waits_for_its_answer() {
        // The keychain asks a person before it hands its item to a build the item does not name.
        // The start reads the databases' key from this store, and an answer it had given up on
        // would leave the window saying the key could not be had to somebody who just allowed it.
        let SecretStore::System(patient) =
            SecretStore::System(SystemCredentialStore::new()).waiting_for_an_answer()
        else {
            panic!("the same kind of store");
        };

        assert!(
            SystemCredentialStore::new().gives_up_after().is_some(),
            "a round's read gives up, so a dialog nobody sees cannot stop mail arriving"
        );
        assert_eq!(
            patient.gives_up_after(),
            None,
            "the start's read gives up, and an answer given late is lost"
        );
    }

    #[test]
    fn the_wait_is_long_enough_for_a_person_and_short_enough_for_a_round() {
        assert!(DEFAULT_ANSWER_WITHIN >= std::time::Duration::from_secs(10));
        assert!(DEFAULT_ANSWER_WITHIN <= std::time::Duration::from_secs(30));
    }

    #[test]
    fn a_reference_carries_no_secret() {
        // It is written to the database and to logs, so it must stay boring.
        let printed = format!("{:?}", reference("acc-1", CredentialPurpose::ImapPassword));
        assert!(printed.contains("acc-1"));
        assert!(!printed.to_lowercase().contains("password="));
    }
}
