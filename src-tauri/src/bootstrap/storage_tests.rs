//! Where the key the databases are written under is kept, and opening them once it can be had.
//!
//! A device whose secrets are in a file of its own can read that key only once a person opens the
//! file, which is after the start. These walk such a device through a first start and the next
//! one, with a real file and real databases in a folder of their own — and check that the store a
//! device keeps its secrets in is the one chosen by the first start that wrote anything, whichever
//! answers later.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use secrecy::SecretString;

use super::{
    in_a_file, init_storage, open_storage, opened_with_the_key_in, opened_within,
    set_aside_what_will_not_open, the_key_of, with_its_key, Storage, Unopened,
};
use crate::commands::error::ErrorCode;
use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::infrastructure::secure_storage::master_password::MasterPasswordStore;
use crate::infrastructure::secure_storage::{SecretStore, SECRETS_FILE};
use crate::ports::credential_store::{CredentialError, CredentialRef, CredentialStore};

/// A folder of its own, taken away afterwards.
struct Folder(std::path::PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "lyonoyl-storage-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).expect("a folder");
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A device with no store of its own: its secrets in a file in `folder`, where the start looks
/// for one, and closed, as every start finds it.
fn a_file_of_secrets(folder: &Folder) -> SecretStore {
    SecretStore::Master(Arc::new(MasterPasswordStore::quick(
        folder.0.join(SECRETS_FILE),
    )))
}

fn master() -> SecretString {
    SecretString::from("correct horse battery staple".to_owned())
}

/// What the start makes of the databases on that device. A file of secrets answers at once, closed,
/// so nothing is left on its way.
fn as_the_start_opens_them(folder: &Folder, secrets: &SecretStore) -> Storage {
    let (storage, coming) = open_storage(&folder.0, secrets);
    assert!(
        coming.is_none(),
        "a closed file of secrets left a key on its way"
    );

    storage
}

/// The databases a start on that device writes once it has their key.
fn databases_written(folder: &Folder) {
    drop(init_storage(&folder.0, &DatabaseKey::for_a_test()));
}

/// A keyring nobody should be asking about, because what is on the disk already answers.
fn never_asked() -> bool {
    panic!("a keyring was asked where the disk already says which store holds the key")
}

/// Opening them the way a person opening the file does.
fn open(storage: &Storage, folder: &Folder, secrets: &SecretStore) -> bool {
    storage.open_once_the_key_is_there(|| with_its_key(&folder.0, secrets))
}

fn code_of(storage: &Storage) -> Option<ErrorCode> {
    storage.profile().err().map(|error| error.code)
}

/// Something only these databases hold, written so a later start can look for it.
fn write_a_mark(storage: &Storage) {
    storage
        .profile()
        .expect("open")
        .get()
        .expect("a connection")
        .execute_batch("CREATE TABLE mark (word TEXT); INSERT INTO mark VALUES ('here');")
        .expect("written");
}

fn the_mark(storage: &Storage) -> Option<String> {
    storage
        .profile()
        .ok()?
        .get()
        .ok()?
        .query_row("SELECT word FROM mark", [], |row| row.get(0))
        .ok()
}

/// A system keyring that holds nothing, and counts what it is asked to keep.
#[derive(Default)]
struct AnEmptyKeyring {
    kept: AtomicUsize,
}

impl CredentialStore for AnEmptyKeyring {
    fn read(&self, _reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        Err(CredentialError::NotFound)
    }

    fn save(
        &self,
        _reference: &CredentialRef,
        _secret: &SecretString,
    ) -> Result<(), CredentialError> {
        self.kept.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn delete(&self, _reference: &CredentialRef) -> Result<(), CredentialError> {
        Ok(())
    }
}

/// A system store holding the key the test databases are written under, and counting what it is
/// asked to keep.
struct AKeyringHolding {
    key: String,
    kept: AtomicUsize,
}

impl AKeyringHolding {
    fn the_tests_key() -> Self {
        Self {
            // What `DatabaseKey::for_a_test` is made from, written the way a store holds a key.
            key: "5a".repeat(32),
            kept: AtomicUsize::new(0),
        }
    }
}

impl CredentialStore for AKeyringHolding {
    fn read(&self, _reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        Ok(SecretString::from(self.key.clone()))
    }

    fn save(
        &self,
        _reference: &CredentialRef,
        _secret: &SecretString,
    ) -> Result<(), CredentialError> {
        self.kept.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn delete(&self, _reference: &CredentialRef) -> Result<(), CredentialError> {
        Ok(())
    }
}

/// The keychain for a build its item does not name: it asks a person first, and hands the key over
/// only once they answer.
struct AKeychainAskingAPerson {
    answer: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    key: String,
}

impl AKeychainAskingAPerson {
    /// The keychain, and the person it asks.
    fn holding_the_tests_key() -> (Self, std::sync::mpsc::Sender<()>) {
        let (person, answer) = std::sync::mpsc::channel();
        let keychain = Self {
            answer: std::sync::Mutex::new(answer),
            key: AKeyringHolding::the_tests_key().key,
        };

        (keychain, person)
    }
}

impl CredentialStore for AKeychainAskingAPerson {
    fn read(&self, _reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        let answered = self.answer.lock().expect("the dialog").recv();
        answered.map_err(|_| CredentialError::Unavailable)?;

        Ok(SecretString::from(self.key.clone()))
    }

    fn save(
        &self,
        _reference: &CredentialRef,
        _secret: &SecretString,
    ) -> Result<(), CredentialError> {
        Ok(())
    }

    fn delete(&self, _reference: &CredentialRef) -> Result<(), CredentialError> {
        Ok(())
    }
}

/// A system keyring that is not running, and counts what it is asked to keep.
#[derive(Default)]
struct ASilentKeyring {
    kept: AtomicUsize,
}

impl CredentialStore for ASilentKeyring {
    fn read(&self, _reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        Err(CredentialError::Unavailable)
    }

    fn save(
        &self,
        _reference: &CredentialRef,
        _secret: &SecretString,
    ) -> Result<(), CredentialError> {
        self.kept.fetch_add(1, Ordering::SeqCst);
        Err(CredentialError::Unavailable)
    }

    fn delete(&self, _reference: &CredentialRef) -> Result<(), CredentialError> {
        Err(CredentialError::Unavailable)
    }
}

#[test]
fn a_device_that_has_written_nothing_keeps_its_secrets_in_a_keyring_that_answers() {
    let folder = Folder::new("nothing-written-keyring");
    let mut asked = 0;

    let file = in_a_file(&folder.0, true, || {
        asked += 1;
        true
    });

    assert!(!file);
    assert_eq!(asked, 1);
}

#[test]
fn a_device_that_has_written_nothing_and_has_no_keyring_keeps_them_in_a_file() {
    let folder = Folder::new("nothing-written-no-keyring");

    assert!(in_a_file(&folder.0, true, || false));
}

#[test]
fn a_file_a_first_start_made_stays_the_store_once_a_keyring_answers() {
    // The key the databases are written under is in that file. A keyring that answers on a later
    // start holds no key for them, and choosing it would leave whole files unread.
    let folder = Folder::new("file-then-keyring");
    assert!(in_a_file(&folder.0, true, || false));
    let secrets = a_file_of_secrets(&folder);
    let storage = as_the_start_opens_them(&folder, &secrets);
    secrets
        .master_password()
        .expect("a device with a file")
        .choose(&master())
        .expect("chosen");
    assert!(open(&storage, &folder, &secrets));

    assert!(
        in_a_file(&folder.0, true, never_asked),
        "a keyring that answered took over from the file its databases' key is in"
    );
}

#[test]
fn databases_with_no_file_beside_them_wait_for_their_keyring() {
    // Written under a key a keyring holds. A file chosen while that keyring is not running would
    // hold everything but that key.
    let folder = Folder::new("keyring-databases");
    databases_written(&folder);

    assert!(
        !in_a_file(&folder.0, true, never_asked),
        "a file was offered over databases a keyring holds the key to"
    );
}

#[test]
fn a_store_that_does_not_answer_is_not_given_a_key() {
    // Not answering is not losing it.
    let folder = Folder::new("store-silent");
    databases_written(&folder);
    let keyring = ASilentKeyring::default();

    assert_eq!(
        the_key_of(&folder.0, &keyring).err(),
        Some(CredentialError::Unavailable)
    );
    assert_eq!(keyring.kept.load(Ordering::SeqCst), 0);
}

#[test]
fn a_store_that_says_it_holds_no_key_over_its_databases_is_not_given_one() {
    // Whichever store it is. A Linux keyring answers that it holds nothing while a locked database
    // hides what it holds, and the macOS keychain answers the same for an item in a keychain that
    // has dropped out of the list it searches: a key made then is kept in front of the one the
    // databases were written under, and opens nothing.
    let folder = Folder::new("store-says-nothing");
    databases_written(&folder);
    let store = AnEmptyKeyring::default();

    assert_eq!(
        the_key_of(&folder.0, &store).err(),
        Some(CredentialError::NotFound)
    );
    assert_eq!(store.kept.load(Ordering::SeqCst), 0);
}

#[test]
fn a_store_always_there_that_holds_no_key_leaves_the_way_out_open_and_makes_none() {
    // The macOS keychain and Credential Manager. Nothing the application waits for brings back a
    // key they answer they do not hold, so the window shows files that will not open, which is the
    // screen with Start fresh on it — and still no key is made over the databases, where it would
    // be kept in front of the one they were written under.
    let folder = Folder::new("always-there-holds-none");
    databases_written(&folder);
    let store = AnEmptyKeyring::default();

    let storage = opened_with_the_key_in(&folder.0, &store, false, false);

    let refused = storage.profile().expect_err("opened with no key");
    assert_eq!(
        refused.code,
        ErrorCode::StorageUnavailable,
        "a screen that offers nothing, saying the key is where it has always been"
    );
    assert_eq!(
        refused.path,
        Some(folder.0.join("profile.sqlite3").display().to_string())
    );
    assert!(storage.may_be_set_aside().is_ok());
    assert_eq!(
        store.kept.load(Ordering::SeqCst),
        0,
        "a key was made over the databases"
    );

    // Start fresh, and the start after it: a key made over no databases, and a profile that opens.
    set_aside_what_will_not_open(&folder.0, 1).expect("set aside");
    let fresh = opened_with_the_key_in(&folder.0, &store, false, false);
    assert!(fresh.is_ready(), "{:?}", code_of(&fresh));
    assert_eq!(store.kept.load(Ordering::SeqCst), 1);
}

#[test]
fn a_store_that_may_hide_the_key_or_would_not_answer_is_waited_for() {
    // A Linux keyring can answer that it holds nothing while a locked database hides what it holds,
    // a file of secrets says what it holds on a screen of its own, and a store that would not answer
    // may answer at the next start. None of them is shown as files that will not open.
    let folder = Folder::new("waited-for");
    databases_written(&folder);
    let (empty, silent) = (AnEmptyKeyring::default(), ASilentKeyring::default());

    let stores: [(&dyn CredentialStore, bool, bool, &str); 4] = [
        (&empty, false, true, "a keyring that says it holds none"),
        (&empty, true, false, "a file that holds no key"),
        (&silent, false, false, "a store always there, silent"),
        (&silent, false, true, "a keyring that would not answer"),
    ];
    for (store, in_a_file, a_keyring_may_hide_it, which) in stores {
        let storage = opened_with_the_key_in(&folder.0, store, in_a_file, a_keyring_may_hide_it);

        assert_eq!(
            code_of(&storage),
            Some(ErrorCode::StorageKeyUnavailable),
            "{which}"
        );
    }
    assert_eq!(empty.kept.load(Ordering::SeqCst), 0);
    assert_eq!(silent.kept.load(Ordering::SeqCst), 0);
}

#[test]
fn a_store_holding_the_key_opens_the_databases_written_under_it_and_makes_none() {
    let folder = Folder::new("store-holds-it");
    databases_written(&folder);
    let store = AKeyringHolding::the_tests_key();

    let key = the_key_of(&folder.0, &store).expect("the key it holds");

    // One connection rather than the pool the start builds: what is asked is whether the key opens
    // the file.
    let profile = rusqlite::Connection::open(folder.0.join("profile.sqlite3")).expect("the file");
    key.unlock(&profile).expect("the key is taken");
    let tables: i64 = profile
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
        .expect("the key the store holds opens the profile");
    assert!(tables > 0, "an empty profile, so this proves nothing");
    assert_eq!(store.kept.load(Ordering::SeqCst), 0);
}

#[test]
fn a_key_handed_over_after_the_start_stopped_waiting_still_opens_the_databases() {
    // The keychain asks a person before it hands its item to a new build, and a person can take
    // longer to answer than the start waits. The window comes up saying the key could not be had;
    // the answer still arrives, and it is the key. Thrown away, the window would go on saying so
    // to somebody who had just allowed it, until they quit and started again.
    let folder = Folder::new("answered-late");
    databases_written(&folder);
    let (keychain, person) = AKeychainAskingAPerson::holding_the_tests_key();

    let (storage, coming) = opened_within(
        &folder.0,
        Arc::new(keychain),
        std::time::Duration::from_millis(50),
        false,
        false,
    );

    assert_eq!(code_of(&storage), Some(ErrorCode::StorageKeyUnavailable));
    let coming = coming.expect("the answer, still coming when the start stopped waiting");

    person.send(()).expect("the person answers");
    let key = coming.wait().expect("the key the person allowed");

    assert!(storage.open_once_the_key_is_there(|| Some(init_storage(&folder.0, &key))));
    assert!(storage.is_ready(), "{:?}", code_of(&storage));
}

#[test]
fn a_key_handed_over_in_time_opens_the_databases_and_leaves_nothing_coming() {
    let folder = Folder::new("answered-in-time");
    databases_written(&folder);

    let (storage, coming) = opened_within(
        &folder.0,
        Arc::new(AKeyringHolding::the_tests_key()),
        std::time::Duration::from_secs(5),
        false,
        false,
    );

    assert!(storage.is_ready(), "{:?}", code_of(&storage));
    assert!(coming.is_none());
}

#[test]
fn a_file_with_no_databases_beside_it_is_still_the_store() {
    // What Start fresh leaves on a device whose secrets are in a file: the file stays, and so does
    // the master password that opens it.
    let folder = Folder::new("file-set-aside");
    std::fs::write(folder.0.join(SECRETS_FILE), b"sealed").expect("a file");

    assert!(in_a_file(&folder.0, true, never_asked));
}

#[test]
fn a_system_with_a_store_of_its_own_never_keeps_them_in_a_file() {
    let folder = Folder::new("store-of-its-own");
    assert!(!in_a_file(&folder.0, false, never_asked));

    databases_written(&folder);
    std::fs::write(folder.0.join(SECRETS_FILE), b"sealed").expect("a file");
    assert!(!in_a_file(&folder.0, false, never_asked));
}

#[test]
fn a_device_with_nothing_written_has_its_key_made_wherever_its_secrets_are() {
    let folder = Folder::new("nothing-written-key");
    let keyring = AnEmptyKeyring::default();

    assert!(the_key_of(&folder.0, &keyring).is_ok());
    assert_eq!(keyring.kept.load(Ordering::SeqCst), 1);
}

#[test]
fn the_databases_wait_for_a_file_nobody_has_opened() {
    let folder = Folder::new("waiting");
    let secrets = a_file_of_secrets(&folder);
    let storage = as_the_start_opens_them(&folder, &secrets);

    assert_eq!(code_of(&storage), Some(ErrorCode::StorageKeyUnavailable));
    assert!(
        !open(&storage, &folder, &secrets),
        "a file nobody has opened opened the databases"
    );
    assert_eq!(code_of(&storage), Some(ErrorCode::StorageKeyUnavailable));
}

#[test]
fn choosing_the_master_password_opens_the_databases_on_a_first_start() {
    let folder = Folder::new("first-start");
    let secrets = a_file_of_secrets(&folder);
    let storage = as_the_start_opens_them(&folder, &secrets);

    secrets
        .master_password()
        .expect("a device with a file")
        .choose(&master())
        .expect("chosen");

    assert!(open(&storage, &folder, &secrets));
    write_a_mark(&storage);
    assert_eq!(the_mark(&storage).as_deref(), Some("here"));
}

#[test]
fn the_same_password_opens_the_same_databases_on_the_next_start() {
    let folder = Folder::new("next-start");
    {
        let secrets = a_file_of_secrets(&folder);
        let storage = as_the_start_opens_them(&folder, &secrets);
        secrets
            .master_password()
            .expect("a device with a file")
            .choose(&master())
            .expect("chosen");
        assert!(open(&storage, &folder, &secrets));
        write_a_mark(&storage);
    }

    // The next start: the file is there, and closed again.
    let secrets = a_file_of_secrets(&folder);
    let storage = as_the_start_opens_them(&folder, &secrets);
    assert_eq!(code_of(&storage), Some(ErrorCode::StorageKeyUnavailable));

    secrets
        .master_password()
        .expect("a device with a file")
        .unlock(&master())
        .expect("opened");

    assert!(open(&storage, &folder, &secrets));
    assert_eq!(
        the_mark(&storage).as_deref(),
        Some("here"),
        "what the first start wrote, read under the same key"
    );
}

#[test]
fn a_wrong_password_leaves_the_databases_waiting() {
    let folder = Folder::new("wrong-password");
    a_file_of_secrets(&folder)
        .master_password()
        .expect("a device with a file")
        .choose(&master())
        .expect("chosen");

    let secrets = a_file_of_secrets(&folder);
    let storage = as_the_start_opens_them(&folder, &secrets);
    let refused = secrets
        .master_password()
        .expect("a device with a file")
        .unlock(&SecretString::from("not the one".to_owned()));

    assert!(refused.is_err(), "a wrong password opened the file");
    assert!(!open(&storage, &folder, &secrets));
    assert_eq!(code_of(&storage), Some(ErrorCode::StorageKeyUnavailable));
}

#[test]
fn a_file_opened_over_databases_written_under_another_key_makes_no_key_of_its_own() {
    // Databases beside a file that holds no key for them. The file is the only store this device
    // reads, and a key made in it now would be kept there for good while opening nothing.
    let folder = Folder::new("another-key");
    databases_written(&folder);

    let secrets = a_file_of_secrets(&folder);
    let storage = as_the_start_opens_them(&folder, &secrets);
    secrets
        .master_password()
        .expect("a device with a file")
        .choose(&master())
        .expect("chosen");

    assert!(!open(&storage, &folder, &secrets));
    assert_eq!(
        code_of(&storage),
        Some(ErrorCode::StorageKeyUnavailable),
        "whole files were called broken"
    );
    assert!(
        DatabaseKey::already_kept_in(&secrets).is_err(),
        "a key the databases were not written under was kept in the file"
    );
}

#[test]
fn opening_twice_opens_once() {
    let folder = Folder::new("twice");
    let secrets = a_file_of_secrets(&folder);
    let storage = as_the_start_opens_them(&folder, &secrets);
    secrets
        .master_password()
        .expect("a device with a file")
        .choose(&master())
        .expect("chosen");
    assert!(open(&storage, &folder, &secrets));

    let mut asked = false;
    let again = storage.open_once_the_key_is_there(|| {
        asked = true;
        None
    });

    assert!(
        !again,
        "a second press would start what runs on the databases a second time"
    );
    assert!(!asked, "the databases were opened a second time");
    assert!(storage.is_ready());
}

#[test]
fn a_device_with_a_store_of_its_own_finds_nothing_waiting() {
    let folder = Folder::new("system-store");
    let storage = init_storage(&folder.0, &DatabaseKey::for_a_test());
    assert!(storage.is_ready());

    let mut asked = false;
    let opened = storage.open_once_the_key_is_there(|| {
        asked = true;
        None
    });

    assert!(!opened);
    assert!(!asked, "databases already open were opened again");
}

#[test]
fn a_file_that_will_not_open_is_not_opened_again_by_a_key() {
    let storage = Storage::failed(
        "profile.sqlite3".to_owned(),
        "file is not a database".to_owned(),
        Unopened::TheFile,
    );

    let mut asked = false;
    let opened = storage.open_once_the_key_is_there(|| {
        asked = true;
        None
    });

    assert!(!opened);
    assert!(
        !asked,
        "a damaged file was opened again on the strength of a key"
    );
    assert_eq!(code_of(&storage), Some(ErrorCode::StorageUnavailable));
}

#[test]
fn two_presses_at_once_open_the_databases_once() {
    let folder = Folder::new("at-once");
    let secrets = a_file_of_secrets(&folder);
    let storage = Arc::new(as_the_start_opens_them(&folder, &secrets));
    secrets
        .master_password()
        .expect("a device with a file")
        .choose(&master())
        .expect("chosen");

    let together = Arc::new(Barrier::new(2));
    let openings = Arc::new(AtomicUsize::new(0));
    let presses: Vec<_> = (0..2)
        .map(|_| {
            let storage = Arc::clone(&storage);
            let secrets = secrets.clone();
            let together = Arc::clone(&together);
            let openings = Arc::clone(&openings);
            let directory = folder.0.clone();

            std::thread::spawn(move || {
                together.wait();
                storage.open_once_the_key_is_there(|| {
                    openings.fetch_add(1, Ordering::SeqCst);
                    with_its_key(&directory, &secrets)
                })
            })
        })
        .collect();
    let answers: Vec<bool> = presses
        .into_iter()
        .map(|press| press.join().expect("a press"))
        .collect();

    assert_eq!(
        answers.iter().filter(|opened| **opened).count(),
        1,
        "{answers:?}"
    );
    assert_eq!(
        openings.load(Ordering::SeqCst),
        1,
        "the databases were opened twice"
    );
}
