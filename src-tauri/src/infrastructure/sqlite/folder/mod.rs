//! The folder the databases are kept in, inside the application's data folder, and the move that
//! brings them into it from the top of that folder.
//!
//! The data folder holds more than the databases — the journal's `logs`, the updater's `updates` —
//! and the databases are kept together in a folder of their own: the profile with the files SQLite
//! keeps beside it, the mail one file per account, and on a Linux desktop with no keyring the file of
//! secrets their key is kept in. They are one set. The profile and its write-ahead log in two places
//! is a profile opened without the transactions the log still holds, and the profile without the
//! file of secrets beside it is a profile whose key is looked for in the wrong store.

use std::path::{Path, PathBuf};

use crate::infrastructure::secure_storage::SECRETS_FILE;
use crate::infrastructure::sqlite::indexes::ACCOUNTS;

/// The folder's name, beside the journal's `logs` and the updater's `updates`.
pub const DATABASES: &str = "databases";

/// The profile, by the name every part of the start looks for it under.
pub const PROFILE: &str = "profile.sqlite3";

/// The set, in the order it is moved. The profile goes last, so a move stopped part of the way
/// leaves the profile at the top of the data folder, where this run finds it whole once the move
/// has taken back what went before it.
const THE_SET: [&str; 6] = [
    SECRETS_FILE,
    ACCOUNTS,
    "profile.sqlite3-journal",
    "profile.sqlite3-shm",
    "profile.sqlite3-wal",
    PROFILE,
];

/// How many times a rename is asked for before the move stops. Another program — something
/// scanning a file that has just appeared, on Windows — can hold a file for a moment.
const TRIES: u32 = 5;

/// How long between two of them.
const BETWEEN_TRIES: std::time::Duration = std::time::Duration::from_millis(100);

/// The folder the databases are kept in.
pub fn databases_in(data: &Path) -> PathBuf {
    data.join(DATABASES)
}

/// Where the databases of this run are: where the profile is.
///
/// Their own folder, unless the profile is at the top of the data folder and not in it — a set a
/// move could not take whole, or one a start found beside a set in the folder already — and with no
/// profile anywhere, their own folder, where a first start makes one.
pub fn where_they_are(data: &Path) -> PathBuf {
    let theirs = databases_in(data);
    match !there(&theirs.join(PROFILE)) && there(&data.join(PROFILE)) {
        true => data.to_path_buf(),
        false => theirs,
    }
}

/// What a start did with the databases it found at the top of the data folder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gathered {
    /// Files and folders moved into the databases' folder.
    pub moved: u32,
    /// Files and folders left at the top, because the databases' folder holds one of that name, or
    /// holds a profile and so is the set this build reads.
    pub left: u32,
}

/// A move that stopped part of the way.
#[derive(Debug)]
pub struct NotGathered {
    /// What could not be moved.
    pub path: PathBuf,
    pub error: std::io::Error,
    /// Whether everything this move took went back where it was. When it did, the set is whole at
    /// the top of the data folder for this run, and the next start moves it again. When it did not,
    /// part of the set is in each folder, and nothing of it may be opened until a start brings the
    /// rest of it over.
    pub put_back: bool,
}

/// Moves the databases from the top of the data folder into their own folder.
///
/// Renamed, never copied: the profile is the only copy of what a person wrote, and a rename either
/// happens or does not. Nothing in the databases' folder is replaced. When it holds a profile, that
/// is the set this build reads and everything at the top is left where it is. When it holds part of
/// a set without the profile, that part is what a move stopped part of the way left there, and the
/// rest of the set joins it. A rename that fails, after the tries it is given, takes back the ones
/// before it.
pub fn gather(data: &Path) -> Result<Gathered, NotGathered> {
    gather_with(data, patiently)
}

/// [`gather`], moving with `rename`, so a test can be a rename that fails.
pub fn gather_with(
    data: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<Gathered, NotGathered> {
    let theirs = databases_in(data);
    let here: Vec<&str> = THE_SET
        .into_iter()
        .filter(|name| there(&data.join(name)))
        .collect();
    if here.is_empty() {
        return Ok(Gathered::default());
    }

    if there(&theirs.join(PROFILE)) {
        return Ok(Gathered {
            left: here.len() as u32,
            ..Gathered::default()
        });
    }

    let (going, staying): (Vec<&str>, Vec<&str>) = here
        .into_iter()
        .partition(|name| !there(&theirs.join(name)));

    a_folder_of_their_own(&theirs).map_err(|error| NotGathered {
        path: theirs.clone(),
        error,
        put_back: true,
    })?;

    let mut moved: Vec<&str> = Vec::new();
    for name in going {
        let (from, to) = (data.join(name), theirs.join(name));
        if let Err(error) = rename(&from, &to) {
            let mut put_back = true;
            for back in moved.iter().rev() {
                put_back &= rename(&theirs.join(back), &data.join(back)).is_ok();
            }
            written_down(data, &theirs);
            return Err(NotGathered {
                path: from,
                error,
                put_back,
            });
        }
        moved.push(name);
    }

    written_down(data, &theirs);
    Ok(Gathered {
        moved: moved.len() as u32,
        left: staying.len() as u32,
    })
}

/// A rename, asked for again after a moment when it fails, as many times as [`TRIES`] says. What
/// is not there and what is in the way already do not change by waiting, and are not asked again.
fn patiently(from: &Path, to: &Path) -> std::io::Result<()> {
    again_and_again(|| std::fs::rename(from, to))
}

/// `once`, asked for again after a moment while it fails with something waiting may change.
fn again_and_again(mut once: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    use std::io::ErrorKind;

    let mut tried = 1;
    loop {
        match once() {
            Ok(()) => return Ok(()),
            Err(error)
                if tried < TRIES
                    && !matches!(error.kind(), ErrorKind::NotFound | ErrorKind::AlreadyExists) =>
            {
                tried += 1;
                std::thread::sleep(BETWEEN_TRIES);
            }
            Err(error) => return Err(error),
        }
    }
}

/// Whether there is anything at `path`, a link that leads nowhere included: a link is moved as the
/// link it is.
fn there(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// The folder, made if it is not there, and reachable by this person alone.
///
/// After it exists rather than through a mode on creation, so a folder made with whatever the
/// umask allowed is narrowed as well. On Windows a folder inside the person's local application
/// data inherits an access list naming them, and there is no portable call to narrow it further.
pub fn a_folder_of_their_own(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))?;
    }

    Ok(())
}

/// Asks the disk to keep the renames in both folders before anything opens what was moved: a
/// rename the disk has not kept can come undone in a power cut, and not all of them alike.
fn written_down(data: &Path, theirs: &Path) {
    #[cfg(unix)]
    for folder in [data, theirs] {
        if let Ok(handle) = std::fs::File::open(folder) {
            let _ = handle.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = (data, theirs);
}

#[cfg(test)]
mod tests;
