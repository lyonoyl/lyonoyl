//! Handing a written message to a server that will carry it.
//!
//! One method, and an answer with three shapes rather than two. SMTP is not idempotent: a
//! conversation that ends after the message has been handed over leaves nobody knowing whether it
//! was accepted, and a client that treats "I do not know" as "it failed" sends it twice. So that
//! outcome has a name of its own here, and the rule about it is written where it cannot be missed.

use thiserror::Error;

use lyonoyl_domain::outgoing::Outgoing;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionError {
    /// The server could not be reached, or would not talk. Worth trying again.
    #[error("the mail server could not be reached")]
    Unreachable,
    /// Sign-in failed. Nothing about the message is wrong, and trying again will not help until
    /// somebody does something about the account.
    #[error("the mail server rejected the credentials")]
    SignInFailed,
    /// The server read the message and said no: an address it will not carry to, a size it will
    /// not take, a refusal to relay. Trying again would say the same thing.
    #[error("the mail server refused the message")]
    Refused,
    /// The conversation ended after the message was handed over.
    ///
    /// **Never retried on its own.** The message may be on its way to everybody it names; sending
    /// it again would be the second copy they read. It waits for a person.
    #[error("the mail server stopped answering after the message was handed over")]
    NobodyKnows,
}

pub trait MailSubmission: Send + Sync {
    /// Hands one message over, under the name and the date it already carries.
    fn submit(
        &self,
        message_id: &str,
        message: &Outgoing,
        written_at: i64,
    ) -> Result<(), SubmissionError>;
}

impl<S: MailSubmission + ?Sized> MailSubmission for &S {
    fn submit(
        &self,
        message_id: &str,
        message: &Outgoing,
        written_at: i64,
    ) -> Result<(), SubmissionError> {
        (**self).submit(message_id, message, written_at)
    }
}
