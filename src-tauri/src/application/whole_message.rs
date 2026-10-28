//! The message itself, as the bytes its server handed over.
//!
//! What opening a message reads, and what passing one on reads. Read from the copy this device
//! keeps, and from the server only where it keeps none — and then kept, so the next reading is from
//! here. One place, because where a message is has one answer, and two readers asking it two ways
//! would be a message opened from here and passed on from a server that has since lost it.

use crate::application::error::UseCaseError;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential::Credential;
use crate::ports::credential_store::{CredentialPurpose, CredentialRef, CredentialStore};
use crate::ports::mail_provider::ProviderFactory;
use crate::ports::message_bodies::MessageBodies;
use lyonoyl_domain::message::{MessageId, Uid, UidValidity};

/// The ports a whole message is read through, borrowed from the use case asking.
pub(crate) struct WholeMessage<'a, R, C, F, S> {
    pub accounts: &'a R,
    pub credentials: &'a C,
    pub providers: &'a F,
    pub kept: &'a S,
}

impl<R, C, F, S> WholeMessage<'_, R, C, F, S>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    S: MessageBodies,
{
    /// The message, from this device or from its server.
    ///
    /// `mailbox`, `uid_validity` and `uid` say where the message sits on the server; all three
    /// come from the index, never from the interface, which knows a message only by its own
    /// identifier. `uid_validity` is absent where the index has no generation stored for the
    /// folder, and then the message is read only from this device.
    pub fn read(
        &self,
        account_id: &str,
        message: MessageId,
        mailbox: &str,
        uid_validity: Option<UidValidity>,
        uid: Uid,
    ) -> Result<Vec<u8>, UseCaseError> {
        // The copy first. A message read once is readable again with no server and with no wait —
        // which is the difference between a mail client and a window onto somebody else's.
        //
        // A store that will not answer is not a message that cannot be read: reading mail must not
        // depend on being able to keep a copy of it, so an unreadable store means the same thing
        // as an empty one.
        match self.kept.held(message).ok().flatten() {
            Some(raw) => Ok(raw),
            None => self.fetch_and_keep(account_id, message, mailbox, uid_validity, uid),
        }
    }

    /// Asks the server, and keeps what it hands over.
    fn fetch_and_keep(
        &self,
        account_id: &str,
        message: MessageId,
        mailbox: &str,
        uid_validity: Option<UidValidity>,
        uid: Uid,
    ) -> Result<Vec<u8>, UseCaseError> {
        // A number with no generation beside it cannot be checked against the mailbox, and reading
        // by it could read somebody else's message and keep it as this one.
        let uid_validity = uid_validity.ok_or(UseCaseError::StorageCorrupted)?;

        let account = self
            .accounts
            .find(account_id)?
            .ok_or(UseCaseError::UnknownAccount)?;

        let password = self.credentials.read(&CredentialRef::new(
            account_id.to_owned(),
            CredentialPurpose::ImapPassword,
        ))?;

        let raw = self
            .providers
            .reading(&account, &Credential::password(password))?
            .whole_message(mailbox, uid_validity, uid)?;

        // Failing to keep it is not failing to read it: the message is in hand, and what is lost
        // is only the next opening being quick.
        let _ = self.kept.keep(message, &raw);

        Ok(raw)
    }
}
