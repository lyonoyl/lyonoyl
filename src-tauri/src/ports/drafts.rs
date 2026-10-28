//! A message being written, kept where it cannot be lost.
//!
//! Nothing here is parsed. A message on its way out is checked before it is written down, because
//! what goes on a wire is somebody else's problem the moment it leaves; a message being written is
//! allowed to be half of one, and refusing to keep it would lose what a person typed at the moment
//! they most need it kept. The checking happens when they press send and not before.
//!
//! The files it carries are kept with it, as bytes. A draft that came back without the invoice it
//! was written to carry is a message somebody sends again without it, and never notices.

use std::sync::Arc;

use lyonoyl_domain::outgoing::OutgoingFile;
use thiserror::Error;

/// A message as it stands in the window, field for field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    /// Everything the person typed into the field, unsplit and unparsed.
    pub recipients: String,
    pub copies: String,
    /// Blind copies, kept because losing them silently is the one thing this exists to prevent.
    pub blind: String,
    pub subject: String,
    pub body: String,
    /// The message being answered, where there is one.
    pub answering: Option<String>,
    /// The conversation it belongs to, oldest first.
    pub chain: Vec<String>,
}

/// One kept, with the number it was kept under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub id: i64,
    pub draft: Draft,
}

/// A file a draft carries, under what the draft knows it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftFile {
    /// What names this file from one keep to the next: the token it was first attached under, or
    /// a name of its own for one that came back from the outbox.
    ///
    /// A file a draft already holds under this is not written again, which is what lets a keep
    /// every second or so of typing cost nothing for a photograph attached ten minutes ago.
    pub kept_as: String,
    /// Shared rather than copied: the same bytes are held for sending while the draft keeps them.
    pub file: Arc<OutgoingFile>,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum DraftError {
    #[error("the profile database is unavailable")]
    Unavailable,
    /// A draft of its own past how many an account keeps, or a keep that would leave an account's
    /// drafts weighing more together than they may (`MOST_DRAFTS`, `HEAVIEST_DRAFTS`).
    #[error("the account keeps as much in its drafts as it may")]
    NoRoom,
}

pub trait Drafts: Send + Sync {
    /// Writes it down, replacing what was kept under the same number, files and all.
    ///
    /// Answers the number, so the next keep of the same message replaces it rather than leaving a
    /// second copy behind every keystroke. `files` is everything it carries now, in order: one it
    /// already holds under the same name is not written again, one it holds and `files` does not
    /// name leaves it, and the text and the files are one change.
    ///
    /// Refused whole with `NoRoom`, and nothing written, when it would be a draft of its own past
    /// how many an account keeps, or would make a draft heavier and leave the drafts of the account
    /// it belongs to heavier together than they may be. A keep that adds nothing is never refused,
    /// so one that takes a file off goes through however heavy the rest are. Checked in the change
    /// that writes the draft, so two keeps at once cannot both take the last of the room.
    fn keep(
        &self,
        account_id: &str,
        under: Option<i64>,
        draft: &Draft,
        files: &[DraftFile],
    ) -> Result<i64, DraftError>;

    /// The files one carries, in the order they were attached.
    ///
    /// Nothing for a draft that is not there: one thrown away has nothing left to carry.
    fn files(&self, id: i64) -> Result<Vec<DraftFile>, DraftError>;

    /// Writes down a message the outbox will not send as a draft that says it came back, with the
    /// files it carried, lets go of the outbox's row of it, and answers the number the draft is
    /// kept under.
    ///
    /// One change writes the draft, moves the files out of the outbox into it, and deletes the row,
    /// with everybody the message was addressed to, blind copies included. So once it is back in
    /// the drafts the draft is the only place on this device it is, and no crash leaves it in both
    /// places or in neither.
    ///
    /// Answers nothing, and writes nothing, for a message the outbox does not hold for the
    /// account: given back once, it has nothing left to give back, and a second draft would be the
    /// message without the files the first one took, whether or not that draft is still there. A
    /// row that names the draft its message came back as goes, and nothing is written either:
    /// that draft holds the message and its files. Never refused for room: refused, the message
    /// would stay in the outbox, where nothing shows it, and stop every round of its account.
    fn came_back(
        &self,
        account_id: &str,
        from_outbox: i64,
        draft: &Draft,
    ) -> Result<Option<i64>, DraftError>;

    /// The one most recently written, for an account that has one.
    fn latest(&self, account_id: &str) -> Result<Option<Kept>, DraftError>;

    /// Every one an account has, newest first.
    ///
    /// What the drafts folder shows above what the server's drafts folder holds. These live here
    /// rather than on the server, so nothing in the index knows about them and no listing of mail
    /// will ever find them.
    fn all(&self, account_id: &str) -> Result<Vec<Kept>, DraftError>;

    /// Forgets it. What happens when a message is sent, and when a person throws it away.
    fn discard(&self, id: i64) -> Result<(), DraftError>;
}

impl<D: Drafts + ?Sized> Drafts for &D {
    fn keep(
        &self,
        account_id: &str,
        under: Option<i64>,
        draft: &Draft,
        files: &[DraftFile],
    ) -> Result<i64, DraftError> {
        (**self).keep(account_id, under, draft, files)
    }

    fn files(&self, id: i64) -> Result<Vec<DraftFile>, DraftError> {
        (**self).files(id)
    }

    fn came_back(
        &self,
        account_id: &str,
        from_outbox: i64,
        draft: &Draft,
    ) -> Result<Option<i64>, DraftError> {
        (**self).came_back(account_id, from_outbox, draft)
    }

    fn all(&self, account_id: &str) -> Result<Vec<Kept>, DraftError> {
        (**self).all(account_id)
    }

    fn latest(&self, account_id: &str) -> Result<Option<Kept>, DraftError> {
        (**self).latest(account_id)
    }

    fn discard(&self, id: i64) -> Result<(), DraftError> {
        (**self).discard(id)
    }
}
