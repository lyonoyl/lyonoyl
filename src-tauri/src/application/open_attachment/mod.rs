//! Opening one file that came with a message, in whatever the system opens such a file with.
//!
//! The same shape as saving one: the interface asks by naming a message and a position in it,
//! never a name and never a path. What differs is what is asked of the file. A file is saved
//! whatever it is, because the person chose where it goes and what happens to it next; a file is
//! opened only when its own bytes say it is a document of pages or a picture, and that is decided
//! here, again, from the bytes read now — the window was told so when the message was drawn, and
//! a message can be read again between the two, with something else now in that place.
//!
//! The message is read the way the reader reads it, from the copy this device keeps and from the
//! server only where it keeps none. A message this device keeps opens its files with no network;
//! one past the size a copy is kept of is read from its server each time.

use crate::application::error::UseCaseError;
use crate::application::whole_message::WholeMessage;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential_store::CredentialStore;
use crate::ports::file_opening::{FileOpening, FileToOpen};
use crate::ports::mail_provider::ProviderFactory;
use crate::ports::message_bodies::MessageBodies;
use crate::ports::message_parts::MessageParts;
use lyonoyl_domain::attachment::opens::{opened_name, opens_as};
use lyonoyl_domain::message::{MessageId, Uid, UidValidity};

pub struct OpenAttachment<R, C, F, S, P, O>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    S: MessageBodies,
    P: MessageParts,
    O: FileOpening,
{
    accounts: R,
    credentials: C,
    providers: F,
    kept: S,
    parts: P,
    opening: O,
}

impl<R, C, F, S, P, O> OpenAttachment<R, C, F, S, P, O>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    S: MessageBodies,
    P: MessageParts,
    O: FileOpening,
{
    pub fn new(accounts: R, credentials: C, providers: F, kept: S, parts: P, opening: O) -> Self {
        Self {
            accounts,
            credentials,
            providers,
            kept,
            parts,
            opening,
        }
    }

    /// Opens the file at `at` in the message, once its bytes say it is a kind that opens.
    ///
    /// A part that is not there answers [`UseCaseError::NoSuchPart`], as saving does. A part whose
    /// bytes are not a kind that opens answers [`UseCaseError::FileNotOpened`], and nothing is
    /// written: saving it is still open to the person, and is what the window offers.
    ///
    /// `mailbox`, `uid_validity` and `uid` say where the message sits on its server, and come from
    /// the index, never from the interface.
    pub fn execute(
        &self,
        account_id: &str,
        message: MessageId,
        mailbox: &str,
        uid_validity: Option<UidValidity>,
        uid: Uid,
        at: u32,
    ) -> Result<(), UseCaseError> {
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

        // A message carried inside this one is saved as a message, whatever it is called.
        if file.a_message {
            return Err(UseCaseError::FileNotOpened);
        }
        let kind = opens_as(&file.name, &file.bytes).ok_or(UseCaseError::FileNotOpened)?;

        Ok(self.opening.open(FileToOpen {
            name: &opened_name(&file.name, kind),
            bytes: &file.bytes,
        })?)
    }
}

#[cfg(test)]
mod tests;
