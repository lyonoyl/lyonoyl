//! The message itself, kept where it was read.
//!
//! Everything else this application stores is a description of a message — who sent it, when it
//! arrived, what its first line says. This is the message: the bytes the server handed over,
//! whole. A copy, and deliberately so: it can be fetched again, and the file it lives in may be
//! deleted without losing anything.
//!
//! What it is for is the difference between a mail client and a window onto a server. A message
//! read once opens again with no network, at once; showing it as plain text, or with its pictures,
//! or under a different colour scheme, is a redraw rather than another conversation with a
//! provider several hundred milliseconds away.

use thiserror::Error;

use lyonoyl_domain::message::MessageId;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MessageBodyError {
    #[error("the message store is unavailable")]
    Unavailable,
}

pub trait MessageBodies: Send + Sync {
    /// The message, if this device already has it.
    fn held(&self, message: MessageId) -> Result<Option<Vec<u8>>, MessageBodyError>;

    /// Keeps what a server just handed over.
    ///
    /// Failing to keep it is not failing to read it: the message is already in hand, and the only
    /// thing lost is the next opening being quick.
    fn keep(&self, message: MessageId, raw: &[u8]) -> Result<(), MessageBodyError>;

    /// Fills in what this device knows about a message but never wrote down.
    ///
    /// A message read into the index before a column existed carries nothing in it, and nothing
    /// puts it there afterwards: a pass reads what is new, and this message is not new. Until it is
    /// filled in, an answer to it is addressed to nobody, it starts a conversation of its own
    /// rather than continuing one, and the shortcut to answering everybody is not offered. A
    /// message the pass could not describe at all — its server would not hand it over, or the
    /// parser broke on it — is a row of its date, size and flags, listed with no subject and no
    /// sender until this fills in what the list shows of it as well. And with no name: this gives
    /// it the one the message goes by, or, where another message of the account already goes by
    /// it, gives that message the row's copies — the two are one message, and the one already
    /// going by its name is the one every later copy of it is found as. Where the copies went is
    /// written down apart from the row, and outlasts it: the window and the queue may still call
    /// the message by the row, and where it sits, whether a folder holds it, where it is read from
    /// and what the reader shows of it are answered for the message it turned out to be, for as
    /// long as the account holds that message. This store goes by the row itself: what it keeps
    /// under the row's identifier is the row's, and goes when the row is forgotten.
    ///
    /// Everything needed is in the message itself, and the message is already here — so this is
    /// done from what is in hand and never by asking a server again. Only what is missing is
    /// written: a column somebody's own build filled in is not this one's to overwrite.
    ///
    /// One thing is written every time, and it is not a repair: that the message was read whole.
    /// The pass asks that before fetching anything, so it has to be true after a reading that
    /// filled in nothing — including one whose bytes were not a message at all. Without it the
    /// message is asked for again every round, for ever.
    fn fill_in_missing(&self, message: MessageId, raw: &[u8]) -> Result<(), MessageBodyError>;
}

impl<S: MessageBodies + ?Sized> MessageBodies for &S {
    fn held(&self, message: MessageId) -> Result<Option<Vec<u8>>, MessageBodyError> {
        (**self).held(message)
    }

    fn fill_in_missing(&self, message: MessageId, raw: &[u8]) -> Result<(), MessageBodyError> {
        (**self).fill_in_missing(message, raw)
    }

    fn keep(&self, message: MessageId, raw: &[u8]) -> Result<(), MessageBodyError> {
        (**self).keep(message, raw)
    }
}
