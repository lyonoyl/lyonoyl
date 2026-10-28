use std::path::Path;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::config::DbConfig;

use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::infrastructure::sqlite::migrations::{
    written_ahead_of_the_index, written_ahead_of_the_profile,
};
use thiserror::Error;

pub type SqlitePool = Pool<SqliteConnectionManager>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("cannot open the database at {path}: {reason}")]
    Open { path: String, reason: String },
    #[error("cannot migrate the database at {path}: {reason}")]
    Migrate { path: String, reason: String },
    /// A newer build wrote this file: its schema is past every migration this one has. Nothing in
    /// it was changed, and nothing here changes it.
    #[error("the database at {path} was written by a newer build, at version {found}")]
    FromANewerBuild { path: String, found: usize },
}

impl StorageError {
    /// The path is shown to the user on the recovery screen or in the sidebar, so it is kept in
    /// the error.
    /// The `reason` string must never carry SQL text or user data.
    pub fn path(&self) -> &str {
        match self {
            Self::Open { path, .. }
            | Self::Migrate { path, .. }
            | Self::FromANewerBuild { path, .. } => path,
        }
    }
}

/// Opens the authoritative profile database.
///
/// `profile.sqlite3` holds the only copy of data that exists on no server: accounts, settings,
/// op_log, outbox and drafts. `synchronous = FULL` keeps a committed row across a power loss.
pub fn open_profile(path: impl AsRef<Path>, key: &DatabaseKey) -> Result<SqlitePool, StorageError> {
    // One file, shared by every account being read. A pass asks it for the account, the queue and
    // the outbox — several at the same moment — so the number here has to cover `schedule::AT_ONCE`
    // passes holding a few each, with room left for the window asking its own questions beside
    // them. At four it was the ceiling on the round however wide the round itself was allowed to
    // be; at anything too small the window waits behind the passes, which is the same fault
    // wearing a different face. A connection nobody asks for is never opened, so this is a
    // ceiling rather than a cost.
    open_holding(path, "FULL", key, 64, written_ahead_of_the_profile)
}

/// Opens the derived mail index.
///
/// `index.sqlite3` is a local copy of what the server already holds, so it trades durability for
/// speed: `synchronous = NORMAL` risks losing the last transactions after a power cut, and those
/// transactions are re-fetched on the next sync.
pub fn open_index(path: impl AsRef<Path>, key: &DatabaseKey) -> Result<SqlitePool, StorageError> {
    // One file per account, so this is per account and not in total: a pass reads its own index
    // and nobody else's. Wide enough that the window can still read the sidebar while a pass has
    // the file — at four it could not, and a sidebar drawn then came back with no folders, which
    // the window read as "this account is new" and asked the server about. The window knows better
    // now, and this is the other half: it should rarely have to.
    open_holding(path, "NORMAL", key, 12, written_ahead_of_the_index)
}

/// Whether this file was written by a build that kept its databases in the clear.
///
/// Every page is encrypted now, header included, so a file that still begins with SQLite's own
/// mark is one this build cannot read. Said out loud rather than left to surface as a corrupt
/// database: nothing has shipped, so there is no upgrade to write — the file goes and the mail
/// arrives again.
fn written_in_the_clear(path: &Path) -> bool {
    use std::io::Read;

    let mut head = [0u8; 16];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut head))
        .is_ok()
        && &head == b"SQLite format 3\0"
}

/// PRAGMAs are applied through `with_init`, so they run on **every** pooled connection.
///
/// These are per-connection settings; applying them once at open time would leave foreign keys
/// and the busy timeout disabled on every other connection in the pool.
///
/// `ahead` says how far past this build the file's own version is, for the kind of file this is.
fn open_holding(
    path: impl AsRef<Path>,
    synchronous: &'static str,
    key: &DatabaseKey,
    connections: u32,
    ahead: fn(&rusqlite::Connection) -> Option<usize>,
) -> Result<SqlitePool, StorageError> {
    let path = path.as_ref().to_owned();
    let display = path.display().to_string();

    if written_in_the_clear(&path) {
        return Err(StorageError::Open {
            path: display,
            reason: "Written before this build encrypted its databases. Remove the data directory \
                     and let the mail arrive again."
                .to_owned(),
        });
    }

    // Asked once, before the pool is built. A key this file does not answer to otherwise presents
    // as thirty seconds of waiting for a connection and then as a database that will not open —
    // the same face a broken file wears, for a cause a person could act on.
    let refused = || StorageError::Open {
        path: display.clone(),
        reason: "The key this device keeps its mail under does not open this file.".to_owned(),
    };
    {
        let connection = rusqlite::Connection::open(&path).map_err(|_| refused())?;
        key.unlock(&connection).map_err(|_| refused())?;
        // Asked before a pool is built over the file and before its mode is changed: a file a
        // newer build wrote is refused as it stands, and that build finds it as it left it. Read
        // from the first page, so it is answered even where the schema is in SQL this build
        // cannot parse.
        if let Some(found) = ahead(&connection) {
            // Closed without a checkpoint. A newer build that quit with its connections open left
            // the last of what it wrote in the log beside the file, and the last connection to
            // close folds the log into the file: that would be this one, writing a newer build's
            // file on the way out.
            let _ = connection.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true);
            return Err(StorageError::FromANewerBuild {
                path: display.clone(),
                found,
            });
        }
        connection
            .query_row("SELECT count(*) FROM sqlite_schema", [], |_| Ok(()))
            .map_err(|_| refused())?;
    }

    let key = key.clone();
    let manager = SqliteConnectionManager::file(&path).with_init(move |connection| {
        // First, and on its own: every statement after it is answered from the encrypted file, and
        // one placed before it would be answered from a file this build cannot read.
        key.unlock(connection)?;

        connection.execute_batch(&format!(
            // `temp_store` and `secure_delete` are here because of what these files hold. A
            // message kept on this device is sealed before it is written, and a large sort or an
            // index merge that spilled to a temporary file would write the parts it is working on
            // somewhere nothing seals — so the spill goes to memory instead. `secure_delete`
            // overwrites a freed page rather than leaving what was on it: a deleted message that
            // is still legible in the file is a deleted message that was not deleted.
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = {synchronous};
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA secure_delete = ON;
             PRAGMA busy_timeout = 5000;"
        ))
    });

    // Readable by this person and nobody else. Mandatory, and not implemented until the mail itself
    // moved into these files: the sealing keeps the messages unreadable, and this keeps everything
    // beside them — who wrote, when, what the subject was — off the terminal of anybody else with
    // an account on the machine. Windows has no mode bits and inherits the directory's ACL instead,
    // which is why this is where it is rather than everywhere.
    let pool = Pool::builder()
        .max_size(connections)
        // One ready, the rest made when somebody asks. Without this the pool opens `max_size`
        // connections before `build` returns, and the window waits all of them: ten files at a
        // dozen connections each is a hundred and seventy opens before anything is on screen,
        // measured at eleven seconds against fifty milliseconds with this line.
        .min_idle(Some(1))
        // Waiting is not the same as hanging. The default is thirty seconds, and a window asking
        // a question while every account is being read waits all of them before it hears anything
        // — which is a frozen interface, not a slow one. Five is long enough for a pass to give a
        // connection back and short enough that a person notices a wrong answer rather than a
        // dead window.
        .connection_timeout(std::time::Duration::from_secs(5))
        .build(manager)
        .map_err(|error| StorageError::Open {
            path: display,
            reason: error.to_string(),
        })?;

    // After the pool, because the pool is what creates the file. Before it there was nothing to
    // open on a first run, the failure went into a discarded result, and the database spent that
    // run readable by anybody with an account on the machine — the run that fills it with mail.
    #[cfg(unix)]
    {
        // The write-ahead log holds what the database holds and does not take its mode from it,
        // so it has to exist before it can be closed off. An empty transaction is what brings it
        // into being; nothing else here would, until the first write.
        if let Ok(connection) = pool.get() {
            let _ = connection.execute_batch("BEGIN IMMEDIATE; COMMIT;");
        }

        for beside in ["", "-wal", "-shm"] {
            // Borrowed rather than owned: the helper reads the path and keeps nothing, and a
            // `PathBuf` here meant an import that only this `cfg(unix)` block used — which left it
            // dangling on Windows, where `-D warnings` refuses an unused import and the build
            // stopped.
            only_for_this_person(Path::new(&format!("{}{beside}", path.display())));
        }
    }

    Ok(pool)
}

/// Closes one file off to everybody but the person running this.
///
/// Best effort on purpose: a filesystem that does not carry modes at all — a network mount, a
/// share — is a worse place to keep mail than a local disk, and refusing to open it there would
/// stop somebody reading their mail over a defence that filesystem never had.
#[cfg(unix)]
fn only_for_this_person(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    if let Ok(handle) = std::fs::File::open(path) {
        let _ = handle.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The run that matters is the first one: it is the run that creates the files and fills them
    /// with mail. Setting a mode on a file that does not exist yet fails into a discarded result,
    /// and nothing said so — which is what happened, in both places, for as long as the mode was
    /// claimed as a shipped control.
    #[cfg(unix)]
    #[test]
    fn a_database_is_unreadable_by_anybody_else_from_the_first_run() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-modes-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("temp dir");

        let path = dir.join("index.sqlite3");
        let pool = open_index(&path, &DatabaseKey::for_a_test()).expect("pool opens");
        {
            // Something is written, so the write-ahead log exists to be looked at too.
            let connection = pool.get().expect("connection");
            connection
                .execute_batch("CREATE TABLE a (b INTEGER); INSERT INTO a VALUES (1);")
                .expect("a write");
        }

        for beside in ["", "-wal", "-shm"] {
            let each = dir.join(format!("index.sqlite3{beside}"));
            if !each.exists() {
                continue;
            }

            let mode = std::fs::metadata(&each)
                .expect("readable")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(
                mode & 0o077,
                0,
                "{} is mode {mode:o}, which anybody with an account on this machine can read",
                each.display()
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A subject written through a pool is not in the file, nor in the log beside it.
    ///
    /// The whole claim in one assertion. The log matters as much as the database: what a database
    /// holds is in its write-ahead log too until a checkpoint, and the log is a file somebody
    /// reading the disk finds in the same directory.
    #[test]
    fn nothing_written_through_a_pool_is_readable_in_the_files_it_writes() {
        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-sealed-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("temp dir");

        let path = dir.join("index.sqlite3");
        let pool = open_index(&path, &DatabaseKey::for_a_test()).expect("pool opens");
        {
            let connection = pool.get().expect("connection");
            connection
                .execute_batch(
                    "CREATE TABLE m (subject TEXT, sender TEXT);
                     INSERT INTO m VALUES ('Quarterly numbers', 'zeta@example.com');",
                )
                .expect("a write");
        }

        let mut looked_at = 0;
        for beside in ["", "-wal"] {
            let each = dir.join(format!("index.sqlite3{beside}"));
            let Ok(bytes) = std::fs::read(&each) else {
                continue;
            };
            looked_at += 1;

            for written in [
                b"Quarterly numbers".as_slice(),
                b"zeta@example.com".as_slice(),
            ] {
                assert!(
                    !bytes.windows(written.len()).any(|part| part == written),
                    "{} holds {} in the clear",
                    each.display(),
                    String::from_utf8_lossy(written)
                );
            }
        }

        assert!(looked_at > 0, "no file was written, so nothing was checked");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file written under one key does not answer to another.
    #[test]
    fn a_file_does_not_open_under_a_key_it_was_not_written_under() {
        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-other-key-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("temp dir");

        let path = dir.join("index.sqlite3");
        {
            let pool = open_index(&path, &DatabaseKey::for_a_test()).expect("pool opens");
            let connection = pool.get().expect("connection");
            connection
                .execute_batch("CREATE TABLE m (subject TEXT); INSERT INTO m VALUES ('Hello');")
                .expect("a write");
        }

        let elsewhere = DatabaseKey::from(&[0x11; 32]);
        let began = std::time::Instant::now();
        let refused = open_index(&path, &elsewhere);

        let Err(StorageError::Open { reason, .. }) = refused else {
            panic!("a file opened under a key it was not written under answered anyway");
        };
        assert!(reason.contains("does not open this file"), "{reason}");
        // Said at once. Waiting for the pool to give up instead is half a minute of a window that
        // looks like it is still working.
        assert!(
            began.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            began.elapsed()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A schema a newer build wrote in SQL this one cannot parse is still that build's file.
    ///
    /// The key opens the file and the version on its first page can be read, so a version past every
    /// migration here says what the file is. A version this build knows says nothing of the kind,
    /// and the file stays one that does not open.
    #[test]
    fn a_schema_this_build_cannot_parse_is_a_newer_builds_when_its_version_says_so() {
        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-unparsed-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("temp dir");
        let key = DatabaseKey::for_a_test();
        let newest = {
            let mut connection = rusqlite::Connection::open_in_memory().expect("in memory");
            crate::infrastructure::sqlite::migrations::migrate_profile(&mut connection, ":memory:")
                .expect("schema applies");
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .expect("readable") as usize
        };

        for (version, newer) in [(newest + 1, true), (1, false)] {
            let path = dir.join(format!("profile-{version}.sqlite3"));
            {
                let pool = open_profile(&path, &key).expect("pool opens");
                pool.get()
                    .expect("connection")
                    .execute_batch(&format!(
                        "CREATE TABLE kept (a INTEGER);
                         PRAGMA writable_schema = ON;
                         UPDATE sqlite_schema
                            SET sql = 'CREATE TABLE kept (a INTEGER) WITH NOTHING THIS BUILD KNOWS'
                          WHERE name = 'kept';
                         PRAGMA writable_schema = OFF;
                         PRAGMA user_version = {version};"
                    ))
                    .expect("written as a newer build would");
            }

            match (open_profile(&path, &key), newer) {
                (Err(StorageError::FromANewerBuild { found, .. }), true) => {
                    assert_eq!(found, version)
                }
                (Err(StorageError::Open { reason, .. }), false) => {
                    assert!(reason.contains("does not open this file"), "{reason}")
                }
                (opened, _) => panic!("version {version}: {:?}", opened.err()),
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file from a build that kept its databases in the clear is refused by name.
    ///
    /// Rather than opened and found broken. The two look alike from inside SQLite, and only one of
    /// them a person can do anything about.
    #[test]
    fn a_file_written_in_the_clear_is_refused_and_says_why() {
        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-plaintext-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("temp dir");

        let path = dir.join("index.sqlite3");
        std::fs::write(&path, b"SQLite format 3\0and then whatever it held").expect("a file");

        let refused = open_index(&path, &DatabaseKey::for_a_test());
        let Err(StorageError::Open { reason, .. }) = refused else {
            panic!("a database written in the clear was opened");
        };
        assert!(
            reason.contains("Remove the data directory"),
            "the refusal does not say what to do about it: {reason}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn applies_pragmas_to_every_pooled_connection() {
        let dir = std::env::temp_dir().join(format!("lyonoyl-pragma-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir is creatable");
        let pool = open_profile(dir.join("profile.sqlite3"), &DatabaseKey::for_a_test())
            .expect("pool opens");

        // Take several connections: a PRAGMA applied only at open time would fail here.
        for _ in 0..3 {
            let connection = pool.get().expect("a pooled connection is available");
            let foreign_keys: i64 = connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                .expect("pragma is readable");
            assert_eq!(
                foreign_keys, 1,
                "foreign keys must be on for every connection"
            );

            let synchronous: i64 = connection
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .expect("pragma is readable");
            assert_eq!(
                synchronous, 2,
                "the profile database must use synchronous = FULL"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn keeps_what_it_sorts_in_memory_and_overwrites_what_it_frees() {
        // Encryption a temporary file or a freed page undoes is not encryption: a sort that spills
        // goes to memory, and a deleted row is written over rather than left in a free page. On
        // every connection of both kinds of file, since these are per-connection settings.
        let dir = std::env::temp_dir().join(format!("lyonoyl-spill-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir is creatable");
        let key = DatabaseKey::for_a_test();
        let pools = [
            open_profile(dir.join("profile.sqlite3"), &key).expect("the profile opens"),
            open_index(dir.join("1.sqlite3"), &key).expect("an account's file opens"),
        ];

        for pool in &pools {
            // Held at once, so they are three connections rather than one handed back three times.
            let held: Vec<_> = (0..3)
                .map(|_| pool.get().expect("a pooled connection is available"))
                .collect();
            for connection in &held {
                let temp_store: i64 = connection
                    .query_row("PRAGMA temp_store", [], |row| row.get(0))
                    .expect("pragma is readable");
                assert_eq!(temp_store, 2, "a sort that spills must go to memory");

                let secure_delete: i64 = connection
                    .query_row("PRAGMA secure_delete", [], |row| row.get(0))
                    .expect("pragma is readable");
                assert_eq!(secure_delete, 1, "a freed page must be written over");
            }
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_index_trades_durability_for_speed() {
        let dir = std::env::temp_dir().join(format!("lyonoyl-pragma-index-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir is creatable");
        let pool =
            open_index(dir.join("index.sqlite3"), &DatabaseKey::for_a_test()).expect("pool opens");

        let connection = pool.get().expect("a pooled connection is available");
        let synchronous: i64 = connection
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("pragma is readable");
        assert_eq!(synchronous, 1, "derived data uses synchronous = NORMAL");

        let journal: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("pragma is readable");
        assert_eq!(journal.to_lowercase(), "wal");

        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod opening_a_workspace {
    use super::*;
    use crate::infrastructure::secure_storage::database_key::DatabaseKey;

    #[test]
    fn a_file_opens_with_one_connection_rather_than_every_one_it_may_ever_need() {
        // What `min_idle` is for, and it is a clock-free way to say it. Left unset, r2d2 opens
        // `max_size` connections before `build` returns, and the window waits all of them.
        //
        // A ceiling rather than a cost: the pool may still grow to its size, it simply does not do
        // it while somebody is waiting to see their mail. The assertion is an upper bound rather
        // than an exact count because `min_idle` is kept by a thread of r2d2's own — opening the
        // file checks a connection out and a replacement may or may not have been made by the time
        // this reads the state. What must never be true is the whole dozen.
        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-pool-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");

        let index = open_index(dir.join("index.sqlite3"), &DatabaseKey::for_a_test())
            .expect("the file opens");

        assert!(
            index.state().connections < 12,
            "every connection the pool may ever hold was opened before anybody asked for one: {}",
            index.state().connections
        );

        drop(index);
        std::fs::remove_dir_all(&dir).ok();
    }
}
