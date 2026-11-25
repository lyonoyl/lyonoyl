//! Handing a file a message carried to the system, to be opened by whatever opens such a file there.
//!
//! A port because the use case decides *what* is opened: this file, under this name, and only once
//! its own bytes say it is one of the kinds that open. Where its copy is written, and what the
//! system is asked, is somebody else's job. On the other side in the application is the system's
//! own way of opening a file; on the other side in a test is a list of what was asked.
//!
//! No path is handed out, in either direction. The copy is written where this application keeps
//! the files it opens, and the system is given it there: the window, which asked, learns nothing
//! but whether it opened.

use thiserror::Error;

/// What is opened, and under what name.
pub struct FileToOpen<'a> {
    /// The name the copy is written under, already safe and ending as the file's bytes say: the
    /// system chooses the program by it.
    pub name: &'a str,
    /// The bytes, already whole, as they are for saving.
    pub bytes: &'a [u8],
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum FileOpeningError {
    /// The copy could not be written, or not as it must be: a full disk, a folder for copies that is
    /// not this application's to write into, a copy heavier than every copy may weigh together, no
    /// randomness to name its folder with, or a mark of a download the disk would not take.
    #[error("the copy to open could not be written")]
    NotWritten,
    /// The copy was written and the system would not open it: nothing there opens such a file, or
    /// what does would not start.
    #[error("the system would not open the file")]
    NotOpened,
}

pub trait FileOpening: Send + Sync {
    /// Writes a copy where this application keeps the files it opens, and asks the system to open
    /// it.
    ///
    /// Answers once the system has been asked, not once anything is on screen: what opens it is
    /// another program, and when it is ready is its own business.
    fn open(&self, file: FileToOpen<'_>) -> Result<(), FileOpeningError>;
}

impl<O: FileOpening + ?Sized> FileOpening for &O {
    fn open(&self, file: FileToOpen<'_>) -> Result<(), FileOpeningError> {
        (**self).open(file)
    }
}
