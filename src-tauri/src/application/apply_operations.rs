//! Emptying the queue: telling the server what a person asked for.
//!
//! The order here is the whole of the rule in one function. An instruction names a message and a
//! folder; where that message actually sits is looked up **now**, from what the last pass stored,
//! and used once. A position resolved yesterday, or written into the instruction when it was
//! asked for, would after a renumbering name a different message — and a `\Deleted` on the wrong
//! message is damage on the server, which nothing here can undo.
//!
//! A message that cannot be found is not resolved to the nearest one and not quietly dropped: the
//! instruction stops and is shown to the person, because only they can decide what it now means.

use crate::application::error::UseCaseError;
use crate::ports::mail_actions::{Acting, MailActionError, MailActions, ServerAction};
use crate::ports::op_log::{OperationId, OperationLog, OperationLogError, RecordedOperation};
use crate::ports::placement::{Placements, Position};
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::operation::{
    after_attempt, delete_mechanism, retry_after, when_the_message_cannot_be_found, Attempt,
    DeleteMechanism, OperationKind, OperationState, Stopped,
};

/// How many instructions one round carries out.
///
/// A round is a burst of requests to one server, and the queue is emptied a round at a time so a
/// morning of clicking does not become one enormous conversation that either finishes or fails
/// whole. Small enough to interleave with reading mail, large enough that a queue of a few dozen
/// clears in a pass or two.
pub const ROUND: usize = 20;

/// What one round did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    pub carried_out: u32,
    /// Left for later: the server could not be reached, or would not do it now.
    pub deferred: u32,
    /// Stopped and shown to the person: it cannot be done at all.
    pub conflicted: u32,
    /// Of what was carried out, how many moved a message or took it away.
    ///
    /// The ones a pass has to read back: the index still has each of them where it was, and only
    /// the server can say what number it has where it went.
    pub relocated: u32,
    /// How many instructions this round was handed, which is how the caller knows whether there
    /// may be more behind them.
    pub handed: u32,
    /// The round stopped before the queue did: the server could not be reached, or stopped
    /// answering. What it had not come to yet was given back untouched, and there is no point
    /// asking again until something has changed.
    pub stopped_short: bool,
}

impl Applied {
    /// Whether there may be more due than one round carries, and it is worth another round now.
    pub fn more(&self) -> bool {
        !self.stopped_short && self.handed as usize >= ROUND
    }
}

/// What became of one instruction, as far as the queue is concerned.
enum Answer {
    /// It was tried, and this is what it became.
    Became(OperationState, Option<Stopped>),
    /// The server heard it and answered "not now", or stopped answering altogether. Written down
    /// as waiting, and nothing more is said to the server this round: whatever the reason, the
    /// next instruction would meet it too.
    NotNow,
    /// It was taken back between being handed out and being told. The server heard nothing, and
    /// there is nothing to write down.
    Withdrawn,
    /// The server could not be reached at all, so nothing was said about this one or any after it.
    Unreachable,
}

/// The conversation a round acts in, opened by the first instruction that needs one.
///
/// Opened late on purpose: a round whose every instruction is settled without the server — a
/// message that is not there any more, a folder that went — never connects at all.
type Conversation<'a> = Option<Box<dyn Acting + 'a>>;

pub struct ApplyOperations<L, P, A>
where
    L: OperationLog,
    P: Placements,
    A: MailActions,
{
    operations: L,
    placements: P,
    actions: A,
}

impl<L, P, A> ApplyOperations<L, P, A>
where
    L: OperationLog,
    P: Placements,
    A: MailActions,
{
    pub fn new(operations: L, placements: P, actions: A) -> Self {
        Self {
            operations,
            placements,
            actions,
        }
    }

    /// `number` is what the index knows the account by, which is how a folder is found by what it
    /// is for.
    ///
    /// One round is up to [`ROUND`] instructions, asked for again until that many have been handed
    /// out or nothing more is due: the queue hands out only the first of each message's line, and
    /// what was behind one carried out a moment ago is due now, in the same round, down the same
    /// conversation.
    pub fn execute(&self, account_id: &str, number: i64) -> Result<Applied, UseCaseError> {
        let mut applied = Applied::default();
        let mut conversation: Conversation<'_> = None;

        'round: while (applied.handed as usize) < ROUND {
            let batch = self
                .operations
                .due(account_id, ROUND - applied.handed as usize)?;
            if batch.is_empty() {
                break;
            }

            for (at, waiting) in batch.iter().enumerate() {
                let answer = self.carry_out(account_id, number, waiting, &mut conversation)?;

                let (became, stopped) = match answer {
                    Answer::Withdrawn => {
                        applied.handed += 1;
                        continue;
                    }
                    // Nothing was said about this one or anything after it, so none of them is
                    // any further along — and none is counted against, or they would come back
                    // from a morning offline to an hour's wait each.
                    Answer::Unreachable => {
                        let untouched: Vec<_> = batch[at..].iter().map(|one| one.id).collect();
                        self.operations.release(&untouched)?;
                        applied.stopped_short = true;
                        break 'round;
                    }
                    Answer::NotNow => (OperationState::Pending, None),
                    Answer::Became(became, stopped) => (became, stopped),
                };

                applied.handed += 1;
                match became {
                    OperationState::Done => {
                        applied.carried_out += 1;
                        if !matches!(waiting.operation.kind, OperationKind::SetFlags { .. }) {
                            applied.relocated += 1;
                        }
                    }
                    OperationState::Conflicted => applied.conflicted += 1,
                    _ => applied.deferred += 1,
                }

                // Written down whatever happened. An instruction whose outcome is not recorded is
                // one the next round asks the server to do again.
                match self.operations.attempted(
                    waiting.id,
                    became,
                    stopped,
                    retry_after(waiting.attempts),
                ) {
                    Ok(()) => {}
                    // Taken back while it was being decided about, or gone with its account.
                    // Nothing is left to say it on, and that is not a reason to stop the round.
                    Err(OperationLogError::ChangedUnderneath) => {}
                    Err(error) => return Err(error.into()),
                }

                if matches!(answer, Answer::NotNow) {
                    let untouched: Vec<_> = batch[at + 1..].iter().map(|one| one.id).collect();
                    self.operations.release(&untouched)?;
                    applied.stopped_short = true;
                    break 'round;
                }
            }
        }

        Ok(applied)
    }

    /// Resolves one instruction and tells the server, answering with what it has become.
    fn carry_out<'a>(
        &'a self,
        account_id: &str,
        number: i64,
        waiting: &RecordedOperation,
        conversation: &mut Conversation<'a>,
    ) -> Result<Answer, UseCaseError> {
        let target = waiting.operation.target;

        // Looked up now, used once. This is the line the whole design is built around.
        let Some(position) = self
            .placements
            .where_it_sits(target.folder, target.message)?
        else {
            // Not where the instruction expects it, and what that means depends on whether the
            // server has already been told. Already told: this is what carrying it out looks like
            // from here, and the run that stopped before writing the outcome down is why nothing
            // says so. Calling that a conflict tells a person their instruction failed when it is
            // the one thing that certainly happened.
            if waiting.already_told {
                return Ok(Answer::Became(OperationState::Done, None));
            }

            // Never told, and this device moved it a moment ago: the server has it wherever the
            // earlier instruction sent it, and the index has not read that back yet. It waits for
            // the pass that does, which is what finding it again means. (Nothing still waiting
            // about the same message can be the reason: the queue hands out nothing behind it.)
            if self.operations.owes_a_read(account_id, target.message)? {
                return Ok(Answer::Became(OperationState::NeedsReresolve, None));
            }

            // Never told, and nothing here moved it: somebody else did, and only a person can
            // decide what that means.
            return Ok(Answer::Became(
                when_the_message_cannot_be_found(),
                Some(Stopped::MessageIsNotThere),
            ));
        };

        let Position {
            mailbox,
            uid,
            uid_validity,
        } = &position;

        match waiting.operation.kind {
            OperationKind::SetFlags { add, remove } => {
                let outcome = self.tell(
                    conversation,
                    waiting.id,
                    ServerAction::SetFlags {
                        mailbox,
                        uid: *uid,
                        uid_validity: *uid_validity,
                        add,
                        remove,
                    },
                )?;

                // What the server just said `OK` to is what it now holds, so it is written onto the
                // copy here, before the instruction is written off. The other order leaves a
                // moment where neither the queue nor the index says the message was read.
                //
                // An index that will not take it leaves the instruction waiting: telling a server
                // the same flag twice is harmless, and a flag the index never learns is one the
                // next pass may never read again.
                if matches!(outcome, Answer::Became(OperationState::Done, _))
                    && self
                        .placements
                        .confirmed_flags(target.folder, &position, add, remove)
                        .is_err()
                {
                    return Ok(Answer::Became(OperationState::Pending, None));
                }

                Ok(outcome)
            }

            OperationKind::MoveTo { folder } => {
                let Some(destination) = self.placements.mailbox_of(folder)? else {
                    // The folder it was to be filed in is not on this device any more. Nothing to
                    // guess at: the person is shown it.
                    return Ok(Answer::Became(
                        OperationState::Conflicted,
                        Some(Stopped::FolderIsNotThere),
                    ));
                };

                self.tell(
                    conversation,
                    waiting.id,
                    ServerAction::MoveTo {
                        mailbox,
                        uid: *uid,
                        uid_validity: *uid_validity,
                        destination: &destination,
                    },
                )
            }

            OperationKind::Delete { intent } => {
                let trash = self.placements.folder_for(number, FolderRole::Trash)?;

                // Decided against the account as it is now, not as it was when the person asked.
                // An account that has since lost its trash folder must not have "delete" quietly
                // become "delete for good".
                let Some(mechanism) = delete_mechanism(intent, trash) else {
                    return Ok(Answer::Became(
                        OperationState::Conflicted,
                        Some(Stopped::NowhereToKeepDeletedMail),
                    ));
                };

                match mechanism {
                    DeleteMechanism::MoveTo(folder) => {
                        let Some(destination) = self.placements.mailbox_of(folder)? else {
                            return Ok(Answer::Became(
                                OperationState::Conflicted,
                                Some(Stopped::FolderIsNotThere),
                            ));
                        };

                        self.tell(
                            conversation,
                            waiting.id,
                            ServerAction::MoveTo {
                                mailbox,
                                uid: *uid,
                                uid_validity: *uid_validity,
                                destination: &destination,
                            },
                        )
                    }
                    DeleteMechanism::MarkAndExpunge => self.tell(
                        conversation,
                        waiting.id,
                        ServerAction::Erase {
                            mailbox,
                            uid: *uid,
                            uid_validity: *uid_validity,
                        },
                    ),
                }
            }
        }
    }

    fn tell<'a>(
        &'a self,
        conversation: &mut Conversation<'a>,
        id: OperationId,
        action: ServerAction<'_>,
    ) -> Result<Answer, UseCaseError> {
        // The conversation before the note that the server is being told. Written down first, an
        // instruction the server never heard would count as sent — and something sent can no
        // longer be taken back or folded, so an undo pressed offline would have to wait for the
        // network to carry out the very thing it undoes.
        let acting = match conversation {
            Some(acting) => acting,
            None => match self.actions.reach() {
                Ok(acting) => conversation.insert(acting),
                Err(_) => return Ok(Answer::Unreachable),
            },
        };

        // Written down before the server hears anything, and committed on its own. Recorded after
        // the telling it would say nothing about the run that stopped in between, which is the
        // only run it exists for.
        //
        // Refused when the instruction was taken back since it was handed out. The server is then
        // told nothing: the person undid it, and a server told anyway would carry it out.
        match self.operations.about_to_tell(id) {
            Ok(()) => {}
            Err(OperationLogError::ChangedUnderneath) => return Ok(Answer::Withdrawn),
            Err(error) => return Err(error.into()),
        }

        let outcome = match acting.carry_out(action) {
            Ok(()) => Attempt::Carried,
            Err(MailActionError::NotNow) => return Ok(Answer::NotNow),
            Err(MailActionError::Refused) => Attempt::Impossible,
            // The address is stale, not the instruction. A pass over that folder rewrites what
            // this was resolved against, and the same instruction then addresses the message it
            // was always about. Showing a person a conflict here would be showing them ours.
            Err(MailActionError::FolderRenumbered) => Attempt::NotNow,
        };

        Ok(Answer::Became(
            after_attempt(outcome),
            // Only a stop has a reason. "Not now" is our problem rather than the person's, and
            // "done" has nothing to explain.
            matches!(outcome, Attempt::Impossible).then_some(Stopped::TheServerRefused),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;
    use crate::ports::op_log::OperationId;
    use lyonoyl_domain::message::{Flags, FolderId, MessageId, Uid, UidValidity};
    use lyonoyl_domain::operation::{DeleteIntent, MailOperation, OperationTarget};

    fn folder(value: u64) -> FolderId {
        FolderId::new(value).expect("non-zero")
    }

    /// The generation every placement in these tests was taken under.
    fn generation() -> UidValidity {
        UidValidity::new(7).expect("non-zero")
    }

    fn target(message: u64, in_folder: u64) -> OperationTarget {
        OperationTarget::new(
            MessageId::new(message).expect("non-zero"),
            folder(in_folder),
        )
    }

    fn waiting(kind: OperationKind, attempts: u32) -> RecordedOperation {
        RecordedOperation {
            id: OperationId(1),
            operation: MailOperation::asked(target(1, 1), kind).expect("a change was asked for"),
            attempts,
            stopped: None,
            already_told: false,
        }
    }

    /// The same, for a queue holding several: each under a number of its own, about a message of
    /// its own.
    fn numbered(id: i64, kind: OperationKind) -> RecordedOperation {
        RecordedOperation {
            id: OperationId(id),
            operation: MailOperation::asked(target(id as u64, 1), kind)
                .expect("a change was asked for"),
            attempts: 0,
            stopped: None,
            already_told: false,
        }
    }

    /// One the server has already been told about, and whose outcome was never written down.
    fn already_told(kind: OperationKind) -> RecordedOperation {
        RecordedOperation {
            already_told: true,
            ..waiting(kind, 1)
        }
    }

    fn read() -> OperationKind {
        OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        }
    }

    fn filed() -> OperationKind {
        OperationKind::MoveTo { folder: folder(2) }
    }

    fn thrown_away() -> OperationKind {
        OperationKind::Delete {
            intent: DeleteIntent::Permanently,
        }
    }

    /// One outcome as the queue was told about it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Outcome {
        id: OperationId,
        became: OperationState,
        stopped: Option<Stopped>,
        retry: Duration,
    }

    /// A queue holding whatever the test put in it, remembering every outcome written down.
    ///
    /// Each instruction is handed out once, as the real queue's hold makes sure of; what is in
    /// `behind` is handed out by the next asking, as the real queue hands out what waited behind
    /// an instruction carried out a moment ago.
    #[derive(Default)]
    struct Queue {
        holding: Mutex<Vec<RecordedOperation>>,
        behind: Mutex<Vec<RecordedOperation>>,
        released: Mutex<Vec<OperationId>>,
        written: Mutex<Vec<Outcome>>,
        asked_for: Mutex<Vec<usize>>,
        /// What happened and in what order, shared with the server fake so the order between the
        /// two can be asserted rather than assumed.
        happened: std::sync::Arc<Mutex<Vec<&'static str>>>,
        racing: bool,
        /// Whether something this device did to the message is still waiting to be read back.
        owes: bool,
        /// Whether the instruction was taken back after being handed out.
        withdrawn: bool,
    }

    impl Queue {
        fn holding(operations: Vec<RecordedOperation>) -> Self {
            Self {
                holding: Mutex::new(operations),
                ..Self::default()
            }
        }

        fn released(&self) -> Vec<OperationId> {
            self.released.lock().expect("test lock").clone()
        }

        fn written(&self) -> Vec<Outcome> {
            self.written.lock().expect("test lock").clone()
        }

        fn outcome(&self) -> Outcome {
            let written = self.written.lock().expect("test lock");
            assert_eq!(written.len(), 1, "expected exactly one outcome");
            written[0]
        }
    }

    impl OperationLog for Queue {
        fn cancel(
            &self,
            _account_id: &str,
            _target: OperationTarget,
        ) -> Result<usize, OperationLogError> {
            unreachable!("carrying something out does not take anything back")
        }

        fn try_again(
            &self,
            _account_id: &str,
            _target: OperationTarget,
        ) -> Result<usize, OperationLogError> {
            unreachable!("carrying something out does not put anything back")
        }

        fn due(
            &self,
            _account_id: &str,
            at_most: usize,
        ) -> Result<Vec<RecordedOperation>, OperationLogError> {
            self.asked_for.lock().expect("test lock").push(at_most);
            let handed = std::mem::take(&mut *self.holding.lock().expect("test lock"));
            if handed.is_empty() {
                return Ok(std::mem::take(&mut *self.behind.lock().expect("test lock")));
            }
            Ok(handed)
        }

        fn release(&self, ids: &[OperationId]) -> Result<(), OperationLogError> {
            self.released.lock().expect("test lock").extend(ids);
            Ok(())
        }
        fn about_to_tell(&self, _id: OperationId) -> Result<(), OperationLogError> {
            if self.withdrawn {
                return Err(OperationLogError::ChangedUnderneath);
            }
            self.happened.lock().expect("test lock").push("warned");
            Ok(())
        }

        fn attempted(
            &self,
            id: OperationId,
            became: OperationState,
            stopped: Option<Stopped>,
            retry: Duration,
        ) -> Result<(), OperationLogError> {
            self.happened
                .lock()
                .expect("test lock")
                .push("written down");
            self.written.lock().expect("test lock").push(Outcome {
                id,
                became,
                stopped,
                retry,
            });

            match self.racing {
                true => Err(OperationLogError::ChangedUnderneath),
                false => Ok(()),
            }
        }

        fn settled_for(
            &self,
            _account_id: &str,
            _target: OperationTarget,
        ) -> Result<Option<RecordedOperation>, OperationLogError> {
            Ok(None)
        }

        fn outstanding_for(
            &self,
            _account_id: &str,
            _target: OperationTarget,
        ) -> Result<Vec<RecordedOperation>, OperationLogError> {
            unreachable!("carrying something out folds nothing")
        }

        fn record(
            &self,
            _account_id: &str,
            _replacing: &[OperationId],
            _operation: Option<MailOperation>,
        ) -> Result<u64, OperationLogError> {
            unreachable!("carrying something out asks for nothing")
        }

        fn folder_reset(
            &self,
            _account_id: &str,
            _folder: FolderId,
        ) -> Result<usize, OperationLogError> {
            unreachable!("carrying something out resets nothing")
        }

        fn folder_found(
            &self,
            _account_id: &str,
            _folder: FolderId,
        ) -> Result<usize, OperationLogError> {
            unreachable!("carrying something out reads no folder")
        }

        fn owed_a_read(
            &self,
            _account_id: &str,
        ) -> Result<Vec<crate::ports::op_log::OwedRead>, OperationLogError> {
            unreachable!("carrying something out reads nothing back")
        }

        fn read_back(&self, _ids: &[OperationId]) -> Result<usize, OperationLogError> {
            unreachable!("carrying something out reads nothing back")
        }

        fn owes_a_read(
            &self,
            _account_id: &str,
            _message: MessageId,
        ) -> Result<bool, OperationLogError> {
            Ok(self.owes)
        }

        fn still_named(
            &self,
            _account_id: &str,
        ) -> Result<Vec<lyonoyl_domain::message::MessageId>, OperationLogError> {
            unreachable!("carrying out an instruction does not sweep the index")
        }

        fn folders_gone(
            &self,
            _account_id: &str,
            _still_there: &[lyonoyl_domain::message::FolderId],
        ) -> Result<usize, OperationLogError> {
            unreachable!("carrying out an instruction reads no folder")
        }
    }

    /// What the index says about where things are.
    struct Index {
        sitting: Option<Position>,
        names: Vec<(u64, &'static str)>,
        trash: Option<FolderId>,
        /// The flags written back, in the order they were, and on which copy.
        confirmed: Mutex<Vec<(FolderId, Uid, Flags, Flags)>>,
        /// Whether writing them back fails.
        will_not_take_it: bool,
        /// The same list the queue and the server write to, when a test watches the order.
        happened: std::sync::Arc<Mutex<Vec<&'static str>>>,
    }

    impl Default for Index {
        fn default() -> Self {
            Self {
                sitting: Some(Position {
                    mailbox: "INBOX".to_owned(),
                    uid: Uid::new(42).expect("non-zero"),
                    uid_validity: generation(),
                }),
                names: vec![(1, "INBOX"), (2, "Archive"), (4, "Trash")],
                trash: Some(folder(4)),
                confirmed: Mutex::new(Vec::new()),
                will_not_take_it: false,
                happened: std::sync::Arc::default(),
            }
        }
    }

    impl Placements for Index {
        fn where_it_sits(
            &self,
            _folder: FolderId,
            _message: MessageId,
        ) -> Result<Option<Position>, crate::ports::placement::PlacementError> {
            Ok(self.sitting.clone())
        }

        fn mailbox_of(
            &self,
            folder: FolderId,
        ) -> Result<Option<String>, crate::ports::placement::PlacementError> {
            Ok(self
                .names
                .iter()
                .find(|(id, _)| *id == folder.get())
                .map(|(_, name)| (*name).to_owned()))
        }

        fn folder_for(
            &self,
            _account: i64,
            role: FolderRole,
        ) -> Result<Option<FolderId>, crate::ports::placement::PlacementError> {
            Ok(match role {
                FolderRole::Trash => self.trash,
                _ => None,
            })
        }

        fn confirmed_flags(
            &self,
            folder: FolderId,
            at: &Position,
            add: Flags,
            remove: Flags,
        ) -> Result<bool, crate::ports::placement::PlacementError> {
            if self.will_not_take_it {
                return Err(crate::ports::placement::PlacementError::Unavailable);
            }
            self.happened
                .lock()
                .expect("test lock")
                .push("written back");
            self.confirmed
                .lock()
                .expect("test lock")
                .push((folder, at.uid, add, remove));
            Ok(true)
        }
    }

    /// A server that remembers what it was told, and answers as the test says.
    struct Server {
        told: Mutex<Vec<String>>,
        answer: Result<(), MailActionError>,
        /// The same list the queue writes to, when a test is watching the order.
        happened: std::sync::Arc<Mutex<Vec<&'static str>>>,
        /// How many conversations were opened with it.
        reached: Mutex<usize>,
        /// Whether a conversation can be opened at all.
        out_of_reach: bool,
    }

    impl Default for Server {
        fn default() -> Self {
            Self {
                told: Mutex::new(Vec::new()),
                answer: Ok(()),
                happened: std::sync::Arc::default(),
                reached: Mutex::new(0),
                out_of_reach: false,
            }
        }
    }

    impl Server {
        fn answering(answer: Result<(), MailActionError>) -> Self {
            Self {
                answer,
                ..Self::default()
            }
        }

        fn told(&self) -> Vec<String> {
            self.told.lock().expect("test lock").clone()
        }

        fn reached(&self) -> usize {
            *self.reached.lock().expect("test lock")
        }
    }

    impl MailActions for Server {
        fn reach(&self) -> Result<Box<dyn Acting + '_>, MailActionError> {
            if self.out_of_reach {
                return Err(MailActionError::NotNow);
            }
            self.happened.lock().expect("test lock").push("reached");
            *self.reached.lock().expect("test lock") += 1;
            Ok(Box::new(Talking(self)))
        }
    }

    /// One conversation with the server above.
    struct Talking<'a>(&'a Server);

    impl Acting for Talking<'_> {
        fn carry_out(&mut self, action: ServerAction<'_>) -> Result<(), MailActionError> {
            self.0.happened.lock().expect("test lock").push("told");
            self.0
                .told
                .lock()
                .expect("test lock")
                .push(format!("{action:?}"));

            self.0.answer
        }
    }

    fn round(queue: &Queue, index: &Index, server: &Server) -> Applied {
        ApplyOperations::new(queue, index, server)
            .execute("acc-1", 1)
            .expect("the round runs")
    }

    #[test]
    fn a_flag_reaches_the_server_at_the_position_looked_up_now() {
        let queue = Queue::holding(vec![waiting(read(), 0)]);
        let server = Server::default();

        let applied = round(&queue, &Index::default(), &server);

        assert_eq!(applied.carried_out, 1);
        assert_eq!(server.told().len(), 1);
        assert!(server.told()[0].contains("INBOX"), "{:?}", server.told());
        assert!(server.told()[0].contains("42"), "{:?}", server.told());
        assert_eq!(queue.outcome().became, OperationState::Done);
    }

    #[test]
    fn every_instruction_carries_the_generation_its_number_was_taken_under() {
        // A number on its own is not an address. What lets a server refuse to act on a folder that
        // has renumbered is being told which generation the number came from, and it can only be
        // told if the instruction carries it.
        for waiting_for in [
            waiting(read(), 0),
            waiting(filed(), 0),
            waiting(thrown_away(), 0),
        ] {
            let queue = Queue::holding(vec![waiting_for]);
            let server = Server::default();

            round(&queue, &Index::default(), &server);

            assert!(
                server.told()[0].contains("uid_validity"),
                "an instruction reached the server with no generation: {:?}",
                server.told()
            );
        }
    }

    #[test]
    fn a_folder_that_renumbered_is_asked_again_rather_than_shown_as_a_conflict() {
        // Ours to fix, not the person's to look at. The pass that reads the folder rewrites what
        // this was resolved against, and the same instruction then addresses the message it was
        // always about.
        let queue = Queue::holding(vec![waiting(read(), 0)]);
        let server = Server {
            answer: Err(MailActionError::FolderRenumbered),
            ..Server::default()
        };

        let applied = round(&queue, &Index::default(), &server);

        assert_eq!(applied.conflicted, 0, "a stale address is not a conflict");
        assert_eq!(queue.outcome().became, OperationState::Pending);
        assert_eq!(
            queue.outcome().stopped,
            None,
            "nothing to tell a person about an address of ours that went out of date"
        );
    }

    #[test]
    fn the_queue_hears_about_it_before_the_server_does() {
        // The whole of the mechanism is the order. Written after the telling it would say nothing
        // about the run that stopped in between, which is the only run it is for.
        let happened = std::sync::Arc::new(Mutex::new(Vec::new()));
        let queue = Queue {
            happened: happened.clone(),
            ..Queue::holding(vec![waiting(read(), 0)])
        };
        let server = Server {
            happened: happened.clone(),
            ..Server::default()
        };

        round(&queue, &Index::default(), &server);

        assert_eq!(
            *happened.lock().expect("test lock"),
            vec!["reached", "warned", "told", "written down"],
            "the server was told before anything was written down about it, or the instruction \
             was marked sent before there was anybody to send it to"
        );
    }

    #[test]
    fn one_the_server_already_heard_is_done_rather_than_a_conflict() {
        // The machine stopped between the server carrying it out and the outcome being written
        // down. The message is not where the instruction expects it because the instruction
        // worked, and telling the person it failed would be telling them the opposite of what
        // happened.
        let queue = Queue::holding(vec![already_told(read())]);
        let server = Server::default();
        let index = Index {
            sitting: None,
            ..Index::default()
        };

        let applied = round(&queue, &index, &server);

        assert_eq!(applied.conflicted, 0);
        assert_eq!(queue.outcome().became, OperationState::Done);
        assert_eq!(queue.outcome().stopped, None);
        assert!(
            server.told().is_empty(),
            "nothing needed telling: it had already been told"
        );
    }

    #[test]
    fn a_message_that_is_not_there_any_more_stops_and_is_shown() {
        // Never resolved to the nearest message, never dropped. It was there when somebody asked
        // and it is not there now, and only they can decide what that means.
        let queue = Queue::holding(vec![waiting(read(), 0)]);
        let server = Server::default();
        let index = Index {
            sitting: None,
            ..Index::default()
        };

        let applied = round(&queue, &index, &server);

        assert_eq!(applied.conflicted, 1);
        assert!(
            server.told().is_empty(),
            "the server was told about a message nobody could find"
        );
        assert_eq!(queue.outcome().became, OperationState::Conflicted);
    }

    #[test]
    fn filing_a_message_names_both_mailboxes() {
        let queue = Queue::holding(vec![waiting(
            OperationKind::MoveTo { folder: folder(2) },
            0,
        )]);
        let server = Server::default();

        round(&queue, &Index::default(), &server);

        assert!(server.told()[0].contains("INBOX"));
        assert!(server.told()[0].contains("Archive"), "{:?}", server.told());
    }

    #[test]
    fn a_destination_that_is_no_longer_here_stops_rather_than_guessing() {
        let queue = Queue::holding(vec![waiting(
            OperationKind::MoveTo { folder: folder(9) },
            0,
        )]);
        let server = Server::default();

        let applied = round(&queue, &Index::default(), &server);

        assert_eq!(applied.conflicted, 1);
        assert!(server.told().is_empty());
    }

    #[test]
    fn deleting_to_the_trash_is_carried_out_as_a_move_there() {
        let queue = Queue::holding(vec![waiting(
            OperationKind::Delete {
                intent: DeleteIntent::ToTrash,
            },
            0,
        )]);
        let server = Server::default();

        round(&queue, &Index::default(), &server);

        assert!(
            server.told()[0].starts_with("MoveTo"),
            "{:?}",
            server.told()
        );
        assert!(server.told()[0].contains("Trash"));
    }

    #[test]
    fn an_account_that_lost_its_trash_does_not_have_delete_become_permanent() {
        // The one case with no good answer, answered by stopping. A message somebody expected to
        // be able to get back must not quietly become one they cannot.
        let queue = Queue::holding(vec![waiting(
            OperationKind::Delete {
                intent: DeleteIntent::ToTrash,
            },
            0,
        )]);
        let server = Server::default();
        let index = Index {
            trash: None,
            ..Index::default()
        };

        let applied = round(&queue, &index, &server);

        assert_eq!(applied.conflicted, 1);
        assert!(
            server.told().is_empty(),
            "a message was erased when the person asked for the trash"
        );
    }

    #[test]
    fn deleting_for_good_is_carried_out_as_that_whatever_folders_exist() {
        let queue = Queue::holding(vec![waiting(
            OperationKind::Delete {
                intent: DeleteIntent::Permanently,
            },
            0,
        )]);
        let server = Server::default();

        round(&queue, &Index::default(), &server);

        assert!(server.told()[0].starts_with("Erase"), "{:?}", server.told());
    }

    #[test]
    fn a_server_that_could_not_be_reached_leaves_the_instruction_waiting() {
        let queue = Queue::holding(vec![waiting(read(), 0)]);
        let server = Server::answering(Err(MailActionError::NotNow));

        let applied = round(&queue, &Index::default(), &server);

        assert_eq!(applied.deferred, 1);
        assert_eq!(queue.outcome().became, OperationState::Pending);
    }

    #[test]
    fn a_server_that_refuses_outright_stops_the_instruction() {
        let queue = Queue::holding(vec![waiting(read(), 0)]);
        let server = Server::answering(Err(MailActionError::Refused));

        let applied = round(&queue, &Index::default(), &server);

        assert_eq!(applied.conflicted, 1);
        assert_eq!(queue.outcome().became, OperationState::Conflicted);
    }

    #[test]
    fn the_wait_grows_with_the_attempts_already_made() {
        // Otherwise a server that is down is asked every fifteen seconds for ever.
        let queue = Queue::holding(vec![waiting(read(), 3)]);
        let server = Server::answering(Err(MailActionError::NotNow));

        round(&queue, &Index::default(), &server);

        assert_eq!(queue.outcome().retry, retry_after(3));
        assert!(queue.outcome().retry > retry_after(0));
    }

    /// The four reasons, one test each: a table of boxed closures reads worse than what it saves.
    fn stopped_by(queue: &Queue, index: &Index, server: &Server) -> Option<Stopped> {
        round(queue, index, server);

        assert_eq!(queue.outcome().became, OperationState::Conflicted);
        queue.outcome().stopped
    }

    #[test]
    fn a_message_that_is_gone_says_that_rather_than_that_it_did_not_happen() {
        // Each reason leads somewhere different: look for the message, choose another folder, deal
        // with the trash you do not have, or take it up with the server. "It did not happen" leads
        // nowhere.
        let index = Index {
            sitting: None,
            ..Index::default()
        };

        assert_eq!(
            stopped_by(
                &Queue::holding(vec![waiting(read(), 0)]),
                &index,
                &Server::default()
            ),
            Some(Stopped::MessageIsNotThere)
        );
    }

    #[test]
    fn a_destination_that_is_gone_says_which_kind_of_gone() {
        let queue = Queue::holding(vec![waiting(
            OperationKind::MoveTo { folder: folder(9) },
            0,
        )]);

        assert_eq!(
            stopped_by(&queue, &Index::default(), &Server::default()),
            Some(Stopped::FolderIsNotThere)
        );
    }

    #[test]
    fn an_account_with_no_trash_says_so_in_those_words() {
        let queue = Queue::holding(vec![waiting(
            OperationKind::Delete {
                intent: DeleteIntent::ToTrash,
            },
            0,
        )]);
        let index = Index {
            trash: None,
            ..Index::default()
        };

        assert_eq!(
            stopped_by(&queue, &index, &Server::default()),
            Some(Stopped::NowhereToKeepDeletedMail)
        );
    }

    #[test]
    fn a_server_saying_no_is_reported_as_the_server_saying_no() {
        assert_eq!(
            stopped_by(
                &Queue::holding(vec![waiting(read(), 0)]),
                &Index::default(),
                &Server::answering(Err(MailActionError::Refused))
            ),
            Some(Stopped::TheServerRefused)
        );
    }

    #[test]
    fn what_happened_and_what_is_merely_waiting_carry_no_reason() {
        // A reason is for a person to act on. "Done" has nothing to explain, and "the server could
        // not be reached" is our problem rather than theirs.
        for (server, expected) in [
            (Server::default(), OperationState::Done),
            (
                Server::answering(Err(MailActionError::NotNow)),
                OperationState::Pending,
            ),
        ] {
            let queue = Queue::holding(vec![waiting(read(), 0)]);

            round(&queue, &Index::default(), &server);

            assert_eq!(queue.outcome().became, expected);
            assert_eq!(
                queue.outcome().stopped,
                None,
                "{expected:?} carried a reason"
            );
        }
    }

    #[test]
    fn one_round_is_a_burst_rather_than_the_whole_queue() {
        // A morning of clicking must not become one enormous conversation that either finishes or
        // fails whole.
        let queue = Queue::default();

        round(&queue, &Index::default(), &Server::default());

        assert_eq!(*queue.asked_for.lock().expect("test lock"), vec![ROUND]);
    }

    #[test]
    fn a_row_gone_by_the_time_its_outcome_is_written_does_not_stop_the_round() {
        // Its account was removed while the server was being told. What the server did stands;
        // there is simply no row left to say so on.
        let queue = Queue {
            racing: true,
            ..Queue::holding(vec![waiting(read(), 0)])
        };
        let server = Server::default();

        let applied = round(&queue, &Index::default(), &server);

        assert_eq!(applied.carried_out, 1);
    }

    #[test]
    fn a_flag_the_server_confirmed_is_written_onto_the_copy_before_it_is_written_off() {
        // Written off first, there would be a moment where neither the queue nor the index says
        // the message was read -- and a list drawn in that moment puts the dot back.
        let happened = std::sync::Arc::new(Mutex::new(Vec::new()));
        let queue = Queue {
            happened: happened.clone(),
            ..Queue::holding(vec![waiting(read(), 0)])
        };
        let index = Index {
            happened: happened.clone(),
            ..Index::default()
        };
        let server = Server {
            happened: happened.clone(),
            ..Server::default()
        };

        round(&queue, &index, &server);

        assert_eq!(
            *happened.lock().expect("test lock"),
            vec!["reached", "warned", "told", "written back", "written down"]
        );
        assert_eq!(
            *index.confirmed.lock().expect("test lock"),
            vec![(
                folder(1),
                Uid::new(42).expect("non-zero"),
                Flags::SEEN,
                Flags::NONE
            )],
            "on the copy it was set on, at the number it was sent to"
        );
        assert_eq!(queue.outcome().became, OperationState::Done);
    }

    #[test]
    fn nothing_the_server_did_not_confirm_is_written_back() {
        for answer in [
            Err(MailActionError::NotNow),
            Err(MailActionError::Refused),
            Err(MailActionError::FolderRenumbered),
        ] {
            let queue = Queue::holding(vec![waiting(read(), 0)]);
            let index = Index::default();

            round(&queue, &index, &Server::answering(answer));

            assert!(
                index.confirmed.lock().expect("test lock").is_empty(),
                "written back on {answer:?}"
            );
        }
    }

    #[test]
    fn a_flag_the_index_will_not_take_is_told_again_rather_than_forgotten() {
        // A flag the server holds and the index does not is a flag the next pass may never read
        // again: a server that tracks changes reports only what moved since last time.
        let queue = Queue::holding(vec![waiting(read(), 0)]);
        let index = Index {
            will_not_take_it: true,
            ..Index::default()
        };

        let applied = round(&queue, &index, &Server::default());

        assert_eq!(queue.outcome().became, OperationState::Pending);
        assert_eq!(queue.outcome().stopped, None);
        assert_eq!(applied.carried_out, 0);
    }

    #[test]
    fn a_move_is_not_written_back_and_is_counted_for_the_read_that_does() {
        let queue = Queue::holding(vec![waiting(filed(), 0), waiting(thrown_away(), 0)]);
        let index = Index::default();

        let applied = round(&queue, &index, &Server::default());

        assert!(index.confirmed.lock().expect("test lock").is_empty());
        assert_eq!(applied.relocated, 2);
        assert_eq!(applied.carried_out, 2);
    }

    #[test]
    fn a_flag_is_not_counted_as_moving_anything() {
        let queue = Queue::holding(vec![waiting(read(), 0)]);

        let applied = round(&queue, &Index::default(), &Server::default());

        assert_eq!(applied.relocated, 0);
    }

    #[test]
    fn an_instruction_taken_back_after_it_was_handed_out_is_not_told() {
        // The person undid it while the round was on its way to the server. Telling the server
        // anyway carries out the one thing they took back, and nothing would ever say so.
        let queue = Queue {
            withdrawn: true,
            ..Queue::holding(vec![waiting(filed(), 0)])
        };
        let server = Server::default();

        let applied = round(&queue, &Index::default(), &server);

        assert!(server.told().is_empty());
        assert!(queue.written.lock().expect("test lock").is_empty());
        assert_eq!(
            applied,
            Applied {
                handed: 1,
                ..Applied::default()
            }
        );
    }

    #[test]
    fn a_message_this_device_moved_a_moment_ago_is_waited_for_rather_than_stopped() {
        // Undo straight after archiving: the way back names the archive, and the index has not
        // read the message into it yet. Nobody else moved it, so there is nothing for a person to
        // look at -- it waits for the pass that reads it back.
        let queue = Queue {
            owes: true,
            ..Queue::holding(vec![waiting(filed(), 0)])
        };
        let server = Server::default();
        let index = Index {
            sitting: None,
            ..Index::default()
        };

        let applied = round(&queue, &index, &server);

        assert_eq!(queue.outcome().became, OperationState::NeedsReresolve);
        assert_eq!(queue.outcome().stopped, None);
        assert_eq!(applied.conflicted, 0);
        assert!(server.told().is_empty());
    }

    #[test]
    fn a_full_round_says_so() {
        let queue = Queue::holding(vec![waiting(read(), 0); ROUND]);

        let applied = round(&queue, &Index::default(), &Server::default());

        assert_eq!(applied.handed, ROUND as u32);
    }

    #[test]
    fn a_server_out_of_reach_is_told_nothing_and_nothing_is_counted_against_what_waits() {
        // Offline. Nothing may be marked sent -- something sent can no longer be taken back, and
        // an undo pressed now would have to wait for the network to carry out what it undoes --
        // and nothing may be charged an attempt, or a morning offline comes back as an hour's
        // wait on every change made in it.
        let happened = std::sync::Arc::new(Mutex::new(Vec::new()));
        let queue = Queue {
            happened: happened.clone(),
            ..Queue::holding(vec![numbered(1, filed()), numbered(2, read())])
        };
        let server = Server {
            out_of_reach: true,
            ..Server::default()
        };

        let applied = round(&queue, &Index::default(), &server);

        assert!(
            happened.lock().expect("test lock").is_empty(),
            "something was marked sent or written down: {:?}",
            happened.lock().expect("test lock")
        );
        assert!(queue.written().is_empty());
        assert_eq!(queue.released(), vec![OperationId(1), OperationId(2)]);
        assert!(applied.stopped_short);
        assert!(
            !applied.more(),
            "a server out of reach is not asked again at once"
        );
        assert_eq!(applied.handed, 0);
    }

    #[test]
    fn not_now_from_the_server_stops_the_round_and_gives_back_the_rest() {
        // The one that met it is charged its attempt. The rest were never said and are not.
        let queue = Queue::holding(vec![
            numbered(1, read()),
            numbered(2, read()),
            numbered(3, filed()),
        ]);
        let server = Server::answering(Err(MailActionError::NotNow));

        let applied = round(&queue, &Index::default(), &server);

        assert_eq!(server.told().len(), 1);
        assert_eq!(queue.outcome().id, OperationId(1));
        assert_eq!(queue.outcome().became, OperationState::Pending);
        assert_eq!(queue.released(), vec![OperationId(2), OperationId(3)]);
        assert!(applied.stopped_short);
    }

    #[test]
    fn a_whole_round_is_one_conversation() {
        let queue = Queue::holding(vec![
            numbered(1, read()),
            numbered(2, filed()),
            numbered(3, thrown_away()),
        ]);
        let server = Server::default();

        let applied = round(&queue, &Index::default(), &server);

        assert_eq!(applied.carried_out, 3);
        assert_eq!(server.told().len(), 3);
        assert_eq!(
            server.reached(),
            1,
            "a connection and a sign-in per message"
        );
    }

    #[test]
    fn a_round_settled_without_the_server_opens_no_conversation() {
        let queue = Queue::holding(vec![numbered(1, read()), numbered(2, filed())]);
        let server = Server::default();
        let index = Index {
            sitting: None,
            ..Index::default()
        };

        round(&queue, &index, &server);

        assert_eq!(server.reached(), 0);
    }

    #[test]
    fn what_waited_behind_an_instruction_carried_out_goes_in_the_same_round() {
        // Archive, then undo, both before the push went: the queue hands out the way back only
        // once the archiving is through, and asking again is how it goes now rather than a round
        // later.
        let queue = Queue {
            behind: Mutex::new(vec![numbered(2, read())]),
            ..Queue::holding(vec![numbered(1, filed())])
        };
        let server = Server::default();

        let applied = round(&queue, &Index::default(), &server);

        assert_eq!(applied.carried_out, 2);
        assert_eq!(server.reached(), 1);
        assert_eq!(
            *queue.asked_for.lock().expect("test lock"),
            vec![ROUND, ROUND - 1, ROUND - 2]
        );
    }

    #[test]
    fn a_full_round_says_there_may_be_more() {
        let queue = Queue::holding((1..=ROUND as i64).map(|id| numbered(id, read())).collect());

        let applied = round(&queue, &Index::default(), &Server::default());

        assert_eq!(applied.handed as usize, ROUND);
        assert!(applied.more());
    }
}
