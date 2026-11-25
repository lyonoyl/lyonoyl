//! Saving one file that came with a message.
//!
//! The same shape as saving the message itself, and for the same reason: the interface asks by
//! naming a message and a position in it, never a place. Where the file goes is decided by the
//! person in the system's own dialog, behind a port — invariant 12, and the reason for it is that
//! a path from the window is a path an attacker who reaches the window can choose.
//!
//! The name is chosen here either, and it is not the one the message carried: a filename in a
//! message is written by whoever sent it.

use crate::application::error::UseCaseError;
use crate::application::whole_message::WholeMessage;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential_store::CredentialStore;
use crate::ports::file_export::{FileExport, FileToSave, SavedFile};
use crate::ports::mail_provider::ProviderFactory;
use crate::ports::message_bodies::MessageBodies;
use crate::ports::message_parts::MessageParts;
use lyonoyl_domain::message::{MessageId, Uid, UidValidity};

pub struct SaveAttachment<R, C, F, S, P, E>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    S: MessageBodies,
    P: MessageParts,
    E: FileExport,
{
    accounts: R,
    credentials: C,
    providers: F,
    kept: S,
    parts: P,
    files: E,
}

impl<R, C, F, S, P, E> SaveAttachment<R, C, F, S, P, E>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    S: MessageBodies,
    P: MessageParts,
    E: FileExport,
{
    pub fn new(accounts: R, credentials: C, providers: F, kept: S, parts: P, files: E) -> Self {
        Self {
            accounts,
            credentials,
            providers,
            kept,
            parts,
            files,
        }
    }

    /// `Ok(None)` is the person closing the dialog.
    ///
    /// A part that is not there answers [`UseCaseError::NoSuchPart`] rather than failing quietly:
    /// a message can be fetched again between the listing and the asking, and what was third a
    /// moment ago may be second now. The person is told, and looks again.
    ///
    /// The message is read as the reader reads it, from the copy this device keeps and from its
    /// server only where it keeps none, so a file of a message this device keeps is saved with no
    /// network, as it is opened; one past the size a copy is kept of is read from its server each
    /// time. `uid_validity` is the generation `uid` was stored in; where the message has to be
    /// fetched, a number without one cannot be checked against the mailbox, and is refused rather
    /// than read by.
    pub fn execute(
        &self,
        account_id: &str,
        message: MessageId,
        mailbox: &str,
        uid_validity: Option<UidValidity>,
        uid: Uid,
        at: u32,
    ) -> Result<Option<SavedFile>, UseCaseError> {
        // The whole message, because a part is only a part of one: IMAP can fetch a section on its
        // own, and doing so would mean trusting the numbering the listing used across two
        // connections. One read, one parse, one answer.
        let raw = WholeMessage {
            accounts: &self.accounts,
            credentials: &self.credentials,
            providers: &self.providers,
            kept: &self.kept,
        }
        .read(account_id, message, mailbox, uid_validity, uid)?;

        let file = self
            .parts
            .file_at(&raw, at)
            .map_err(|_| UseCaseError::TooBusyToRead)?
            .ok_or(UseCaseError::NoSuchPart)?;

        Ok(self.files.save(FileToSave {
            suggested_name: &file.name,
            bytes: &file.bytes,
        })?)
    }
}

#[cfg(test)]
mod tests;
