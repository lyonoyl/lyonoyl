//! Putting a message back, whether or not the server has heard yet.
//!
//! Undo has two halves and they are different things. While an instruction is still waiting, taking
//! it back is removing it: nothing happened, and nothing has to be undone. Once a server has
//! carried it out there is nothing left to remove, and putting the message back is an instruction
//! of its own — a second thing asked for, which goes through the queue like any other and can
//! itself fail, wait or be shown as stopped.
//!
//! The two are one action to the person, so they are one use case here. What they must never be is
//! one *pretending* to be the other: a way back offered after the fact and quietly doing nothing is
//! worse than no way back at all.

use crate::application::act_on_mail::ActOnMail;
use crate::application::error::UseCaseError;
use crate::ports::op_log::{OperationLog, RecordedOperation};
use crate::ports::placement::Placements;
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::operation::{undoing, OperationKind, OperationState, OperationTarget};

/// What happened when somebody asked for a message back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakenBack {
    /// The instruction had not gone yet, and now there is none.
    Removed,
    /// It had gone, and a second instruction will put the message back.
    AskedFor,
    /// It had gone and cannot be undone — a message deleted for good, one this device has
    /// forgotten it ever moved, or an instruction that stopped rather than happened.
    NotPossible,
}

pub struct TakeBack<L: OperationLog, P: Placements> {
    operations: L,
    placements: P,
}

impl<L: OperationLog + Clone, P: Placements> TakeBack<L, P> {
    pub fn new(operations: L, placements: P) -> Self {
        Self {
            operations,
            placements,
        }
    }

    /// `number` is what the index knows the account by, which is how a folder is found by what it
    /// is for.
    pub fn execute(
        &self,
        account_id: &str,
        number: i64,
        target: OperationTarget,
    ) -> Result<TakenBack, UseCaseError> {
        // Still waiting: nothing happened, so nothing has to be undone.
        if self.operations.cancel(account_id, target)? > 0 {
            return Ok(TakenBack::Removed);
        }

        let Some(settled) = self.operations.settled_for(account_id, target)? else {
            return Ok(TakenBack::NotPossible);
        };

        // The newest instruction about it stopped, so what the person is taking back did not
        // happen, and what it is shown as — stopped, on its row — is already the answer. Undoing
        // whatever was carried out before it instead, the mark as read from opening it, would
        // change a message nobody asked about and say the undo had worked.
        if settled.operation.state == OperationState::Conflicted {
            return Ok(TakenBack::NotPossible);
        }

        let Some(back) = undoing(settled.operation.kind, target.folder) else {
            return Ok(TakenBack::NotPossible);
        };

        // Where the message is now, which is what the new instruction is about: one filed into the
        // archive is taken back by moving it *out of* the archive.
        let Some(now_in) = self.where_it_went(account_id, number, &settled, target)? else {
            return Ok(TakenBack::NotPossible);
        };

        ActOnMail::new(self.operations.clone()).ask_for(
            account_id,
            OperationTarget::new(target.message, now_in),
            back,
        )?;

        Ok(TakenBack::AskedFor)
    }

    /// The folder the message ended up in.
    ///
    /// A move says where it went. A deletion does not — what it became depends on what the server
    /// could do, so the folder deleted mail is kept in is asked for, and an account with no such
    /// folder has nowhere for the message to have gone and nothing to bring it back from.
    fn where_it_went(
        &self,
        account_id: &str,
        number: i64,
        settled: &RecordedOperation,
        target: OperationTarget,
    ) -> Result<Option<lyonoyl_domain::message::FolderId>, UseCaseError> {
        let went = match settled.operation.kind {
            OperationKind::MoveTo { folder } => Some(folder),
            OperationKind::Delete { .. } => {
                self.placements.folder_for(number, FolderRole::Trash)?
            }
            // A flag was never anywhere else.
            OperationKind::SetFlags { .. } => Some(target.folder),
        };

        let Some(went) = went else {
            return Ok(None);
        };

        // And it has to actually be there. A message the server has since moved again, or that
        // never arrived where this device thought, is not one to write an instruction about.
        if self
            .placements
            .where_it_sits(went, target.message)?
            .is_some()
        {
            return Ok(Some(went));
        }

        // Unless this device is the reason the index does not have it there yet: the instruction
        // is still on its way to the server, or the server has carried it out and no pass has
        // read the message into where it went. Then it is there as far as anybody knows, and the
        // way back waits behind that pass rather than being refused.
        let not_read_back_yet = settled.operation.state != OperationState::Done
            || self.operations.owes_a_read(account_id, target.message)?;

        Ok(not_read_back_yet.then_some(went))
    }
}

#[cfg(test)]
mod tests {
    /// The generation a placement in these tests was taken under.
    fn generation() -> lyonoyl_domain::message::UidValidity {
        lyonoyl_domain::message::UidValidity::new(7).expect("non-zero")
    }

    use super::*;
    use crate::ports::op_log::{OperationId, OperationLogError, RecordedOperation};
    use crate::ports::placement::{PlacementError, Position};
    use lyonoyl_domain::message::{FolderId, MessageId, Uid};
    use lyonoyl_domain::operation::{DeleteIntent, MailOperation};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Queue {
        cancels: usize,
        settled: Option<MailOperation>,
        asked: Arc<Mutex<Vec<MailOperation>>>,
        /// Whether the move being undone is still waiting to be read back.
        owes: bool,
    }

    impl OperationLog for Queue {
        fn due(&self, _: &str, _: usize) -> Result<Vec<RecordedOperation>, OperationLogError> {
            unreachable!("taking something back asks no server")
        }

        fn about_to_tell(&self, _id: OperationId) -> Result<(), OperationLogError> {
            Ok(())
        }

        fn release(&self, _ids: &[OperationId]) -> Result<(), OperationLogError> {
            unreachable!("taking something back hands nothing out")
        }

        fn attempted(
            &self,
            _: OperationId,
            _: lyonoyl_domain::operation::OperationState,
            _: Option<lyonoyl_domain::operation::Stopped>,
            _: std::time::Duration,
        ) -> Result<(), OperationLogError> {
            unreachable!("nothing is attempted here")
        }

        fn cancel(&self, _: &str, _: OperationTarget) -> Result<usize, OperationLogError> {
            Ok(self.cancels)
        }

        fn try_again(&self, _: &str, _: OperationTarget) -> Result<usize, OperationLogError> {
            unreachable!("nothing stopped here")
        }

        fn folder_reset(&self, _: &str, _: FolderId) -> Result<usize, OperationLogError> {
            unreachable!("no folder was renumbered")
        }

        fn folder_found(&self, _: &str, _: FolderId) -> Result<usize, OperationLogError> {
            unreachable!("no folder was read")
        }

        fn owed_a_read(
            &self,
            _: &str,
        ) -> Result<Vec<crate::ports::op_log::OwedRead>, OperationLogError> {
            unreachable!("taking something back reads nothing back")
        }

        fn read_back(&self, _: &[OperationId]) -> Result<usize, OperationLogError> {
            unreachable!("taking something back reads nothing back")
        }

        fn owes_a_read(&self, _: &str, _: MessageId) -> Result<bool, OperationLogError> {
            Ok(self.owes)
        }

        fn still_named(
            &self,
            _account_id: &str,
        ) -> Result<Vec<lyonoyl_domain::message::MessageId>, OperationLogError> {
            unreachable!("putting a message back does not sweep the index")
        }

        fn folders_gone(
            &self,
            _account_id: &str,
            _still_there: &[lyonoyl_domain::message::FolderId],
        ) -> Result<usize, OperationLogError> {
            unreachable!("putting a message back reads no folder")
        }

        fn settled_for(
            &self,
            _: &str,
            _: OperationTarget,
        ) -> Result<Option<RecordedOperation>, OperationLogError> {
            Ok(self.settled.map(|operation| RecordedOperation {
                id: OperationId(1),
                operation,
                attempts: 1,
                stopped: None,
                already_told: false,
            }))
        }

        fn outstanding_for(
            &self,
            _: &str,
            _: OperationTarget,
        ) -> Result<Vec<RecordedOperation>, OperationLogError> {
            Ok(Vec::new())
        }

        fn record(
            &self,
            _: &str,
            _: &[OperationId],
            operation: Option<MailOperation>,
        ) -> Result<u64, OperationLogError> {
            let mut asked = self.asked.lock().expect("test lock");
            if let Some(operation) = operation {
                asked.push(operation);
            }
            Ok(asked.len() as u64)
        }
    }

    #[derive(Default)]
    struct Where {
        trash: Option<FolderId>,
        holding: Vec<(u64, u64)>,
    }

    impl Placements for Where {
        fn where_it_sits(
            &self,
            folder: FolderId,
            message: MessageId,
        ) -> Result<Option<Position>, PlacementError> {
            Ok(self
                .holding
                .contains(&(folder.get(), message.get()))
                .then(|| Position {
                    mailbox: "INBOX".to_owned(),
                    uid: Uid::new(1).expect("non-zero"),
                    uid_validity: generation(),
                }))
        }

        fn mailbox_of(&self, _: FolderId) -> Result<Option<String>, PlacementError> {
            unreachable!("nothing is sent here")
        }

        fn folder_for(&self, _: i64, role: FolderRole) -> Result<Option<FolderId>, PlacementError> {
            Ok(match role {
                FolderRole::Trash => self.trash,
                _ => None,
            })
        }

        fn confirmed_flags(
            &self,
            _: FolderId,
            _: &Position,
            _: lyonoyl_domain::message::Flags,
            _: lyonoyl_domain::message::Flags,
        ) -> Result<bool, PlacementError> {
            unreachable!("taking something back tells no server")
        }
    }

    fn folder(id: u64) -> FolderId {
        FolderId::new(id).expect("non-zero")
    }

    fn was_in_the_inbox() -> OperationTarget {
        OperationTarget::new(MessageId::new(7).expect("non-zero"), folder(1))
    }

    /// A filing the server has carried out.
    fn filed_into(id: u64) -> MailOperation {
        MailOperation {
            state: OperationState::Done,
            ..on_its_way_into(id)
        }
    }

    /// A filing the server is being told about right now.
    fn on_its_way_into(id: u64) -> MailOperation {
        MailOperation::asked(
            was_in_the_inbox(),
            OperationKind::MoveTo { folder: folder(id) },
        )
        .expect("something was asked for")
    }

    #[test]
    fn an_instruction_that_stopped_is_not_taken_back_by_undoing_the_one_before_it() {
        let queue = Queue {
            settled: Some(MailOperation {
                state: OperationState::Conflicted,
                ..on_its_way_into(2)
            }),
            ..Queue::default()
        };
        let asked = Arc::clone(&queue.asked);

        let taken = TakeBack::new(
            queue,
            Where {
                holding: vec![(2, 7), (1, 7)],
                ..Where::default()
            },
        )
        .execute("acc-1", 1, was_in_the_inbox())
        .expect("answers");

        assert_eq!(taken, TakenBack::NotPossible);
        assert!(asked.lock().expect("test lock").is_empty());
    }

    #[test]
    fn an_instruction_the_server_has_not_heard_is_simply_removed() {
        let queue = Queue {
            cancels: 1,
            ..Queue::default()
        };
        let asked = Arc::clone(&queue.asked);

        let outcome = TakeBack::new(queue, Where::default())
            .execute("acc-1", 1, was_in_the_inbox())
            .expect("it runs");

        assert_eq!(outcome, TakenBack::Removed);
        assert!(
            asked.lock().expect("test lock").is_empty(),
            "nothing happened, so nothing had to be undone"
        );
    }

    #[test]
    fn a_message_already_filed_away_is_asked_for_back() {
        let queue = Queue {
            settled: Some(filed_into(9)),
            ..Queue::default()
        };
        let asked = Arc::clone(&queue.asked);

        let outcome = TakeBack::new(
            queue,
            Where {
                holding: vec![(9, 7)],
                ..Where::default()
            },
        )
        .execute("acc-1", 1, was_in_the_inbox())
        .expect("it runs");

        assert_eq!(outcome, TakenBack::AskedFor);

        let asked = asked.lock().expect("test lock");
        assert_eq!(asked.len(), 1);
        assert_eq!(
            asked[0].target,
            OperationTarget::new(MessageId::new(7).expect("non-zero"), folder(9)),
            "the way back is about where the message is now, not where it was"
        );
        assert_eq!(asked[0].kind, OperationKind::MoveTo { folder: folder(1) });
    }

    #[test]
    fn a_message_in_the_trash_is_asked_for_out_of_the_trash() {
        let queue = Queue {
            settled: Some(
                MailOperation::asked(
                    was_in_the_inbox(),
                    OperationKind::Delete {
                        intent: DeleteIntent::ToTrash,
                    },
                )
                .expect("something was asked for"),
            ),
            ..Queue::default()
        };
        let asked = Arc::clone(&queue.asked);

        let outcome = TakeBack::new(
            queue,
            Where {
                trash: Some(folder(4)),
                holding: vec![(4, 7)],
            },
        )
        .execute("acc-1", 1, was_in_the_inbox())
        .expect("it runs");

        assert_eq!(outcome, TakenBack::AskedFor);
        assert_eq!(
            asked.lock().expect("test lock")[0].target.folder,
            folder(4),
            "a deletion says nothing about where the message went; the trash folder does"
        );
    }

    #[test]
    fn a_message_deleted_for_good_cannot_be_asked_for_back() {
        // There is no instruction that undoes it, and writing one down would be a queue that
        // pretends.
        let queue = Queue {
            settled: Some(
                MailOperation::asked(
                    was_in_the_inbox(),
                    OperationKind::Delete {
                        intent: DeleteIntent::Permanently,
                    },
                )
                .expect("something was asked for"),
            ),
            ..Queue::default()
        };
        let asked = Arc::clone(&queue.asked);

        let outcome = TakeBack::new(
            queue,
            Where {
                trash: Some(folder(4)),
                holding: vec![(4, 7)],
            },
        )
        .execute("acc-1", 1, was_in_the_inbox())
        .expect("it runs");

        assert_eq!(outcome, TakenBack::NotPossible);
        assert!(asked.lock().expect("test lock").is_empty());
    }

    #[test]
    fn a_message_that_is_not_where_it_was_left_is_not_written_about() {
        // The server may have moved it again, or it may never have arrived where this device
        // thought. Either way there is nothing here to write an instruction about.
        let queue = Queue {
            settled: Some(filed_into(9)),
            ..Queue::default()
        };
        let asked = Arc::clone(&queue.asked);

        let outcome = TakeBack::new(queue, Where::default())
            .execute("acc-1", 1, was_in_the_inbox())
            .expect("it runs");

        assert_eq!(outcome, TakenBack::NotPossible);
        assert!(asked.lock().expect("test lock").is_empty());
    }

    #[test]
    fn nothing_this_device_remembers_doing_cannot_be_undone() {
        let queue = Queue::default();

        let outcome = TakeBack::new(queue, Where::default())
            .execute("acc-1", 1, was_in_the_inbox())
            .expect("it runs");

        assert_eq!(outcome, TakenBack::NotPossible);
    }

    #[test]
    fn a_filing_the_index_has_not_read_back_is_still_asked_for_back() {
        // Undo pressed a second after archiving: the server has done it, and the pass that reads
        // the message into the archive has not run. Refused, a person would be told their undo
        // was too late about a message that moved a second ago.
        let queue = Queue {
            settled: Some(filed_into(9)),
            owes: true,
            ..Queue::default()
        };
        let asked = Arc::clone(&queue.asked);

        let outcome = TakeBack::new(queue, Where::default())
            .execute("acc-1", 1, was_in_the_inbox())
            .expect("it runs");

        assert_eq!(outcome, TakenBack::AskedFor);
        assert_eq!(
            asked.lock().expect("test lock")[0].target,
            OperationTarget::new(MessageId::new(7).expect("non-zero"), folder(9)),
            "asked of where it went, which is where the read will find it"
        );
    }

    #[test]
    fn a_filing_on_its_way_to_the_server_is_asked_for_back_behind_it() {
        // Too late to take out of the queue -- the server may be carrying it out this moment --
        // and not too late to undo: the way back is written behind it.
        let queue = Queue {
            settled: Some(on_its_way_into(9)),
            ..Queue::default()
        };
        let asked = Arc::clone(&queue.asked);

        let outcome = TakeBack::new(queue, Where::default())
            .execute("acc-1", 1, was_in_the_inbox())
            .expect("it runs");

        assert_eq!(outcome, TakenBack::AskedFor);
        assert_eq!(
            asked.lock().expect("test lock")[0].kind,
            OperationKind::MoveTo { folder: folder(1) }
        );
    }
}
