//! The system's own way of opening a file, and the copy it is given.
//!
//! What opens a file is another program, which reads it from the disk. So a copy of it is written,
//! in the clear, into the folder this application keeps for the files it opens, and the system is
//! asked to open that copy with whatever opens such a file there — the program a double click in
//! the system's file manager would start. The copy is the one thing a message puts on the disk
//! unencrypted without the person choosing a place, and it is kept to that:
//!
//! - **A folder of its own for each copy**, named at random, so two files of one name never meet
//!   and nothing already on the disk is written through.
//! - **Marked as a download**, on macOS and Windows, as a file saved out of a message is.
//! - **Read-only, and its folder too.** A program that changes it saves the change somewhere the
//!   person chooses, never into a copy that is about to go.
//! - **Bounded.** The copies held at once weigh at most [`MOST_HELD`] together, each counting for
//!   at least [`LEAST_COUNTED`], and the oldest go first to make room for a new one.
//! - **Marked, or not opened.** A copy the disk would not take the mark on is taken away and not
//!   handed to anything: this application chose the folder, and a folder that takes no mark is not
//!   one a copy is opened from.
//! - **Gone at the next start**, with every other copy an earlier run opened: not as the
//!   application quits, since the program showing it may still have it open.
//!
//! The folder is under the system's folder for caches, outside the data directory: on macOS
//! `~/Library/Caches`, on Linux `$XDG_CACHE_HOME`. Windows has no folder for caches apart from the
//! one the data directory is in, so there it lies beside the databases'.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use ring::rand::{SecureRandom, SystemRandom};

use crate::ports::file_opening::{FileOpening, FileOpeningError, FileToOpen};

/// What the folder for opened files is called, under the folder this application keeps its
/// caches in.
pub const OPENED: &str = "Opened";

/// How many random bytes name the folder a copy is written in. Written as twice as many lowercase
/// hexadecimal digits, a name of that shape is one this application made, and the sweep takes no
/// other.
const NAME_BYTES: usize = 16;

/// The most the copies held at once may weigh together: eight of the largest message this build
/// reads.
///
/// A bound, because nothing else bounds them. Each opening writes a copy, a person can open file
/// after file for weeks between two starts, and a window somebody has taken over can ask for one
/// opening after another without a press. Past it the oldest copies go first, rather than the new
/// one being refused: a copy opened an hour ago has long been read by the program showing it, and
/// refusing would leave a person who opens many files unable to open the next one until a restart.
pub const MOST_HELD: u64 = 8 * crate::infrastructure::html::LARGEST_MESSAGE as u64;

/// What one copy counts for against [`MOST_HELD`], however small it is: a folder and a file take
/// room on any disk and a line in the sweep at the start, and counted by their bytes alone, copies
/// of a few bytes each would be held by the million. At most 4,096 copies are held at once.
pub const LEAST_COUNTED: u64 = 64 * 1024;

/// How much the copies held at once may weigh, and the least any one of them counts for.
#[derive(Debug, Clone, Copy)]
struct Bound {
    most: u64,
    least: u64,
}

/// One writing at a time. Each makes room within the bound and then writes, and two side by side
/// would each count without the other and could take away the folder the other has just made.
static WRITING: Mutex<()> = Mutex::new(());

pub struct SystemOpening {
    folder: PathBuf,
}

impl SystemOpening {
    /// `folder` is where the copies go, the one [`clear_what_an_earlier_run_opened`] clears at
    /// every start.
    pub fn new(folder: PathBuf) -> Self {
        Self { folder }
    }
}

impl FileOpening for SystemOpening {
    fn open(&self, file: FileToOpen<'_>) -> Result<(), FileOpeningError> {
        let copy = written_to_open(&self.folder, file.name, file.bytes)?;

        // From Rust, where the window's permissions do not reach: the window is granted no opener
        // of its own, and asks for this by naming a message and a place in it. The path goes as a
        // path: a string made from it would be another path wherever it is not text.
        tauri_plugin_opener::open_path(&copy, None::<&str>).map_err(|_| {
            if let Some(place) = copy.parent() {
                let _ = taken_away(place);
            }
            FileOpeningError::NotOpened
        })
    }
}

/// Writes the copy a file is opened from, and answers where it is.
///
/// In a folder of its own under `folder`, made new, so a folder already there under that name is
/// refused rather than written into; under `name`, which the use case made safe and ending as the
/// file's bytes say. Marked as a download as it is written, then made read-only, the file and the
/// folder it is in. Room is made for it first, within [`MOST_HELD`].
fn written_to_open(folder: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, FileOpeningError> {
    written_within(
        folder,
        name,
        bytes,
        Bound {
            most: MOST_HELD,
            least: LEAST_COUNTED,
        },
    )
}

/// [`written_to_open`], within the bound given: what the tests ask with a small one.
fn written_within(
    folder: &Path,
    name: &str,
    bytes: &[u8],
    bound: Bound,
) -> Result<PathBuf, FileOpeningError> {
    // A writing that panicked left nothing the next one cannot count again from the disk.
    let _one_at_a_time = WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    a_folder_for_copies(folder)?;
    room_for(folder, bytes.len() as u64, bound)?;

    let place = folder.join(random_name().ok_or(FileOpeningError::NotWritten)?);
    std::fs::create_dir(&place).map_err(|_| FileOpeningError::NotWritten)?;
    let copy = place.join(name);

    let written =
        crate::infrastructure::dialog::written_and_marked(&copy, bytes, "opened.notMarked")
            .and_then(|marked| read_only(&copy, &place).map(|()| marked));

    if !matches!(written, Ok(true)) {
        // Half a copy, or one without its mark, is not one to hand over or to leave behind for
        // the next start: a full disk is exactly where what is left over matters.
        let _ = taken_away(&place);
        return Err(FileOpeningError::NotWritten);
    }

    Ok(copy)
}

/// Makes the folder for copies where there is none, and answers whether the one there is this
/// application's to write into: a folder, not a link to one, and on Unix its person's own and
/// nobody else's to write in — and the folder it is in the same.
///
/// A link is refused rather than followed. The sweep at the start never looks into a link, so a
/// copy written through one would stay wherever it led for good; and what put a link there is not
/// something to write the person's files after.
fn a_folder_for_copies(folder: &Path) -> Result<(), FileOpeningError> {
    let above = folder.parent().ok_or(FileOpeningError::NotWritten)?;
    if std::fs::symlink_metadata(above).is_err() {
        std::fs::create_dir_all(above).map_err(|_| FileOpeningError::NotWritten)?;
    }
    this_persons_own(above)?;

    if std::fs::symlink_metadata(folder).is_err() {
        // Made new, so whatever appeared under the name meanwhile is looked at below rather than
        // taken for this one.
        let _ = std::fs::create_dir(folder);
    }
    this_persons_own(folder)
}

/// Whether `folder` is a folder rather than a link or a file, and on Unix this person's own: then
/// it is made theirs alone, as the data directory is. Somebody else's folder is refused rather
/// than written into.
fn this_persons_own(folder: &Path) -> Result<(), FileOpeningError> {
    let entry = std::fs::symlink_metadata(folder).map_err(|_| FileOpeningError::NotWritten)?;
    if !entry.is_dir() {
        return Err(FileOpeningError::NotWritten);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        if entry.uid() != rustix::process::getuid().as_raw() {
            return Err(FileOpeningError::NotWritten);
        }
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| FileOpeningError::NotWritten)?;
    }

    Ok(())
}

/// Takes the oldest copies away until one of `size` bytes fits within the bound, and answers
/// whether it does. Every copy counts for at least the bound's least. A copy that cannot be taken
/// now, held open on a system that refuses to delete such a file, keeps its room.
fn room_for(folder: &Path, size: u64, bound: Bound) -> Result<(), FileOpeningError> {
    let size = size.max(bound.least);
    if size > bound.most {
        return Err(FileOpeningError::NotWritten);
    }

    let mut held: Vec<(SystemTime, u64, PathBuf)> = ours(folder)
        .map(|place| {
            let made = std::fs::symlink_metadata(&place)
                .and_then(|entry| entry.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (made, weight_of(&place).max(bound.least), place)
        })
        .collect();
    held.sort_by_key(|(made, _, _)| *made);

    let mut total: u64 = held.iter().map(|(_, weight, _)| weight).sum();
    for (_, weight, place) in held {
        if total + size <= bound.most {
            break;
        }
        if taken_away(&place).is_ok() {
            total -= weight;
        }
    }

    if total + size <= bound.most {
        Ok(())
    } else {
        Err(FileOpeningError::NotWritten)
    }
}

/// Takes away every copy an earlier run opened.
///
/// Called at the start, before the window is up and so before anything opens: whatever is in the
/// folder then was opened by a run that has ended. Only what this application made goes — a folder
/// named as [`written_to_open`] names one, and what is in it. A link is taken away as a link, never
/// followed, and a folder that is itself a link is not looked into: a sweep must not reach out of
/// the folder it sweeps. What cannot be taken now, held open on a system that refuses to delete
/// such a file, is taken at a later start.
pub fn clear_what_an_earlier_run_opened(folder: &Path) {
    let Ok(entry) = std::fs::symlink_metadata(folder) else {
        return;
    };
    if !entry.is_dir() {
        return;
    }

    for place in ours(folder) {
        let _ = taken_away(&place);
    }
}

/// Every entry of the folder named as a copy's folder is, and nothing else.
fn ours(folder: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.len() == NAME_BYTES * 2 && name.bytes().all(is_lower_hex))
        })
        .map(|entry| entry.path())
}

/// What the files in one copy's folder weigh. A link is not followed and weighs nothing.
fn weight_of(place: &Path) -> u64 {
    if !std::fs::symlink_metadata(place).is_ok_and(|entry| entry.is_dir()) {
        return 0;
    }

    std::fs::read_dir(place)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|inside| std::fs::symlink_metadata(inside.path()).ok())
        .filter(|entry| entry.is_file())
        .map(|entry| entry.len())
        .sum()
}

/// A name nobody can guess and no two copies share. `None` when the machine has no randomness to
/// spare: a name that is not random is one another copy may already have.
fn random_name() -> Option<String> {
    let mut bytes = [0u8; NAME_BYTES];
    SystemRandom::new().fill(&mut bytes).ok()?;

    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

/// The copy and its folder, made so that nothing writes into either.
fn read_only(copy: &Path, place: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(copy, std::fs::Permissions::from_mode(0o400))?;
        std::fs::set_permissions(place, std::fs::Permissions::from_mode(0o500))
    }

    // A folder's read-only attribute means nothing to Windows, so there the file alone is.
    #[cfg(not(unix))]
    {
        let _ = place;
        let mut permissions = std::fs::metadata(copy)?.permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(copy, permissions)
    }
}

/// One copy's folder, and everything in it. The folder is made writable first, since a read-only
/// folder cannot lose what it holds; a file only when deleting it was refused, since Windows will
/// not delete a read-only file — and made read-only again when it still cannot go, held open by
/// the program showing it, so that no copy is ever left writable.
///
/// A link inside is taken away as a link. Nothing here follows one.
fn taken_away(place: &Path) -> std::io::Result<()> {
    let entry = std::fs::symlink_metadata(place)?;
    if !entry.is_dir() {
        return std::fs::remove_file(place);
    }

    writable(place)?;
    for inside in std::fs::read_dir(place)?.flatten() {
        let path = inside.path();
        match inside.file_type() {
            Ok(kind) if kind.is_dir() => taken_away(&path)?,
            Ok(kind) if kind.is_file() => match std::fs::remove_file(&path) {
                Err(refused) if refused.kind() == std::io::ErrorKind::PermissionDenied => {
                    writable(&path)?;
                    if let Err(still) = std::fs::remove_file(&path) {
                        let _ = read_only_file(&path);
                        return Err(still);
                    }
                }
                gone => gone?,
            },
            _ => std::fs::remove_file(&path)?,
        }
    }

    std::fs::remove_dir(place)
}

/// One file made read-only again.
fn read_only_file(path: &Path) -> std::io::Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(path, permissions)
}

fn writable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = if std::fs::symlink_metadata(path)?.is_dir() {
            0o700
        } else {
            0o600
        };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }

    // On Windows this clears the read-only attribute and nothing else; the lint is about the Unix
    // meaning of the same call, which this branch never has.
    #[cfg(not(unix))]
    #[allow(clippy::permissions_set_readonly_false)]
    {
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions)
    }
}

#[cfg(test)]
mod tests;
