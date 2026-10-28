// Implemented by a provider adapter. Until one exists, the request fields are written by the sync
// use case and read only by its tests.
#![allow(dead_code)]

use std::ops::RangeInclusive;
use std::time::Duration;

use thiserror::Error;

use crate::ports::credential::Credential;
use lyonoyl_domain::account::MailAccount;
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{FlagScope, FolderId, ModSeq, Uid, UidValidity};
use lyonoyl_domain::sync::{SyncBatch, SyncCursor};

/// What a mail provider can fail with.
///
/// The variants are the ones something above reacts to differently: the interface waiting out a
/// network outage, asking for a password again, or reporting a defect — and a pass telling a
/// server's moment from a refusal it repeats on every pass, which the interface sees as an outage.
/// Adapter-specific detail is mapped inside the adapter and does not appear here.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum MailProviderError {
    #[error("the provider cannot be reached")]
    Offline,
    #[error("the provider rejected the credentials")]
    Unauthorized,
    #[error("the provider answered in a way this build cannot use")]
    Protocol,
    #[error("the operation was cancelled")]
    Cancelled,
    /// The server refused for a moment and said so: a part of it down, the mailbox in use. The
    /// same question asked later may well be answered whole, so what is missing from this answer
    /// says nothing about any message.
    #[error("the provider refused for now")]
    RefusedForNow,
}

/// A mailbox as the provider describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredMailbox {
    /// The name on the server, in its own hierarchy.
    pub name: String,
    /// What the provider says the folder is for, or `Custom` when it says nothing.
    pub role: FolderRole,
    pub uid_validity: Option<UidValidity>,
    pub uid_next: Option<Uid>,
}

/// Listing the mailboxes of an account and what each one is for.
pub trait MailboxDiscovery: Send + Sync {
    fn list_mailboxes(&self) -> Result<Vec<DiscoveredMailbox>, MailProviderError>;
}

/// A provider handed out as a boxed trait object is still a provider.
///
/// The composition root does not know which protocol an account uses, so what it has is a box; the
/// use cases above it take a provider by value.
impl MailboxDiscovery for Box<dyn MailboxDiscovery> {
    fn list_mailboxes(&self) -> Result<Vec<DiscoveredMailbox>, MailProviderError> {
        (**self).list_mailboxes()
    }
}

/// Builds a provider for one account.
///
/// The composition root decides which protocol backs an account; nothing above this port knows
/// whether it is IMAP or something else. The credential is borrowed for the call and not kept —
/// and it may be a password or a token, which only the adapter below needs to know.
pub trait ProviderFactory: Send + Sync {
    /// How many conversations with a server this factory's providers have opened, ever.
    ///
    /// Only ever goes up, so what a stretch of work cost is the difference across it — no reset,
    /// no lock, and nothing to get wrong when two accounts are being read at once.
    ///
    /// Here rather than in the adapter because the use case is what knows which part of a pass it
    /// is in, and the cost of a pass is mostly this number. A factory that opens nothing — a fake,
    /// a file on disk — answers nought and is telling the truth.
    fn conversations(&self) -> (u64, std::time::Duration) {
        (0, std::time::Duration::ZERO)
    }

    fn discovery(
        &self,
        account: &MailAccount,
        credential: &Credential,
    ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError>;

    fn sync(
        &self,
        account: &MailAccount,
        credential: &Credential,
    ) -> Result<Box<dyn MailSync>, MailProviderError>;

    fn watch(
        &self,
        account: &MailAccount,
        credential: &Credential,
    ) -> Result<Box<dyn MailWatch>, MailProviderError>;

    fn reading(
        &self,
        account: &MailAccount,
        credential: &Credential,
    ) -> Result<Box<dyn MailFetch>, MailProviderError>;
}

/// A provider handed out as a boxed trait object syncs like any other.
/// Borrowed rather than owned, so one provider can read every folder of a pass.
///
/// A provider keeps its connection, so building one per folder is a connection and a sign-in per
/// folder — which is what this exists to stop being the shape of the code.
impl<T: MailSync + ?Sized> MailSync for &T {
    fn flag_scope(&self) -> FlagScope {
        (**self).flag_scope()
    }

    fn fetch(&self, request: &SyncRequest) -> Result<SyncBatch, MailProviderError> {
        (**self).fetch(request)
    }
}

impl MailSync for Box<dyn MailSync> {
    fn flag_scope(&self) -> FlagScope {
        (**self).flag_scope()
    }

    fn fetch(&self, request: &SyncRequest) -> Result<SyncBatch, MailProviderError> {
        (**self).fetch(request)
    }
}

/// How a watch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watched {
    /// The mailbox came to hold more than it did: mail arrived. Read the account now.
    Arrived,
    /// What the mailbox already held changed — a flag, a message gone — and nothing arrived. Read
    /// the account now, unless it is what this device has just told the server coming back.
    Changed,
    /// Nothing happened while the watch lasted. Not a failure — watch again.
    Quiet,
    /// This server cannot tell anyone anything until it is asked. Back to the timer.
    CannotPush,
}

/// How far into its mailbox a watch had seen when it ended: where the next watch takes up.
///
/// Opaque above the adapter, like a sync cursor: what it holds is whatever the protocol can compare
/// a mailbox against later. Between two watches nothing is listening — the news is being read, or
/// the next connection is being opened — and a watch that began from the mailbox as it found it
/// would count what arrived meanwhile as already there, and never report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watermark(String);

impl Watermark {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How a watch ended, and how far into the mailbox it had seen by then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEnded {
    pub watched: Watched,
    /// Where the next watch on the same mailbox takes up. Nothing where the watch never opened it.
    pub seen: Option<Watermark>,
}

impl From<Watched> for WatchEnded {
    /// A watch that says nothing about where it got to, so the next one starts where it finds the
    /// mailbox.
    fn from(watched: Watched) -> Self {
        Self {
            watched,
            seen: None,
        }
    }
}

/// Waiting for a server to say that something arrived, instead of asking it every two minutes.
///
/// A separate port from reading, because it is a different bargain: a watch holds a connection open
/// for as long as it lasts, and what it gives back is the fact that something changed and whether
/// mail arrived with it — never what changed. The pass that follows is the same pass a timer would
/// have run.
pub trait MailWatch: Send + Sync {
    /// Waits until the server reports a change in `mailbox`, or until `at_most` has passed.
    ///
    /// `since` is where the last watch on the mailbox ended. Mail that arrived after that is
    /// reported the moment the mailbox is opened, without waiting for more.
    fn watch(
        &self,
        mailbox: &str,
        since: Option<&Watermark>,
        at_most: Duration,
    ) -> Result<WatchEnded, MailProviderError>;
}

impl MailWatch for Box<dyn MailWatch> {
    fn watch(
        &self,
        mailbox: &str,
        since: Option<&Watermark>,
        at_most: Duration,
    ) -> Result<WatchEnded, MailProviderError> {
        (**self).watch(mailbox, since, at_most)
    }
}

/// Reading one message whole, as it stands on the server.
///
/// Separate from a pass: a pass reads what changed about many messages, this reads everything about
/// one, and it happens because a person opened it rather than because a timer came round.
pub trait MailFetch: Send + Sync {
    /// The bytes of one message, headers and body together, without marking it read.
    ///
    /// `uid_validity` is the generation `uid` was stored under. A number counts in its generation
    /// and nowhere else: a mailbox that has renumbered since gave it to another message, so the
    /// provider refuses rather than reads when the mailbox counts in any other.
    fn whole_message(
        &self,
        mailbox: &str,
        uid_validity: UidValidity,
        uid: Uid,
    ) -> Result<Vec<u8>, MailProviderError>;

    /// The same, for several messages of one mailbox, in one conversation with the server.
    ///
    /// Asked for as a group because the cost of reading a message is not the message: measured
    /// against a server on this machine, opening the connection and signing in was 74% of what one
    /// message cost, and fifty messages read one at a time cost 4.4 times what the same fifty cost
    /// on one connection. A mailbox of ten thousand is two hundred rounds of fifty, and at one
    /// connection each that is ten thousand sign-ins for mail the person already owns.
    ///
    /// Answers with what it got, in any order, and says nothing about what it did not: a message
    /// the server would not hand over is one message missing from the answer, not a failed round
    /// for the other forty-nine. The caller pairs them back up by `Uid`. A server that ends the
    /// conversation over one message rather than saying so fails the request whole, as
    /// [`Unread::Failed`], and what it named can then be asked for a message at a time for as
    /// long as the server can still be asked at all — which [`Unread::NotAsked`] says it cannot.
    ///
    /// The default is the old way, one at a time, so a fake that has nothing to gain from batching
    /// does not have to say so twice.
    fn whole_messages(
        &self,
        mailbox: &str,
        uid_validity: UidValidity,
        uids: &[Uid],
    ) -> Result<Vec<(Uid, Vec<u8>)>, Unread> {
        Ok(uids
            .iter()
            .filter_map(|uid| Some((*uid, self.whole_message(mailbox, uid_validity, *uid).ok()?)))
            .collect())
    }
}

/// Why messages asked for whole did not arrive.
///
/// Two different things, and what a caller may do next depends on which. A request that went out
/// and failed may have failed over one message in it, which a server ending the conversation at a
/// message it cannot read does, and the others are worth asking for one at a time. A request that
/// never went out says nothing about any message: the next one goes out the same way, and asked
/// one at a time, each is another connection tried and given up on — or another refused sign-in
/// counted toward locking the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unread {
    /// Nothing was asked: no conversation could be had, the credentials were refused, or the
    /// mailbox would not open under the generation the numbers were stored in.
    NotAsked(MailProviderError),
    /// Asked, and the answer failed: refused outright, or ended in the middle.
    Failed(MailProviderError),
}

/// Borrowed, for the same reason as above: one provider reads the bodies of every folder.
impl<T: MailFetch + ?Sized> MailFetch for &T {
    fn whole_message(
        &self,
        mailbox: &str,
        uid_validity: UidValidity,
        uid: Uid,
    ) -> Result<Vec<u8>, MailProviderError> {
        (**self).whole_message(mailbox, uid_validity, uid)
    }

    fn whole_messages(
        &self,
        mailbox: &str,
        uid_validity: UidValidity,
        uids: &[Uid],
    ) -> Result<Vec<(Uid, Vec<u8>)>, Unread> {
        (**self).whole_messages(mailbox, uid_validity, uids)
    }
}

impl MailFetch for Box<dyn MailFetch> {
    fn whole_message(
        &self,
        mailbox: &str,
        uid_validity: UidValidity,
        uid: Uid,
    ) -> Result<Vec<u8>, MailProviderError> {
        (**self).whole_message(mailbox, uid_validity, uid)
    }

    fn whole_messages(
        &self,
        mailbox: &str,
        uid_validity: UidValidity,
        uids: &[Uid],
    ) -> Result<Vec<(Uid, Vec<u8>)>, Unread> {
        (**self).whole_messages(mailbox, uid_validity, uids)
    }
}

/// What a pass is for.
///
/// Two directions, and they are not the same job. Forward is "what has happened since last time":
/// it moves the cursor and may state what disappeared. Older is "read further back": it moves the
/// floor, leaves the cursor alone, and says nothing about disappearances — a range being read for
/// the first time cannot report that anything went missing from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncIntent {
    Forward,
    /// The range to read, decided by the caller rather than the adapter: how much history to take
    /// at a time is a policy, and policy does not belong in a protocol adapter.
    Older(RangeInclusive<Uid>),
}

/// Where a sync pass resumes from.
pub struct SyncRequest {
    pub folder_id: FolderId,
    pub intent: SyncIntent,
    /// The name the provider knows the folder by, in the provider's own hierarchy.
    pub mailbox: String,
    pub cursor: Option<SyncCursor>,
    /// The generation the stored UIDs belong to, so the adapter can notice a reset itself.
    pub uid_validity: Option<UidValidity>,
    /// The UIDs the index already holds for this folder.
    ///
    /// A pass checks what is still there inside this range and what its flags are now. Outside it
    /// the adapter knows nothing, and says so.
    pub known_window: Option<RangeInclusive<Uid>>,
    /// The highest modification sequence already applied, where the provider keeps them.
    pub highest_modseq: Option<ModSeq>,
    /// How many placements the folder holds.
    ///
    /// A server says how many messages a mailbox has as soon as it is opened. Comparing that with
    /// this tells a pass whether anything disappeared at all — and when nothing did, it has no
    /// reason to list the mailbox to find out.
    pub known_count: usize,
    /// Whether the server refused a reading of this kind for a moment and none has been applied
    /// since: the new mail, or the range further back, as `intent` says.
    ///
    /// Not whether the last time it was asked was refused: a reading answered and then not
    /// applied, or one that failed another way, leaves it standing. Refused so again while it
    /// stands, the reading is refused like any other, and what arrived is mail and the rest is
    /// kept as what the server says it knows of each message: a server that says it of one message
    /// on every pass would otherwise stop the folder there for good.
    pub refused_for_now_last_time: bool,
}

/// Reading changes from a provider.
///
/// Separate from mutation and submission because the three vary independently: an IMAP account
/// syncs over one connection, sends over another, and a provider API answers all three as HTTP.
pub trait MailSync: Send + Sync {
    /// Whether flags belong to the message or to each placement of it.
    fn flag_scope(&self) -> FlagScope;

    fn fetch(&self, request: &SyncRequest) -> Result<SyncBatch, MailProviderError>;
}
