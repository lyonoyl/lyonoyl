//! How much of this device the mail on it is taking, per account.
//!
//! Nothing here clears anything. The application does not delete somebody's mail from their own
//! machine to save room, and that is a decision rather than an omission — reading every message
//! whole is what makes one open without a network and findable by a word from anywhere in it.
//! What a person is owed is the number, so that clearing is a thing they can choose.

use crate::application::error::UseCaseError;
use crate::ports::account_repository::AccountRepository;
use crate::ports::mail_files::MailFiles;

/// One account and what its mail weighs on this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub account_id: String,
    pub email: String,
    pub display_name: String,
    /// Bytes. Zero for an account that has never synchronised, which is an answer.
    pub bytes: u64,
}

pub struct WhatIsHeld<A: AccountRepository, F: MailFiles> {
    accounts: A,
    files: F,
}

impl<A: AccountRepository, F: MailFiles> WhatIsHeld<A, F> {
    pub fn new(accounts: A, files: F) -> Self {
        Self { accounts, files }
    }

    /// Every account, largest first.
    ///
    /// Largest first because the question behind the screen is "what is taking the room", and an
    /// order by name would make somebody read fifty rows to find the answer.
    ///
    /// An account whose file cannot be measured is reported as holding nothing rather than
    /// failing the whole list: one unreadable file must not hide the other ninety-nine, and a
    /// person who sees a zero beside an account that plainly has mail has been told something is
    /// wrong more clearly than an empty screen would.
    pub fn execute(&self) -> Result<Vec<Held>, UseCaseError> {
        let mut held = Vec::new();

        for account in self.accounts.list()? {
            let number = self.accounts.number_of(account.id())?;

            held.push(Held {
                account_id: account.id().to_owned(),
                email: account.email().to_owned(),
                display_name: account.display_name().to_owned(),
                bytes: number
                    .and_then(|number| self.files.held_by(number).ok())
                    .unwrap_or(0),
            });
        }

        held.sort_by(|left, right| {
            right
                .bytes
                .cmp(&left.bytes)
                .then_with(|| left.email.cmp(&right.email))
        });

        Ok(held)
    }
}

#[cfg(test)]
mod tests;
