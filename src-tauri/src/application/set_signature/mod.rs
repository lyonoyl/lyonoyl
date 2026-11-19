//! Keeping what a mailbox puts at the foot of the messages written from it.

use lyonoyl_domain::signature::{Signature, SignatureError};

use crate::application::error::UseCaseError;
use crate::ports::signatures::Signatures;

pub struct SetSignature<S> {
    signatures: S,
}

impl<S: Signatures> SetSignature<S> {
    pub fn new(signatures: S) -> Self {
        Self { signatures }
    }

    /// Keeps the signature typed for this mailbox, and answers with it as kept: on lines of its
    /// own, without the blank lines before it or the spaces after it, which is what the window then
    /// shows and the composer puts into a message.
    pub fn execute(&self, account_id: &str, typed: &str) -> Result<Signature, UseCaseError> {
        let signature = Signature::new(typed).map_err(|error| match error {
            SignatureError::TooLong => UseCaseError::SignatureTooLong,
        })?;

        if !self.signatures.set_signature(account_id, &signature)? {
            return Err(UseCaseError::UnknownAccount);
        }

        Ok(signature)
    }
}

#[cfg(test)]
mod tests;
