//! Passing a message on to somebody else: what it says and every file it carries, or the message
//! itself as one file.
//!
//! Read from the message itself rather than from what the reader drew of it, which is a picture of
//! a message and not one. From this device where it is here, as it is for somebody who has just
//! read it, and otherwise from its server, the way opening it reads it: a message passed on without
//! the files it carries is a different message, and nothing on screen would say so.
//!
//! What it goes to, and the lines above it saying whose it was, are the window's: the first is the
//! person's to choose, and the second is prose written in their language.

use crate::application::error::UseCaseError;
use crate::application::whole_message::WholeMessage;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential_store::CredentialStore;
use crate::ports::mail_provider::ProviderFactory;
use crate::ports::message_bodies::MessageBodies;
use crate::ports::message_parts::{MessageParts, PassedOn};
use lyonoyl_domain::attachment::message_file_name;
use lyonoyl_domain::message::{MessageId, Uid, UidValidity};

pub struct ForwardMessage<R, C, F, S, P>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    S: MessageBodies,
    P: MessageParts,
{
    accounts: R,
    credentials: C,
    providers: F,
    kept: S,
    parts: P,
}

impl<R, C, F, S, P> ForwardMessage<R, C, F, S, P>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    S: MessageBodies,
    P: MessageParts,
{
    pub fn new(accounts: R, credentials: C, providers: F, kept: S, parts: P) -> Self {
        Self {
            accounts,
            credentials,
            providers,
            kept,
            parts,
        }
    }

    /// What the message says and carries, as it is passed on.
    ///
    /// Where the message sits on its server is the index's to say, as it is for opening it. A
    /// message nowhere could read is [`UseCaseError::TooBusyToRead`] rather than a message with
    /// nothing in it: passed on empty, it would go to somebody as a message saying nothing.
    pub fn execute(
        &self,
        account_id: &str,
        message: MessageId,
        mailbox: &str,
        uid_validity: Option<UidValidity>,
        uid: Uid,
    ) -> Result<PassedOn, UseCaseError> {
        let raw = WholeMessage {
            accounts: &self.accounts,
            credentials: &self.credentials,
            providers: &self.providers,
            kept: &self.kept,
        }
        .read(account_id, message, mailbox, uid_validity, uid)?;

        self.parts
            .passed_on(&raw)
            .ok_or(UseCaseError::TooBusyToRead)
    }

    /// The message itself, as one file to go inside another, and the name it goes by.
    ///
    /// The bytes its server handed over, unchanged, read the way [`Self::execute`] reads them: an
    /// exact copy with everything it carries, which nothing on this side draws or edits. Named as
    /// a saved message is, from `subject`, which the index holds.
    pub fn whole(
        &self,
        account_id: &str,
        message: MessageId,
        mailbox: &str,
        uid_validity: Option<UidValidity>,
        uid: Uid,
        subject: Option<&str>,
    ) -> Result<(String, Vec<u8>), UseCaseError> {
        let raw = WholeMessage {
            accounts: &self.accounts,
            credentials: &self.credentials,
            providers: &self.providers,
            kept: &self.kept,
        }
        .read(account_id, message, mailbox, uid_validity, uid)?;

        Ok((message_file_name(subject), raw))
    }
}

#[cfg(test)]
mod tests;
