use thiserror::Error;

use lyonoyl_domain::account::{MailAccount, MailAccountSummary};

/// Failures a credential-free account store can report.
///
/// Free of adapter types: the adapter maps its own errors into these variants.
#[derive(Debug, Error)]
pub enum AccountRepositoryError {
    /// The store cannot be reached: the file is locked, missing or corrupted.
    #[error("the local account store is unavailable")]
    Unavailable,
    /// A stored row violates a domain invariant — usually written by another version of the app.
    #[error("a stored account is invalid")]
    Corrupted,
    /// This device has numbered as many accounts as an identifier can carry.
    ///
    /// A message identifier holds the number of the index it came from beside the number of the
    /// row, so an account number past what that leaves room for would make two accounts mint the
    /// same identifier. Refusing to add the account is the only answer that does not corrupt the
    /// ones already here.
    #[error("this device cannot number another account")]
    NoNumbersLeft,
}

/// Read and write access to non-secret account metadata.
///
/// Secrets do not pass through this port; they live in the store this device keeps its secrets in.
pub trait AccountRepository: Send + Sync {
    fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError>;

    /// The accounts somebody has told where to send from.
    ///
    /// One question rather than one per account: a message written from an account with no server
    /// to hand it to would sit in the outbox for ever with nothing said, and the way to avoid that
    /// is to say so before it is written rather than after.
    fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError>;

    /// One account with everything needed to reach its mail, or nothing when it is not here.
    fn find(&self, account_id: &str) -> Result<Option<MailAccount>, AccountRepositoryError>;

    /// The identifier the account with this address is stored under, whatever its case.
    ///
    /// What lets a person supply a password again for an account they already have, instead of
    /// being told they already have it.
    fn find_by_email(&self, email: &str) -> Result<Option<String>, AccountRepositoryError>;

    fn save(&self, account: &MailAccount) -> Result<(), AccountRepositoryError>;

    /// Gets its caller with the screen that removes an account; exercised by the adapter tests.
    #[allow(dead_code)]
    fn delete(&self, account_id: &str) -> Result<(), AccountRepositoryError>;

    /// The number the index knows this account by, which is what its mail file is named after.
    ///
    /// A storage fact rather than a domain one, which is why it is here and not on the account: the
    /// domain has no opinion about how many files the mail is kept in.
    fn number_of(&self, account_id: &str) -> Result<Option<i64>, AccountRepositoryError>;

    /// Whether the account's folders are folded away in the sidebar. Stored with the account so
    /// the sidebar looks the way it was left after a restart.
    fn set_collapsed(
        &self,
        account_id: &str,
        collapsed: bool,
    ) -> Result<(), AccountRepositoryError>;
}
