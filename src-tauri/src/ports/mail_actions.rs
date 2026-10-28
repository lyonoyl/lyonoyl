//! Telling the server to do something.
//!
//! The one port in the application that changes somebody else's mailbox. Everything above it deals
//! in messages and folders; what crosses here is already resolved to a position, and it is
//! resolved that late on purpose.
//!
//! The implementation is `infrastructure/imap/actions.rs`, and it is the one place in this
//! application that changes somebody else's mailbox. The gate in `scripts/check-layering.sh`
//! refuses `STORE`, `MOVE`, `EXPUNGE` and `APPEND` everywhere except there.

use thiserror::Error;

use lyonoyl_domain::message::{Flags, Uid, UidValidity};
use lyonoyl_domain::outgoing::Outgoing;

/// One thing to tell the server, already addressed.
///
/// Every one of them carries the generation of the mailbox its number was taken under. An
/// implementation opens the mailbox, reads the `UIDVALIDITY` the server states, and does nothing
/// at all unless the two agree: a folder that renumbered has handed the same numbers to different
/// messages, and acting on the old one acts on a message nobody asked about. The generation is
/// part of the address rather than a check a caller may remember to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerAction<'a> {
    SetFlags {
        mailbox: &'a str,
        uid: Uid,
        uid_validity: UidValidity,
        add: Flags,
        remove: Flags,
    },
    MoveTo {
        mailbox: &'a str,
        uid: Uid,
        uid_validity: UidValidity,
        destination: &'a str,
    },
    /// Mark and expunge: for an account with nowhere to keep deleted mail, and only ever when the
    /// person asked for that plainly.
    Erase {
        mailbox: &'a str,
        uid: Uid,
        uid_validity: UidValidity,
    },
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MailActionError {
    /// The server could not be reached, or said "not now": a moment of network, a mailbox locked
    /// by another client, a server being restarted. Worth asking again later.
    #[error("the server could not be reached, or would not do it now")]
    NotNow,
    /// It cannot be done and asking again will not change that: the message is not there any more,
    /// the mailbox is gone, the server refused outright.
    #[error("the server will not do it at all")]
    Refused,
    /// The mailbox states a different generation than the address was taken under, so nothing was
    /// done. Not the person's problem and not a refusal: the address is out of date, and the pass
    /// that reads the folder is what brings it up to date. Asking again after that works.
    ///
    #[error("the mailbox has renumbered since this was addressed")]
    FolderRenumbered,
}

/// What became of a copy this device tried to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filing {
    /// This device put it there.
    Added,
    /// The server had already put it there itself, so nothing was added.
    AlreadyThere,
    /// The account has no folder for sent mail that this device can find, so there is nowhere to
    /// put it. Not a failure: a message was still sent, and there is nothing to try again.
    Nowhere,
}

/// Why a copy was not kept.
///
/// Told apart by what it is about, because that is what decides the copies behind it. A failure
/// about the account is met by every copy after this one as well, so asking them is time spent
/// for nothing. A refusal is about this copy, and says nothing about the next: a message too large
/// says nothing about the short one sent after it.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum FilingError {
    /// The folder for sent mail was not reached, and nothing was heard about this copy: the server
    /// could not be reached, refused the sign-in, would not list its folders or open that one, said
    /// the folder is not there or not this account's to add to, stopped answering before this copy
    /// was offered to it, or answered the search or the offer out of step with the conversation.
    #[error("the folder for sent mail could not be reached")]
    FolderUnreachable,
    /// The server refused this copy and said it may take it later — over quota, a part of it down,
    /// the folder in use — or said nothing about until when, a limit reached among them, or ended
    /// the conversation once the copy was offered to it rather than answer. Or it would not say
    /// whether the copy is there already, and the copy was not offered.
    #[error("the server refused this copy for now")]
    RefusedForNow,
    /// The server refused this copy and said it never will: too large for it, or something it can
    /// never do.
    #[error("the server will never take this copy")]
    RefusedForGood,
}

/// What filing a copy makes of a server that refuses to search the folder for sent mail for it.
///
/// The search is asked first either way, and an answer to it decides: a copy it names is not added
/// again, even where the server goes on to refuse the search. Only a refusal that names no copy is
/// decided here, and only one about this copy: a refusal that says the folder is not there or not
/// this account's to add to is the folder not reached, and so is a search cut short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusedSearch {
    /// Nothing is added, and the copy counts as refused for now. A server that files sent mail
    /// itself and will not say whether it has may have put this very copy there.
    Heeded,
    /// The copy is offered all the same. A server that files its own copy may be given a second
    /// one, and so may one that kept a copy offered on an earlier try whose answer never came; kept
    /// back for as long as the server refuses the search, the copy is never filed at all.
    PassedOver,
}

/// Keeping a copy of a message this device has just sent.
///
/// Separate from the trait below because it is a different kind of thing: everything there names a
/// message that is already in a mailbox and changes it, and this adds one. It is also the only
/// place this application adds mail anywhere, which is why the guard on the stream names it
/// separately.
pub trait SentMail: Send + Sync {
    /// Puts a copy where the account keeps sent mail, unless one is already there.
    ///
    /// Whether one is there is asked by the name the message carries, which this device chose
    /// before the first attempt to send it. Servers differ on whether they file a sent message
    /// themselves, and the ones that do take a moment about it — so this is asked long enough
    /// afterwards that a server which files has finished. Nothing is added unless that question
    /// was answered, or `refused_search` passes a refusal of it over: a server that will not say
    /// whether a copy is there may have one.
    ///
    /// Takes the message rather than the text of it. Writing an RFC 5322 message is a thing an
    /// adapter does, and a port that asks for bytes is a port that decides who assembles them.
    fn file(
        &self,
        message_id: &str,
        message: &Outgoing,
        written_at: i64,
        refused_search: RefusedSearch,
    ) -> Result<Filing, FilingError>;
}

impl<S: SentMail + ?Sized> SentMail for &S {
    fn file(
        &self,
        message_id: &str,
        message: &Outgoing,
        written_at: i64,
        refused_search: RefusedSearch,
    ) -> Result<Filing, FilingError> {
        (**self).file(message_id, message, written_at, refused_search)
    }
}

/// One conversation with the server, opened to act in.
///
/// Every instruction of a round goes down the same one: a burst of changes is one connection and
/// one sign-in rather than one of each per message, which is the difference between a trash of a
/// thousand being emptied and a server refusing the account for asking too often.
///
/// Ended when it is dropped. An implementation ends it without expunging anything: on a mailbox
/// opened for writing, the polite way out of a mailbox also removes whatever is marked deleted.
pub trait Acting {
    fn carry_out(&mut self, action: ServerAction<'_>) -> Result<(), MailActionError>;
}

pub trait MailActions: Send + Sync {
    /// Opens a conversation to act in: the connection and the sign-in, and nothing asked yet.
    ///
    /// Separate from carrying anything out because the two failures mean different things. A
    /// conversation that could not be opened has told the server nothing at all — no instruction
    /// is any further along, and none of them is written down as sent. One that failed after it
    /// was opened may have been heard.
    fn reach(&self) -> Result<Box<dyn Acting + '_>, MailActionError>;
}

impl<A: MailActions + ?Sized> MailActions for &A {
    fn reach(&self) -> Result<Box<dyn Acting + '_>, MailActionError> {
        (**self).reach()
    }
}
