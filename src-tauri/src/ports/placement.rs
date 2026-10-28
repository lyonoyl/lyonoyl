//! Where a message actually sits, asked at the moment something is about to be done to it, and
//! what the server has just confirmed about it.
//!
//! An instruction names a message and a folder and never a position, because a position stops
//! meaning the same message the moment the folder renumbers. The position is looked up here, from
//! what the last pass stored, and it is good for exactly as long as the attempt that uses it.
//!
//! The one write here is the server's own answer. A flag the server has just said `OK` to is
//! written onto the copy it was sent to, under the generation it was sent under — which is what
//! the server now holds, and what the next pass would read. Nothing a server has not confirmed is
//! written, and nothing here moves, adds or removes a message.

use thiserror::Error;

use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{Flags, FolderId, MessageId, Uid, UidValidity};

/// A message's address on the server right now: the mailbox by the name the server knows, the
/// number it has in that mailbox, and the generation of the folder that number belongs to.
///
/// The generation travels with the number because a number without one is not an address. A folder
/// that renumbers reissues every number it had, so a `uid` taken before that points at whatever
/// message holds it now — somebody else's message, acted on with this person's instruction. What
/// prevents that is the server being asked to confirm the generation before it does anything, and
/// it can only be asked if the generation is here to ask with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    pub mailbox: String,
    pub uid: Uid,
    pub uid_validity: UidValidity,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum PlacementError {
    #[error("the mail index is unavailable")]
    Unavailable,
}

pub trait Placements: Send + Sync {
    /// Where the message sits in that folder, or nothing if it is not there any more.
    ///
    /// Nothing is not a failure and not a reason to guess: it means the instruction cannot be
    /// carried out, and that is a thing a person is shown rather than something resolved to the
    /// nearest message. A folder this device has never finished a pass over answers nothing for
    /// the same reason — there is a number stored, and nothing that says what it counts.
    ///
    /// A message named by a row that has since handed its copies to the message it turned out to
    /// be sits where those copies are: that is not resolving to the nearest message, it is the
    /// same one.
    fn where_it_sits(
        &self,
        folder: FolderId,
        message: MessageId,
    ) -> Result<Option<Position>, PlacementError>;

    /// The name the server knows a folder by.
    fn mailbox_of(&self, folder: FolderId) -> Result<Option<String>, PlacementError>;

    /// The folder an account keeps a given kind of mail in, where it has one.
    ///
    /// What decides whether "delete" can mean "put it where it can be got back". An account with
    /// no such folder is the one case with no good answer, and the domain answers it by refusing
    /// rather than by deleting for good.
    fn folder_for(
        &self,
        account: i64,
        role: FolderRole,
    ) -> Result<Option<FolderId>, PlacementError>;

    /// Writes the flags the server has just confirmed onto the one copy they were set on.
    ///
    /// Only while the folder is still the generation the instruction was sent under: a folder that
    /// renumbered in between holds a different message at that number, and its flags are not the
    /// ones the server confirmed. Answers whether anything changed.
    fn confirmed_flags(
        &self,
        folder: FolderId,
        at: &Position,
        add: Flags,
        remove: Flags,
    ) -> Result<bool, PlacementError>;
}

impl<P: Placements + ?Sized> Placements for &P {
    fn where_it_sits(
        &self,
        folder: FolderId,
        message: MessageId,
    ) -> Result<Option<Position>, PlacementError> {
        (**self).where_it_sits(folder, message)
    }

    fn mailbox_of(&self, folder: FolderId) -> Result<Option<String>, PlacementError> {
        (**self).mailbox_of(folder)
    }

    fn folder_for(
        &self,
        account: i64,
        role: FolderRole,
    ) -> Result<Option<FolderId>, PlacementError> {
        (**self).folder_for(account, role)
    }

    fn confirmed_flags(
        &self,
        folder: FolderId,
        at: &Position,
        add: Flags,
        remove: Flags,
    ) -> Result<bool, PlacementError> {
        (**self).confirmed_flags(folder, at, add, remove)
    }
}
