//! What a mailbox puts at the foot of every message written from it.
//!
//! The person's own words, typed beside the mailbox in the settings and put under what they write.
//! Plain text on lines of its own, the way the composer writes everything else.

use std::fmt;

use thiserror::Error;

/// The longest signature kept, in characters.
///
/// Far past a name, a title, a telephone number and an address. A signature goes into every message
/// written from its mailbox, so one with no end would grow every one of them. Counted in characters
/// rather than bytes, as a group's name is, so a signature in Greek is worth as much as one in
/// English.
pub const LONGEST_SIGNATURE: usize = 2_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SignatureError {
    #[error("a signature may hold at most {LONGEST_SIGNATURE} characters")]
    TooLong,
}

/// A mailbox's signature, empty when it has none.
///
/// Kept the way it is put into a message: every line ending in one `\n`, which is how the composer
/// ends a line; no character that steers a terminal rather than writes anything, a tab aside; and no
/// blank line before it or space after it, which would only push the message apart from it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Signature(String);

impl Signature {
    pub fn new(typed: &str) -> Result<Self, SignatureError> {
        let lines = typed.replace("\r\n", "\n").replace('\r', "\n");
        let written: String = lines
            .chars()
            .filter(|character| matches!(character, '\n' | '\t') || !character.is_control())
            .collect();

        let written = written.trim_end();
        let blank_before: usize = written
            .split_inclusive('\n')
            .take_while(|line| line.trim().is_empty())
            .map(str::len)
            .sum();
        let kept = &written[blank_before..];

        if kept.chars().count() > LONGEST_SIGNATURE {
            return Err(SignatureError::TooLong);
        }

        Ok(Self(kept.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// How long it is and nothing more: a signature carries a name, a telephone number and an address,
/// none of which belongs in a log.
impl fmt::Debug for Signature {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Signature({} characters)",
            self.0.chars().count()
        )
    }
}

#[cfg(test)]
mod tests;
