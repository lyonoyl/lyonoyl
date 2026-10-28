//! Putting a file where the person said, and nowhere else.
//!
//! A port because the use case decides *what* is offered — this message, under this name — and the
//! choosing is somebody else's job. On the other side in the application is the system's own save
//! panel; on the other side in a test is a directory that answers without asking anyone.
//!
//! Why the port both asks and writes: between choosing a path and writing to it there is a gap, and
//! a path that crossed a layer boundary in that gap is a path something else could have changed.
//! The two happen in one place, and no path is ever handed out.

use thiserror::Error;

/// What is being offered, and under what name.
pub struct FileToSave<'a> {
    /// The name to offer in the dialog. A suggestion: the person renames it if they like, and the
    /// system decides what a name may hold here.
    pub suggested_name: &'a str,
    /// The bytes, already whole. Nothing streams: a message is a thing that fits in memory, and
    /// what does not fit was refused long before this.
    pub bytes: &'a [u8],
}

/// Where a file went, said in the only terms the interface may hear.
///
/// The name and nothing else. A full path would put the shape of somebody's home directory into
/// the webview, which is the direction paths are not allowed to travel (invariant 12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFile {
    pub name: String,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum FileExportError {
    /// There is no way to ask on this system — no portal, no panel, no desktop at all.
    #[error("there is no way to ask where a file should go")]
    CannotAsk,
    /// Somewhere was chosen and the writing failed: a full disk, a read-only volume, a name the
    /// filesystem refused.
    #[error("the file could not be written where it was asked to go")]
    NotWritten,
}

pub trait FileExport: Send + Sync {
    /// Asks where the file goes and writes it there.
    ///
    /// `Ok(None)` is the person closing the dialog. Deciding not to save is an answer, not a
    /// failure, and nothing above here should treat it as one.
    fn save(&self, file: FileToSave<'_>) -> Result<Option<SavedFile>, FileExportError>;
}

impl<E: FileExport + ?Sized> FileExport for &E {
    fn save(&self, file: FileToSave<'_>) -> Result<Option<SavedFile>, FileExportError> {
        (**self).save(file)
    }
}
