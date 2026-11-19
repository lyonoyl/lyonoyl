use lyonoyl_domain::signature::Signature;

use super::account_repository::AccountRepositoryError;

/// Where each mailbox keeps what it puts at the foot of the messages written from it.
///
/// Kept with the mailbox, so it goes when the mailbox does; read back with it, as part of what the
/// window is told about each mailbox.
pub trait Signatures: Send + Sync {
    /// Keeps the signature for the mailbox, an empty one being none. Answers whether the mailbox is
    /// here at all.
    fn set_signature(
        &self,
        account_id: &str,
        signature: &Signature,
    ) -> Result<bool, AccountRepositoryError>;
}
