pub mod adapters;

/// Asking a webview engine what it does, and writing the answer down. Debug builds only: it opens a
/// window, renders one message and stops.
#[cfg(debug_assertions)]
pub mod engine_probe;
pub mod window;

mod servers;
pub mod updates;
pub mod workspace_sync;
pub use servers::{mail_actions, mail_servers, mail_submission, opening};

/// The build made for screenshots: a profile of its own, filled from a scene, and mail read from
/// that scene rather than from any server. Compiled into that build and its tests, and out of every
/// other.
#[cfg(any(test, feature = "screenshots"))]
pub mod scene;
#[cfg(test)]
mod scene_tests;

mod gathering;
#[cfg(test)]
mod gathering_tests;

pub use gathering::{databases_of, gather_the_databases};
#[cfg(test)]
mod storage_tests;

use crate::infrastructure::html::serving;
use crate::infrastructure::schedule::{self, InFlight};
use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::infrastructure::secure_storage::{answer_within, SecretStore};
use crate::infrastructure::sqlite::indexes::Indexes;
use crate::ports::account_sync::AccountChanged;
use crate::ports::credential_store::{CredentialError, CredentialStore};
use workspace_sync::WorkspaceSync;

use std::path::Path;
use std::sync::Arc;

use crate::commands::error::{CommandError, ErrorCode};
use crate::infrastructure::sqlite::{
    connection::{open_profile, SqlitePool, StorageError},
    migrations::migrate_profile,
};

/// The folder the application keeps its data in, named the way each platform expects.
///
/// Named after the product rather than after the bundle identifier: a person opening their data
/// directory should recognise the folder without decoding a reverse-domain string. Linux keeps the
/// lowercase, hyphenated form its file managers and shells expect.
#[cfg(all(target_os = "linux", not(feature = "screenshots")))]
const DATA_DIRECTORY: &str = "lyonoyl-mail";
#[cfg(all(not(target_os = "linux"), not(feature = "screenshots")))]
const DATA_DIRECTORY: &str = "Lyonoyl Mail";

/// The build made for screenshots keeps its profile beside the real one, under a name nothing else
/// uses, and never opens the real one. What it writes there it throws away at its next start.
#[cfg(all(target_os = "linux", feature = "screenshots"))]
const DATA_DIRECTORY: &str = "lyonoyl-mail-screenshots";
#[cfg(all(not(target_os = "linux"), feature = "screenshots"))]
const DATA_DIRECTORY: &str = "Lyonoyl Mail Screenshots";

/// Resolves that folder. Failing to resolve it is fatal: there is nowhere to keep anything.
///
/// Windows uses the local application data folder rather than the roaming one: the mail index
/// reaches gigabytes, and a roaming profile would copy all of it over the network at every sign-in.
/// macOS gets `~/Library/Application Support`, Linux gets `$XDG_DATA_HOME`.
pub fn data_directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, tauri::Error> {
    use tauri::Manager;

    #[cfg(target_os = "windows")]
    let base = app.path().local_data_dir()?;
    #[cfg(not(target_os = "windows"))]
    let base = app.path().data_dir()?;

    Ok(base.join(DATA_DIRECTORY))
}

/// Result of wiring up local storage.
///
/// Storage failure is not fatal: the window is shown regardless, and the UI renders a recovery
/// screen with the path and the reason. Crashing with no window is the worst possible first
/// user incident ("it won't start", no text, no way to recover).
///
/// What it holds is behind a lock and is replaced at most once. A device whose secrets are in a
/// file of its own cannot read the key its databases are written under until a person opens that
/// file, which is after the window is up: the databases are opened then, by
/// `open_what_waited_for_its_key`, and whatever asked before was told they were not there.
pub struct Storage {
    held: std::sync::RwLock<Held>,
}

enum Held {
    Ready {
        /// Accounts, settings, drafts and unsent mail. The only copy.
        profile: SqlitePool,
        /// The files the mail is kept in: one per account, opened as they are asked for.
        /// Everything in them also exists on a server.
        indexes: Indexes,
    },
    Failed {
        path: String,
        reason: String,
        why: Unopened,
    },
}

/// Why storage is not open, which decides what the window says and what it offers.
///
/// Told apart here because the window cannot tell from a path: a file that will not open may be
/// damaged, a key that could not be had leaves everything whole, and a file a newer build wrote is
/// whole and is that build's to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unopened {
    /// A file that will not open, or files whose key a store that is always there answers it does
    /// not hold: nothing this application waits for opens them, and setting them aside is the way
    /// out.
    TheFile,
    /// The one failure that can stop being one while the application runs.
    TheKey,
    ANewerBuild,
}

impl Storage {
    fn ready(profile: SqlitePool, indexes: Indexes) -> Self {
        Self::holding(Held::Ready { profile, indexes })
    }

    fn failed(path: String, reason: String, why: Unopened) -> Self {
        Self::holding(Held::Failed { path, reason, why })
    }

    fn holding(held: Held) -> Self {
        Self {
            held: std::sync::RwLock::new(held),
        }
    }

    /// What is held now. A writer that panicked left nothing half made: replacing is one
    /// assignment, so what the lock guards is whole either way.
    fn held(&self) -> std::sync::RwLockReadGuard<'_, Held> {
        self.held
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn profile(&self) -> Result<SqlitePool, CommandError> {
        match &*self.held() {
            Held::Ready { profile, .. } => Ok(profile.clone()),
            failed => Err(failure_of(failed)),
        }
    }

    /// Used by the read model and the sync use cases; unused until mail arrives.
    #[allow(dead_code)]
    /// The file holding the folder named, whichever account owns it.
    ///
    /// Six commands ask the index whose folder this is before doing anything else. Saying so here
    /// is the whole of the change: the answer is the same file it has always been.
    pub fn index_holding_folder(&self, folder: u64) -> Result<SqlitePool, CommandError> {
        self.indexes()?
            .holding_folder(folder)
            .map_err(|error| self.failure_opening(error))
    }

    /// The file holding the message named.
    pub fn index_holding_message(&self, message: u64) -> Result<SqlitePool, CommandError> {
        self.indexes()?
            .holding_message(message)
            .map_err(|error| self.failure_opening(error))
    }

    /// Every file on this device, for work that spans accounts.
    pub fn index_across(&self) -> Result<Vec<(i64, SqlitePool)>, CommandError> {
        self.indexes()?
            .across()
            .map_err(|error| self.failure_opening(error))
    }

    /// Takes an account's mail off the disk.
    pub fn forget_the_mail_of(&self, account: i64) -> Result<(), CommandError> {
        use crate::ports::mail_files::MailFiles;

        self.indexes()?.remove(account).map_err(|_| self.failure())
    }

    /// Which file each account's mail is kept in, for work that resolves as it goes.
    pub fn every_index(&self) -> Result<Indexes, CommandError> {
        self.indexes()
    }

    /// The file one account's mail is kept in.
    pub fn index_of_account(&self, account: i64) -> Result<SqlitePool, CommandError> {
        self.indexes()?
            .of(account)
            .map_err(|error| self.failure_opening(error))
    }

    fn indexes(&self) -> Result<Indexes, CommandError> {
        match &*self.held() {
            Held::Ready { indexes, .. } => Ok(indexes.clone()),
            failed => Err(failure_of(failed)),
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(&*self.held(), Held::Ready { .. })
    }

    fn failure(&self) -> CommandError {
        failure_of(&self.held())
    }

    /// What a command is told when one account's own file would not open, though the profile did.
    ///
    /// A file a newer build wrote says so and names itself, because it is the one file this is
    /// about. Anything else stays the failure it was.
    fn failure_opening(&self, error: StorageError) -> CommandError {
        match error {
            StorageError::FromANewerBuild { .. } => {
                CommandError::new(ErrorCode::StorageFromANewerBuild, error.to_string())
                    .with_path(error.path().to_owned())
            }
            _ => self.failure(),
        }
    }

    /// Whether the files may be set aside for a fresh start, which they may not while the profile is
    /// a newer build's.
    ///
    /// That profile is whole, and it is that build's to read: moved, it would leave that build a
    /// fresh, empty profile to find in its place. Known only of a profile that was read, so it holds
    /// where the key it is written under could be had — without it, the version on its first page
    /// cannot be read at all. One account's file from a newer build moves with the rest when the
    /// profile opened and something else is wrong: left behind under a fresh profile, the next
    /// start would sweep it away as mail that belongs to no account.
    pub fn may_be_set_aside(&self) -> Result<(), CommandError> {
        match &*self.held() {
            newer @ Held::Failed {
                why: Unopened::ANewerBuild,
                ..
            } => Err(failure_of(newer)),
            _ => Ok(()),
        }
    }

    /// Opens what was waiting for its key, and says whether this call was the one that did.
    ///
    /// Only storage that failed for want of its key is opened: a file that will not open stays the
    /// failure it was, and storage already open is left alone. `open` runs under the lock, so two
    /// people pressing at once open the databases once, and the answer is what decides which of
    /// them starts what runs on them. `open` answers nothing while the key still cannot be had.
    pub fn open_once_the_key_is_there(&self, open: impl FnOnce() -> Option<Storage>) -> bool {
        let mut held = self
            .held
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        if !matches!(
            *held,
            Held::Failed {
                why: Unopened::TheKey,
                ..
            }
        ) {
            return false;
        }
        let Some(opened) = open() else {
            return false;
        };

        *held = opened
            .held
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        matches!(*held, Held::Ready { .. })
    }
}

/// What a command is told when what it asked for is not open.
fn failure_of(held: &Held) -> CommandError {
    match held {
        Held::Ready { .. } => CommandError::new(ErrorCode::Internal, "Storage is available."),
        Held::Failed { path, reason, why } => {
            let code = match why {
                Unopened::TheFile => ErrorCode::StorageUnavailable,
                Unopened::TheKey => ErrorCode::StorageKeyUnavailable,
                Unopened::ANewerBuild => ErrorCode::StorageFromANewerBuild,
            };

            CommandError::new(code, reason.clone()).with_path(path.clone())
        }
    }
}

/// The composition root: the only place that picks production adapters.
pub fn init_storage(databases: &Path, key: &DatabaseKey) -> Storage {
    match open_the_profile(databases, key) {
        Ok(profile) => {
            let indexes = Indexes::under(databases, profile.clone(), key.clone());

            // Before anything opens anything. A removal that could not finish — something still
            // held the file, which on Windows is enough to refuse — left mail on the disk belonging
            // to no account. Nothing holds it now. Its failing is not this one's failing: the
            // profile opened, and a file that could not be taken this time is one the next start
            // tries again.
            let _ = indexes.sweep();

            Storage::ready(profile, indexes)
        }
        Err(error) => {
            let why = match error {
                StorageError::FromANewerBuild { .. } => Unopened::ANewerBuild,
                _ => Unopened::TheFile,
            };
            Storage::failed(error.path().to_owned(), error.to_string(), why)
        }
    }
}

/// What the application is when the key its databases are written under cannot be had.
///
/// Not an empty application and not a plaintext one: the recovery screen, saying which file could
/// not be opened and why, the same as a database that will not open.
pub fn storage_without_a_key(databases: &Path) -> Storage {
    Storage::failed(
        databases.join("profile.sqlite3").display().to_string(),
        "The key this device keeps its mail under could not be read.".to_owned(),
        Unopened::TheKey,
    )
}

/// Where this device keeps its secrets: in whichever store the key its databases are written under
/// was made in.
///
/// macOS and Windows always have a store of their own. A Linux desktop may have no Secret Service
/// at all, and then the secrets go into a file opened with a master password — announced, never as
/// a quiet substitute.
#[cfg(not(feature = "screenshots"))]
pub fn the_store_for(databases: &Path) -> SecretStore {
    use crate::infrastructure::secure_storage::master_password::MasterPasswordStore;
    use crate::infrastructure::secure_storage::{
        system_store_answers, SystemCredentialStore, SECRETS_FILE,
    };

    match in_a_file(databases, cfg!(target_os = "linux"), system_store_answers) {
        true => SecretStore::Master(Arc::new(MasterPasswordStore::at(
            databases.join(SECRETS_FILE),
        ))),
        false => SecretStore::System(SystemCredentialStore::new()),
    }
}

/// Where the build made for screenshots keeps its secrets: in memory, for as long as it runs. What
/// it holds is a made-up password for each account in its scene and the key to databases it makes
/// again at every start, and the system's store is never asked anything.
#[cfg(feature = "screenshots")]
pub fn the_store_for(_databases: &Path) -> SecretStore {
    SecretStore::Scene(Default::default())
}

/// Whether the secrets go into a file, answered by what is already beside the databases rather than
/// by whichever store answers this time.
///
/// The key the databases are written under stays in the store it was made in, and a store chosen
/// afresh on every start asks the other one for it. A file beside them is where that key is, even
/// once a keyring answers. Databases with no file beside them were written under a key a keyring
/// holds, and a keyring that does not answer is waited for rather than replaced: a file made then
/// would hold everything but that key. Only a device with neither is asked whether a keyring is
/// there at all, and only on a system that may have none — a first start, a start after one that
/// wrote nothing, and a start after Start fresh set the databases aside and left no file.
#[cfg(any(test, not(feature = "screenshots")))]
fn in_a_file(
    databases: &Path,
    may_have_no_store: bool,
    a_keyring_answers: impl FnOnce() -> bool,
) -> bool {
    use crate::infrastructure::secure_storage::SECRETS_FILE;

    let file = databases.join(SECRETS_FILE).exists();
    let profile = databases.join("profile.sqlite3").exists();

    match (may_have_no_store, file, profile) {
        (false, _, _) => false,
        (true, true, _) => true,
        (true, false, true) => false,
        (true, false, false) => !a_keyring_answers(),
    }
}

/// A key the start stopped waiting for before the store answered, still being asked for.
///
/// The keychain asks a person before it hands its item to a build the item's access list does not
/// name, and a person can take longer to answer than the start waits. Their answer still arrives,
/// and when it is the key, the databases open with it: [`open_when_the_key_arrives`].
pub struct KeyOnItsWay(std::sync::mpsc::Receiver<Result<DatabaseKey, CredentialError>>);

impl KeyOnItsWay {
    /// The store's answer, waited for as long as it takes.
    pub fn wait(self) -> Result<DatabaseKey, CredentialError> {
        self.0.recv().unwrap_or(Err(CredentialError::Unavailable))
    }
}

/// The databases as the start finds them: open under the key this device's store holds for them,
/// or the recovery screen saying why not — and the key still on its way, when the store had not
/// answered by the time the start stopped waiting.
///
/// A key that cannot be had is a storage failure like any other and reaches the same screen: there
/// is nothing useful to do with these files without it, and opening them unencrypted instead would
/// be this application quietly deciding to write somebody's mail in the clear.
pub fn open_storage(databases: &Path, secrets: &SecretStore) -> (Storage, Option<KeyOnItsWay>) {
    opened_within(
        databases,
        Arc::new(secrets.waiting_for_an_answer()),
        answer_within(),
        secrets.master_password().is_some(),
        cfg!(target_os = "linux"),
    )
}

/// The start's half of [`open_storage`], with the store handed in as any store, how long to wait
/// for it, and what kind it is said beside it, so a test can be the store a start reads.
///
/// The key is asked for on a thread of its own and waited for `within`. A store that has not
/// answered by then leaves the recovery screen saying the key could not be had, and the answer it
/// is still going to give.
fn opened_within(
    databases: &Path,
    store: Arc<dyn CredentialStore>,
    within: std::time::Duration,
    in_a_file: bool,
    a_keyring_may_hide_it: bool,
) -> (Storage, Option<KeyOnItsWay>) {
    use std::sync::mpsc::RecvTimeoutError;

    let (answer, waiting) = std::sync::mpsc::channel();
    let directory = databases.to_path_buf();

    std::thread::spawn(move || {
        // Nobody may be listening by the time the store answers; the key then goes with the message.
        let _ = answer.send(the_key_of(&directory, store.as_ref()));
    });

    match waiting.recv_timeout(within) {
        Ok(answered) => (
            opened_with(databases, answered, in_a_file, a_keyring_may_hide_it),
            None,
        ),
        Err(RecvTimeoutError::Timeout) => {
            (storage_without_a_key(databases), Some(KeyOnItsWay(waiting)))
        }
        Err(RecvTimeoutError::Disconnected) => (storage_without_a_key(databases), None),
    }
}

/// [`opened_with`] the key `store` answers, for a test that has no reason to wait for it.
#[cfg(test)]
fn opened_with_the_key_in(
    databases: &Path,
    store: &dyn CredentialStore,
    in_a_file: bool,
    a_keyring_may_hide_it: bool,
) -> Storage {
    opened_with(
        databases,
        the_key_of(databases, store),
        in_a_file,
        a_keyring_may_hide_it,
    )
}

/// The databases opened with what the store answered for their key.
///
/// **What the window offers follows from what the store answered.** A store that would not answer
/// is waited for, and so is a Linux keyring that answers it holds nothing, which a locked database
/// can make it say, and a file of secrets, which says what it holds on a screen of its own. The
/// macOS keychain and Credential Manager are always there, and when one of them answers that it
/// holds no key, nothing this application waits for brings it back: the databases are files that
/// will not open, on the screen that offers Start fresh. That moves them aside rather than deleting
/// them, and the start after it makes a key over no databases.
fn opened_with(
    databases: &Path,
    answered: Result<DatabaseKey, CredentialError>,
    in_a_file: bool,
    a_keyring_may_hide_it: bool,
) -> Storage {
    match answered {
        Ok(key) => init_storage(databases, &key),
        Err(why) => {
            // Said in a development build, where the next thing somebody does is look for a
            // reason: the reason and nothing else — no key, no material, no path. A device whose
            // secrets are in a file has that file closed at every start, which is not a failure and
            // has nothing to say.
            //
            // What follows the reason differs by system, because the way back does: the keychain
            // binds an item to the application that wrote it and an ad-hoc signed build is a
            // different application after every rebuild, which is a macOS sentence and reads as
            // noise to somebody whose Secret Service said no. A keychain that answers it holds no
            // key at all is a different thing again, and is said as that.
            #[cfg(all(debug_assertions, target_os = "macos"))]
            match why {
                CredentialError::NotFound => eprintln!(
                    "lyonoyl: the keychain holds no database key, and the profile beside it was \
                     written under one, so none is made over it. The profile is not damaged: \
                     look for the item in Keychain Access, in every keychain, before Start fresh \
                     sets the files aside."
                ),
                _ => eprintln!(
                    "lyonoyl: the database key could not be had -- {why:?}. A development build \
                     is signed ad-hoc, so every rebuild is a new application to the keychain and \
                     the stored item's access list no longer names it: answer the dialog with \
                     Always Allow, or give the item access to all applications. The profile is \
                     not damaged and Start fresh is not the way back."
                ),
            }
            #[cfg(all(debug_assertions, not(target_os = "macos")))]
            if !in_a_file {
                eprintln!(
                    "lyonoyl: the database key could not be had -- {why:?}. The profile is not \
                     damaged."
                );
            }

            match matches!(why, CredentialError::NotFound) && !in_a_file && !a_keyring_may_hide_it {
                true => Storage::failed(
                    databases.join("profile.sqlite3").display().to_string(),
                    "The store this device keeps its secrets in holds no key for these databases."
                        .to_owned(),
                    Unopened::TheFile,
                ),
                false => storage_without_a_key(databases),
            }
        }
    }
}

/// The key the databases in `databases` are written under, read from `store`, and made there
/// only when there is no database written under one yet.
///
/// One answer for the start and for the moment a file of secrets is opened: the same question,
/// asked again once the store can answer it.
///
/// **Over databases already written, no store is given a new key**, whichever store it is. A key
/// made then opens nothing, and it is kept for good in front of the one they were written under —
/// which a store that answers that it holds none can still have: a keyring answering for a locked
/// database, a keychain the item is in that has dropped out of the list searched. The key is
/// reported as not to be had instead, which is what the recovery screen is for; the first start
/// that finds no databases makes one.
fn the_key_of(
    databases: &Path,
    store: &dyn CredentialStore,
) -> Result<DatabaseKey, CredentialError> {
    match databases.join("profile.sqlite3").exists() {
        true => DatabaseKey::already_kept_in(store),
        false => DatabaseKey::of_this_device(store),
    }
}

/// The databases, opened with the key the store holds for them, or nothing while it holds none
/// that can be read.
fn with_its_key(databases: &Path, secrets: &SecretStore) -> Option<Storage> {
    the_key_of(databases, secrets)
        .ok()
        .map(|key| init_storage(databases, &key))
}

/// Opens the databases once the key they are written under can be had, and starts what runs on
/// them: the rounds, the pushes and the badge, as the start would have.
///
/// A device whose secrets are in a file of its own has the key only once a person opens that file,
/// after the start that would have opened the databases; without this they stay unopened for as
/// long as the application runs, and again on every start after.
pub fn open_what_waited_for_its_key(app: &tauri::AppHandle) {
    use tauri::Manager;

    let secrets = app.state::<crate::AppState>().secrets.clone();

    // A system store that had not handed the key over when the start stopped waiting is answered
    // by the answer it is still going to give, which `open_when_the_key_arrives` waits for. Asked
    // again here, it would hold the lock everything reads storage through for as long as that
    // store takes to answer.
    if secrets.master_password().is_none() {
        return;
    }

    open_with(app, |directory| with_its_key(directory, &secrets));
}

/// The event the window listens for when the key its databases were waiting for has arrived.
pub const KEY_ARRIVED: &str = "storage:key-arrived";

/// Opens the databases with a key the store handed over after the start stopped waiting for it,
/// starts what runs on them, and tells the window, which reads everything again.
///
/// Nothing else opens storage on a device with a store of its own, so the storage this finds is
/// the storage the start left waiting for its key. The window is told whatever the databases made
/// of the key — open, or a file that will not open under it, which is a different screen from the
/// one saying the key could not be had.
pub fn open_when_the_key_arrives(app: tauri::AppHandle, coming: KeyOnItsWay) {
    std::thread::spawn(move || {
        use tauri::Emitter;

        let Ok(key) = coming.wait() else {
            return;
        };

        open_with(&app, |directory| Some(init_storage(directory, &key)));
        let _ = app.emit(KEY_ARRIVED, ());
    });
}

/// Opens storage that waited for its key with what `open` makes of the data directory, and starts
/// what runs on the databases.
fn open_with(app: &tauri::AppHandle, open: impl FnOnce(&Path) -> Option<Storage>) {
    use tauri::Manager;

    let state = app.state::<crate::AppState>();
    let Ok(data) = data_directory(app) else {
        return;
    };
    let directory = databases_of(&data);

    let opened = state
        .storage
        .open_once_the_key_is_there(|| open(&directory));
    if !opened {
        // The key could be had and the databases are still not open: there is no key for them, or
        // they will not open under the one there is. Nothing runs from here on, and the journal
        // says so the way the start says it.
        if !state.storage.is_ready() {
            use crate::ports::journal::{Journal, Note};

            let journal = crate::infrastructure::journal::journal();
            if journal.keeps(Note::Trouble) {
                journal.write_down(Note::Trouble, "sync.neverStarted", &[]);
            }
        }
        return;
    }

    let pushes = start_background_sync(
        app,
        &state.storage,
        state.in_flight.clone(),
        state.credentials.sharing(state.secrets.clone()),
        state.bell.clone(),
        Arc::clone(&state.reading_at_once),
    );
    if let Some(pushes) = pushes {
        let _ = state.pushes.set(pushes);
    }

    redraw_the_badge(app);
}

fn open_the_profile(databases: &Path, key: &DatabaseKey) -> Result<SqlitePool, StorageError> {
    let profile_path = databases.join("profile.sqlite3");

    std::fs::create_dir_all(databases).map_err(|error| StorageError::Open {
        path: profile_path.display().to_string(),
        reason: error.to_string(),
    })?;

    // The folder the databases and the sealed secrets live in, reachable by this person alone.
    // After the folder exists rather than through a mode on creation, so a folder made with
    // whatever the umask allowed is narrowed as well; and after rather than before, because on a
    // first start there is nothing to set a mode on until it is made.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let _ = std::fs::set_permissions(databases, std::fs::Permissions::from_mode(0o700));
    }

    open_and_migrate(
        &profile_path,
        |path| open_profile(path, key),
        migrate_profile,
    )
}

fn open_and_migrate(
    path: &Path,
    open: impl Fn(&Path) -> Result<SqlitePool, StorageError>,
    migrate: fn(&mut rusqlite::Connection, &str) -> Result<(), StorageError>,
) -> Result<SqlitePool, StorageError> {
    let display = path.display().to_string();

    let pool = open(path)?;
    let mut connection = pool.get().map_err(|error| StorageError::Open {
        path: display.clone(),
        reason: error.to_string(),
    })?;
    migrate(&mut connection, &display)?;
    drop(connection);

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_cannot_reverse_the_address_it_is_asking_about() {
        // This panel exists so a person can read where a link goes before it opens. A
        // right-to-left override inside the address reverses everything after it on the screen
        // while the bytes stay as they are, so a message could show one host and carry another —
        // in the one place built to answer exactly that question.
        let asked = asked_about("Read more", "https://example.com/\u{202E}gpj.exe");

        assert!(
            !asked.url.contains('\u{202E}'),
            "the address is shown as the message wrote it: {}",
            asked.url
        );
        assert!(
            asked.url.contains('\u{FFFD}'),
            "something was taken out and the reader is not told: {}",
            asked.url
        );
        assert!(
            asked.url.starts_with("https://example.com/"),
            "the rest of the address survived: {}",
            asked.url
        );
    }

    /// The host a reader that does not take a backslash for a slash finds in an address: RFC
    /// 3986's own split, which is how macOS reads one, with whatever stands before the last `@`
    /// taken as a sign-in.
    fn host_read_without_the_standard(address: &str) -> &str {
        let after_scheme = address.split_once("://").map_or("", |(_, rest)| rest);
        let authority = after_scheme
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default();

        authority.rsplit('@').next().unwrap_or_default()
    }

    #[test]
    fn the_host_a_person_is_asked_about_is_the_host_that_opens() {
        // A backslash before `@` is a slash to the standard the panel reads the host with, and
        // nothing to the parser macOS opens links with: held as the message wrote it, the person
        // is asked about `good.example` and `evil.example` opens.
        let raw = [
            "From: zeta@example.com",
            "To: kappa@example.net",
            "Subject: An invoice",
            "MIME-Version: 1.0",
            "Content-Type: text/html; charset=utf-8",
            "",
            "<p><a href=\"https://good.example\\@evil.example/\">pay</a></p>",
        ]
        .join("\r\n");

        let rendered = crate::infrastructure::html::body_of(
            raw.as_bytes(),
            crate::ports::body::Viewing::plainly(),
        );
        assert_eq!(rendered.links.len(), 1, "the link was not offered");
        let held = &rendered.links[0];

        let asked = asked_about(&held.name, &held.url);

        assert!(!held.url.contains('\\'), "held as written: {}", held.url);
        assert_eq!(
            asked.url, held.url,
            "the panel shows something other than what opens"
        );
        assert_eq!(asked.host, "good.example");
        assert_eq!(
            host_read_without_the_standard(&held.url),
            asked.host,
            "the person is asked about one host and {} opens",
            held.url
        );
    }

    #[test]
    fn the_data_folder_follows_the_platform_convention() {
        if cfg!(target_os = "linux") {
            assert_eq!(DATA_DIRECTORY, "lyonoyl-mail");
            assert!(
                !DATA_DIRECTORY.contains(' '),
                "XDG paths are not written with spaces"
            );
        } else {
            assert_eq!(DATA_DIRECTORY, "Lyonoyl Mail");
        }
    }

    #[test]
    fn a_fresh_directory_yields_ready_storage() {
        let dir = std::env::temp_dir().join(format!("lyonoyl-bootstrap-ok-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        let storage = init_storage(&dir, &DatabaseKey::for_a_test());

        assert!(
            storage.is_ready(),
            "a fresh directory must produce working storage"
        );
        assert!(dir.join("profile.sqlite3").exists());
        assert!(
            !dir.join("accounts").exists(),
            "a device with no accounts keeps no mail, so it has no file to keep it in"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The directory the databases and the sealed secrets live in is this person's alone, whether
    /// this start made it or found it made with whatever the umask allowed.
    #[cfg(unix)]
    #[test]
    fn the_data_directory_is_this_persons_alone_however_it_was_made() {
        use std::os::unix::fs::PermissionsExt;

        let made_here =
            std::env::temp_dir().join(format!("lyonoyl-mode-new-{}", std::process::id()));
        let found = std::env::temp_dir().join(format!("lyonoyl-mode-old-{}", std::process::id()));
        std::fs::remove_dir_all(&made_here).ok();
        std::fs::remove_dir_all(&found).ok();
        std::fs::create_dir_all(&found).expect("temp dir is creatable");
        std::fs::set_permissions(&found, std::fs::Permissions::from_mode(0o777))
            .expect("a mode is settable");

        for dir in [&made_here, &found] {
            let _storage = init_storage(dir, &DatabaseKey::for_a_test());
            let mode = std::fs::metadata(dir)
                .expect("the directory is there")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700, "{} is open to others", dir.display());
            std::fs::remove_dir_all(dir).ok();
        }
    }

    /// The sweep takes what belongs to no account, and only that.
    ///
    /// What a removal could not finish is finished here. Written against a file put there by hand
    /// rather than against one a failed removal left behind, because the failure is Windows's and
    /// this has to hold on every system.
    #[test]
    fn the_sweep_takes_a_file_that_belongs_to_no_account_and_leaves_the_rest() {
        let dir = std::env::temp_dir().join(format!("lyonoyl-sweep-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let storage = init_storage(&dir, &DatabaseKey::for_a_test());

        let profile = storage.profile().expect("the profile opens");
        profile
            .get()
            .expect("a connection")
            .execute_batch(
                "INSERT INTO mail_accounts
                     (id, email, display_name, provider, imap_host, imap_port, imap_encryption,
                      imap_username, index_id)
                 VALUES ('one', 'zeta@example.com', 'Zeta', 'imap', 'localhost', 1143, 0,
                         'zeta', 1),
                        ('two', 'kappa@example.net', 'Kappa', 'imap', 'localhost', 1143, 0,
                         'kappa', 2);",
            )
            .expect("an account");

        storage.index_of_account(1).expect("its file is made");

        let accounts = dir.join("accounts");
        let orphan = accounts.join("9.sqlite3");
        std::fs::write(&orphan, b"mail of an account nobody has").expect("the orphan is written");
        std::fs::write(accounts.join("9.sqlite3-wal"), b"and its log").expect("the log is written");

        // Marked to go by a removal that could not finish, which is what Windows leaves behind: a
        // marker beside a file it was not allowed to touch. Its number is an account that still
        // exists, and the mail must go all the same — dropping an account's mail is not the same
        // as dropping the account.
        let marked = accounts.join("2.sqlite3");
        std::fs::write(&marked, b"mail an account was told to forget").expect("written");
        std::fs::write(accounts.join("2.sqlite3.forget"), b"").expect("the marker is written");

        // Not ours, and not touched: this directory holds one file per account, and nothing says
        // anything else in it is this build's to remove. Named to look like one — a number, a dot,
        // and something that is not a database — because a file that looks nothing like ours is
        // refused by the number and proves only that.
        let stranger = accounts.join("9.txt");
        std::fs::write(&stranger, b"somebody put this here").expect("the stranger is written");

        let taken = storage
            .every_index()
            .expect("the indexes are there")
            .sweep()
            .expect("the sweep runs");

        assert_eq!(taken, 3, "the orphan, its log, and the one marked to go");
        assert!(
            !marked.exists(),
            "mail an account was told to forget is still there"
        );
        assert!(
            !accounts.join("2.sqlite3.forget").exists(),
            "the marker outlived what it named"
        );
        assert!(
            !orphan.exists(),
            "mail belonging to no account is still there"
        );
        assert!(
            !accounts.join("9.sqlite3-wal").exists(),
            "the log holds what the database holds, and it is still there"
        );
        assert!(
            accounts.join("1.sqlite3").exists(),
            "the sweep took an account that is still here"
        );
        assert!(stranger.exists(), "the sweep took a file that is not ours");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file made again leaves nothing in the queue naming a number the file before it handed out.
    ///
    /// The counter starts at `account << 40` whenever the file is made, so a file that vanished and
    /// came back mints the very numbers it minted before — and something else is wearing them now.
    /// An instruction naming one of those numbers would land on a different message, which is the
    /// failure `op_log` addressing by identifier exists to prevent. Sent back to be found again is
    /// not enough: the first pass over the new file reads a folder wearing the old folder's number,
    /// and finding that folder is what puts an instruction back in the queue.
    #[test]
    fn a_file_made_again_leaves_nothing_in_the_queue_naming_its_numbers() {
        use crate::ports::op_log::OperationLog;

        let dir = std::env::temp_dir().join(format!("lyonoyl-remade-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let storage = init_storage(&dir, &DatabaseKey::for_a_test());

        let profile = storage.profile().expect("the profile opens");
        profile
            .get()
            .expect("a connection")
            .execute_batch(
                "INSERT INTO mail_accounts
                     (id, email, display_name, provider, imap_host, imap_port, imap_encryption,
                      imap_username, index_id)
                 VALUES ('seven', 'seven@example.com', 'Seven', 'imap', 'localhost', 1143, 0,
                         'seven', 7);",
            )
            .expect("an account");

        storage.index_of_account(7).expect("its file is made");

        let inbox = (7i64 << 40) | 1;
        let instruction = |message: i64, state: &str, sent: Option<i64>| {
            profile
                .get()
                .expect("a connection")
                .execute(
                    "INSERT INTO op_log
                         (account_id, message_id, folder_id, kind, delete_intent, state, asked_at,
                          sent_at)
                     VALUES ('seven', ?1, ?2, 'delete', 'permanently', ?3, 1700000000, ?4)",
                    rusqlite::params![message, inbox, state, sent],
                )
                .expect("an instruction");
        };

        // Asked for and not yet told to the server, sent back to be found again, on its way, and
        // stopped for a person: every state an instruction can wait in.
        instruction((7i64 << 40) | 1, "pending", None);
        instruction((7i64 << 40) | 3, "needs_reresolve", None);
        instruction((7i64 << 40) | 4, "pending", Some(1700000001));
        instruction((7i64 << 40) | 5, "conflicted", None);

        // And a move the server carried out that no pass has read back, which every view lays
        // over the index by its message number.
        profile
            .get()
            .expect("a connection")
            .execute(
                "INSERT INTO op_log
                     (account_id, message_id, folder_id, kind, move_to, state, asked_at,
                      owes_a_read)
                 VALUES ('seven', ?1, ?2, 'move', ?3, 'done', 1700000000, 1)",
                [(7i64 << 40) | 2, inbox, (7i64 << 40) | 2],
            )
            .expect("a move owed a read");

        // The file goes, and is asked for again — a restore from a backup, a disk that lost it, a
        // rebuild. The account is still in the profile, so it is made again.
        storage.forget_the_mail_of(7).expect("its file goes");
        storage.index_of_account(7).expect("its file is made again");

        // The first pass over the new file reads its inbox, which wears the old inbox's number.
        let log = crate::infrastructure::sqlite::op_log::SqliteOperationLog::new(profile.clone());
        log.folder_found(
            "seven",
            lyonoyl_domain::message::FolderId::new(inbox as u64).expect("non-zero"),
        )
        .expect("the queue is asked");

        assert!(
            log.due("seven", 10).expect("readable").is_empty(),
            "an instruction from the old file is due against the new one"
        );

        let left: i64 = profile
            .get()
            .expect("a connection")
            .query_row("SELECT count(*) FROM op_log", [], |row| row.get(0))
            .expect("readable");

        assert_eq!(
            left, 0,
            "something in the queue still names a number the new file hands out again"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Removing an account takes its file with it, and the numbers it used are not handed out again.
    ///
    /// The counter that names accounts never goes back, so the next account gets a number this one
    /// never had — and an identifier that reached the interface, a cache or the queue cannot start
    /// naming a different account's message.
    #[test]
    fn removing_an_account_takes_its_file_and_leaves_the_others() {
        let dir = std::env::temp_dir().join(format!("lyonoyl-forget-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let storage = init_storage(&dir, &DatabaseKey::for_a_test());

        storage
            .profile()
            .expect("the profile opens")
            .get()
            .expect("a connection")
            .execute_batch(
                "INSERT INTO mail_accounts
                     (id, email, display_name, provider, imap_host, imap_port, imap_encryption,
                      imap_username, index_id)
                 VALUES ('one', 'one@example.com', 'One', 'imap', 'localhost', 1143, 0, 'one', 1),
                        ('seven', 'seven@example.com', 'Seven', 'imap', 'localhost', 1143, 0,
                         'seven', 7);",
            )
            .expect("two accounts");

        let every = storage.index_across().expect("both files open");
        assert!(dir.join("accounts").join("1.sqlite3").exists());
        assert!(dir.join("accounts").join("7.sqlite3").exists());

        // Mail in it, and the file left open, so the write-ahead log is still beside the database
        // rather than folded into it. Without this the log is not there to begin with and asserting
        // it went would assert nothing.
        let seven = every
            .iter()
            .find(|(account, _)| *account == 7)
            .map(|(_, pool)| pool.clone())
            .expect("the account's file");
        let held = seven.get().expect("a connection");
        held.execute(
            "INSERT INTO messages (account_id, received_at) VALUES (7, 1700000000)",
            [],
        )
        .expect("a message");
        assert!(
            dir.join("accounts").join("7.sqlite3-wal").exists(),
            "the fixture did not produce the log this test is about"
        );

        storage.forget_the_mail_of(7).expect("its mail goes");

        // Held open on purpose above, so this is the hard case: on Windows a file somebody has open
        // cannot be removed at all, and asserting it went would be asserting about macOS. What is
        // promised here is that the account is forgotten — the file belongs to nobody and is taken
        // by the sweep.
        assert!(
            dir.join("accounts").join("1.sqlite3").exists(),
            "removing one account took another account's mail with it"
        );

        // Nothing holds it now, which is the state every start begins in.
        drop(held);
        drop(seven);
        drop(every);

        storage
            .every_index()
            .expect("the indexes are there")
            .sweep()
            .expect("the sweep runs");

        assert!(
            !dir.join("accounts").join("7.sqlite3").exists(),
            "the mail of the account that was removed is still on the disk"
        );
        assert!(
            !dir.join("accounts").join("7.sqlite3-wal").exists(),
            "the write-ahead log holds what the database holds, and it is still there"
        );
        assert!(
            dir.join("accounts").join("1.sqlite3").exists(),
            "the sweep took an account that is still here"
        );

        // Asking again is not a failure: what was asked for already holds.
        storage
            .forget_the_mail_of(7)
            .expect("asking twice is allowed");

        // And the file it was kept open by is not handed out again. A pool left behind would go on
        // answering from a database nothing points at any more — the mail would look removed and
        // still be readable, which is the worst of both.
        let again = storage
            .index_of_account(7)
            .expect("a file for the account is made again");
        let left: i64 = again
            .get()
            .expect("a connection")
            .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
            .expect("readable");
        assert_eq!(left, 0, "the mail came back from a file that was removed");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// An account's mail is kept in its own file, and the numbers that file hands out say so.
    ///
    /// Two accounts numbered 1 and 7 — never 1 and 2, so a prefix or an off-by-one cannot pass by
    /// looking right. The point is not that two files exist but that a number minted in one of them
    /// names its own file and could not have come from the other.
    #[test]
    fn an_account_keeps_its_mail_in_its_own_file_and_says_so_in_every_number() {
        let dir = std::env::temp_dir().join(format!("lyonoyl-shards-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let storage = init_storage(&dir, &DatabaseKey::for_a_test());
        assert!(storage.is_ready());

        let profile = storage.profile().expect("the profile opens");
        profile
            .get()
            .expect("a connection")
            .execute_batch(
                "INSERT INTO mail_accounts
                     (id, email, display_name, provider, imap_host, imap_port, imap_encryption,
                      imap_username, index_id)
                 VALUES ('one', 'one@example.com', 'One', 'imap', 'localhost', 1143, 0,
                         'one', 1),
                        ('seven', 'seven@example.com', 'Seven', 'imap', 'localhost', 1143, 0,
                         'seven', 7);",
            )
            .expect("two accounts");

        let every = storage.index_across().expect("every account resolves");
        assert_eq!(
            every.len(),
            2,
            "each account keeps its mail in its own file"
        );
        assert!(dir.join("accounts").join("1.sqlite3").exists());
        assert!(dir.join("accounts").join("7.sqlite3").exists());

        // What each file hands out carries the account it was minted in.
        let mut minted = Vec::new();
        for (account, pool) in [(1i64, &every[0].1), (7, &every[1].1)] {
            let connection = pool.get().expect("a connection");
            connection
                .execute(
                    "INSERT INTO messages (account_id, received_at) VALUES (?1, 1700000000)",
                    [account],
                )
                .expect("a message");
            let id: i64 = connection
                .query_row("SELECT max(id) FROM messages", [], |row| row.get(0))
                .expect("readable");
            assert_eq!(
                id >> 40,
                account,
                "a number minted in the file of account {account} does not name it"
            );
            minted.push(id as u64);
        }

        // And the number is enough to find the file again, with nothing looked up.
        for (account, id) in [(1i64, minted[0]), (7, minted[1])] {
            let found = storage
                .index_holding_message(id)
                .expect("the number names a file");
            let seen: i64 = found
                .get()
                .expect("a connection")
                .query_row("SELECT account_id FROM messages LIMIT 1", [], |row| {
                    row.get(0)
                })
                .expect("readable");
            assert_eq!(seen, account, "the number led to the wrong file");
        }

        // A number naming no account is refused rather than answered from whichever file is first.
        assert!(
            storage.index_holding_message(42).is_err(),
            "an untagged number must not resolve to a file"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Opening an account's file again does not make it hand out numbers it has already given.
    ///
    /// The counter is set when the file is opened, and a file is opened afresh every time the
    /// application starts — so a seed that *set* rather than *raised* would rewind it on the second
    /// run and mint every number a second time. A mutation that removed the `seq <` from the
    /// statement survived every other test, because none of them started twice.
    #[test]
    fn opening_an_account_again_does_not_rewind_the_numbers_it_hands_out() {
        let dir = std::env::temp_dir().join(format!("lyonoyl-restart-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        let account = 7i64;
        let seed_accounts = |storage: &Storage| {
            storage
                .profile()
                .expect("the profile opens")
                .get()
                .expect("a connection")
                .execute(
                    "INSERT OR IGNORE INTO mail_accounts
                         (id, email, display_name, provider, imap_host, imap_port,
                          imap_encryption, imap_username, index_id)
                     VALUES ('seven', 'seven@example.com', 'Seven', 'imap', 'localhost', 1143, 0,
                             'seven', ?1)",
                    [account],
                )
                .expect("an account");
        };

        let minted = |storage: &Storage, how_many: usize| -> Vec<i64> {
            let pool = storage.index_of_account(account).expect("its file opens");
            let connection = pool.get().expect("a connection");
            (0..how_many)
                .map(|_| {
                    connection
                        .execute(
                            "INSERT INTO messages (account_id, received_at) VALUES (?1, 1700000000)",
                            [account],
                        )
                        .expect("a message");
                    connection
                        .query_row("SELECT max(id) FROM messages", [], |row| row.get::<_, i64>(0))
                        .expect("readable")
                })
                .collect()
        };

        let first_run = init_storage(&dir, &DatabaseKey::for_a_test());
        seed_accounts(&first_run);
        let before = minted(&first_run, 3);

        // Emptied, which is the only state where rewinding the counter is visible: while rows
        // remain, SQLite takes the larger of the counter and the highest number in the table and
        // hands out nothing twice by itself. With the rows gone it has only the counter to go on,
        // and this is where a resync, a reset UIDVALIDITY or a folder emptied by the server leaves
        // an account.
        first_run
            .index_of_account(account)
            .expect("its file opens")
            .get()
            .expect("a connection")
            .execute_batch("DELETE FROM messages;")
            .expect("the account loses everything");
        drop(first_run);

        // A second run of the application over the same directory.
        let second_run = init_storage(&dir, &DatabaseKey::for_a_test());
        seed_accounts(&second_run);
        let after = minted(&second_run, 3);

        let highest = *before.iter().max().expect("some numbers");
        for given in &after {
            assert!(
                *given > highest,
                "starting again handed out {given}, and {highest} had already been used"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A storage that did not open refuses every way of asking, with the code the screen needs.
    #[test]
    fn a_storage_that_did_not_open_refuses_every_way_of_asking() {
        let dir = std::env::temp_dir().join(format!("lyonoyl-seam-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir is creatable");
        std::fs::create_dir_all(dir.join("profile.sqlite3")).expect("decoy directory is creatable");

        let storage = init_storage(&dir, &DatabaseKey::for_a_test());
        assert!(!storage.is_ready());

        for refused in [
            storage.index_holding_folder(1).expect_err("refuses"),
            storage.index_holding_message(1).expect_err("refuses"),
            storage.index_across().expect_err("refuses"),
        ] {
            assert!(
                matches!(refused.code, ErrorCode::StorageUnavailable),
                "the recovery screen needs one code however the index was asked for"
            );
            assert!(refused.path.is_some(), "the screen shows the path");
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unopenable_database_degrades_instead_of_panicking() {
        let dir =
            std::env::temp_dir().join(format!("lyonoyl-bootstrap-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir is creatable");
        // A directory where the database file is expected: opening it can never succeed.
        std::fs::create_dir_all(dir.join("profile.sqlite3")).expect("decoy directory is creatable");

        let storage = init_storage(&dir, &DatabaseKey::for_a_test());

        assert!(
            !storage.is_ready(),
            "storage must report failure rather than panic"
        );
        let error = storage
            .profile()
            .expect_err("commands surface a typed error");
        assert!(
            matches!(error.code, ErrorCode::StorageUnavailable),
            "the UI needs a stable code to show the recovery screen"
        );
        assert!(
            error.path.is_some(),
            "the recovery screen shows the database path"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A profile a newer build wrote is refused with a code of its own, and left as it was.
    ///
    /// Every way of asking says the same thing, the files are not set aside, and the bytes on the
    /// disk are the ones the newer build left: it reads them as they stand.
    #[test]
    fn a_profile_a_newer_build_wrote_is_refused_with_its_own_code_and_left_alone() {
        let dir =
            std::env::temp_dir().join(format!("lyonoyl-newer-profile-{}", std::process::id()));
        let left = std::env::temp_dir().join(format!("lyonoyl-newer-left-{}", std::process::id()));
        for each in [&dir, &left] {
            std::fs::remove_dir_all(each).ok();
            std::fs::create_dir_all(each).expect("temp dir is creatable");
        }
        let key = DatabaseKey::for_a_test();
        {
            let first_run = init_storage(&dir, &key);
            let connection = first_run
                .profile()
                .expect("opens")
                .get()
                .expect("a connection");
            let newest = connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .expect("readable") as usize;
            connection
                .execute_batch(&format!("PRAGMA user_version = {};", newest + 1))
                .expect("a newer build's version");
            // What a newer build leaves when it quits: the process ends with its connections open,
            // so nothing checkpoints, and the last of what it wrote is still in the log beside it.
            for name in [
                "profile.sqlite3",
                "profile.sqlite3-wal",
                "profile.sqlite3-shm",
            ] {
                std::fs::copy(dir.join(name), left.join(name)).expect("copied while open");
            }
        }
        let profile = left.join("profile.sqlite3");
        let as_left = || {
            ["profile.sqlite3", "profile.sqlite3-wal"]
                .map(|name| std::fs::read(left.join(name)).expect("still there"))
        };
        let before = as_left();

        let storage = init_storage(&left, &key);

        for refused in [
            storage.profile().expect_err("refuses"),
            storage.index_across().expect_err("refuses"),
            storage.may_be_set_aside().expect_err("not set aside"),
        ] {
            assert_eq!(refused.code, ErrorCode::StorageFromANewerBuild);
            assert_eq!(
                refused.path.as_deref(),
                Some(profile.display().to_string().as_str())
            );
        }
        drop(storage);
        assert!(
            as_left() == before,
            "the file a newer build wrote, or the log beside it, was changed"
        );

        for each in [&dir, &left] {
            std::fs::remove_dir_all(each).ok();
        }
    }

    /// One account's file a newer build wrote says so and names itself, and the profile still opens.
    #[test]
    fn an_accounts_file_a_newer_build_wrote_names_itself() {
        let dir =
            std::env::temp_dir().join(format!("lyonoyl-newer-account-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("temp dir is creatable");
        let key = DatabaseKey::for_a_test();
        {
            let first_run = init_storage(&dir, &key);
            // In the profile, or the start sweeps the file away as mail that belongs to nobody.
            first_run
                .profile()
                .expect("the profile opens")
                .get()
                .expect("a connection")
                .execute(
                    "INSERT INTO mail_accounts
                         (id, email, display_name, provider, imap_host, imap_port,
                          imap_encryption, imap_username, index_id)
                     VALUES ('zeta', 'zeta@example.com', 'Zeta Quorin', 'imap', 'localhost', 1143,
                             0, 'zeta', 1)",
                    [],
                )
                .expect("an account");
            let connection = first_run
                .index_of_account(1)
                .expect("made")
                .get()
                .expect("a connection");
            let newest = connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .expect("readable") as usize;
            connection
                .execute_batch(&format!("PRAGMA user_version = {};", newest + 1))
                .expect("a newer build's version");
        }

        let storage = init_storage(&dir, &key);

        assert!(
            storage.profile().is_ok(),
            "the profile is not what is wrong"
        );
        // Asked for alone and asked for with every other account, which is how the sidebar asks.
        for refused in [
            storage.index_of_account(1).expect_err("refuses"),
            storage.index_across().expect_err("refuses"),
        ] {
            assert_eq!(refused.code, ErrorCode::StorageFromANewerBuild);
            assert!(
                refused
                    .path
                    .as_deref()
                    .is_some_and(|path| path.ends_with("1.sqlite3")),
                "{:?}",
                refused.path
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Starts reading accounts on a timer and pushing what a person does, and tells the window when
/// something changed.
///
/// Composition only: what to read is decided by the use cases, when to read it by the scheduler,
/// and this is the one place that knows both exist.
///
/// Answers with the handle a command uses to push a change to its server. Nothing to read into
/// means no handle: the window still opens, and it explains why there is no mail in it.
pub fn start_background_sync(
    app: &tauri::AppHandle,
    storage: &Storage,
    in_flight: InFlight,
    credentials: crate::Credentials,
    bell: crate::infrastructure::bell::Bell,
    // The one limit on mailboxes talking to a server at once, shared with the window's own way of
    // asking so that a round and a person pressing refresh cannot be twice it between them.
    at_once: Arc<tokio::sync::Semaphore>,
) -> Option<crate::infrastructure::pushes::Pushes> {
    let (Ok(profile), Ok(indexes)) = (storage.profile(), storage.every_index()) else {
        // Nothing runs from here on: no round, no watch, and so no pass that could ever clear the
        // last failure off the screen. That is the shape worth saying out loud — a workspace whose
        // sentence about itself can no longer change, because the only thing that rewrites it was
        // never started.
        use crate::ports::journal::{Journal, Note};

        let journal = crate::infrastructure::journal::journal();
        if journal.keeps(Note::Trouble) {
            journal.write_down(Note::Trouble, "sync.neverStarted", &[]);
        }

        return None;
    };

    let runtime = tauri::async_runtime::handle().inner().clone();
    let sync = WorkspaceSync::new(profile, indexes, runtime.clone(), credentials);

    schedule::start(
        Arc::clone(&sync),
        in_flight.clone(),
        runtime.clone(),
        Arc::clone(&at_once),
        announcing(app, bell.clone()),
    );

    let pushes = crate::infrastructure::pushes::Pushes::start(
        Arc::clone(&sync) as Arc<dyn crate::ports::account_sync::AccountPush>,
        in_flight.clone(),
        runtime.clone(),
        Arc::clone(&at_once),
        Arc::new(announcing(app, bell.clone())),
    );

    // Both are needed and neither replaces the other: a server that can push says so at once, and
    // one that cannot is read by the timer like it always was.
    schedule::watch_accounts(
        Arc::clone(&sync),
        sync,
        in_flight,
        runtime,
        at_once,
        announcing(app, bell),
    );

    Some(pushes)
}

/// Telling the window that an account changed, however the change was found.
///
/// A round and a watch announce the same thing in the same words: what reached the window is the
/// mail, not the reason it was looked for.
/// Asks the bell, on a timer, and interrupts if the answer is yes.
///
/// A timer rather than a call at the end of a round: a wave is closed by time passing, and the
/// last account to report may be the one that never reports again. Its own thread because the
/// question is asked while nothing else is happening, and because a round must not wait on a
/// notification.
pub fn ring_when_there_is_something_to_say(
    app: &tauri::AppHandle,
    bell: crate::infrastructure::bell::Bell,
) {
    use crate::application::announce_mail::AnnounceMail;
    use crate::infrastructure::notification::SystemNotifier;
    use crate::infrastructure::sqlite::settings_repository::SqlitePreferencesRepository;

    let app = app.clone();
    let product = app.config().product_name.clone().unwrap_or_default();

    std::thread::spawn(move || loop {
        // A second is short enough that a wave closes when it should and long enough that this is
        // a thread doing nothing rather than a thread spinning.
        std::thread::sleep(std::time::Duration::from_secs(1));

        let Some(unseen) = bell.what_to_say(std::time::Instant::now()) else {
            continue;
        };

        let Some(state) = tauri::Manager::try_state::<crate::AppState>(&app) else {
            continue;
        };

        // Nothing is said until the window has handed over the words. Rust writing its own prose
        // is forbidden outright, and English in a Russian interface would be worse than silence.
        let Some(sentence) = state.words.for_new_mail() else {
            continue;
        };

        let (Ok(profile), Ok(indexes)) = (state.storage.profile(), state.storage.index_across())
        else {
            continue;
        };

        let _ = AnnounceMail::new(
            SqlitePreferencesRepository::new(profile),
            state.attention.clone(),
            SystemNotifier::new(app.clone()),
            crate::infrastructure::sqlite::arrivals::SqliteArrivals::spanning(indexes),
            product.clone(),
        )
        .execute(unseen, &sentence);
    });
}

/// Puts what nobody has read on the application's own icon.
///
/// Called wherever the count can have moved rather than on a timer: the query walks every
/// account's inbox, and a second is far too often to ask a question whose answer changes when mail
/// arrives or somebody opens a message.
///
/// Every failure here is silence. A badge is the smallest thing on the screen, and nothing about
/// mail should stop because a system would not draw a number on an icon.
pub fn redraw_the_badge(app: &tauri::AppHandle) {
    use crate::ports::badge::Badge;

    let Some(state) = tauri::Manager::try_state::<crate::AppState>(app) else {
        return;
    };
    let (Ok(profile), Ok(indexes)) = (state.storage.profile(), state.storage.index_across()) else {
        return;
    };
    let Ok(unread) = crate::read_model::mailboxes::MailboxReadModel::spanning(profile, indexes)
        .unread_everywhere()
    else {
        return;
    };

    let _ = crate::infrastructure::badge::WindowBadge::new(app.clone()).show(unread);
}

/// Redraws the number on the icon off the thread that asked: one drawing at a time and one behind
/// it, however fast the changes come, because the count is a walk over every inbox.
pub fn redraw_the_badge_soon(app: &tauri::AppHandle) {
    use crate::infrastructure::schedule::OneBehind;

    static DRAWING: OneBehind = OneBehind::new();
    if DRAWING.asked() {
        let drawing = app.clone();
        tauri::async_runtime::spawn_blocking(move || loop {
            redraw_the_badge(&drawing);
            if !DRAWING.finished() {
                break;
            }
        });
    }
}

/// Tells the window that an account's mail changed, and redraws the number on the icon with it.
///
/// One function for a round, a watcher and a check a person asked for, so the list and the icon
/// cannot come to disagree about which of them moved the mail.
pub fn mail_changed(
    app: &tauri::AppHandle,
    account_id: &str,
    arrived: u32,
    updated: u32,
    removed: u32,
) {
    use tauri::Emitter;

    redraw_the_badge_soon(app);

    // The interface reads again when it hears this. What changed is included so a future
    // version can be quieter about it than "read everything again".
    let _ = app.emit(
        MAIL_CHANGED,
        MailChanged {
            account_id: account_id.to_owned(),
            arrived,
            updated,
            removed,
        },
    );
}

fn announcing(
    app: &tauri::AppHandle,
    // Handed in rather than read from the state: this runs before the state is managed, and a
    // bell fetched here would be a second one that nothing ever asks.
    bell: crate::infrastructure::bell::Bell,
) -> impl Fn(&str, AccountChanged) + Send + Sync + 'static {
    let window = app.clone();

    move |account_id: &str, changed: AccountChanged| {
        bell.reports(
            account_id,
            changed.unseen_in_inbox,
            std::time::Instant::now(),
        );

        // The number on the icon goes with the list: mail arriving raises it here, and a message
        // being opened lowers it the moment it is written down, from the command.
        mail_changed(
            &window,
            account_id,
            changed.arrived,
            changed.updated,
            changed.removed,
        );
    }
}

/// Registers the scheme a message body is served on.
///
/// Composition, and the only place that knows both that bodies are held somewhere and that a
/// webview asks for them over HTTP. What comes back carries its own policy in its own header: the
/// application's policy cannot weaken it, and the frame's origin is not the application's.
pub fn serve_message_bodies(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder.register_asynchronous_uri_scheme_protocol(
        serving::SCHEME,
        |context, request, responder| {
            use tauri::Manager;

            use serving::{answer, document};

            if request.method() != tauri::http::Method::GET {
                responder.respond(document(405, Vec::new()));
                return;
            }

            let held = context
                .app_handle()
                .state::<crate::AppState>()
                .bodies
                .clone();
            let name = request.uri().path().trim_start_matches('/');

            if let Some((page, policy)) = held.page(name) {
                responder.respond(answer(
                    200,
                    "text/html; charset=utf-8",
                    policy,
                    page.into_bytes(),
                ));
                return;
            }

            // A link the message offered, arriving here rather than at the navigation handler.
            //
            // On this machine's engine it never does: the navigation is refused before a request is
            // made. WebView2 does not hand a frame's navigation to that handler at all, so on
            // Windows the request arrives, and this is what answers it — the window is told, the
            // same as it would have been, and nothing is served. Whether the frame then keeps the
            // message on screen is the engine's decision and is written down as unverified.
            if let Some(name) = name.strip_prefix(&format!("{}/", serving::LINKS)) {
                link_followed(context.app_handle(), name);

                responder.respond(answer(
                    204,
                    "text/plain",
                    serving::NOTHING_AT_ALL,
                    Vec::new(),
                ));
                return;
            }

            // A picture that travelled inside the message, asked for by the body that refers to it.
            // Served under a policy that lets it do nothing at all, in case anything ever asks for
            // one as a page rather than as a picture.
            if let Some(part) = held.part(name) {
                responder.respond(answer(
                    200,
                    &part.content_type,
                    serving::NOTHING_AT_ALL,
                    part.bytes,
                ));
                return;
            }

            // A name that is not one of the ones alive fetches nothing, and says nothing about
            // why -- to the frame. To whoever is running a development build it says it out loud:
            // a picture asked for and not here is a message rendering with holes in it, and every
            // hour spent on that has started with nobody able to tell it from a picture that was
            // never fetched.
            // Nothing about which name: a name is a secret, and what the reader needs is that a
            // picture is missing rather than which.
            {
                use tauri::Emitter;

                let _ = context.app_handle().emit(PICTURE_MISSING, ());
            }

            #[cfg(debug_assertions)]
            eprintln!(
                "lyonoyl: the frame asked for something this store does not have; \
                 {} missed so far",
                held.asked()
                    .missed
                    .load(std::sync::atomic::Ordering::Relaxed)
            );

            responder.respond(document(404, Vec::new()));
        },
    )
}

/// The event the window listens for when a message's link is followed.
pub const LINK_FOLLOWED: &str = "message:link";

/// What the window is told when a picture the page points at is asked for and is not here.
///
/// The counts the reader is shown are worked out when the message is opened, and the frame comes
/// for the pictures after that -- the page reaches it long before they do. So a picture can be
/// counted as shown and then be gone by the time it is wanted, and nothing said so: the message
/// simply rendered with holes in it, and the reader was told a story in which everything arrived.
/// This is the frame saying otherwise, and it is what turns that into something a person can see
/// and something a report can name.
pub const PICTURE_MISSING: &str = "message:picture-missing";

use crate::commands::body::asked_about;

/// Tells the window that a link was followed, so it can ask the person about it.
///
/// Called from the navigation handler, which has already refused the navigation: the message is
/// still on screen, and being asked is the only thing that happened.
pub fn link_followed(app: &tauri::AppHandle, name: &str) {
    use tauri::{Emitter, Manager};

    let held = app.state::<crate::AppState>().bodies.clone();

    if let Some(url) = held.link(name) {
        let _ = app.emit(LINK_FOLLOWED, asked_about(name, &url));
    }
}

/// The event the window listens for.
pub const MAIL_CHANGED: &str = "mail:changed";

/// What the window hears the moment a person's change is written down, before any server has.
///
/// Its own event rather than [`MAIL_CHANGED`], because the two are answered differently: the
/// list already shows what the person did, so only the counts around it are read again.
pub const MAIL_ASKED: &str = "mail:asked";

#[derive(Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct MailAsked {
    pub account_id: String,
}

#[derive(Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct MailChanged {
    pub account_id: String,
    pub arrived: u32,
    pub updated: u32,
    pub removed: u32,
}

/// Moves aside the files that would not open, so the application can start.
///
/// Moved and never deleted. `profile.sqlite3` holds the only copy of what a person wrote and has
/// not sent, and the rule that it is never removed on the application's own initiative is not
/// softened by the person having pressed something: what a press may do is decide that this file
/// stops being the one in use, and what it may not do is make it unrecoverable.
///
/// So each file is renamed with the moment beside it, the directory is left otherwise alone, and
/// what comes back is where the old ones went — which is what the window shows, because a person
/// told their mail was set aside and not told where has been told nothing.
pub fn set_aside_what_will_not_open(databases: &Path, at: u64) -> Result<Vec<String>, String> {
    // The mail files as well as the profile. They are a copy of what the server holds, so nothing
    // in them is lost — but a build that has changed how it writes them cannot read the old ones
    // either, and leaving them would be starting fresh with yesterday's mail in the way.
    const MOVED: [&str; 2] = ["profile.sqlite3", "index.sqlite3"];

    let mut moved = Vec::new();

    for name in MOVED {
        // The write-ahead log and the shared-memory file travel with the database: what it holds
        // is in them until a checkpoint, so a database moved without them is a database moved in
        // part.
        for suffix in ["", "-wal", "-shm"] {
            let from = databases.join(format!("{name}{suffix}"));
            if !from.exists() {
                continue;
            }

            let to = databases.join(format!("{name}{suffix}.set-aside-{at}"));
            std::fs::rename(&from, &to).map_err(|error| format!("{}: {error}", from.display()))?;

            if suffix.is_empty() {
                moved.push(to.display().to_string());
            }
        }
    }

    // Mail kept one file per account. Set aside whole, for the reason above.
    let accounts = databases.join("accounts");
    if accounts.exists() {
        let to = databases.join(format!("accounts.set-aside-{at}"));
        std::fs::rename(&accounts, &to)
            .map_err(|error| format!("{}: {error}", accounts.display()))?;
        moved.push(to.display().to_string());
    }

    Ok(moved)
}
