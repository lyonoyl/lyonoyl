// Reached from the composition root once an account can be added.
#![allow(dead_code)]

use secrecy::SecretString;
use thiserror::Error;

/// Where secrets live.
///
/// Passwords and refresh tokens never reach SQLite, a log line, an error message or a sync payload.
/// The database keeps a [`CredentialRef`], which names an entry in the store and is useless on its
/// own. The store is the system's own, or on a Linux desktop whose first start found none, a file
/// opened with a master password.

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum CredentialError {
    /// The store cannot be reached: the system's own locked, absent or refused, or a file of secrets
    /// closed or unwritable.
    #[error("the credential store is unavailable")]
    Unavailable,
    /// There is no entry under that name.
    #[error("no credential is stored under that name")]
    NotFound,
}

/// What a stored secret is for. Two secrets of one account never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialPurpose {
    /// The password used to read mail.
    ImapPassword,
    /// The password used to send mail, when the provider wants a separate one.
    SmtpPassword,
    /// The token that buys new access tokens.
    OAuthRefreshToken,
    /// The key the mail kept on this device is sealed under. Not an account's: one per
    /// installation, so a message stays readable after the account it arrived in is removed.
    MessageKey,
    /// The key the databases themselves are encrypted with. One per installation, and separate
    /// material from the one messages are sealed under: two things unlocked by one number is one
    /// number that cannot be changed for either.
    DatabaseKey,
}

impl CredentialPurpose {
    /// The text that goes into the entry name. Stored, so it must not be renamed.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImapPassword => "imap-password",
            Self::SmtpPassword => "smtp-password",
            Self::OAuthRefreshToken => "oauth-refresh-token",
            Self::MessageKey => "message-key",
            Self::DatabaseKey => "database-key",
        }
    }
}

/// Names one entry in the store. Carries no secret and is safe to store and to log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRef {
    pub account_id: String,
    pub purpose: CredentialPurpose,
}

impl CredentialRef {
    pub fn new(account_id: impl Into<String>, purpose: CredentialPurpose) -> Self {
        Self {
            account_id: account_id.into(),
            purpose,
        }
    }
}

/// A store reached through a reference is still a store. Lets a wrapper — a cache, a decorator —
/// be handed to a use case without cloning what it holds.
impl<C: CredentialStore + ?Sized> CredentialStore for &C {
    fn save(
        &self,
        reference: &CredentialRef,
        secret: &SecretString,
    ) -> Result<(), CredentialError> {
        (**self).save(reference, secret)
    }

    fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        (**self).read(reference)
    }

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        (**self).delete(reference)
    }
}

/// Reading and writing secrets in the store this device keeps them in.
pub trait CredentialStore: Send + Sync {
    fn save(&self, reference: &CredentialRef, secret: &SecretString)
        -> Result<(), CredentialError>;

    fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError>;

    /// Removing an account removes its secrets. A missing entry is not a failure: the account is
    /// gone either way.
    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError>;
}
