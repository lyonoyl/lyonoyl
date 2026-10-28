//! What someone asked to happen to a message, kept until the server has been told.
//!
//! A port because the deciding and the keeping are different jobs. What may be folded into what is
//! a rule about mail, and it lives in the domain; where the result is written down, and how one
//! writer is kept from racing another, is a question about storage.
//!
//! Nothing here can name a position in a mailbox. The trait speaks in [`OperationTarget`], which
//! holds a message and a folder and no `Uid` at all.

use std::time::Duration;

use lyonoyl_domain::message::{FolderId, MessageId};
use lyonoyl_domain::operation::{MailOperation, OperationState, OperationTarget, Stopped};
use thiserror::Error;

/// The log's own name for one recorded operation.
///
/// Opaque on purpose: it exists so that an operation folded into another can be pointed at and
/// removed, and for nothing else. Nothing above the port takes it apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationId(pub i64);

/// One operation as the log holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedOperation {
    pub id: OperationId,
    pub operation: MailOperation,
    /// How many times the server has been asked and not answered. What turns a retry into a
    /// conflict rather than into a loop.
    pub attempts: u32,
    /// Why it stopped, for one that has. Nothing for one still on its way.
    pub stopped: Option<Stopped>,
    /// Whether the server has already been told about this one at least once.
    ///
    /// Written before the telling, so it survives a machine that stops in the moment between the
    /// server carrying the instruction out and the outcome being written down. What it decides is
    /// the answer to a message that is no longer where the instruction expects it: never told
    /// means somebody else moved it and a person has to look; already told means this is what
    /// being carried out looks like from here.
    pub already_told: bool,
}

/// Where an instruction the server carried out sent its message, for the pass that reads it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WentTo {
    /// Into another folder of the same account.
    Folder(FolderId),
    /// Into whatever the account keeps deleted mail in.
    Trash,
    /// Nowhere: it was deleted for good.
    Nowhere,
}

/// A move or a delete the server has carried out and no pass has read back yet.
///
/// The index still has the message where it was, because what the index holds is what a pass
/// read. Until a pass has read both ends, this is the one thing that says where the message went,
/// and every view lays it over the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwedRead {
    pub id: OperationId,
    pub message: MessageId,
    pub from: FolderId,
    pub to: WentTo,
    /// Carried out longer ago than a done instruction is kept at all. The index has had every
    /// pass since to catch up, and what it says by now is what the server says.
    pub overdue: bool,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum OperationLogError {
    #[error("the profile database is unavailable")]
    Unavailable,
    #[error("the operation log holds something this build cannot read")]
    Corrupted,
    /// Something else wrote first. The caller reads again and folds again rather than overwriting:
    /// what it folded was a picture of a moment that has passed.
    #[error("the operation log changed while this was being decided")]
    ChangedUnderneath,
}

pub trait OperationLog: Send + Sync {
    /// What is due to be carried out for one account, oldest first.
    ///
    /// Only what is waiting *and* whose next attempt has come round: something that failed a
    /// minute ago is not tried again this second. Oldest first because the order things were asked
    /// in is the order they make sense in — a message filed and then flagged must not be flagged
    /// where it no longer is.
    ///
    /// And only the first of each message's line. An instruction behind another about the same
    /// message is not handed out while the one before it is still waiting, whatever its own wait
    /// says: told in any other order, a way back reaches the server before what it undoes, and the
    /// server ends up holding the person's first word rather than their last.
    fn due(
        &self,
        account_id: &str,
        at_most: usize,
    ) -> Result<Vec<RecordedOperation>, OperationLogError>;

    /// Records what happened when the server was told.
    ///
    /// `retry_after` is how long to wait before this one is due again, and is only meaningful for
    /// something still waiting. Anything else has nothing left to wait for.
    fn attempted(
        &self,
        id: OperationId,
        became: OperationState,
        stopped: Option<Stopped>,
        retry_after: Duration,
    ) -> Result<(), OperationLogError>;

    /// Writes down that the server is about to be told, before it is.
    ///
    /// The order is the whole of it. Recorded after the telling, it records nothing about the run
    /// that stopped in between — which is the run this exists for.
    ///
    /// `ChangedUnderneath` when the instruction is no longer waiting to be told: taken back, or
    /// folded away, between being handed out and this. The server is then told nothing — telling
    /// it would carry out something the person has already undone.
    fn about_to_tell(&self, id: OperationId) -> Result<(), OperationLogError>;

    /// Gives back what a round was handed and never told the server, due again at once.
    ///
    /// What a round does with the rest of what it holds when it stops early: the server could not
    /// be reached, or stopped answering. Nothing was said about these, so nothing is counted
    /// against them — a machine offline for a morning does not come back to an hour's wait on
    /// every change made while it was. Only what is still waiting is given back.
    fn release(&self, ids: &[OperationId]) -> Result<(), OperationLogError>;

    /// Takes back what was asked for about one message, while it is still only asked for.
    ///
    /// What "undo" means before a server has heard: the instruction never happened, and the
    /// message goes back to being what the index says it is. Only what has not been sent —
    /// something the server has done, or is being told right now, is undone by asking for the
    /// opposite, which is a new instruction and not the absence of an old one. Answers with how
    /// many were taken back.
    fn cancel(&self, account_id: &str, target: OperationTarget)
        -> Result<usize, OperationLogError>;

    /// Puts what stopped about one message back in the queue, to be tried again.
    ///
    /// From the beginning: the attempt count and the wait are cleared, because a person asking
    /// again is a new decision rather than the next retry of an old one — and what stopped it may
    /// well have been fixed in between. Answers with how many were put back.
    fn try_again(
        &self,
        account_id: &str,
        target: OperationTarget,
    ) -> Result<usize, OperationLogError>;

    /// Sends everything waiting in one folder back to be found again.
    ///
    /// Called when a folder's `UIDVALIDITY` changed: every position in it now names a different
    /// message, so nothing waiting may be carried out until a pass has found the messages again.
    /// Answers with how many were moved, which is a number worth logging and nothing more.
    fn folder_reset(&self, account_id: &str, folder: FolderId) -> Result<usize, OperationLogError>;

    /// Puts what was sent back to be found again into the queue, a pass having found it.
    ///
    /// The other half of `folder_reset`, and without it the first half is a hole: an instruction
    /// sent back waits for a pass to find the message, and a pass that never says so leaves it
    /// waiting for the life of the profile. What a pass has just written *is* where the messages
    /// are, so a pass that read the folder without throwing its numbering away is the answer.
    fn folder_found(&self, account_id: &str, folder: FolderId) -> Result<usize, OperationLogError>;

    /// What is still outstanding for one message in one folder.
    ///
    /// Only what the server has yet to be told. What has happened, what is being told right now,
    /// and what a person has to look at are not things to fold into a new instruction: the server
    /// may already have done the first half of the fold.
    fn outstanding_for(
        &self,
        account_id: &str,
        target: OperationTarget,
    ) -> Result<Vec<RecordedOperation>, OperationLogError>;

    /// The last thing the server was actually told about this message, if it still remembers.
    ///
    /// What a way back is built from. While an instruction is waiting, taking it back is removing
    /// it; once it has gone through there is nothing to remove, and putting the message back is a
    /// second instruction — which has to know what the first one did.
    ///
    /// An instruction the server is being told about right now counts: whatever it answers, the
    /// way back is the opposite, asked for behind it.
    ///
    /// An instruction that stopped counts as well, when it is the newest: the caller has to see
    /// that what it would be undoing did not happen, rather than be handed an older instruction
    /// about the same message and undo that instead.
    ///
    /// Nothing once it has been forgotten, which is an honest answer: a filing from last week is
    /// not something anybody is undoing from a line under the list.
    fn settled_for(
        &self,
        account_id: &str,
        target: OperationTarget,
    ) -> Result<Option<RecordedOperation>, OperationLogError>;

    /// What the server has carried out for one account and no pass has read back yet.
    fn owed_a_read(&self, account_id: &str) -> Result<Vec<OwedRead>, OperationLogError>;

    /// Settles what a pass has now read back, so it stops being laid over the index.
    ///
    /// Answers with how many were settled. Only what is owed: a row that has since been settled
    /// by another pass, or forgotten, is left as it is.
    fn read_back(&self, ids: &[OperationId]) -> Result<usize, OperationLogError>;

    /// Every message one account's queue still speaks about: waiting, sent back to be found
    /// again, stopped for a person, or carried out and not read back.
    ///
    /// What a pass keeps in the index however long no folder holds it. Each of these is found
    /// again, or shown, under the name it has now, and a message forgotten and read in again comes
    /// back under a new one.
    fn still_named(&self, account_id: &str) -> Result<Vec<MessageId>, OperationLogError>;

    /// Puts back in the queue what waits to be found again in a folder the account no longer has.
    ///
    /// `still_there` is every folder the account's server listed. A folder renamed or removed on
    /// another device is never read again, so nothing waiting for it to be read will ever be let
    /// go by `folder_found` — and while it waits, everything asked after it about the same
    /// message waits behind it. Back in the queue, it is decided about the way anything is when
    /// its message is not where it expects: carried out, or stopped and shown to a person.
    fn folders_gone(
        &self,
        account_id: &str,
        still_there: &[FolderId],
    ) -> Result<usize, OperationLogError>;

    /// Whether something the server did to this message is still waiting to be read back.
    ///
    /// What tells an instruction that cannot find its message because this device moved it a
    /// moment ago from one that cannot find it because somebody else did.
    fn owes_a_read(&self, account_id: &str, message: MessageId) -> Result<bool, OperationLogError>;

    /// Writes one operation down and removes the ones it was folded from, together.
    ///
    /// One call because it is one change. Removing first and writing after leaves a moment where
    /// the person's instruction exists nowhere — and a crash in that moment loses it silently,
    /// which is the one thing an authoritative store may not do. `replacing` may be empty.
    ///
    /// `None` for the operation means the fold cancelled out: what is left is the removal, and the
    /// server is told nothing.
    ///
    /// Answers with where this write stands among everything asked for on this device: one more
    /// than the write before it, a cancelling one included. A read of the queue that says how far
    /// it had got holds this write exactly when that number is at least this one.
    fn record(
        &self,
        account_id: &str,
        replacing: &[OperationId],
        operation: Option<MailOperation>,
    ) -> Result<u64, OperationLogError>;
}

impl<L: OperationLog + ?Sized> OperationLog for &L {
    fn due(
        &self,
        account_id: &str,
        at_most: usize,
    ) -> Result<Vec<RecordedOperation>, OperationLogError> {
        (**self).due(account_id, at_most)
    }

    fn about_to_tell(&self, id: OperationId) -> Result<(), OperationLogError> {
        (**self).about_to_tell(id)
    }

    fn release(&self, ids: &[OperationId]) -> Result<(), OperationLogError> {
        (**self).release(ids)
    }

    fn attempted(
        &self,
        id: OperationId,
        became: OperationState,
        stopped: Option<Stopped>,
        retry_after: Duration,
    ) -> Result<(), OperationLogError> {
        (**self).attempted(id, became, stopped, retry_after)
    }

    fn cancel(
        &self,
        account_id: &str,
        target: OperationTarget,
    ) -> Result<usize, OperationLogError> {
        (**self).cancel(account_id, target)
    }

    fn try_again(
        &self,
        account_id: &str,
        target: OperationTarget,
    ) -> Result<usize, OperationLogError> {
        (**self).try_again(account_id, target)
    }

    fn folder_reset(&self, account_id: &str, folder: FolderId) -> Result<usize, OperationLogError> {
        (**self).folder_reset(account_id, folder)
    }

    fn folder_found(&self, account_id: &str, folder: FolderId) -> Result<usize, OperationLogError> {
        (**self).folder_found(account_id, folder)
    }

    fn outstanding_for(
        &self,
        account_id: &str,
        target: OperationTarget,
    ) -> Result<Vec<RecordedOperation>, OperationLogError> {
        (**self).outstanding_for(account_id, target)
    }

    fn settled_for(
        &self,
        account_id: &str,
        target: OperationTarget,
    ) -> Result<Option<RecordedOperation>, OperationLogError> {
        (**self).settled_for(account_id, target)
    }

    fn owed_a_read(&self, account_id: &str) -> Result<Vec<OwedRead>, OperationLogError> {
        (**self).owed_a_read(account_id)
    }

    fn read_back(&self, ids: &[OperationId]) -> Result<usize, OperationLogError> {
        (**self).read_back(ids)
    }

    fn owes_a_read(&self, account_id: &str, message: MessageId) -> Result<bool, OperationLogError> {
        (**self).owes_a_read(account_id, message)
    }

    fn still_named(&self, account_id: &str) -> Result<Vec<MessageId>, OperationLogError> {
        (**self).still_named(account_id)
    }

    fn folders_gone(
        &self,
        account_id: &str,
        still_there: &[FolderId],
    ) -> Result<usize, OperationLogError> {
        (**self).folders_gone(account_id, still_there)
    }

    fn record(
        &self,
        account_id: &str,
        replacing: &[OperationId],
        operation: Option<MailOperation>,
    ) -> Result<u64, OperationLogError> {
        (**self).record(account_id, replacing, operation)
    }
}
