//! The databases a start finds at the top of the data folder, brought into their own folder and
//! opened there with everything they held — the transactions their write-ahead logs still hold
//! among it.

use super::gathering::after_gathering;
use super::{databases_of, gather_the_databases, in_a_file, init_storage};
use super::{Storage, Unopened};
use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::infrastructure::secure_storage::SECRETS_FILE;
use crate::infrastructure::sqlite::folder::{Gathered, NotGathered};

/// A folder of its own, taken away afterwards.
struct Folder(std::path::PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "lyonoyl-gathering-{name}-{}-{}",
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

/// Something only this profile holds, written so a later start can look for it.
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

/// A profile at the top of a data folder as an application stopped without folding its log in
/// leaves it: the files copied while it is open, its last transactions in the log alone. Each
/// file goes to where `put` says, under the data folder.
fn stopped_with_a_mark_in_its_log(data: &Folder, put: impl Fn(&str) -> String) {
    let running = Folder::new("running");
    let storage = init_storage(&running.0, &DatabaseKey::for_a_test());
    write_a_mark(&storage);

    let log = std::fs::metadata(running.0.join("profile.sqlite3-wal"))
        .expect("a write-ahead log")
        .len();
    assert!(
        log > 0,
        "the log holds nothing, and the test would prove nothing"
    );

    for name in [
        "profile.sqlite3",
        "profile.sqlite3-wal",
        "profile.sqlite3-shm",
    ] {
        let to = data.0.join(put(name));
        std::fs::create_dir_all(to.parent().expect("a parent")).expect("its folder");
        std::fs::copy(running.0.join(name), &to).expect("copied");
    }
    drop(storage);
}

#[test]
fn a_profile_whose_last_transactions_are_in_its_log_opens_with_them_in_its_folder() {
    let data = Folder::new("whole");
    stopped_with_a_mark_in_its_log(&data, str::to_owned);

    let (databases, refused) = gather_the_databases(&data.0);

    assert!(refused.is_none());
    assert_eq!(databases, data.0.join("databases"));
    assert!(!data.0.join("profile.sqlite3").exists());
    let opened = init_storage(&databases, &DatabaseKey::for_a_test());
    assert_eq!(the_mark(&opened).as_deref(), Some("here"));
}

#[test]
fn a_profile_carried_without_its_log_loses_what_the_log_held() {
    // The other half of the test above, and the reason the log travels with the profile: carried
    // alone, the profile opens without the transactions only the log held.
    let data = Folder::new("without");
    stopped_with_a_mark_in_its_log(&data, str::to_owned);
    std::fs::remove_file(data.0.join("profile.sqlite3-wal")).expect("the log taken away");
    std::fs::remove_file(data.0.join("profile.sqlite3-shm")).expect("its index taken away");

    let (databases, _) = gather_the_databases(&data.0);

    let opened = init_storage(&databases, &DatabaseKey::for_a_test());
    assert_eq!(the_mark(&opened), None);
}

#[test]
fn a_set_a_stopped_move_left_in_two_folders_is_made_whole_by_the_next_start() {
    // The log in the folder and the profile at the top, as a move that could not put it back
    // leaves them.
    let data = Folder::new("split");
    stopped_with_a_mark_in_its_log(&data, |name| match name {
        "profile.sqlite3-wal" => format!("databases/{name}"),
        _ => name.to_owned(),
    });

    let (databases, refused) = gather_the_databases(&data.0);

    assert!(refused.is_none());
    assert_eq!(databases, data.0.join("databases"));
    let opened = init_storage(&databases, &DatabaseKey::for_a_test());
    assert_eq!(the_mark(&opened).as_deref(), Some("here"));
}

#[test]
fn a_run_whose_move_could_not_put_everything_back_opens_neither_folder() {
    let data = Folder::new("refused");
    let stopped = NotGathered {
        path: data.0.join("profile.sqlite3"),
        error: std::io::Error::other("held by somebody else"),
        put_back: false,
    };

    let (_, refused) = after_gathering(&data.0, Err(stopped));

    let refused = refused.expect("the run refuses its databases");
    assert!(refused.profile().is_err());
    assert!(matches!(
        &*refused.held(),
        super::Held::Failed {
            why: Unopened::TheFile,
            path,
            ..
        } if path == &data.0.join("profile.sqlite3").display().to_string()
    ));
}

#[test]
fn a_run_whose_move_put_everything_back_reads_the_set_where_it_is() {
    // Whole at the top: this run reads it there, and the next start moves it again.
    let data = Folder::new("put-back");
    std::fs::write(data.0.join("profile.sqlite3"), b"a profile").expect("written");
    let stopped = NotGathered {
        path: data.0.join("profile.sqlite3"),
        error: std::io::Error::other("held for a moment"),
        put_back: true,
    };

    let (databases, refused) = after_gathering(&data.0, Err(stopped));

    assert!(refused.is_none());
    assert_eq!(databases, data.0);
}

#[test]
fn a_start_that_moved_nothing_reads_the_databases_folder() {
    let data = Folder::new("first");

    let (databases, refused) = after_gathering(&data.0, Ok(Gathered::default()));

    assert!(refused.is_none());
    assert_eq!(databases, data.0.join("databases"));
    assert_eq!(databases_of(&data.0), databases);
}

#[test]
fn the_store_a_key_is_read_from_is_decided_beside_the_gathered_profile() {
    // A device whose secrets are in a file: after the move the file is beside the profile in the
    // folder, and the start chooses it without asking a keyring.
    let data = Folder::new("store");
    std::fs::write(data.0.join(SECRETS_FILE), b"secrets").expect("written");
    std::fs::write(data.0.join("profile.sqlite3"), b"a profile").expect("written");

    let (databases, _) = gather_the_databases(&data.0);

    assert!(in_a_file(&databases, true, || panic!(
        "a keyring was asked where the folder already says the key is in a file"
    )));
    assert!(
        !data.0.join(SECRETS_FILE).exists(),
        "the file stayed at the top"
    );
}

#[cfg(unix)]
#[test]
fn the_data_folder_is_reachable_by_this_person_alone_whatever_it_holds() {
    // Nothing of the databases at its top any more: the folder is narrowed for what else it holds.
    use std::os::unix::fs::PermissionsExt;

    let data = Folder::new("narrowed");
    std::fs::set_permissions(&data.0, std::fs::Permissions::from_mode(0o755)).expect("widened");

    gather_the_databases(&data.0);

    let mode = std::fs::metadata(&data.0)
        .expect("the folder")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
}
