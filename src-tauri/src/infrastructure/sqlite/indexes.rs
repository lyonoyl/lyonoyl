//! Which file an account's mail is kept in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::infrastructure::sqlite::connection::{open_index, SqlitePool, StorageError};
use crate::infrastructure::sqlite::migrations::migrate_index;

/// The directory holding one file per account.
pub(crate) const ACCOUNTS: &str = "accounts";

/// What is added to a file's name to say that what it names is to go.
///
/// A file of its own rather than a rename, because on Windows a database somebody has open can be
/// neither removed nor renamed — measured, with a connection held — while making a new file beside
/// it always works.
const FORGOTTEN: &str = ".forget";

/// The marker that says this file is to go.
fn marker_for(path: &std::path::Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(FORGOTTEN);
    PathBuf::from(name)
}

/// How many bits of an identifier name the file it came from.
///
/// The rest — forty — number the row inside that file, which is a million million messages in one
/// account. Above them sit the account's own number and the sign bit, so the largest account this
/// device can number still mints identifiers that are positive.
const ROWS_PER_FILE: u32 = 40;

/// The files this device keeps mail in, opened as they are asked for.
///
/// An account's mail is in the file named after the number the index knows it by, and nowhere else.
/// That is what makes an identifier able to say where it came from: the file seeds its own counter
/// at `account << 40`, so every number it mints carries the account in its top bits and no two
/// files can mint the same one.
#[derive(Clone)]
pub struct Indexes {
    directory: PathBuf,
    /// What every one of these files is encrypted with. One key, so a file belongs to the
    /// installation rather than to whichever account happened to make it.
    key: DatabaseKey,
    /// Where the accounts are listed. The index does not know what accounts exist; the profile does.
    profile: SqlitePool,
    /// Opened on demand and kept, because opening one means migrating it.
    open: Arc<Mutex<HashMap<i64, SqlitePool>>>,
}

impl Indexes {
    pub fn under(databases: &Path, profile: SqlitePool, key: DatabaseKey) -> Self {
        Self {
            directory: databases.join(ACCOUNTS),
            key,
            profile,
            open: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The file this account's mail is kept in, opening it if this is the first time.
    ///
    /// Opening is also creating: a file that is not there is made, migrated and given its counter.
    /// An account added while the application runs therefore needs nothing done to it here.
    pub fn of(&self, account: i64) -> Result<SqlitePool, StorageError> {
        // Zero is nobody: the counter that hands out account numbers starts at one, so a zero here
        // is an identifier that was never tagged rather than an account that owns nothing.
        if account <= 0 {
            return Err(StorageError::Open {
                path: self.directory.display().to_string(),
                reason: "an identifier that names no account".to_owned(),
            });
        }

        let mut open = self.open.lock().map_err(|_| StorageError::Open {
            path: self.directory.display().to_string(),
            reason: "the list of open files is poisoned".to_owned(),
        })?;

        if let Some(pool) = open.get(&account) {
            return Ok(pool.clone());
        }

        let pool = self.make(account)?;
        open.insert(account, pool.clone());
        Ok(pool)
    }

    /// The file holding the folder named, which the number says.
    pub fn holding_folder(&self, folder: u64) -> Result<SqlitePool, StorageError> {
        self.of(kept_in(folder))
    }

    /// The file holding the message named, which the number says.
    pub fn holding_message(&self, message: u64) -> Result<SqlitePool, StorageError> {
        self.of(kept_in(message))
    }

    /// Every file holding mail on this device.
    ///
    /// The accounts come from the profile rather than from the directory: a file left behind by an
    /// account that was removed is not mail this device has, and reading the directory would show
    /// it as though it were.
    pub fn across(&self) -> Result<Vec<(i64, SqlitePool)>, StorageError> {
        let mut every = Vec::new();
        for account in self.numbers()? {
            every.push((account, self.of(account)?));
        }
        Ok(every)
    }

    /// Takes this account's mail off the disk.
    ///
    /// The file is closed first, by dropping the one this holds — but `of` hands out a clone to
    /// everybody who asks, and a round that is still running holds one. On macOS and Linux the file
    /// goes anyway, because a file can be unlinked while it is open. On Windows it cannot, and the
    /// removal fails with the mail still there.
    ///
    /// So what this promises is that the account is forgotten, and what `sweep` promises is that
    /// its bytes follow. A file that could not be removed now is removed the next time the
    /// application starts, when nothing holds it — and until then it belongs to no account, which
    /// is why `across` reads the profile rather than the directory.
    ///
    /// Three files, not one: SQLite keeps the write-ahead log and its shared-memory index beside
    /// the database, and what the database holds is in them too until a checkpoint. Leaving either
    /// would leave mail on the disk. A file that is not there is not a failure — the state asked
    /// for is the state that already holds.
    fn forget(&self, account: i64) -> Result<(), StorageError> {
        if let Ok(mut open) = self.open.lock() {
            open.remove(&account);
        }

        let base = self.directory.join(format!("{account}.sqlite3"));
        for path in [
            base.clone(),
            base.with_extension("sqlite3-wal"),
            base.with_extension("sqlite3-shm"),
        ] {
            // Deleted where it can be, and written down where it cannot. Windows refuses to
            // touch a file that is open, and `of` hands out a clone of the pool to everybody who
            // asks, so a round still running holds one. Measured on the guest, with a connection
            // open: removing it fails with "used by another process", and renaming it fails the
            // same way — so there is nothing to do to the file itself until it is closed.
            //
            // What can always be done is make a new file beside it. That marker is what `sweep`
            // reads at the next start, when nothing holds anything.
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    let _ = std::fs::write(marker_for(&path), b"");
                }
            }
        }

        Ok(())
    }

    /// Takes off the disk every file that belongs to no account, and everything marked to go.
    ///
    /// Run at the start, before anything opens anything: a file left behind by a removal that could
    /// not finish — because something still held it, which on Windows is enough to refuse — has
    /// nobody holding it now. The accounts come from the profile, so a file is an orphan when its
    /// number is not among them, and nothing else in the directory is touched.
    ///
    /// Returns how many it took, because a number nobody reads is still a number a test can assert.
    pub fn sweep(&self) -> Result<usize, StorageError> {
        let known = self.numbers()?;

        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            // No directory is no orphans. It is made when the first account is added.
            return Ok(0);
        };

        let mut taken = 0;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();

            // `7.sqlite3`, `7.sqlite3-wal`, `7.sqlite3-shm` — the number up to the first dot, and
            // only when the whole of it is digits. Anything else in here was not put there by this.
            let Some((number, rest)) = name.split_once('.') else {
                continue;
            };
            if !rest.starts_with("sqlite3") || number.is_empty() {
                continue;
            }
            let Ok(account) = number.parse::<i64>() else {
                continue;
            };

            // A marker is not mail, and it goes once what it names has gone.
            if rest.ends_with(FORGOTTEN) {
                continue;
            }

            // Told to go by a removal that could not finish. The account may still exist — the
            // mail of an account can be dropped without the account going with it — so this is
            // asked before the list is.
            let marked = marker_for(&entry.path()).exists();

            if !marked && known.contains(&account) {
                continue;
            }

            if std::fs::remove_file(entry.path()).is_ok() {
                taken += 1;
            }
        }

        // The markers last: one whose mail is gone has nothing left to say, and one whose mail is
        // still held says it again at the next start.
        for entry in std::fs::read_dir(&self.directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = entry.file_name();
            if !name.to_string_lossy().ends_with(FORGOTTEN) {
                continue;
            }
            let named = entry.path().with_extension("");
            if !named.exists() {
                let _ = std::fs::remove_file(entry.path());
            }
        }

        Ok(taken)
    }

    /// Drops the queue of an account whose file is being made.
    ///
    /// A file that is made rather than opened starts its counter at `account << 40` again, so it
    /// mints the very numbers the file before it minted. Everything in the queue names a number
    /// from the file that is gone — and there is now a different message wearing it. A "delete
    /// permanently" carried out against that number would erase a stranger's message on the
    /// server.
    ///
    /// `AUTOINCREMENT` prevents a number being handed out twice **within one file**. It says
    /// nothing about the file being made again, which is what this is for.
    ///
    /// Dropped rather than kept waiting, and every state alike. No pass can say where a number
    /// from the old file went, so nothing kept here could ever be found again honestly: sent back
    /// to be found, an instruction is put back in the queue by the first pass that reads its
    /// folder — whose number the new file hands out again — and carried out against whatever
    /// wears its message number now. Laid over the new file, a stopped one, a move owed a read
    /// or one on its way would show a stranger's message stopped, leaving or arriving. The new
    /// file is read from the server whole, which is where what was carried out already shows.
    ///
    /// For an account being added there is nothing in the queue and this costs a statement that
    /// removes no rows. There is no way to tell that case from a file that vanished, and no need:
    /// the answer is right for both.
    fn drop_what_was_waiting(&self, account: i64) -> Result<(), StorageError> {
        let failed = |reason: String| StorageError::Open {
            path: "profile.sqlite3".to_owned(),
            reason,
        };

        self.profile
            .get()
            .map_err(|error| failed(error.to_string()))?
            .execute(
                "DELETE FROM op_log
                 WHERE account_id = (SELECT id FROM mail_accounts WHERE index_id = ?1)",
                [account],
            )
            .map(|_| ())
            .map_err(|error| failed(error.to_string()))
    }

    fn numbers(&self) -> Result<Vec<i64>, StorageError> {
        let connection = self.profile.get().map_err(|error| StorageError::Open {
            path: "profile.sqlite3".to_owned(),
            reason: error.to_string(),
        })?;

        let mut statement = connection
            .prepare("SELECT index_id FROM mail_accounts ORDER BY index_id")
            .map_err(|error| StorageError::Open {
                path: "profile.sqlite3".to_owned(),
                reason: error.to_string(),
            })?;

        let found = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            .map_err(|error| StorageError::Open {
                path: "profile.sqlite3".to_owned(),
                reason: error.to_string(),
            })?;

        Ok(found)
    }

    fn make(&self, account: i64) -> Result<SqlitePool, StorageError> {
        std::fs::create_dir_all(&self.directory).map_err(|error| StorageError::Open {
            path: self.directory.display().to_string(),
            reason: error.to_string(),
        })?;

        let path = self.directory.join(format!("{account}.sqlite3"));
        let display = path.display().to_string();
        // Whether this file is being made rather than opened. A file that was there and is not is
        // the dangerous case, and it looks identical to a brand-new account from here.
        let made_now = !path.exists();

        let pool = open_index(&path, &self.key)?;
        let mut connection = pool.get().map_err(|error| StorageError::Open {
            path: display.clone(),
            reason: error.to_string(),
        })?;

        migrate_index(&mut connection, &display)?;
        seed_the_counter(&connection, account, &display)?;
        drop(connection);

        if made_now {
            self.drop_what_was_waiting(account)?;
        }

        Ok(pool)
    }
}

/// Which file a number was minted in.
fn kept_in(identifier: u64) -> i64 {
    (identifier >> ROWS_PER_FILE) as i64
}

/// Starts this file's counter past every number another file could mint.
///
/// Run whenever the file is opened, not only when it is made, because a file is opened afresh every
/// time the application starts. So it must never lower what it finds: a counter told to begin again
/// would hand out numbers a second time, which is the whole thing this exists to prevent.
///
/// Raising it rather than setting it is what makes running this every time safe, and the condition
/// says so in the statement rather than in a comment: `seq < ?2` cannot go backwards however often
/// it runs. A migration leaves the counter at zero — the rebuild that gives the tables their
/// sequence registers them — so there is a row to raise rather than one to add.
fn seed_the_counter(
    connection: &rusqlite::Connection,
    account: i64,
    path: &str,
) -> Result<(), StorageError> {
    let first = account << ROWS_PER_FILE;

    for table in ["messages", "folders"] {
        let raised = connection
            .execute(
                "UPDATE sqlite_sequence SET seq = ?2 WHERE name = ?1 AND seq < ?2",
                rusqlite::params![table, first],
            )
            .map_err(|error| StorageError::Open {
                path: path.to_owned(),
                reason: error.to_string(),
            })?;

        // Nothing to raise means nothing has been counted here yet and SQLite has no row for it.
        if raised == 0 {
            connection
                .execute(
                    "INSERT INTO sqlite_sequence (name, seq)
                     SELECT ?1, ?2
                     WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = ?1)",
                    rusqlite::params![table, first],
                )
                .map_err(|error| StorageError::Open {
                    path: path.to_owned(),
                    reason: error.to_string(),
                })?;
        }
    }

    Ok(())
}

impl crate::ports::mail_files::MailFiles for Indexes {
    fn remove(&self, account: i64) -> Result<(), crate::ports::mail_files::MailFilesError> {
        self.forget(account)
            .map_err(|_| crate::ports::mail_files::MailFilesError::Unavailable)
    }

    fn held_by(&self, account: i64) -> Result<u64, crate::ports::mail_files::MailFilesError> {
        let base = self.directory.join(format!("{account}.sqlite3"));

        // The three files together. A file that is not there weighs nothing rather than failing:
        // an account that never synchronised has no file, and the log and the shared-memory index
        // exist only while something is open.
        let held = [
            base.clone(),
            base.with_extension("sqlite3-wal"),
            base.with_extension("sqlite3-shm"),
        ]
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|about| about.len())
        .sum();

        Ok(held)
    }
}
