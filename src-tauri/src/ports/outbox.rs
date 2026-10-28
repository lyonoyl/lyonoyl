//! Where a message waits between being written and being accepted by a server, and then until its
//! copy is in the folder for sent mail.
//!
//! The one place in this application whose contents cannot be fetched again. Everything else on
//! this device is a copy of what a server holds; a message somebody has just written exists here
//! and nowhere else, and stays here until a server says it has taken it. Once its copy is in the
//! folder for sent mail and the account has been read back, the account turns out to have no such
//! folder, or its server will not take the copy, nothing of it is kept here: not its text, not who
//! it went to, not the files it carried. The same holds once it goes back to the drafts because it
//! will not be tried again: the draft it comes back as is then the only place it is.

use std::time::Duration;
use thiserror::Error;

use lyonoyl_domain::outgoing::Outgoing;

/// How long after a server took a message its copy is looked for in the folder for sent mail.
///
/// Long enough for a server that files a sent message itself to have done so in the ordinary
/// case, short enough that the message is in Sent seconds after it went. A server slower than this
/// is given a second copy.
pub const A_COPY_SETTLES: Duration = Duration::from_secs(3);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OutboxError {
    #[error("the outbox is unavailable")]
    Unavailable,
    /// The same message written twice. A `Message-ID` names one message for ever.
    #[error("that message is already waiting")]
    AlreadyWaiting,
    /// A stored message violates a rule this build enforces — written by an older version, before
    /// the rule existed. It is not sent and it is not discarded: the outbox is the one thing here
    /// that is nobody's copy of anything.
    #[error("a message waiting in the outbox cannot be read by this version")]
    Corrupted,
}

/// Why a message stopped, in terms a person can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhyHeld {
    /// The server refused it outright: a bad address, a message too large, a refusal to relay.
    TheServerRefused,
    /// Sign-in failed. Nothing about the message is wrong.
    SignInFailed,
    /// The conversation ended after the message was handed over, and nobody knows whether it
    /// arrived. Never retried on its own — sending twice is worse than not sending.
    NobodyKnows,
}

/// One message the outbox is holding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub id: i64,
    /// The name it will carry for ever, written before the first attempt.
    pub message_id: String,
    pub message: Outgoing,
    pub attempts: u32,
    /// When the person pressed send.
    ///
    /// The `Date` of the message, and the same one whether it is being handed to a server now or
    /// filed in the sent folder a minute later. Taken from the row rather than from the clock,
    /// because the clock gives a different answer each time it is asked.
    pub written_at: i64,
    /// How many times a server has refused its copy for now. Only a message a server has taken has
    /// a copy to refuse, and nothing but filing reads this.
    pub copy_refused: u32,
}

pub trait Outbox: Send + Sync {
    /// Writes a message down, under the name it will keep.
    ///
    /// Answers with nothing rather than with a position: what a person is shown next is read back,
    /// and a caller that remembered a number would be remembering it across a commit it does not
    /// own.
    fn queue(
        &self,
        account_id: &str,
        message_id: &str,
        message: &Outgoing,
    ) -> Result<(), OutboxError>;

    /// What is ready to be handed to a server now, oldest first.
    fn ready(&self, account_id: &str, at_most: usize) -> Result<Vec<Held>, OutboxError>;

    /// Takes one waiting message to hand over, and says so on the disk before a byte of it leaves.
    ///
    /// Answers whether it was taken, and only a waiting message is: one already taken, held for a
    /// person or sent is not this round's to hand over. One taken and never given an outcome is
    /// still taken when the next round starts, and is not handed over again: see `interrupted`.
    fn claim(&self, id: i64) -> Result<bool, OutboxError>;

    /// Messages a round took to hand over and never said what became of.
    ///
    /// The server may have taken them, and nothing here can find out — so nothing but a person
    /// hands one over again.
    fn interrupted(&self, account_id: &str) -> Result<Vec<Held>, OutboxError>;

    /// Says a server has taken it, after which nothing hands it over again. What is left to do is
    /// its copy in the folder for sent mail, and `filed` is what ends its stay here.
    fn accepted(&self, id: i64) -> Result<(), OutboxError>;

    /// Says it stopped, and why: given a wait, it is offered again once the wait is over; given
    /// none, it is held, and no round hands it over again. About a row that is gone, it changes
    /// nothing and does not fail.
    ///
    /// A round only ever gives it a wait. A message a round will not try again goes back to the
    /// drafts instead, and the outbox lets go of it there (`Drafts::came_back`), so nothing is
    /// left here to say it stopped.
    fn stopped(&self, id: i64, why: WhyHeld, retry_after: Option<u64>) -> Result<(), OutboxError>;

    /// Messages a server has taken and that are not yet in the folder for sent mail.
    ///
    /// Only ones a server took longer ago than `settled`, counted from when it took them rather
    /// than from when they were written: a message that waited an hour for a network is taken a
    /// moment ago all the same. A server that files a sent message itself does not always do it in
    /// the same breath, and looking too soon finds nothing and puts a second copy there — which is
    /// the one mistake this whole path exists to avoid. Nor one whose copy was refused for now and
    /// is not to be asked about again yet: see `refused_for_now`.
    fn awaiting_filing(
        &self,
        account_id: &str,
        settled: Duration,
        at_most: usize,
    ) -> Result<Vec<Held>, OutboxError>;

    /// Says the server refused its copy for now: counted, and not offered for filing again until
    /// `retry_after` seconds have passed.
    ///
    /// Only a message a server has taken has a copy to refuse, so nothing else is changed, and
    /// about one that is gone this changes nothing and does not fail.
    fn refused_for_now(&self, id: i64, retry_after: u64) -> Result<(), OutboxError>;

    /// Says its filing is over: its copy is in the folder for sent mail, however it got there, the
    /// account has no such folder to put one in, or its server will not take the copy.
    ///
    /// Marked rather than let go. The list of sent mail shows the message from this row until the
    /// account is read back and shows the server's copy instead; let go here, the message would
    /// vanish from Sent for as long as that read takes. Only a message a server has taken is
    /// marked: one waiting, being handed over or held for a person is somebody's writing, and
    /// stays whatever this is called with.
    fn filed(&self, id: i64) -> Result<(), OutboxError>;

    /// Lets go of every message of the account whose filing is over, once the account has been
    /// read back.
    ///
    /// A sent message is not kept on this device after that. The message, everybody it went to,
    /// blind copies included, and the files it carried all go in one change, and nothing would
    /// read any of them again. Nothing goes with them that is still to be filed, not yet sent, or
    /// another account's.
    fn forget_filed(&self, account_id: &str) -> Result<(), OutboxError>;

    /// Lets go of every message of the account whose row names the draft it came back as.
    ///
    /// A message given back leaves no row to name it on: `Drafts::came_back` lets the row go in
    /// the change that writes the draft. A row that names one holds nothing that draft does not,
    /// files included, so each goes whole, whatever state it was left in. A row that names none
    /// stays, held or not, and so does every row of another account: what it holds may be held
    /// nowhere else, its files among them.
    fn forget_given_back(&self, account_id: &str) -> Result<(), OutboxError>;
}

impl<O: Outbox + ?Sized> Outbox for &O {
    fn awaiting_filing(
        &self,
        account_id: &str,
        settled: Duration,
        at_most: usize,
    ) -> Result<Vec<Held>, OutboxError> {
        (**self).awaiting_filing(account_id, settled, at_most)
    }

    fn refused_for_now(&self, id: i64, retry_after: u64) -> Result<(), OutboxError> {
        (**self).refused_for_now(id, retry_after)
    }

    fn filed(&self, id: i64) -> Result<(), OutboxError> {
        (**self).filed(id)
    }

    fn forget_filed(&self, account_id: &str) -> Result<(), OutboxError> {
        (**self).forget_filed(account_id)
    }

    fn forget_given_back(&self, account_id: &str) -> Result<(), OutboxError> {
        (**self).forget_given_back(account_id)
    }

    fn queue(
        &self,
        account_id: &str,
        message_id: &str,
        message: &Outgoing,
    ) -> Result<(), OutboxError> {
        (**self).queue(account_id, message_id, message)
    }

    fn ready(&self, account_id: &str, at_most: usize) -> Result<Vec<Held>, OutboxError> {
        (**self).ready(account_id, at_most)
    }

    fn claim(&self, id: i64) -> Result<bool, OutboxError> {
        (**self).claim(id)
    }

    fn interrupted(&self, account_id: &str) -> Result<Vec<Held>, OutboxError> {
        (**self).interrupted(account_id)
    }

    fn accepted(&self, id: i64) -> Result<(), OutboxError> {
        (**self).accepted(id)
    }

    fn stopped(&self, id: i64, why: WhyHeld, retry_after: Option<u64>) -> Result<(), OutboxError> {
        (**self).stopped(id, why, retry_after)
    }
}
