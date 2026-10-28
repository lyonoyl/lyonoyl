//! What someone asked to happen to a message, before it has happened on the server.
//!
//! An operation is written down the moment it is asked for, applied when the server can be reached,
//! and survives a restart in between. That is what makes acting on mail work with no network: the
//! list shows the answer at once, and the truth catches up.
//!
//! # Why a message is addressed the way it is
//!
//! A `Uid` is a position, not a name. It is meaningful in one mailbox until that mailbox says
//! `UIDVALIDITY` changed, and after that the same number means a different message. An operation
//! that carried one would, after a reset, apply to whatever message now holds that number — and
//! `\Deleted` landing on a stranger's message is damage on the *server*, which no local recovery
//! undoes.
//!
//! So an operation names a message by [`MessageId`], which is this device's own stable name for it,
//! and a folder by [`FolderId`]. The position is looked up when the operation is applied, not when
//! it is recorded. No type here can hold a `Uid`, which is the enforcement: not a rule someone has
//! to remember, a field that does not exist.

use crate::folder::FolderRole;
use crate::message::{Flags, FolderId, MessageId};

/// Where an operation applies: a message, and the folder it was in when it was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationTarget {
    pub message: MessageId,
    pub folder: FolderId,
}

impl OperationTarget {
    pub fn new(message: MessageId, folder: FolderId) -> Self {
        Self { message, folder }
    }
}

/// What was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    /// Flags to add and flags to take away, in one operation because that is how a server is told.
    ///
    /// A flag in both halves is a contradiction the constructor refuses rather than resolving
    /// silently: which half wins is exactly the kind of guess that shows up later as a message
    /// that will not stay read.
    SetFlags { add: Flags, remove: Flags },
    /// To another folder on the same account, named by what this device calls it.
    MoveTo { folder: FolderId },
    /// What the person meant by "delete", which is not the same thing on every server.
    Delete { intent: DeleteIntent },
}

/// What "delete" was meant to mean.
///
/// Kept apart from how it is carried out. A server with a trash folder is told to move; one without
/// is told to mark and expunge; and which of those happens is decided against the server's own
/// capabilities at the time, not written down here days earlier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteIntent {
    /// Recoverable: it goes where the account keeps deleted mail.
    ToTrash,
    /// Not recoverable. Only ever asked for explicitly, and never resolved to from `ToTrash`.
    Permanently,
}

/// How far an operation has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationState {
    /// Written down, not yet carried out.
    Pending,
    /// Where the message sits is not known yet, so it waits for a pass to find it: the folder's
    /// `UIDVALIDITY` changed underneath it, or this device moved the message a moment ago and no
    /// pass has read it into where it went. It is applied again once a pass has read the folder.
    NeedsReresolve,
    /// It cannot be carried out and a person has to see it. Never reached silently, and never
    /// cleaned up by anything but a person.
    Conflicted,
    /// The server did it.
    Done,
}

/// One thing a person asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailOperation {
    pub target: OperationTarget,
    pub kind: OperationKind,
    pub state: OperationState,
}

impl MailOperation {
    /// Records an intention. `None` when the operation says nothing — the caller asked for a change
    /// that is not one, and writing it down would mean the server is told to do nothing.
    pub fn asked(target: OperationTarget, kind: OperationKind) -> Option<Self> {
        match kind {
            OperationKind::SetFlags { add, remove } => {
                if add.is_empty() && remove.is_empty() {
                    return None;
                }
                // A flag named on both sides is a contradiction, not a preference: there is no
                // answer to give the server, and picking one half would be inventing an intention.
                if add.bits() & remove.bits() != 0 {
                    return None;
                }
            }
            // Moving a message to the folder it is already in is not a move.
            OperationKind::MoveTo { folder } if folder == target.folder => return None,
            _ => {}
        }

        Some(Self {
            target,
            kind,
            state: OperationState::Pending,
        })
    }

    /// What this operation becomes when the folder it addresses resets its `UIDVALIDITY`.
    ///
    /// Everything still waiting has to be found again: the position it would have been applied at
    /// now names a different message. What has already happened is left alone.
    pub fn after_uid_validity_changed(self, folder: FolderId) -> Self {
        if self.target.folder != folder {
            return self;
        }

        match self.state {
            OperationState::Pending => Self {
                state: OperationState::NeedsReresolve,
                ..self
            },
            _ => self,
        }
    }

    /// Whether the server still has to be told about this one.
    pub fn is_outstanding(self) -> bool {
        matches!(
            self.state,
            OperationState::Pending | OperationState::NeedsReresolve
        )
    }
}

/// How long to wait before asking the server again, in seconds, by how many attempts have failed.
///
/// The first wait is short because most failures are a moment of network; the last is an hour
/// because a server that has refused five times is not going to relent in ten seconds, and asking
/// it every ten seconds is how a client gets rate-limited or blocked. Past the end of the list the
/// wait stays at an hour rather than growing without bound: a mailbox that comes back should be
/// caught up with within the hour, not the day.
const RETRY_SECONDS: [u64; 5] = [15, 60, 240, 900, 3600];

/// What happened when the server was told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    /// The server did it.
    Carried,
    /// The server could not be reached, or answered in a way that says "not now": a moment of
    /// network, a locked mailbox, a server being restarted.
    NotNow,
    /// It cannot be done, and asking again will not change that — the message is not there any
    /// more, the folder is gone, the server refused outright.
    Impossible,
}

/// What an operation becomes after the server has been told.
///
/// The one thing this may never do is drop an instruction. Something that cannot be carried out
/// becomes a conflict, which is a state a person is shown; it does not quietly disappear, and it
/// is not retried for ever in the hope that a message that no longer exists comes back.
pub fn after_attempt(outcome: Attempt) -> OperationState {
    match outcome {
        Attempt::Carried => OperationState::Done,
        Attempt::NotNow => OperationState::Pending,
        Attempt::Impossible => OperationState::Conflicted,
    }
}

/// How long to wait before the next attempt.
pub fn retry_after(attempts: u32) -> std::time::Duration {
    let at = (attempts as usize).min(RETRY_SECONDS.len() - 1);

    std::time::Duration::from_secs(RETRY_SECONDS[at])
}

/// Why an instruction stopped.
///
/// Kept apart from the state because a person shown "this did not happen" and nothing else can only
/// guess, and the four answers below lead to four different things to do: find the message
/// somewhere, choose another folder, empty the trash you do not have, or take it up with the
/// server. What is never said is which message it was about — that is the row it is shown on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// It was in that folder when somebody asked, and it is not there now: moved on another
    /// device, deleted elsewhere, or lost to a folder that reset and came back different.
    MessageIsNotThere,
    /// The folder it was to be filed in is not on this device any more.
    FolderIsNotThere,
    /// "Delete" meant "put it where I can get it back", and the account has nowhere to put it.
    NowhereToKeepDeletedMail,
    /// The server said no, and asking again gets the same answer.
    TheServerRefused,
}

impl Stopped {
    /// The stored form, and the same string the interface names its explanation by.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MessageIsNotThere => "messageIsNotThere",
            Self::FolderIsNotThere => "folderIsNotThere",
            Self::NowhereToKeepDeletedMail => "nowhereToKeepDeletedMail",
            Self::TheServerRefused => "theServerRefused",
        }
    }

    /// Reads a stored value. Unknown means a newer build wrote a reason this one has no words for,
    /// and the honest answer then is that the server refused: something stopped, a person has to
    /// look, and this build will not invent a more specific story than it knows.
    pub fn parse(value: &str) -> Self {
        match value {
            "messageIsNotThere" => Self::MessageIsNotThere,
            "folderIsNotThere" => Self::FolderIsNotThere,
            "nowhereToKeepDeletedMail" => Self::NowhereToKeepDeletedMail,
            _ => Self::TheServerRefused,
        }
    }

    /// Every reason, in no particular order — what a settings screen or a test enumerates.
    pub const ALL: [Self; 4] = [
        Self::MessageIsNotThere,
        Self::FolderIsNotThere,
        Self::NowhereToKeepDeletedMail,
        Self::TheServerRefused,
    ];
}

/// What an operation becomes when the message it names cannot be found where it was asked for.
///
/// It was there when somebody asked. It is not there now — moved on another device, deleted
/// elsewhere, or lost to a folder that reset and came back different. None of those is something
/// this application may decide for the person, so the instruction stops and is shown to them. What
/// it may never do is guess at another message.
pub fn when_the_message_cannot_be_found() -> OperationState {
    OperationState::Conflicted
}

/// Two operations asked for one after the other, as one operation — when they can be.
///
/// A person who reads a message, marks it unread and reads it again should not make a server do
/// three things, and a queue that grew while offline should not replay a morning of clicking. What
/// this may never do is change *which* message is acted on, or turn a recoverable intention into an
/// unrecoverable one.
///
/// `None` means the two have to stay two.
pub fn coalesced(earlier: MailOperation, later: MailOperation) -> Option<MailOperation> {
    // Never across messages. This is the line, and everything below stays on this side of it:
    // whatever comes out acts on the message that was named.
    if earlier.target.message != later.target.message {
        return None;
    }

    // Only what is still waiting. Folding something already carried out into something waiting
    // would tell the server to do it twice.
    if !earlier.is_outstanding() || !later.is_outstanding() {
        return None;
    }

    // A move is the one thing that changes where the message is, so the operation asked for after
    // it is asked for from somewhere else. Every other pair has to be about the same folder, or
    // they are two different things that happen to look alike: a message sits in more than one
    // folder, and flagging it in one is not flagging it in the other.
    let chained = matches!(
        (earlier.kind, later.kind),
        (OperationKind::MoveTo { folder }, OperationKind::MoveTo { .. }) if folder == later.target.folder
    );

    if !chained && earlier.target.folder != later.target.folder {
        return None;
    }

    let kind = match (earlier.kind, later.kind) {
        (
            OperationKind::SetFlags {
                add: first_add,
                remove: first_remove,
            },
            OperationKind::SetFlags {
                add: last_add,
                remove: last_remove,
            },
        ) => {
            // The later word wins per flag: what it adds is added even if the earlier one took it
            // away, and the other way about.
            let add = first_add.without(last_remove).with(last_add);
            let remove = first_remove.without(last_add).with(last_remove);

            if add.is_empty() && remove.is_empty() {
                // They cancelled out. Something has to be recorded — dropping the pair here would
                // leave the earlier one applied and the later one forgotten — so what is recorded
                // is the pair, saying nothing, which the queue drops as a whole.
                return Some(MailOperation {
                    target: earlier.target,
                    kind: OperationKind::SetFlags {
                        add: Flags::NONE,
                        remove: Flags::NONE,
                    },
                    state: worse_of(earlier.state, later.state),
                });
            }

            OperationKind::SetFlags { add, remove }
        }

        // A message filed somewhere and then filed somewhere else goes to the last place named,
        // in one move rather than two. Only where the second was asked for from where the first
        // was sending it — otherwise the two are about different messages in different places and
        // the chain does not link up.
        (OperationKind::MoveTo { .. }, OperationKind::MoveTo { folder }) => {
            if folder == earlier.target.folder {
                // Filed away and then brought back: it stays where it is, and the server is told
                // nothing at all.
                return Some(MailOperation {
                    target: earlier.target,
                    kind: OperationKind::SetFlags {
                        add: Flags::NONE,
                        remove: Flags::NONE,
                    },
                    state: worse_of(earlier.state, later.state),
                });
            }
            OperationKind::MoveTo { folder }
        }

        // Deleting swallows what was asked before it: flags on a message about to go are of no
        // interest, and a move into the folder it is being deleted from is undone by the delete.
        (_, OperationKind::Delete { intent }) => OperationKind::Delete { intent },

        // Anything after a delete stays separate. Marking a deleted message read is either a
        // mistake or a different message with the same name, and neither is folded away here.
        (OperationKind::Delete { .. }, _) => return None,

        // A move and a flag are two different instructions to the server, in an order that
        // matters: the flag is set where the message is at the time.
        _ => return None,
    };

    Some(MailOperation {
        target: earlier.target,
        kind,
        state: worse_of(earlier.state, later.state),
    })
}

/// The state a pair takes: whichever of the two still needs more done to it.
fn worse_of(first: OperationState, second: OperationState) -> OperationState {
    match (first, second) {
        (OperationState::NeedsReresolve, _) | (_, OperationState::NeedsReresolve) => {
            OperationState::NeedsReresolve
        }
        _ => OperationState::Pending,
    }
}

/// Where a delete actually goes, decided against what the account has rather than against what was
/// asked for days ago.
///
/// `ToTrash` with nowhere to put it is the one case that has no good answer, and it is answered by
/// refusing: a message the person expected to be able to get back must not quietly become one they
/// cannot.
pub fn delete_mechanism(intent: DeleteIntent, trash: Option<FolderId>) -> Option<DeleteMechanism> {
    match (intent, trash) {
        (DeleteIntent::ToTrash, Some(folder)) => Some(DeleteMechanism::MoveTo(folder)),
        (DeleteIntent::ToTrash, None) => None,
        (DeleteIntent::Permanently, _) => Some(DeleteMechanism::MarkAndExpunge),
    }
}

/// How a delete is carried out on this server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteMechanism {
    /// The account keeps deleted mail somewhere, so the message is moved there and can be had back.
    MoveTo(FolderId),
    /// There is nowhere to keep it. `\Deleted` and then the message is gone.
    MarkAndExpunge,
}

/// Whether a folder is one an operation may send a message to.
///
/// A message moved into the folder the account uses for drafts, or for what it has already sent, is
/// What puts a message back where it was, once the server has already been told.
///
/// The other half of undo. While an instruction is still waiting, taking it back is removing it;
/// once a server has carried it out there is nothing to remove, and the way back is an instruction
/// of its own — a second thing asked for, not a rubbing out of the first.
///
/// `was_in` is the folder the instruction was about, which is where the message goes back to.
///
/// Nothing at all comes back for a message deleted for good. There is no instruction that undoes
/// that, and offering one would be a button that lies.
pub fn undoing(done: OperationKind, was_in: FolderId) -> Option<OperationKind> {
    match done {
        OperationKind::MoveTo { .. } => Some(OperationKind::MoveTo { folder: was_in }),
        OperationKind::Delete {
            intent: DeleteIntent::ToTrash,
        } => Some(OperationKind::MoveTo { folder: was_in }),
        OperationKind::Delete {
            intent: DeleteIntent::Permanently,
        } => None,
        // The flags the other way round. A message marked read is taken back by marking it unread,
        // and one that was marked both ways at once is put back both ways at once.
        OperationKind::SetFlags { add, remove } => Some(OperationKind::SetFlags {
            add: remove,
            remove: add,
        }),
    }
}

/// a message in a place the interface will show as something it is not.
pub fn may_receive(role: FolderRole) -> bool {
    !matches!(role, FolderRole::Drafts | FolderRole::Sent)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn message(value: u64) -> MessageId {
        MessageId::new(value).expect("non-zero")
    }

    fn folder(value: u64) -> FolderId {
        FolderId::new(value).expect("non-zero")
    }

    fn target(message_id: u64, folder_id: u64) -> OperationTarget {
        OperationTarget::new(message(message_id), folder(folder_id))
    }

    fn asked(target: OperationTarget, kind: OperationKind) -> MailOperation {
        MailOperation::asked(target, kind).expect("a change was asked for")
    }

    fn read() -> OperationKind {
        OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        }
    }

    fn unread() -> OperationKind {
        OperationKind::SetFlags {
            add: Flags::NONE,
            remove: Flags::SEEN,
        }
    }

    #[test]
    fn an_operation_starts_out_waiting_to_happen() {
        let operation = asked(target(1, 1), read());

        assert_eq!(operation.state, OperationState::Pending);
        assert!(operation.is_outstanding());
    }

    #[test]
    fn a_change_that_is_not_one_is_not_written_down() {
        // A queue that holds instructions to do nothing is a queue that keeps a server busy for
        // nothing, and a person waiting on it.
        assert_eq!(
            MailOperation::asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::NONE,
                    remove: Flags::NONE
                }
            ),
            None
        );
        assert_eq!(
            MailOperation::asked(target(1, 7), OperationKind::MoveTo { folder: folder(7) }),
            None
        );
    }

    #[test]
    fn a_flag_asked_for_and_against_at_once_is_refused_rather_than_guessed() {
        assert_eq!(
            MailOperation::asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::SEEN,
                }
            ),
            None
        );
    }

    #[test]
    fn reading_and_unreading_the_same_message_asks_the_server_once() {
        let first = asked(target(1, 1), read());
        let second = asked(target(1, 1), unread());

        let one = coalesced(first, second).expect("one message, one folder, both waiting");

        assert_eq!(
            one.kind,
            OperationKind::SetFlags {
                add: Flags::NONE,
                remove: Flags::SEEN
            },
            "the last word wins"
        );
    }

    #[test]
    fn a_pair_that_cancels_out_becomes_one_operation_that_says_nothing() {
        // Not two dropped operations: dropping the pair separately would leave the first applied
        // and the second forgotten. They become one, saying nothing, and the queue drops that.
        let one = coalesced(
            asked(target(1, 1), read()),
            asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::NONE,
                    remove: Flags::SEEN,
                },
            ),
        )
        .expect("they fold");

        let second = coalesced(
            one,
            asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            ),
        )
        .expect("they fold again");

        assert_eq!(
            second.kind,
            OperationKind::SetFlags {
                add: Flags::SEEN,
                remove: Flags::NONE
            }
        );
    }

    #[test]
    fn flags_asked_for_separately_are_carried_together() {
        let one = coalesced(
            asked(target(1, 1), read()),
            asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::FLAGGED,
                    remove: Flags::NONE,
                },
            ),
        )
        .expect("they fold");

        assert_eq!(
            one.kind,
            OperationKind::SetFlags {
                add: Flags::SEEN.with(Flags::FLAGGED),
                remove: Flags::NONE
            }
        );
    }

    #[test]
    fn two_operations_about_two_messages_stay_two() {
        // The whole point of the addressing: what is folded is folded per message, and a fold
        // across two of them would apply one person's intention to the other's message.
        assert_eq!(
            coalesced(asked(target(1, 1), read()), asked(target(2, 1), unread())),
            None
        );
    }

    #[test]
    fn the_same_message_in_two_folders_is_two_things_to_do() {
        // A message can sit in more than one folder, and flagging it in one is not flagging it in
        // the other.
        assert_eq!(
            coalesced(asked(target(1, 1), read()), asked(target(1, 2), read())),
            None
        );
    }

    #[test]
    fn a_delete_swallows_what_was_asked_before_it() {
        let one = coalesced(
            asked(target(1, 1), read()),
            asked(
                target(1, 1),
                OperationKind::Delete {
                    intent: DeleteIntent::ToTrash,
                },
            ),
        )
        .expect("they fold");

        assert_eq!(
            one.kind,
            OperationKind::Delete {
                intent: DeleteIntent::ToTrash
            }
        );
    }

    #[test]
    fn nothing_is_folded_into_a_delete_that_was_already_asked_for() {
        // Whatever this is, it is not the same intention: either a mistake or a different message
        // wearing the same name. It stays separate and a person can see both.
        assert_eq!(
            coalesced(
                asked(
                    target(1, 1),
                    OperationKind::Delete {
                        intent: DeleteIntent::ToTrash
                    }
                ),
                asked(target(1, 1), read())
            ),
            None
        );
    }

    #[test]
    fn a_move_and_a_flag_stay_in_the_order_they_were_asked_for() {
        // The flag is set where the message is at the time, so folding them would set it in the
        // wrong folder.
        assert_eq!(
            coalesced(
                asked(target(1, 1), OperationKind::MoveTo { folder: folder(2) }),
                asked(target(1, 1), read())
            ),
            None
        );
    }

    #[test]
    fn a_message_filed_twice_goes_where_it_was_last_sent() {
        // The second move is asked for from where the first one was sending it: by then the list
        // already shows the message there.
        let one = coalesced(
            asked(target(1, 1), OperationKind::MoveTo { folder: folder(2) }),
            asked(target(1, 2), OperationKind::MoveTo { folder: folder(3) }),
        )
        .expect("they fold");

        assert_eq!(one.kind, OperationKind::MoveTo { folder: folder(3) });
        assert_eq!(
            one.target,
            target(1, 1),
            "one move, from where the message really is"
        );
    }

    #[test]
    fn two_moves_that_do_not_meet_stay_two() {
        // The same message, filed from two different folders it sits in. Nothing links them, and
        // folding them would take one of the two copies somewhere nobody sent it.
        assert_eq!(
            coalesced(
                asked(target(1, 1), OperationKind::MoveTo { folder: folder(2) }),
                asked(target(1, 5), OperationKind::MoveTo { folder: folder(3) })
            ),
            None
        );
    }

    #[test]
    fn a_message_filed_away_and_brought_back_stays_where_it_is() {
        let one = coalesced(
            asked(target(1, 1), OperationKind::MoveTo { folder: folder(2) }),
            asked(target(1, 2), OperationKind::MoveTo { folder: folder(1) }),
        )
        .expect("they fold");

        assert_eq!(
            one.kind,
            OperationKind::SetFlags {
                add: Flags::NONE,
                remove: Flags::NONE
            },
            "nothing left to tell the server"
        );
    }

    #[test]
    fn what_has_already_happened_is_never_folded_into_what_has_not() {
        let done = MailOperation {
            state: OperationState::Done,
            ..asked(target(1, 1), read())
        };

        assert_eq!(coalesced(done, asked(target(1, 1), unread())), None);
        assert_eq!(coalesced(asked(target(1, 1), unread()), done), None);
    }

    #[test]
    fn a_conflict_is_never_folded_away() {
        // A conflict is the one state a person has to see. Folding it into something waiting would
        // be the application deciding on their behalf that it did not matter.
        let conflicted = MailOperation {
            state: OperationState::Conflicted,
            ..asked(target(1, 1), read())
        };

        assert_eq!(coalesced(conflicted, asked(target(1, 1), read())), None);
    }

    #[test]
    fn a_folder_that_reset_sends_everything_waiting_in_it_to_be_found_again() {
        let waiting = asked(target(1, 1), read()).after_uid_validity_changed(folder(1));

        assert_eq!(waiting.state, OperationState::NeedsReresolve);
        assert!(waiting.is_outstanding(), "it still has to happen");
    }

    #[test]
    fn a_reset_elsewhere_leaves_an_operation_alone() {
        let waiting = asked(target(1, 1), read()).after_uid_validity_changed(folder(9));

        assert_eq!(waiting.state, OperationState::Pending);
    }

    #[test]
    fn a_reset_does_not_reopen_what_is_finished_or_hide_what_conflicted() {
        for state in [OperationState::Done, OperationState::Conflicted] {
            let settled = MailOperation {
                state,
                ..asked(target(1, 1), read())
            };

            assert_eq!(settled.after_uid_validity_changed(folder(1)).state, state);
        }
    }

    #[test]
    fn folding_something_waiting_to_be_found_keeps_it_waiting_to_be_found() {
        let lost = asked(target(1, 1), read()).after_uid_validity_changed(folder(1));
        let one = coalesced(lost, asked(target(1, 1), unread())).expect("they fold");

        assert_eq!(
            one.state,
            OperationState::NeedsReresolve,
            "where the message sits is still unknown"
        );
    }

    #[test]
    fn deleting_to_the_trash_needs_somewhere_to_put_it() {
        // The one case with no good answer. A message somebody expects to get back must not
        // quietly become one they cannot.
        assert_eq!(
            delete_mechanism(DeleteIntent::ToTrash, Some(folder(4))),
            Some(DeleteMechanism::MoveTo(folder(4)))
        );
        assert_eq!(delete_mechanism(DeleteIntent::ToTrash, None), None);
    }

    #[test]
    fn deleting_for_good_never_becomes_a_move_and_a_move_never_becomes_deleting_for_good() {
        assert_eq!(
            delete_mechanism(DeleteIntent::Permanently, Some(folder(4))),
            Some(DeleteMechanism::MarkAndExpunge),
            "asked for plainly, carried out plainly, whatever folders exist"
        );
        assert!(
            !matches!(
                delete_mechanism(DeleteIntent::ToTrash, Some(folder(4))),
                Some(DeleteMechanism::MarkAndExpunge)
            ),
            "recoverable never turns into unrecoverable"
        );
    }

    #[test]
    fn a_message_is_not_moved_into_a_folder_that_means_something_else() {
        assert!(!may_receive(FolderRole::Drafts));
        assert!(!may_receive(FolderRole::Sent));
        assert!(may_receive(FolderRole::Inbox));
        assert!(may_receive(FolderRole::Archive));
        assert!(may_receive(FolderRole::Trash));
    }

    #[test]
    fn a_server_that_did_it_leaves_nothing_to_do() {
        assert_eq!(after_attempt(Attempt::Carried), OperationState::Done);
    }

    #[test]
    fn a_moment_of_network_is_not_a_thing_to_show_a_person() {
        // The mailbox will be there later, and telling somebody their message could not be marked
        // read because the wifi dropped is telling them about our problem.
        assert_eq!(after_attempt(Attempt::NotNow), OperationState::Pending);
    }

    #[test]
    fn something_that_cannot_be_done_stops_and_is_shown_rather_than_dropped() {
        // Never retried for ever in the hope a deleted message comes back, and never forgotten:
        // a person asked for this, and only a person can decide what it now means.
        assert_eq!(
            after_attempt(Attempt::Impossible),
            OperationState::Conflicted
        );
        assert_eq!(
            when_the_message_cannot_be_found(),
            OperationState::Conflicted
        );
    }

    #[test]
    fn the_first_wait_is_short_and_the_waits_grow() {
        // Most failures are a moment of network, so the first ask comes back quickly. A server
        // that has refused several times is not going to relent in ten seconds, and asking it
        // every ten seconds is how a client gets itself blocked.
        let waits: Vec<u64> = (0..6).map(|n| retry_after(n).as_secs()).collect();

        assert_eq!(waits[0], 15);
        for pair in waits.windows(2) {
            assert!(pair[1] >= pair[0], "a wait got shorter: {waits:?}");
        }
    }

    #[test]
    fn the_wait_stops_growing_rather_than_reaching_a_day() {
        // A mailbox that comes back should be caught up with within the hour. An unbounded backoff
        // means a message marked read this morning reaching the server tomorrow.
        let hour = std::time::Duration::from_secs(3600);

        assert_eq!(retry_after(4), hour);
        assert_eq!(retry_after(50), hour);
        assert_eq!(retry_after(u32::MAX), hour, "no overflow, no day-long wait");
    }

    #[test]
    fn every_reason_an_instruction_stops_for_survives_being_written_down() {
        for reason in Stopped::ALL {
            assert_eq!(Stopped::parse(reason.as_str()), reason, "{reason:?}");
        }
    }

    #[test]
    fn a_reason_this_build_has_no_words_for_is_not_invented() {
        // Something stopped and a person has to look at it. Saying "the message is not there" when
        // this build does not know that would send them looking for a message that is fine.
        assert_eq!(
            Stopped::parse("some-newer-reason"),
            Stopped::TheServerRefused
        );
        assert_eq!(Stopped::parse(""), Stopped::TheServerRefused);
    }

    fn any_kind() -> impl Strategy<Value = OperationKind> {
        prop_oneof![
            (0u32..32, 0u32..32).prop_map(|(add, remove)| OperationKind::SetFlags {
                add: Flags::from_bits(add & !remove),
                remove: Flags::from_bits(remove),
            }),
            (1u64..8).prop_map(|folder| OperationKind::MoveTo {
                folder: FolderId::new(folder).expect("non-zero")
            }),
            prop_oneof![Just(DeleteIntent::ToTrash), Just(DeleteIntent::Permanently)]
                .prop_map(|intent| OperationKind::Delete { intent }),
        ]
    }

    fn any_operation() -> impl Strategy<Value = MailOperation> {
        ((1u64..6), (1u64..4), any_kind()).prop_filter_map("says something", |(m, f, kind)| {
            MailOperation::asked(target(m, f), kind)
        })
    }

    proptest! {
        /// The invariant the whole design rests on.
        ///
        /// Whatever is folded into whatever, the thing that comes out acts on the message that was
        /// named, in the folder it was named in. An operation that drifted onto another message is
        /// damage on the server, and the server is the one place nothing here can undo.
        #[test]
        fn folding_never_moves_an_operation_onto_another_message(
            first in any_operation(),
            second in any_operation(),
        ) {
            if let Some(one) = coalesced(first, second) {
                prop_assert_eq!(one.target.message, first.target.message);
                prop_assert_eq!(one.target.message, second.target.message);
                // And it starts from where the earlier one did: a fold may drop work, never move
                // the place the work starts from.
                prop_assert_eq!(one.target.folder, first.target.folder);
            }
        }

        /// Folding is only ever allowed to reduce work, never to invent it.
        #[test]
        fn folding_never_produces_something_neither_side_asked_for(
            first in any_operation(),
            second in any_operation(),
        ) {
            if let Some(one) = coalesced(first, second) {
                match one.kind {
                    // A delete only comes out if one went in, and with the intention that went in.
                    OperationKind::Delete { intent } => {
                        let asked_for = [first.kind, second.kind].iter().any(|kind| {
                            matches!(kind, OperationKind::Delete { intent: was } if *was == intent)
                        });

                        prop_assert!(asked_for, "a delete nobody asked for");
                    }
                    // A move only ever lands where one of the two said.
                    OperationKind::MoveTo { folder } => {
                        let asked_for = [first.kind, second.kind].iter().any(|kind| {
                            matches!(kind, OperationKind::MoveTo { folder: to } if *to == folder)
                        });

                        prop_assert!(asked_for, "a destination nobody named");
                    }
                    // No flag is set that neither asked to set, and none removed that neither
                    // asked to remove.
                    OperationKind::SetFlags { add, remove } => {
                        let asked_add = flags_of(first.kind, true).with(flags_of(second.kind, true));
                        let asked_remove =
                            flags_of(first.kind, false).with(flags_of(second.kind, false));

                        prop_assert!(asked_add.contains(add));
                        prop_assert!(asked_remove.contains(remove));
                        prop_assert!(add.bits() & remove.bits() == 0);
                    }
                }
            }
        }

        /// A message that has to be found again does not quietly stop having to be.
        #[test]
        fn folding_never_loses_the_need_to_find_a_message_again(
            first in any_operation(),
            second in any_operation(),
            reset in 1u64..4,
        ) {
            let first = first.after_uid_validity_changed(folder(reset));
            let second = second.after_uid_validity_changed(folder(reset));

            if let Some(one) = coalesced(first, second) {
                if first.state == OperationState::NeedsReresolve
                    || second.state == OperationState::NeedsReresolve
                {
                    prop_assert_eq!(one.state, OperationState::NeedsReresolve);
                }
            }
        }
    }

    /// The flags one operation asks to set, or to take away. Nothing for the kinds that ask for
    /// neither.
    fn flags_of(kind: OperationKind, adding: bool) -> Flags {
        match kind {
            OperationKind::SetFlags { add, remove } => match adding {
                true => add,
                false => remove,
            },
            _ => Flags::NONE,
        }
    }
}

#[cfg(test)]
mod what_puts_it_back {
    use super::*;

    fn folder(id: u64) -> FolderId {
        FolderId::new(id).expect("non-zero")
    }

    #[test]
    fn a_message_filed_away_goes_back_where_it_was() {
        assert_eq!(
            undoing(OperationKind::MoveTo { folder: folder(9) }, folder(1)),
            Some(OperationKind::MoveTo { folder: folder(1) })
        );
    }

    #[test]
    fn a_message_in_the_trash_comes_back_out_of_it() {
        assert_eq!(
            undoing(
                OperationKind::Delete {
                    intent: DeleteIntent::ToTrash
                },
                folder(1)
            ),
            Some(OperationKind::MoveTo { folder: folder(1) })
        );
    }

    #[test]
    fn nothing_brings_back_a_message_deleted_for_good() {
        // There is no instruction that undoes it, and offering one would be a button that lies.
        assert_eq!(
            undoing(
                OperationKind::Delete {
                    intent: DeleteIntent::Permanently
                },
                folder(1)
            ),
            None
        );
    }

    #[test]
    fn a_flag_is_taken_back_by_the_opposite_flag() {
        assert_eq!(
            undoing(
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE
                },
                folder(1)
            ),
            Some(OperationKind::SetFlags {
                add: Flags::NONE,
                remove: Flags::SEEN
            })
        );
    }

    #[test]
    fn two_flags_moved_at_once_are_put_back_at_once() {
        assert_eq!(
            undoing(
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::FLAGGED
                },
                folder(1)
            ),
            Some(OperationKind::SetFlags {
                add: Flags::FLAGGED,
                remove: Flags::SEEN
            })
        );
    }
}
