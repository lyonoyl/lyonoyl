//! Acting on a message: marking it read, filing it, throwing it away.
//!
//! Nothing here talks to a server. What a person asked for is written down, the list is free to
//! show the answer at once, and the telling happens later — which is what makes any of this work
//! with no network, and what makes it survive a restart on the way.
//!
//! The whole job is: fold what was just asked into what is already waiting, and write the result.
//! What may fold into what is a rule about mail and lives in the domain; this puts the pieces
//! together and makes sure the folding and the writing are one change and not two.

use crate::application::error::UseCaseError;
use crate::ports::op_log::{OperationLog, OperationLogError};
use lyonoyl_domain::message::{Flags, FolderId};
use lyonoyl_domain::operation::{
    coalesced, DeleteIntent, MailOperation, OperationKind, OperationTarget,
};

/// How many times a fold is attempted before giving up.
///
/// Reading the queue and writing the result are two steps, and something else may write between
/// them. That is not a failure — it is a picture of a moment that passed — so the answer is to
/// look again. A small number, because with one person clicking, losing twice already means
/// something is wrong rather than busy.
const ATTEMPTS: usize = 3;

pub struct ActOnMail<L: OperationLog> {
    operations: L,
}

impl<L: OperationLog> ActOnMail<L> {
    pub fn new(operations: L) -> Self {
        Self { operations }
    }

    /// Marks a message read, or unread.
    ///
    /// Answers with where the write stands among everything asked for here, which is what a window
    /// holding the change compares a read of the counts against.
    pub fn set_read(
        &self,
        account_id: &str,
        target: OperationTarget,
        read: bool,
    ) -> Result<u64, UseCaseError> {
        let kind = match read {
            true => OperationKind::SetFlags {
                add: Flags::SEEN,
                remove: Flags::NONE,
            },
            false => OperationKind::SetFlags {
                add: Flags::NONE,
                remove: Flags::SEEN,
            },
        };

        self.ask(account_id, target, kind)
    }

    /// Files a message in another folder.
    pub fn move_to(
        &self,
        account_id: &str,
        target: OperationTarget,
        folder: FolderId,
    ) -> Result<(), UseCaseError> {
        self.ask(account_id, target, OperationKind::MoveTo { folder })
            .map(|_| ())
    }

    /// Throws a message away, in the sense the person meant.
    pub fn delete(
        &self,
        account_id: &str,
        target: OperationTarget,
        intent: DeleteIntent,
    ) -> Result<(), UseCaseError> {
        self.ask(account_id, target, OperationKind::Delete { intent })
            .map(|_| ())
    }

    /// Asks for something already decided elsewhere.
    ///
    /// What putting a message back needs: the rule for *what* undoes what lives in the domain, and
    /// the folding and the writing are the same as for anything anybody asks for by hand.
    pub fn ask_for(
        &self,
        account_id: &str,
        target: OperationTarget,
        kind: OperationKind,
    ) -> Result<(), UseCaseError> {
        self.ask(account_id, target, kind).map(|_| ())
    }

    /// Folds one instruction into what is already waiting, and writes the result.
    ///
    /// Answers with where the write stands among everything asked for here, and with nought where
    /// nothing was written: every reading of the queue already holds nothing.
    fn ask(
        &self,
        account_id: &str,
        target: OperationTarget,
        kind: OperationKind,
    ) -> Result<u64, UseCaseError> {
        // An instruction that says nothing is not written down: a queue full of "do nothing" is a
        // server kept busy for nothing and a person waiting on it.
        let Some(asked) = MailOperation::asked(target, kind) else {
            return Ok(0);
        };

        let mut left = ATTEMPTS;

        loop {
            let waiting = self.operations.outstanding_for(account_id, target)?;

            // Folded newest-last: the queue is in the order things were asked for, and the thing
            // just asked for is the latest word.
            let mut folded_into = Vec::new();
            let mut operation = asked;

            for earlier in &waiting {
                match coalesced(earlier.operation, operation) {
                    Some(one) => {
                        folded_into.push(earlier.id);
                        operation = one;
                    }
                    // What will not fold stays where it is, in the order it was asked for.
                    None => continue,
                }
            }

            match self
                .operations
                .record(account_id, &folded_into, said(operation))
            {
                Ok(written) => return Ok(written),
                Err(OperationLogError::ChangedUnderneath) => {
                    left -= 1;
                    if left == 0 {
                        return Err(UseCaseError::OperationRaced);
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

/// The operation, unless folding left it with nothing to tell the server.
///
/// A pair that cancelled out is not an operation to carry out; what is left of it is the removal
/// of the ones it replaced, which the caller does in the same breath.
fn said(operation: MailOperation) -> Option<MailOperation> {
    match operation.kind {
        OperationKind::SetFlags { add, remove } if add.is_empty() && remove.is_empty() => None,
        _ => Some(operation),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::ports::op_log::{OperationId, RecordedOperation};
    use lyonoyl_domain::message::MessageId;
    use lyonoyl_domain::operation::OperationState;

    fn target(message: u64, folder: u64) -> OperationTarget {
        OperationTarget::new(
            MessageId::new(message).expect("non-zero"),
            FolderId::new(folder).expect("non-zero"),
        )
    }

    /// What one call to `record` was told to do.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Written {
        replacing: Vec<OperationId>,
        operation: Option<MailOperation>,
    }

    /// A log that keeps what it holds and remembers what it was told.
    #[derive(Default)]
    struct FakeLog {
        holding: Mutex<Vec<RecordedOperation>>,
        written: Mutex<Vec<Written>>,
        /// How many times `record` refuses before letting a write through, so the retry can be
        /// watched rather than assumed.
        races: Mutex<usize>,
    }

    impl FakeLog {
        fn holding(operations: Vec<MailOperation>) -> Self {
            Self {
                holding: Mutex::new(
                    operations
                        .into_iter()
                        .enumerate()
                        .map(|(at, operation)| RecordedOperation {
                            id: OperationId(at as i64 + 1),
                            operation,
                            attempts: 0,
                            stopped: None,
                            already_told: false,
                        })
                        .collect(),
                ),
                ..Self::default()
            }
        }

        fn racing(times: usize) -> Self {
            Self {
                races: Mutex::new(times),
                ..Self::default()
            }
        }

        fn written(&self) -> Vec<Written> {
            self.written.lock().expect("test lock").clone()
        }

        fn only_write(&self) -> Written {
            let written = self.written();
            assert_eq!(written.len(), 1, "expected exactly one write");
            written[0].clone()
        }
    }

    impl OperationLog for FakeLog {
        fn cancel(
            &self,
            _account_id: &str,
            _target: OperationTarget,
        ) -> Result<usize, OperationLogError> {
            unreachable!("asking for something does not take anything back")
        }

        fn try_again(
            &self,
            _account_id: &str,
            _target: OperationTarget,
        ) -> Result<usize, OperationLogError> {
            unreachable!("asking for something does not put anything back")
        }

        fn due(
            &self,
            _account_id: &str,
            _at_most: usize,
        ) -> Result<Vec<RecordedOperation>, OperationLogError> {
            unreachable!("asking for something does not carry anything out")
        }

        fn about_to_tell(&self, _id: OperationId) -> Result<(), OperationLogError> {
            Ok(())
        }

        fn release(&self, _ids: &[OperationId]) -> Result<(), OperationLogError> {
            unreachable!("asking for something hands nothing out to give back")
        }

        fn attempted(
            &self,
            _id: OperationId,
            _became: lyonoyl_domain::operation::OperationState,
            _stopped: Option<lyonoyl_domain::operation::Stopped>,
            _retry_after: std::time::Duration,
        ) -> Result<(), OperationLogError> {
            unreachable!("asking for something does not tell a server about it")
        }

        fn folder_reset(
            &self,
            _account_id: &str,
            _folder: FolderId,
        ) -> Result<usize, OperationLogError> {
            unreachable!("asking for something does not reset a folder")
        }

        fn folder_found(
            &self,
            _account_id: &str,
            _folder: FolderId,
        ) -> Result<usize, OperationLogError> {
            unreachable!("asking for something does not read a folder")
        }

        fn owed_a_read(
            &self,
            _account_id: &str,
        ) -> Result<Vec<crate::ports::op_log::OwedRead>, OperationLogError> {
            unreachable!("asking for something reads nothing back")
        }

        fn read_back(&self, _ids: &[OperationId]) -> Result<usize, OperationLogError> {
            unreachable!("asking for something reads nothing back")
        }

        fn owes_a_read(
            &self,
            _account_id: &str,
            _message: lyonoyl_domain::message::MessageId,
        ) -> Result<bool, OperationLogError> {
            unreachable!("asking for something does not look for the message")
        }

        fn still_named(
            &self,
            _account_id: &str,
        ) -> Result<Vec<lyonoyl_domain::message::MessageId>, OperationLogError> {
            unreachable!("asking for something does not sweep the index")
        }

        fn folders_gone(
            &self,
            _account_id: &str,
            _still_there: &[lyonoyl_domain::message::FolderId],
        ) -> Result<usize, OperationLogError> {
            unreachable!("asking for something reads no folder")
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
            target: OperationTarget,
        ) -> Result<Vec<RecordedOperation>, OperationLogError> {
            Ok(self
                .holding
                .lock()
                .expect("test lock")
                .iter()
                .filter(|held| held.operation.target.message == target.message)
                .copied()
                .collect())
        }

        fn record(
            &self,
            _account_id: &str,
            replacing: &[OperationId],
            operation: Option<MailOperation>,
        ) -> Result<u64, OperationLogError> {
            let mut races = self.races.lock().expect("test lock");
            if *races > 0 {
                *races -= 1;
                return Err(OperationLogError::ChangedUnderneath);
            }

            let mut written = self.written.lock().expect("test lock");
            written.push(Written {
                replacing: replacing.to_vec(),
                operation,
            });

            // Counted the way the queue counts: one more for every write, cancelling ones too.
            Ok(written.len() as u64)
        }
    }

    fn read_op(target: OperationTarget) -> MailOperation {
        MailOperation::asked(
            target,
            OperationKind::SetFlags {
                add: Flags::SEEN,
                remove: Flags::NONE,
            },
        )
        .expect("a change was asked for")
    }

    #[test]
    fn marking_a_message_read_writes_one_instruction_down() {
        let log = FakeLog::default();

        ActOnMail::new(&log)
            .set_read("acc-1", target(1, 1), true)
            .expect("it is written");

        let written = log.only_write();

        assert_eq!(written.replacing, Vec::new(), "there was nothing to fold");
        assert_eq!(
            written
                .operation
                .expect("something to tell the server")
                .kind,
            OperationKind::SetFlags {
                add: Flags::SEEN,
                remove: Flags::NONE
            }
        );
    }

    #[test]
    fn asking_again_replaces_what_was_waiting_rather_than_adding_to_it() {
        // Read, unread, read: one instruction reaches the server, not three.
        let log = FakeLog::holding(vec![read_op(target(1, 1))]);

        ActOnMail::new(&log)
            .set_read("acc-1", target(1, 1), false)
            .expect("it is written");

        let written = log.only_write();

        assert_eq!(
            written.replacing,
            vec![OperationId(1)],
            "the earlier one is removed in the same breath"
        );
        assert_eq!(
            written
                .operation
                .expect("something to tell the server")
                .kind,
            OperationKind::SetFlags {
                add: Flags::NONE,
                remove: Flags::SEEN
            },
            "the last word wins"
        );
    }

    #[test]
    fn an_instruction_that_cancels_what_was_waiting_leaves_nothing_to_do() {
        // Marking read something already waiting to be marked read is not an instruction; but the
        // one it cancels still has to go, and both happen together.
        let log = FakeLog::holding(vec![MailOperation::asked(
            target(1, 1),
            OperationKind::SetFlags {
                add: Flags::NONE,
                remove: Flags::SEEN,
            },
        )
        .expect("a change was asked for")]);

        ActOnMail::new(&log)
            .set_read("acc-1", target(1, 1), true)
            .expect("it is written");

        let written = log.only_write();

        assert_eq!(written.replacing, vec![OperationId(1)]);
        assert_eq!(
            written.operation,
            Some(read_op(target(1, 1))),
            "reading a message waiting to be unread is still an instruction"
        );
    }

    #[test]
    fn a_pair_that_gives_the_server_nothing_is_written_as_nothing() {
        let log = FakeLog::holding(vec![MailOperation::asked(
            target(1, 1),
            OperationKind::MoveTo {
                folder: FolderId::new(2).expect("non-zero"),
            },
        )
        .expect("a change was asked for")]);

        // Filed away, then brought back to where it started.
        ActOnMail::new(&log)
            .move_to("acc-1", target(1, 2), FolderId::new(1).expect("non-zero"))
            .expect("it is written");

        let written = log.only_write();

        assert_eq!(written.replacing, vec![OperationId(1)]);
        assert_eq!(
            written.operation, None,
            "the server is told nothing, and what was waiting goes"
        );
    }

    #[test]
    fn an_instruction_that_says_nothing_never_reaches_the_log() {
        let log = FakeLog::default();

        ActOnMail::new(&log)
            .move_to("acc-1", target(1, 1), FolderId::new(1).expect("non-zero"))
            .expect("nothing to do is not a failure");

        assert!(
            log.written().is_empty(),
            "a queue of instructions to do nothing keeps a server busy for nothing"
        );
    }

    #[test]
    fn what_will_not_fold_is_left_where_it_is() {
        // A move and a flag are two instructions in an order that matters: the flag is set where
        // the message is at the time.
        let log = FakeLog::holding(vec![MailOperation::asked(
            target(1, 1),
            OperationKind::MoveTo {
                folder: FolderId::new(2).expect("non-zero"),
            },
        )
        .expect("a change was asked for")]);

        ActOnMail::new(&log)
            .set_read("acc-1", target(1, 1), true)
            .expect("it is written");

        let written = log.only_write();

        assert_eq!(
            written.replacing,
            Vec::new(),
            "the move stays in the queue, ahead of the flag"
        );
        assert!(written.operation.is_some());
    }

    #[test]
    fn a_delete_takes_the_place_of_what_was_asked_before_it() {
        let log = FakeLog::holding(vec![read_op(target(1, 1))]);

        ActOnMail::new(&log)
            .delete("acc-1", target(1, 1), DeleteIntent::ToTrash)
            .expect("it is written");

        let written = log.only_write();

        assert_eq!(written.replacing, vec![OperationId(1)]);
        assert_eq!(
            written
                .operation
                .expect("something to tell the server")
                .kind,
            OperationKind::Delete {
                intent: DeleteIntent::ToTrash
            }
        );
    }

    #[test]
    fn another_message_waiting_is_none_of_this_ones_business() {
        // The fake answers about the message asked for, and this checks the use case asked about
        // the right one: folding across messages is the failure that must be prevented.
        let log = FakeLog::holding(vec![read_op(target(2, 1))]);

        ActOnMail::new(&log)
            .set_read("acc-1", target(1, 1), true)
            .expect("it is written");

        assert_eq!(
            log.only_write().replacing,
            Vec::new(),
            "another message's instruction was folded into this one"
        );
    }

    #[test]
    fn a_queue_that_changed_underneath_is_read_again_rather_than_overwritten() {
        // What was folded described a moment that has passed. Writing it anyway would drop
        // whatever the other writer put there.
        let log = FakeLog::racing(2);

        ActOnMail::new(&log)
            .set_read("acc-1", target(1, 1), true)
            .expect("it is written on the third go");

        assert_eq!(log.written().len(), 1);
    }

    #[test]
    fn losing_the_race_every_time_is_reported_rather_than_retried_for_ever() {
        let log = FakeLog::racing(ATTEMPTS + 1);

        let outcome = ActOnMail::new(&log).set_read("acc-1", target(1, 1), true);

        assert!(matches!(outcome, Err(UseCaseError::OperationRaced)));
        assert!(log.written().is_empty());
    }

    #[test]
    fn what_a_person_has_to_look_at_is_not_folded_away() {
        // A conflicted operation waits for a person. The domain refuses to fold it; this checks
        // the use case does not quietly drop it on the way past.
        let conflicted = MailOperation {
            state: OperationState::Conflicted,
            ..read_op(target(1, 1))
        };
        let log = FakeLog::holding(vec![conflicted]);

        ActOnMail::new(&log)
            .set_read("acc-1", target(1, 1), false)
            .expect("it is written");

        assert_eq!(
            log.only_write().replacing,
            Vec::new(),
            "a conflict a person has to see was folded away"
        );
    }
}
