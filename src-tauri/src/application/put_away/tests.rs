use std::sync::Mutex;
use std::time::Duration;

use super::*;
use crate::ports::op_log::{OperationId, OperationLogError, OwedRead, RecordedOperation};
use crate::ports::placement::{PlacementError, Position};
use lyonoyl_domain::message::{Flags, FolderId, MessageId, Uid, UidValidity};
use lyonoyl_domain::operation::{MailOperation, OperationKind, OperationState, Stopped};

/// What the account is called where instructions are written.
const ACCOUNT: &str = "account-3";
/// What the index knows the same account by.
const NUMBER: i64 = 3;

/// The queue, holding what was written to it and for which account, and what was waiting in it.
#[derive(Default)]
struct Queue {
    written: Mutex<Vec<(String, MailOperation)>>,
    /// What the server has yet to be told about the message.
    waiting: Vec<MailOperation>,
}

impl Queue {
    fn written(&self) -> Vec<(String, MailOperation)> {
        self.written.lock().expect("test lock").clone()
    }
}

impl OperationLog for Queue {
    fn due(&self, _: &str, _: usize) -> Result<Vec<RecordedOperation>, OperationLogError> {
        unreachable!("a press tells no server")
    }

    fn attempted(
        &self,
        _: OperationId,
        _: OperationState,
        _: Option<Stopped>,
        _: Duration,
    ) -> Result<(), OperationLogError> {
        unreachable!("nothing is attempted here")
    }

    fn about_to_tell(&self, _: OperationId) -> Result<(), OperationLogError> {
        unreachable!("a press tells no server")
    }

    fn refused_untouched(&self, _: OperationId) -> Result<(), OperationLogError> {
        unreachable!("a press tells no server")
    }

    fn let_go(&self, _: OperationId) -> Result<(), OperationLogError> {
        unreachable!("a press tells no server")
    }

    fn release(&self, _: &[OperationId]) -> Result<(), OperationLogError> {
        unreachable!("a press hands nothing out")
    }

    fn cancel(&self, _: &str, _: OperationTarget) -> Result<usize, OperationLogError> {
        unreachable!("a press takes nothing back")
    }

    fn try_again(&self, _: &str, _: OperationTarget) -> Result<usize, OperationLogError> {
        unreachable!("a press asks for nothing again")
    }

    fn give_up(&self, _: &str, _: OperationTarget) -> Result<usize, OperationLogError> {
        unreachable!("a press takes nothing away")
    }

    fn folder_reset(&self, _: &str, _: FolderId) -> Result<usize, OperationLogError> {
        unreachable!("no folder was renumbered")
    }

    fn folder_found(&self, _: &str, _: FolderId) -> Result<usize, OperationLogError> {
        unreachable!("no folder was read")
    }

    fn outstanding_for(
        &self,
        _: &str,
        _: OperationTarget,
    ) -> Result<Vec<RecordedOperation>, OperationLogError> {
        Ok(self
            .waiting
            .iter()
            .enumerate()
            .map(|(at, operation)| RecordedOperation {
                id: OperationId(at as i64 + 1),
                operation: *operation,
                attempts: 0,
                stopped: None,
                already_told: false,
            })
            .collect())
    }

    fn settled_for(
        &self,
        _: &str,
        _: OperationTarget,
    ) -> Result<Option<RecordedOperation>, OperationLogError> {
        unreachable!("a press undoes nothing")
    }

    fn owed_a_read(&self, _: &str) -> Result<Vec<OwedRead>, OperationLogError> {
        unreachable!("a press reads nothing back")
    }

    fn read_back(&self, _: &[OperationId]) -> Result<usize, OperationLogError> {
        unreachable!("a press reads nothing back")
    }

    fn still_named(&self, _: &str) -> Result<Vec<MessageId>, OperationLogError> {
        unreachable!("a press does not sweep the index")
    }

    fn folders_gone(&self, _: &str, _: &[FolderId]) -> Result<usize, OperationLogError> {
        unreachable!("a press reads no folder")
    }

    fn owes_a_read(&self, _: &str, _: MessageId) -> Result<bool, OperationLogError> {
        unreachable!("a press reads nothing back")
    }

    fn record(
        &self,
        account_id: &str,
        _: &[OperationId],
        operation: Option<MailOperation>,
    ) -> Result<u64, OperationLogError> {
        let mut written = self.written.lock().expect("test lock");
        if let Some(operation) = operation {
            written.push((account_id.to_owned(), operation));
        }
        Ok(written.len() as u64)
    }
}

/// What the index says of the account: where its messages sit, and its two places.
#[derive(Default)]
struct Account {
    /// Folder and message, for every message where it sits.
    holding: Vec<(u64, u64)>,
    trash: Option<FolderId>,
    junk: Option<FolderId>,
    /// Every place asked for by what it is for, and in which account.
    asked: Mutex<Vec<(i64, FolderRole)>>,
    /// An index that cannot be read.
    unreadable: bool,
}

impl Placements for Account {
    fn where_it_sits(
        &self,
        folder: FolderId,
        message: MessageId,
    ) -> Result<Option<Position>, PlacementError> {
        if self.unreadable {
            return Err(PlacementError::Unavailable);
        }
        Ok(self
            .holding
            .contains(&(folder.get(), message.get()))
            .then(|| Position {
                mailbox: "INBOX".to_owned(),
                uid: Uid::new(1).expect("non-zero"),
                uid_validity: UidValidity::new(7).expect("non-zero"),
            }))
    }

    fn mailbox_of(&self, _: FolderId) -> Result<Option<String>, PlacementError> {
        unreachable!("a press tells no server")
    }

    fn name_of(&self, _: MessageId) -> Result<Option<String>, PlacementError> {
        unreachable!("a press tells no server")
    }

    fn folder_for(
        &self,
        account: i64,
        role: FolderRole,
    ) -> Result<Option<FolderId>, PlacementError> {
        self.asked.lock().expect("test lock").push((account, role));
        Ok(match role {
            FolderRole::Trash => self.trash,
            FolderRole::Junk => self.junk,
            _ => None,
        })
    }

    fn confirmed_flags(
        &self,
        _: FolderId,
        _: &Position,
        _: Flags,
        _: Flags,
    ) -> Result<bool, PlacementError> {
        unreachable!("a press tells no server")
    }
}

fn folder(id: u64) -> FolderId {
    FolderId::new(id).expect("non-zero")
}

/// Message 7 in the inbox, folder 1, as the notice named it.
fn the_message() -> OperationTarget {
    OperationTarget::new(MessageId::new(7).expect("non-zero"), folder(1))
}

/// An account holding the message where the notice saw it, with a trash, folder 4, and a junk
/// folder, 5.
fn keeping_both() -> Account {
    Account {
        holding: vec![(1, 7)],
        trash: Some(folder(4)),
        junk: Some(folder(5)),
        ..Account::default()
    }
}

fn pressed(queue: &Queue, account: &Account, into: PutInto) -> PutAwayOutcome {
    PutAway::new(queue, account)
        .execute(ACCOUNT, NUMBER, the_message(), into)
        .expect("the press is answered")
}

#[test]
fn move_to_trash_asks_for_the_delete_the_windows_own_trash_button_asks_for() {
    let queue = Queue::default();
    let account = keeping_both();

    assert_eq!(
        pressed(&queue, &account, PutInto::Trash),
        PutAwayOutcome::Deleted
    );

    let written = queue.written();
    assert_eq!(written.len(), 1, "{written:?}");
    let (whose, operation) = &written[0];
    assert_eq!(whose, ACCOUNT);
    assert_eq!(operation.target, the_message());
    assert_eq!(
        operation.kind,
        OperationKind::Delete {
            intent: DeleteIntent::ToTrash
        },
        "never for good, from a button that says trash"
    );
}

#[test]
fn move_to_junk_moves_the_message_into_its_own_accounts_junk_folder() {
    let queue = Queue::default();
    let account = keeping_both();

    assert_eq!(
        pressed(&queue, &account, PutInto::Junk),
        PutAwayOutcome::Moved
    );

    let written = queue.written();
    assert_eq!(written.len(), 1, "{written:?}");
    let (whose, operation) = &written[0];
    assert_eq!(whose, ACCOUNT);
    assert_eq!(operation.target, the_message());
    assert_eq!(operation.kind, OperationKind::MoveTo { folder: folder(5) });
    // Looked for in the account the message is in, and by what it is for.
    assert_eq!(
        *account.asked.lock().expect("test lock"),
        vec![(NUMBER, FolderRole::Junk)]
    );
}

#[test]
fn a_message_no_longer_where_the_notice_saw_it_is_left_alone() {
    for into in [PutInto::Trash, PutInto::Junk] {
        let queue = Queue::default();
        // Filed since: the index holds it in another folder, and not in the one the notice named.
        let account = Account {
            holding: vec![(2, 7)],
            ..keeping_both()
        };

        assert_eq!(
            pressed(&queue, &account, into),
            PutAwayOutcome::NotThere,
            "{into:?}"
        );
        assert!(queue.written().is_empty(), "{into:?}");
    }
}

#[test]
fn a_message_a_delete_is_already_taking_away_is_left_alone() {
    // Thrown away in the window a moment ago, and the server not told yet: the index still has it
    // where the notice saw it.
    let thrown_away = MailOperation::asked(
        the_message(),
        OperationKind::Delete {
            intent: DeleteIntent::ToTrash,
        },
    )
    .expect("something was asked for");

    for into in [PutInto::Trash, PutInto::Junk] {
        let queue = Queue {
            waiting: vec![thrown_away],
            ..Queue::default()
        };

        assert_eq!(
            pressed(&queue, &keeping_both(), into),
            PutAwayOutcome::NotThere,
            "{into:?}"
        );
        assert!(queue.written().is_empty(), "{into:?}");
    }
}

#[test]
fn a_message_waiting_only_to_be_marked_read_is_still_put_away() {
    // Opened in the window, which marks it read: it is still where the notice saw it.
    let marked_read = MailOperation::asked(
        the_message(),
        OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        },
    )
    .expect("something was asked for");
    let queue = Queue {
        waiting: vec![marked_read],
        ..Queue::default()
    };

    assert_eq!(
        pressed(&queue, &keeping_both(), PutInto::Trash),
        PutAwayOutcome::Deleted
    );
    assert_eq!(queue.written().len(), 1);
}

#[test]
fn a_place_the_account_no_longer_keeps_asks_for_nothing() {
    // No trash: the delete could only stop, with nowhere to keep the message.
    let queue = Queue::default();
    let account = Account {
        trash: None,
        ..keeping_both()
    };
    assert_eq!(
        pressed(&queue, &account, PutInto::Trash),
        PutAwayOutcome::NowhereToPut
    );
    assert!(queue.written().is_empty());

    // No junk folder: nowhere to move it. Each button asks after its own place, so the trash still
    // takes the message.
    let queue = Queue::default();
    let account = Account {
        junk: None,
        ..keeping_both()
    };
    assert_eq!(
        pressed(&queue, &account, PutInto::Junk),
        PutAwayOutcome::NowhereToPut
    );
    assert!(queue.written().is_empty());
    assert_eq!(
        pressed(&queue, &account, PutInto::Trash),
        PutAwayOutcome::Deleted
    );
}

#[test]
fn an_index_that_cannot_be_read_asks_for_nothing() {
    let queue = Queue::default();
    let account = Account {
        unreadable: true,
        ..keeping_both()
    };

    let answer =
        PutAway::new(&queue, &account).execute(ACCOUNT, NUMBER, the_message(), PutInto::Trash);

    assert!(answer.is_err(), "{answer:?}");
    assert!(queue.written().is_empty());
}
