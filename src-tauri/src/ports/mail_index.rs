use std::ops::RangeInclusive;

use thiserror::Error;

use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{FlagScope, FolderId, MessageId, ModSeq, Uid, UidValidity};
use lyonoyl_domain::sync::{FolderDelta, SyncCursor};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MailIndexError {
    #[error("the mail index is unavailable")]
    Unavailable,
    #[error("the mail index holds data this build cannot read")]
    Corrupted,
    #[error("the folder is not in the index")]
    UnknownFolder,
}

/// What the index knows about a folder before a pass is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSyncState {
    /// The name the provider knows the folder by. A pass has to name the mailbox it is reading,
    /// and only the index knows which name a folder identifier belongs to.
    pub name: String,
    pub uid_validity: Option<UidValidity>,
    pub cursor: Option<SyncCursor>,
    /// The UIDs the folder holds, lowest to highest, or nothing when it holds none.
    ///
    /// What a pass may make a statement about. A provider that lists a range can say "these are
    /// the ones that still exist" only about a range it knows the index has something in.
    pub known_window: Option<RangeInclusive<Uid>>,
    /// The lowest UID this folder has ever been read down to.
    ///
    /// Not the lowest UID it holds: a message removed from the bottom of the folder must not make
    /// the reading start over from there.
    pub window_floor: Option<Uid>,
    /// Whether the folder has been read all the way back to its first message.
    pub history_complete: bool,
    /// The highest modification sequence this folder has applied, where the provider keeps them.
    ///
    /// What lets a pass ask "what changed since" instead of listing a window and comparing. Taken
    /// from the placements rather than kept separately: it is exactly what has been stored, and it
    /// cannot drift from it.
    pub highest_modseq: Option<ModSeq>,
    /// Whether the last pass applied to this folder read new mail only as far as one window, so
    /// that what lies past it has not been seen — a message filed into the folder a moment ago
    /// among it. Written with the cursor that pass stored.
    pub stopped_short: bool,
    /// Whether the server refused a reading of the folder's new mail for a moment, and no reading
    /// of new mail has been applied since.
    ///
    /// Asked again on the next pass, and refused so again while this stands it is a refusal like
    /// any other.
    pub new_mail_refused_for_now: bool,
    /// The same, for the reading further back.
    pub further_back_refused_for_now: bool,
}

/// Which reading of a folder something is about: what arrived since the last pass, or a range
/// read further back. Each is refused, and asked again, on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    NewMail,
    FurtherBack,
}

/// Which stored UIDs a deletion decision needs.
#[derive(Debug, Clone, PartialEq, Eq)]
///
/// Never "all of them": a folder holds a hundred thousand placements, and a decision only ever
/// concerns the ones a report actually talks about.
pub enum UidScope {
    /// The UIDs a report named.
    Listed(Vec<Uid>),
    /// Everything stored inside the window a listing covered.
    Window(RangeInclusive<Uid>),
}

/// What applying one delta changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AppliedSummary {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

/// A mailbox as it is to be recorded, stripped of everything only the provider cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRecord {
    /// The name on the server, in its own hierarchy. Unique within an account.
    pub name: String,
    pub role: FolderRole,
}

/// The write side of the mail index.
pub trait MailIndex: Send + Sync {
    /// Records the mailboxes an account has, and answers with the identifier of each.
    ///
    /// Nothing is removed. A mailbox missing from one listing is not proof that it is gone — a
    /// server can answer a partial list, and the messages in a folder are the only copy on this
    /// device until a pass says otherwise. Removing a folder is a separate, deliberate act.
    fn record_folders(
        &self,
        account: i64,
        folders: &[FolderRecord],
    ) -> Result<Vec<(String, FolderId)>, MailIndexError>;

    fn folder_state(&self, folder_id: FolderId) -> Result<FolderSyncState, MailIndexError>;

    /// The inbox of an account, by the name its server knows it under.
    ///
    /// What a watch needs: a server has something to say about a mailbox only while that mailbox is
    /// open, and the inbox is the one worth holding a connection for. `None` before an account's
    /// first pass has recorded its mailboxes.
    fn inbox_of(&self, account: i64) -> Result<Option<String>, MailIndexError>;

    fn known_uids(&self, folder_id: FolderId, scope: &UidScope)
        -> Result<Vec<Uid>, MailIndexError>;

    /// How many placements the folder holds. The deletion cap is a share of this number.
    fn placement_count(&self, folder_id: FolderId) -> Result<usize, MailIndexError>;

    /// Messages in this folder this device has not read whole yet, newest first.
    ///
    /// Not "whose body is not stored", which is a different question and was the answer given for
    /// a while. A message too large to be worth keeping has no stored body however many times it
    /// is read, so that answer named it every round for ever; excluding it by its size stopped
    /// that and took its text out of the index, because a message never fetched is never read.
    /// What is asked here is what was done, and it is recorded when it is done.
    ///
    /// Newest first because that is the order somebody would want them in if they could only have
    /// some: the mail they are still dealing with. Bounded because a folder of a hundred thousand
    /// is not something to read in one pass.
    fn not_read_whole(
        &self,
        folder_id: FolderId,
        at_most: u32,
    ) -> Result<Vec<(MessageId, Uid)>, MailIndexError>;

    /// Records how far back the folder has been read, and whether that is all of it.
    ///
    /// Separate from applying a delta because a range can be read and hold nothing: the folder is
    /// then further along than before, with not one new message to show for it, and a floor that
    /// did not move would have the next pass ask for the same empty range forever.
    fn record_window_floor(
        &self,
        folder_id: FolderId,
        floor: Uid,
        complete: bool,
    ) -> Result<(), MailIndexError>;

    /// Applies arrivals, flag changes, the given deletions and the cursor as one transaction.
    ///
    /// The cursor is written inside that transaction and never before it: a cursor stored ahead of
    /// the data it describes makes the next pass skip changes that were never applied. Whether the
    /// pass stopped short of the folder's newest mail is the delta's to say, and is written with
    /// it.
    fn apply(
        &self,
        delta: &FolderDelta,
        deletions: &[Uid],
        cursor: &SyncCursor,
        flag_scope: FlagScope,
    ) -> Result<AppliedSummary, MailIndexError>;

    /// Drops every placement of the folder and stores the new generation.
    ///
    /// Called when a folder's UID space is reset. The messages themselves are kept: they still
    /// exist on the server and will be re-linked by the pass that follows.
    fn reset_folder(
        &self,
        folder_id: FolderId,
        uid_validity: UidValidity,
    ) -> Result<(), MailIndexError>;

    /// Takes back that the folder's last pass stopped short, without anything else changing.
    ///
    /// For a folder that failed on the pass after one that stopped short: that failing pass keeps
    /// what lost its last copy, because it may be past where the window ended, and it does so once.
    /// Left standing, a folder that then fails on every pass would keep every deleted message of
    /// the account, its body on disk and its words in every search, for good.
    fn forget_it_stopped_short(&self, folder_id: FolderId) -> Result<(), MailIndexError>;

    /// Writes down whether one reading of the folder stands refused for a moment: refused so, and
    /// no reading of that kind applied since.
    ///
    /// What the next reading needs to bound how long a moment may last. A server may say it of
    /// the same message on every pass, and taken at its word every time, the folder stops there
    /// for good: no new mail, no flag, no removal, nothing older. Refused that way again while
    /// this stands, the reading keeps what the server would say about the rest, as it does for any
    /// other refusal. Taken back by the next reading of that kind that is applied, and by the
    /// folder renumbering — and by nothing else: a reading answered and then not applied, or one
    /// that failed another way, leaves it standing.
    fn refused_for_now(
        &self,
        folder_id: FolderId,
        reading: Reading,
        refused: bool,
    ) -> Result<(), MailIndexError>;

    /// Whether the index holds a copy of this message in this folder.
    ///
    /// What a pass asks before it stops laying a move this device made over the index: the index
    /// has caught up with the move only once it agrees about the folder the message left. Asked
    /// by a row that has since handed its copies to the message it turned out to be, it is asked
    /// of that message.
    fn holds(&self, folder_id: FolderId, message: MessageId) -> Result<bool, MailIndexError>;

    /// The messages of an account that no folder holds any more.
    ///
    /// Asked by a pass before it reads anything, so that one which then cannot read every folder
    /// can tell what lost its last copy while it read from what had none before it began.
    fn unplaced(&self, account: i64) -> Result<Vec<MessageId>, MailIndexError>;

    /// Forgets the messages of an account that no folder holds any more, except those in `named`.
    ///
    /// Not done as each copy goes. A message moved from one folder to another is gone from the
    /// first before a pass has read it into the second; forgotten in between, it comes back under
    /// a new name, and every instruction and every way back naming the old one names nothing. A
    /// pass forgets them once, after it has been through every folder.
    ///
    /// `named` is what the queue still speaks about. A message it names is kept however long no
    /// folder holds it: an instruction waiting to be found again after a folder renumbered, or a
    /// move whose destination has not been read, finds it again under the same name only while
    /// the index still has that name. So is a message a row it names handed its copies to, which
    /// is the message the queue means by that row.
    fn forget_the_unplaced(&self, account: i64, named: &[MessageId]) -> Result<(), MailIndexError>;
}
