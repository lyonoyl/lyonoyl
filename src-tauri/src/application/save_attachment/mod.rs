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
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential::Credential;
use crate::ports::credential_store::{CredentialPurpose, CredentialRef, CredentialStore};
use crate::ports::file_export::{FileExport, FileToSave, SavedFile};
use crate::ports::mail_provider::ProviderFactory;
use crate::ports::message_parts::MessageParts;
use lyonoyl_domain::message::{Uid, UidValidity};

pub struct SaveAttachment<R, C, F, E, P>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    E: FileExport,
    P: MessageParts,
{
    accounts: R,
    credentials: C,
    providers: F,
    files: E,
    parts: P,
}

impl<R, C, F, E, P> SaveAttachment<R, C, F, E, P>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    E: FileExport,
    P: MessageParts,
{
    pub fn new(accounts: R, credentials: C, providers: F, files: E, parts: P) -> Self {
        Self {
            accounts,
            credentials,
            providers,
            files,
            parts,
        }
    }

    /// `Ok(None)` is the person closing the dialog.
    ///
    /// A part that is not there answers [`UseCaseError::NoSuchPart`] rather than failing quietly:
    /// a message can be fetched again between the listing and the asking, and what was third a
    /// moment ago may be second now. The person is told, and looks again.
    ///
    /// `uid_validity` is the generation `uid` was stored in. Without one the number cannot be
    /// checked against the mailbox, and is refused rather than read by.
    pub fn execute(
        &self,
        account_id: &str,
        mailbox: &str,
        uid_validity: Option<UidValidity>,
        uid: Uid,
        at: u32,
    ) -> Result<Option<SavedFile>, UseCaseError> {
        let uid_validity = uid_validity.ok_or(UseCaseError::StorageCorrupted)?;

        let account = self
            .accounts
            .find(account_id)?
            .ok_or(UseCaseError::UnknownAccount)?;

        let password = self.credentials.read(&CredentialRef::new(
            account_id.to_owned(),
            CredentialPurpose::ImapPassword,
        ))?;

        // The whole message, because a part is only a part of one: IMAP can fetch a section on its
        // own, and doing so would mean trusting the numbering the listing used across two
        // connections. One fetch, one parse, one answer.
        let raw = self
            .providers
            .reading(&account, &Credential::password(password))?
            .whole_message(mailbox, uid_validity, uid)?;

        let (name, bytes) = self
            .parts
            .file_at(&raw, at)
            .ok_or(UseCaseError::NoSuchPart)?;

        Ok(self.files.save(FileToSave {
            suggested_name: &name,
            bytes: &bytes,
        })?)
    }
}

#[cfg(test)]
mod tests;
