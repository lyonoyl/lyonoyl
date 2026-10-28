// Reached once the composer can attach a file.
#![allow(dead_code)]

use thiserror::Error;

/// Asking the person for a file, and reading it.
///
/// The mirror of [`crate::ports::file_export`], and for the same reason: the window says "let me
/// attach something" and never says which file. What comes back is a name and the bytes — no path,
/// in either direction (invariant 12).
///
/// The bytes come back rather than a handle, and that is a decision rather than convenience. A
/// handle is a thing to read *later*, and later the file may be gone, changed, or on a volume that
/// has been unplugged: the message would go out carrying something other than what its writer
/// attached. What is attached is attached at the moment it is chosen.
pub trait FileImport: Send + Sync {
    /// `Ok(None)` is the person closing the dialog, which is an answer.
    fn choose(&self) -> Result<Option<ChosenFile>, FileImportError>;
}

/// A file somebody picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChosenFile {
    /// The name it had, already made safe: it is about to be written into a message header, and a
    /// filename with a newline in it is a header somebody else wrote.
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum FileImportError {
    /// There is no way to ask on this system — no portal, no panel, no desktop at all.
    #[error("there is no way to ask which file to attach")]
    CannotAsk,
    /// Something was chosen and could not be read: it went away, or was never readable.
    #[error("the file could not be read")]
    NotRead,
    /// Larger than this application will hold in memory to attach.
    ///
    /// Its own answer rather than "not read", because the two mean different things to the person:
    /// one is a file that is gone and one is a file that is too big, and only the second has an
    /// obvious next step.
    #[error("that file is too large to attach")]
    TooLarge,
}
