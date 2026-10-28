//! Removing what an account's mail is kept in.

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MailFilesError {
    /// What holds the mail could not be removed, so it is still on this device.
    #[error("the mail could not be removed from this device")]
    Unavailable,
}

/// What an account's mail is kept in, for the one operation that is not a read.
///
/// Separate from the index port because it is not about mail at all: it is about the file, which
/// nothing above the composition root otherwise knows exists. Removing an account is the only
/// thing that needs to say so.
pub trait MailFiles: Send + Sync {
    /// Removes everything this account's mail is kept in.
    ///
    /// An account that never synchronised has no file, and removing it is not a failure: the state
    /// asked for — nothing of this account's mail on this device — is the state that already holds.
    fn remove(&self, account: i64) -> Result<(), MailFilesError>;

    /// How many bytes this account's mail takes on this device.
    ///
    /// The file and everything beside it — the write-ahead log holds what has not been folded back
    /// in yet, and a person looking at a number wants the number, not the number minus whatever
    /// happens to be waiting.
    ///
    /// An account that never synchronised holds nothing, and that is an answer rather than a
    /// failure: zero is what is on the disk.
    fn held_by(&self, account: i64) -> Result<u64, MailFilesError>;
}
