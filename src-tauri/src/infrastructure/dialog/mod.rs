//! The system's own save panel, and the writing that follows it.
//!
//! Which panel that is depends on where the application is running: `NSSavePanel` on macOS,
//! `IFileSaveDialog` on Windows, and on Linux the desktop portal over D-Bus — an out-of-process
//! dialog, chosen over the toolkit backend because this process already runs a GTK event loop of
//! its own and a second one inside it is a class of problem nobody needs.
//!
//! Nothing here decides anything. Where the file goes is the person's answer, and the only thing
//! that leaves is the name it ended up with.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::ThreadId;

use tauri::{AppHandle, Runtime};

use crate::ports::file_export::{FileExport, FileExportError, FileToSave, SavedFile};
use crate::ports::file_import::{ChosenFile, FileImport, FileImportError};
use crate::ports::journal::{Journal, Note};
use lyonoyl_domain::outgoing::HEAVIEST_MESSAGE;

pub struct SystemSaveDialog<R: Runtime> {
    app: AppHandle<R>,
    /// The thread the event loop runs on, remembered where it is known.
    ///
    /// A dialog is raised there and nowhere else — macOS refuses outright, and the other two are
    /// only accidentally forgiving. Which means a caller already *on* that thread would hand the
    /// closure to itself and wait for a thread that is waiting: a hang with nothing on screen.
    /// Comparing turns that into an answer.
    event_loop: ThreadId,
}

impl<R: Runtime> SystemSaveDialog<R> {
    pub fn new(app: AppHandle<R>, event_loop: ThreadId) -> Self {
        Self { app, event_loop }
    }

    /// Raises the panel on the event loop's thread and waits for what came back.
    fn ask(&self, suggested_name: &str) -> Result<Option<PathBuf>, FileExportError> {
        if std::thread::current().id() == self.event_loop {
            return Err(FileExportError::CannotAsk);
        }

        let suggested_name = suggested_name.to_owned();
        let (answer, wait) = mpsc::channel();

        self.app
            .run_on_main_thread(move || {
                // Blocks the event loop for as long as the panel is up, which is what a modal
                // dialog is: the system spins its own loop underneath and the window keeps
                // drawing. Nothing after this line runs until the person has answered.
                let chosen = rfd::FileDialog::new()
                    .set_file_name(suggested_name)
                    .save_file();

                let _ = answer.send(chosen);
            })
            .map_err(|_| FileExportError::CannotAsk)?;

        // Closed rather than answered if the sender went away — the closure panicked, or the
        // application is shutting down and the loop will not run it.
        wait.recv().map_err(|_| FileExportError::CannotAsk)
    }
}

impl<R: Runtime> FileExport for SystemSaveDialog<R> {
    fn save(&self, file: FileToSave<'_>) -> Result<Option<SavedFile>, FileExportError> {
        let Some(path) = self.ask(file.suggested_name)? else {
            return Ok(None);
        };

        // Written whole, with whatever permissions this person's system gives a file they asked
        // for. Not the tight mode the secrets file gets: this one is an export, and locking it
        // down would be deciding something the person did not ask to have decided.
        written_from_outside(&path, file.bytes)?;

        // The name and not the path. What the interface says is "saved as this"; where somebody
        // keeps their files is not something a webview has any business learning.
        Ok(Some(SavedFile {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| file.suggested_name.to_owned()),
        }))
    }
}

/// Writes a file saved out of a message, and marks it as one that came from outside.
///
/// The mark is what a browser puts on a download. On macOS it is what Gatekeeper reads before an
/// application, a script or an installer runs — one saved out of a message, or carried in an
/// archive or a disk image saved out of one — and what a disk image mounted from such a file is
/// marked with in turn. It goes on through the open file, so it is on the file that holds the bytes.
///
/// A symbolic link under the chosen name is removed and a file made in its place, never written
/// through: a link left there — by an archive, say, pointing out of the folder — would otherwise take
/// the bytes somewhere the person never chose, unmarked. A file already there is written over in
/// place, which is what the panel's *Replace* asked for and what works wherever a file can be
/// written at all — a network share that refuses a second name, or a folder that refuses a new
/// entry, included.
fn written_from_outside(path: &Path, bytes: &[u8]) -> Result<(), FileExportError> {
    let mut options = File::options();
    options.write(true);

    match std::fs::symlink_metadata(path) {
        Ok(entry) if entry.file_type().is_symlink() => {
            std::fs::remove_file(path).map_err(|_| FileExportError::NotWritten)?;
            options.create_new(true);
        }
        Ok(_) => {
            options.truncate(true);
        }
        // Made new rather than created-or-opened, so whatever appears under the name meanwhile is
        // refused rather than followed.
        Err(_) => {
            options.create_new(true);
        }
    }

    let mut file = options
        .open(path)
        .map_err(|_| FileExportError::NotWritten)?;

    if !marked_as_from_outside(&file) {
        // A disk that refuses extended attributes, and the `._` file macOS keeps them in where a
        // disk has none — a name too long for one, some network shares — takes the file without
        // the mark, as it would from a browser. The file is the person's either way, and refusing
        // it would leave them nothing safer than a copy made by hand. Written down at the level
        // everything about one message is.
        crate::infrastructure::journal::journal().write_down(Note::Detail, "saved.notMarked", &[]);
    }

    file.write_all(bytes)
        .map_err(|_| FileExportError::NotWritten)
}

/// What macOS calls the mark.
#[cfg(target_os = "macos")]
const QUARANTINE: &str = "com.apple.quarantine";

/// Marks the file, and answers whether the mark is on it.
///
/// `0081` is what a file arriving over AirDrop carries: quarantined, and not yet opened. Then the
/// moment in hexadecimal seconds, then what the system names when it asks — the product, never a
/// person — and no event identifier, as a sandboxed application writes it: the database of those
/// events is the system's.
#[cfg(target_os = "macos")]
fn marked_as_from_outside(file: &File) -> bool {
    use xattr::FileExt;

    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default();

    file.set_xattr(
        QUARANTINE,
        format!("0081;{at:08x};Lyonoyl Mail;").as_bytes(),
    )
    .is_ok()
}

/// Windows and Linux are built from source, and nothing here marks a file on either: Linux has no
/// such mark, and the one Windows keeps in a second stream is code nobody here has built or run.
#[cfg(not(target_os = "macos"))]
fn marked_as_from_outside(_file: &File) -> bool {
    true
}

/// The largest file this application will read into memory to attach: what one message may weigh.
///
/// A heavier file could never go with a message, nor be kept in the draft of one, so it is refused
/// as it is chosen and before a byte of it is read. Bounded at all because a file chosen in a
/// dialog is a file of any size the disk can hold, and reading it whole is what happens next.
const LARGEST_TO_ATTACH: u64 = HEAVIEST_MESSAGE as u64;

/// The system's own open panel, and the reading that follows it.
///
/// The same shape as the save panel above and for the same reasons — raised on the event loop's
/// thread, answered by the person, and the only thing that leaves is a name and the bytes.
pub struct SystemOpenDialog<R: Runtime> {
    app: AppHandle<R>,
    event_loop: ThreadId,
}

impl<R: Runtime> SystemOpenDialog<R> {
    pub fn new(app: AppHandle<R>, event_loop: ThreadId) -> Self {
        Self { app, event_loop }
    }
}

impl<R: Runtime> FileImport for SystemOpenDialog<R> {
    fn choose(&self) -> Result<Option<ChosenFile>, FileImportError> {
        if std::thread::current().id() == self.event_loop {
            return Err(FileImportError::CannotAsk);
        }

        let (answer, wait) = mpsc::channel();

        self.app
            .run_on_main_thread(move || {
                let _ = answer.send(rfd::FileDialog::new().pick_file());
            })
            .map_err(|_| FileImportError::CannotAsk)?;

        let Some(path) = wait.recv().map_err(|_| FileImportError::CannotAsk)? else {
            return Ok(None);
        };

        let bytes = read_to_attach(&path, LARGEST_TO_ATTACH)?;

        Ok(Some(ChosenFile {
            // Made safe here, at the edge, because it is about to be written into a message header
            // — and because the person chose this file rather than being sent it, which makes the
            // name less hostile and not harmless: a file downloaded from anywhere carries whatever
            // name it was given.
            name: lyonoyl_domain::attachment::safe_name(
                &path.file_name().unwrap_or_default().to_string_lossy(),
            ),
            bytes,
        }))
    }
}

/// The bytes of a file chosen to attach, read once and never past the largest allowed.
///
/// Asked about before it is opened, because what is not a file of bytes on a disk does its harm as
/// soon as it is opened or read: a pipe opened for reading waits for somebody to write into it,
/// holding the thread that asked until then, and a device such as `/dev/zero` answers for ever
/// while the size the system gives for it is nought. A file larger than the limit is refused as
/// what it is, before a byte of it is read.
///
/// Asked again of what was opened, because the name may lead somewhere else by then, and read
/// through a limit whatever either answer said: a file can grow between being measured and being
/// read. The limit is one byte past the largest allowed, and that byte is how a file that grew is
/// told apart from one that fits exactly.
fn read_to_attach(path: &Path, largest: u64) -> Result<Vec<u8>, FileImportError> {
    let about = std::fs::metadata(path).map_err(|_| FileImportError::NotRead)?;
    if !about.is_file() {
        return Err(FileImportError::NotRead);
    }
    if about.len() > largest {
        return Err(FileImportError::TooLarge);
    }

    let file = File::open(path).map_err(|_| FileImportError::NotRead)?;
    if !file.metadata().is_ok_and(|opened| opened.is_file()) {
        return Err(FileImportError::NotRead);
    }

    at_most(file, largest)
}

/// Everything a reader holds, or the answer that it holds more than `largest` — found out by
/// reading one byte past it and no further.
fn at_most(reader: impl Read, largest: u64) -> Result<Vec<u8>, FileImportError> {
    let mut bytes = Vec::new();
    reader
        .take(largest.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| FileImportError::NotRead)?;

    if bytes.len() as u64 > largest {
        return Err(FileImportError::TooLarge);
    }

    Ok(bytes)
}

#[cfg(test)]
mod reading_tests {
    use super::*;

    /// A folder of its own for one test, empty. Each test removes it before asserting anything,
    /// so a test that fails leaves nothing behind either.
    fn a_folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("lyonoyl-attach-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&folder).ok();
        std::fs::create_dir_all(&folder).expect("a folder");
        folder
    }

    #[test]
    fn a_file_chosen_to_attach_is_read_whole() {
        let folder = a_folder("whole");
        let path = folder.join("invoice.pdf");
        std::fs::write(&path, b"%PDF-1.7 a document").expect("a file");

        let read = read_to_attach(&path, 1024);
        std::fs::remove_dir_all(&folder).ok();

        assert_eq!(read, Ok(b"%PDF-1.7 a document".to_vec()));
    }

    #[test]
    fn a_file_the_size_of_the_limit_is_read_and_one_byte_more_is_refused() {
        let folder = a_folder("limit");
        let fits = folder.join("fits.bin");
        let over = folder.join("over.bin");
        let empty = folder.join("empty.bin");
        std::fs::write(&fits, [7u8; 16]).expect("a file");
        std::fs::write(&over, [7u8; 17]).expect("a file");
        std::fs::write(&empty, []).expect("a file");

        let read = [&fits, &over, &empty].map(|path| read_to_attach(path, 16));
        std::fs::remove_dir_all(&folder).ok();

        assert_eq!(
            read,
            [
                Ok(vec![7u8; 16]),
                Err(FileImportError::TooLarge),
                Ok(Vec::new())
            ]
        );
    }

    #[test]
    fn a_file_heavier_than_a_message_may_be_is_refused_as_it_is_chosen() {
        // It could never go with a message, nor be kept in a draft. Taken, every keep of a draft
        // carrying it would be refused instead, with whatever was typed after it.
        let folder = a_folder("heaviest");
        let fits = folder.join("fits.bin");
        let over = folder.join("over.bin");
        for (path, size) in [(&fits, HEAVIEST_MESSAGE), (&over, HEAVIEST_MESSAGE + 1)] {
            File::create(path)
                .and_then(|file| file.set_len(size as u64))
                .expect("a file");
        }

        let read = [&fits, &over]
            .map(|path| read_to_attach(path, LARGEST_TO_ATTACH).map(|bytes| bytes.len()));
        std::fs::remove_dir_all(&folder).ok();

        assert_eq!(read, [Ok(HEAVIEST_MESSAGE), Err(FileImportError::TooLarge)]);
    }

    #[test]
    fn what_is_not_a_file_is_not_read() {
        let folder = a_folder("folder");

        let read = [folder.clone(), folder.join("gone.pdf")].map(|path| read_to_attach(&path, 16));
        std::fs::remove_dir_all(&folder).ok();

        assert_eq!(
            read,
            [Err(FileImportError::NotRead), Err(FileImportError::NotRead)]
        );
    }

    #[test]
    fn a_file_that_grew_after_it_was_measured_is_read_one_byte_past_the_limit_and_no_further() {
        // What a file still being written looks like from here: measured at one size and found
        // at another. Read to its end, it is read whatever size it reached.
        let mut growing = std::io::Cursor::new(vec![7u8; 4096]);

        assert_eq!(at_most(&mut growing, 16), Err(FileImportError::TooLarge));
        assert_eq!(growing.position(), 17, "read past the byte that settles it");
    }

    #[cfg(unix)]
    #[test]
    fn a_device_that_never_ends_is_refused_rather_than_read() {
        // Its size is nought, so the measuring passes it, and reading it to the end never ends.
        assert_eq!(
            read_to_attach(Path::new("/dev/zero"), 16),
            Err(FileImportError::NotRead)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_pipe_is_refused_without_waiting_for_somebody_to_write_into_it() {
        let folder = a_folder("pipe");
        let pipe = folder.join("invoice.pdf");
        let made = std::process::Command::new("mkfifo").arg(&pipe).status();

        // On a thread of its own: opened for reading, a pipe waits for a writer, and a test that
        // waits with it is a suite that never finishes rather than one that says why.
        let (answer, wait) = mpsc::channel();
        let asked = pipe.clone();
        std::thread::spawn(move || {
            let _ = answer.send(read_to_attach(&asked, 16));
        });
        let read = wait.recv_timeout(std::time::Duration::from_secs(10));
        std::fs::remove_dir_all(&folder).ok();

        assert!(
            made.is_ok_and(|status| status.success()),
            "no pipe was made"
        );
        assert_eq!(
            read.expect("the read is still waiting for a writer"),
            Err(FileImportError::NotRead)
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// A file in a folder of its own, under a name a phishing message would give it.
    fn a_place(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("lyonoyl-saved-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&folder).ok();
        std::fs::create_dir_all(&folder).expect("a folder");
        folder.join("invoice.docm")
    }

    /// The mark on a file, as its four fields.
    fn the_mark(path: &Path) -> Vec<String> {
        let mark = xattr::get(path, QUARANTINE)
            .expect("readable")
            .expect("the mark is on the file");
        String::from_utf8(mark)
            .expect("text")
            .split(';')
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_file_saved_out_of_a_message_carries_the_mark_of_a_download() {
        let path = a_place("marked");

        written_from_outside(&path, b"PK\x03\x04").expect("written");

        assert_eq!(std::fs::read(&path).expect("readable"), b"PK\x03\x04");
        let mark = the_mark(&path);
        assert_eq!(mark.len(), 4, "{mark:?}");
        assert_eq!(mark[0], "0081", "quarantined, and not yet opened");
        let at = u64::from_str_radix(&mark[1], 16).expect("seconds, in hexadecimal");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_secs();
        assert!(now.abs_diff(at) < 60, "marked at {at}, and it is {now}");
        assert_eq!(mark[2], "Lyonoyl Mail");
        assert_eq!(mark[3], "", "no event the system's database does not hold");

        std::fs::remove_dir_all(path.parent().expect("a folder")).ok();
    }

    #[test]
    fn a_file_saved_over_one_already_there_is_marked_as_well() {
        // The panel asks before replacing, and what was there before — a mark of its own
        // included — says nothing about what was just written over it.
        let path = a_place("over");
        std::fs::write(&path, b"made here").expect("a file already there");
        xattr::set(&path, QUARANTINE, b"00c1;00000000;Other;").expect("an earlier mark");

        written_from_outside(&path, b"from outside").expect("written");

        assert_eq!(std::fs::read(&path).expect("readable"), b"from outside");
        let mark = the_mark(&path);
        assert_eq!(mark[0], "0081", "{mark:?}");
        assert_eq!(mark[2], "Lyonoyl Mail", "{mark:?}");

        std::fs::remove_dir_all(path.parent().expect("a folder")).ok();
    }

    #[test]
    fn a_link_under_the_chosen_name_is_replaced_rather_than_written_through() {
        // Replace means the entry with that name. Written through, the bytes would land on what
        // the link points at, unmarked, and the link would carry the mark.
        let path = a_place("link");
        let folder = path.parent().expect("a folder").to_path_buf();
        let theirs = folder.join("kept.docm");
        std::fs::write(&theirs, b"somebody's own").expect("a file the link points at");
        std::os::unix::fs::symlink(&theirs, &path).expect("a link under the chosen name");

        written_from_outside(&path, b"from outside").expect("written");

        let entry = std::fs::symlink_metadata(&path).expect("there");
        assert!(entry.file_type().is_file(), "still a link: {entry:?}");
        assert_eq!(std::fs::read(&path).expect("readable"), b"from outside");
        assert_eq!(the_mark(&path)[0], "0081");
        assert_eq!(std::fs::read(&theirs).expect("readable"), b"somebody's own");
        assert!(xattr::get(&theirs, QUARANTINE).expect("readable").is_none());

        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn a_link_pointing_out_of_the_folder_takes_nothing_out_of_it() {
        // What an archive can leave behind: a link to a place that does not exist yet, outside
        // the folder the person chose. Followed, saving would make that file.
        let path = a_place("away");
        let folder = path.parent().expect("a folder").to_path_buf();
        let elsewhere = folder.with_extension("elsewhere");
        std::fs::create_dir_all(&elsewhere).expect("a folder outside the chosen one");
        let away = elsewhere.join("agent.plist");
        std::os::unix::fs::symlink(&away, &path).expect("a link to nowhere yet");

        written_from_outside(&path, b"from outside").expect("written");

        assert!(!away.exists(), "the bytes went where the link pointed");
        assert!(std::fs::symlink_metadata(&path)
            .expect("there")
            .file_type()
            .is_file());
        assert_eq!(the_mark(&path)[0], "0081");

        std::fs::remove_dir_all(&folder).ok();
        std::fs::remove_dir_all(&elsewhere).ok();
    }
}
