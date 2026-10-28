use std::collections::BTreeMap;

use proptest::prelude::*;

use super::*;

fn message(id: u64) -> MessageId {
    MessageId::new(id).expect("non-zero")
}

fn folder(id: u64) -> FolderId {
    FolderId::new(id).expect("non-zero")
}

fn asked(on: u64, in_folder: u64, change: Change) -> Asked {
    Asked {
        message: message(on),
        folder: folder(in_folder),
        change,
    }
}

fn into(id: u64) -> Change {
    Change::Leaves {
        to: Some(folder(id)),
    }
}

fn held(on: u64, in_folder: u64, unread: bool) -> Held {
    Held {
        message: message(on),
        folder: folder(in_folder),
        unread,
    }
}

fn inbox(id: FolderId) -> bool {
    id == folder(1)
}

#[test]
fn nothing_asked_is_nothing_to_lay_over() {
    let folded = AsAsked::of([]);

    assert!(folded.is_empty());
    assert_eq!(
        folded.counted(&[held(1, 1, true)], inbox),
        Counted::default()
    );
}

#[test]
fn the_later_word_wins() {
    let folded = AsAsked::of([
        asked(1, 1, Change::Seen(true)),
        asked(1, 1, Change::Seen(false)),
    ]);

    assert_eq!(
        folded.copy(message(1), folder(1)).and_then(|one| one.seen),
        Some(false)
    );
}

#[test]
fn a_flag_is_about_the_copy_it_names_and_no_other() {
    // A message sits in more than one folder, and a server keeps its flags per folder: reading
    // it in the inbox says nothing about the copy in the archive.
    let folded = AsAsked::of([asked(1, 1, Change::Seen(true))]);

    assert_eq!(folded.copy(message(1), folder(2)), None);
    assert_eq!(
        folded.counted(&[held(1, 1, true), held(1, 2, true)], inbox),
        Counted {
            folders: BTreeMap::from([(folder(1), (0, -1))]),
            inbox_unread: -1,
        }
    );
}

#[test]
fn a_message_opened_is_one_fewer_unread_in_its_folder_and_its_inbox() {
    let folded = AsAsked::of([asked(1, 1, Change::Seen(true))]);

    assert_eq!(
        folded.counted(&[held(1, 1, true)], inbox),
        Counted {
            folders: BTreeMap::from([(folder(1), (0, -1))]),
            inbox_unread: -1,
        }
    );
}

#[test]
fn a_message_marked_unread_again_is_one_more() {
    let folded = AsAsked::of([asked(1, 1, Change::Seen(false))]);

    assert_eq!(
        folded.counted(&[held(1, 1, false)], inbox),
        Counted {
            folders: BTreeMap::from([(folder(1), (0, 1))]),
            inbox_unread: 1,
        }
    );
}

#[test]
fn a_message_filed_away_leaves_one_folder_and_arrives_in_the_other() {
    let folded = AsAsked::of([asked(1, 1, into(2))]);

    assert!(folded
        .copy(message(1), folder(1))
        .is_some_and(|one| one.leaving));
    assert_eq!(folded.arriving_into(folder(2)), vec![message(1)]);
    assert_eq!(
        folded.counted(&[held(1, 1, true)], inbox),
        Counted {
            folders: BTreeMap::from([(folder(1), (-1, -1)), (folder(2), (1, 1))]),
            inbox_unread: -1,
        }
    );
}

#[test]
fn what_was_said_about_a_message_travels_with_it() {
    // Read in the inbox and then archived: it arrives in the archive read, which is what the
    // server does with a flag on a message it moves.
    let folded = AsAsked::of([asked(1, 1, Change::Seen(true)), asked(1, 1, into(2))]);

    assert_eq!(
        folded
            .copy(message(1), folder(2))
            .map(|one| one.arrived_seen(true)),
        Some(true)
    );
    assert_eq!(
        folded.counted(&[held(1, 1, true)], inbox),
        Counted {
            folders: BTreeMap::from([(folder(1), (-1, -1)), (folder(2), (1, 0))]),
            inbox_unread: -1,
        }
    );
}

#[test]
fn a_move_a_pass_has_read_in_is_counted_as_the_copy_it_read() {
    // Between a pass writing where the message now is and the move being settled, the index holds
    // it in the archive and no longer in the inbox, and the move is still laid over it. The copy
    // in the archive is the message itself: unread there, it is counted unread there.
    let folded = AsAsked::of([asked(1, 1, into(2))]);

    assert_eq!(
        folded.counted(&[held(1, 2, true)], inbox),
        Counted::default()
    );
}

#[test]
fn a_message_sent_back_where_it_was_is_where_it_was() {
    // Archived and then taken back: whatever the server has done so far, where the message ends
    // up is where it started, and no count has moved.
    let folded = AsAsked::of([asked(1, 1, into(2)), asked(1, 2, into(1))]);

    assert!(folded
        .copy(message(1), folder(1))
        .is_some_and(|one| !one.leaving));
    assert!(folded.arriving_into(folder(2)).is_empty());
    assert_eq!(
        folded.counted(&[held(1, 1, true)], inbox),
        Counted::default()
    );
}

#[test]
fn a_message_moved_twice_arrives_where_it_was_sent_last_from_where_the_index_has_it() {
    let folded = AsAsked::of([asked(1, 1, into(2)), asked(1, 2, into(3))]);

    assert!(folded.arriving_into(folder(2)).is_empty());
    assert_eq!(folded.arriving_into(folder(3)), vec![message(1)]);
    assert_eq!(
        folded
            .copy(message(1), folder(3))
            .and_then(|one| one.arriving)
            .map(|arriving| arriving.from),
        Some(folder(1))
    );
}

#[test]
fn a_message_deleted_for_good_arrives_nowhere() {
    let folded = AsAsked::of([asked(1, 1, Change::Leaves { to: None })]);

    assert_eq!(
        folded.counted(&[held(1, 1, false)], inbox),
        Counted {
            folders: BTreeMap::from([(folder(1), (-1, 0))]),
            inbox_unread: 0,
        }
    );
}

#[test]
fn what_stopped_is_shown_and_changes_nothing_else() {
    let folded = AsAsked::of([asked(1, 1, Change::Stopped(Stopped::MessageIsNotThere))]);

    assert_eq!(
        folded.copy(message(1), folder(1)),
        Some(&Shown {
            stopped: Some(Stopped::MessageIsNotThere),
            ..Shown::default()
        })
    );
    assert_eq!(
        folded.counted(&[held(1, 1, true)], inbox),
        Counted::default()
    );
}

#[test]
fn a_message_filed_where_it_already_is_changes_nothing() {
    let folded = AsAsked::of([asked(1, 1, into(1))]);

    assert!(folded.is_empty());
}

#[test]
fn a_message_in_two_inboxes_is_one_unread_for_its_account() {
    // An account counts messages, not the copies of them.
    let folded = AsAsked::of([asked(1, 1, Change::Seen(true))]);
    let two_inboxes = |id: FolderId| id == folder(1) || id == folder(2);

    assert_eq!(
        folded
            .counted(&[held(1, 1, true), held(1, 2, true)], two_inboxes)
            .inbox_unread,
        0,
        "still unread in the other inbox, so still one unread message"
    );
}

#[test]
fn a_count_never_goes_below_nothing() {
    assert_eq!(moved_by(0, -1), 0);
    assert_eq!(moved_by(3, -1), 2);
    assert_eq!(moved_by(3, 2), 5);
}

#[test]
fn what_an_instruction_does_is_read_off_what_it_is() {
    use crate::operation::DeleteIntent;

    let trash = Some(folder(9));
    let read = OperationKind::SetFlags {
        add: Flags::SEEN,
        remove: Flags::NONE,
    };

    assert_eq!(
        Change::of(read, OperationState::Pending, None, trash),
        Some(Change::Seen(true))
    );
    assert_eq!(
        Change::of(
            OperationKind::SetFlags {
                add: Flags::NONE,
                remove: Flags::SEEN
            },
            OperationState::Done,
            None,
            trash
        ),
        Some(Change::Seen(false))
    );
    assert_eq!(
        Change::of(
            OperationKind::SetFlags {
                add: Flags::FLAGGED,
                remove: Flags::NONE
            },
            OperationState::Pending,
            None,
            trash
        ),
        None,
        "a flag no view shows changes nothing a view shows"
    );
    assert_eq!(
        Change::of(
            OperationKind::Delete {
                intent: DeleteIntent::ToTrash
            },
            OperationState::Done,
            None,
            trash
        ),
        Some(Change::Leaves { to: trash })
    );
    assert_eq!(
        Change::of(
            OperationKind::Delete {
                intent: DeleteIntent::Permanently
            },
            OperationState::Pending,
            None,
            trash
        ),
        Some(Change::Leaves { to: None })
    );
    assert_eq!(
        Change::of(read, OperationState::Conflicted, None, trash),
        Some(Change::Stopped(Stopped::TheServerRefused)),
        "a stop with no reason this build knows is the server refusing"
    );
}

/// What a person does to one message, before it is known which copy it is done to.
#[derive(Debug, Clone, Copy)]
enum Act {
    Seen(bool),
    File(u64),
    Bin,
    Erase,
}

fn any_act() -> impl Strategy<Value = Act> {
    prop_oneof![
        any::<bool>().prop_map(Act::Seen),
        (1u64..=TRASH).prop_map(Act::File),
        Just(Act::Bin),
        Just(Act::Erase),
    ]
}

/// The folder deleted mail goes to, in the generated accounts.
const TRASH: u64 = 4;

/// Where each message is, and whether it is unread there: one copy each.
type World = BTreeMap<u64, (u64, bool)>;

/// What a folder shows, message by message, and whether each is unread.
type Shows = BTreeMap<u64, BTreeMap<u64, bool>>;

fn shows(world: &World) -> Shows {
    let mut by_folder: Shows = BTreeMap::new();
    for (message, (in_folder, unread)) in world {
        by_folder
            .entry(*in_folder)
            .or_default()
            .insert(*message, *unread);
    }
    by_folder
}

/// What a folder shows once the fold is laid over the index, the way a list reads it.
fn laid_over(index: &World, folded: &AsAsked) -> Shows {
    let mut by_folder: Shows = BTreeMap::new();

    for (id, (in_folder, unread)) in index {
        let shown = folded.copy(message(*id), folder(*in_folder));
        if shown.is_some_and(|one| one.leaving) {
            continue;
        }
        let unread = match shown {
            Some(one) if one.arriving.is_some() => !one.arrived_seen(*unread),
            Some(Shown {
                seen: Some(seen), ..
            }) => !seen,
            _ => *unread,
        };
        by_folder.entry(*in_folder).or_default().insert(*id, unread);
    }

    for into in 1..=TRASH {
        for arriving in folded.arriving_into(folder(into)) {
            let (from, from_unread) = index[&arriving.get()];
            if from == into {
                continue;
            }
            let shown = folded
                .copy(arriving, folder(into))
                .expect("it is arriving, so it was asked about");
            by_folder
                .entry(into)
                .or_default()
                .insert(arriving.get(), !shown.arrived_seen(from_unread));
        }
    }

    by_folder
}

/// The counts a sidebar shows: messages and unread per folder, and unread in the inbox.
fn counts(shows: &Shows) -> (BTreeMap<u64, (i64, i64)>, i64) {
    let folders = shows
        .iter()
        .map(|(id, messages)| {
            (
                *id,
                (
                    messages.len() as i64,
                    messages.values().filter(|unread| **unread).count() as i64,
                ),
            )
        })
        .filter(|(_, (total, _))| *total > 0)
        .collect();
    let inbox_unread = shows.get(&1).map_or(0, |inbox| {
        inbox.values().filter(|unread| **unread).count() as i64
    });
    (folders, inbox_unread)
}

proptest! {
    /// The whole promise, over any run of clicking: what the fold shows before the server has
    /// heard is what the server holds once it has, and a pass has read it back.
    ///
    /// A server is played here the way IMAP behaves — a flag set on one copy, a move carrying the
    /// flags with it, a delete going to the trash or away — and every instruction is asked of the
    /// copy the person is looking at when they ask. The list and the counts are then read the two
    /// ways and compared.
    #[test]
    fn the_fold_over_the_index_is_what_the_server_holds_afterwards(
        start in proptest::collection::btree_map(1u64..6, (1u64..=TRASH, any::<bool>()), 1..5),
        clicks in proptest::collection::vec((1u64..6, any_act()), 0..12),
    ) {
        let index: World = start;
        let mut server = index.clone();
        let mut asked_for = Vec::new();

        for (id, act) in clicks {
            let Some(&(in_folder, unread)) = server.get(&id) else {
                continue;
            };
            let change = match act {
                Act::Seen(seen) => {
                    server.insert(id, (in_folder, !seen));
                    Change::Seen(seen)
                }
                Act::File(_) | Act::Bin if to_of(act) == in_folder => continue,
                Act::File(_) | Act::Bin => {
                    server.insert(id, (to_of(act), unread));
                    Change::Leaves { to: Some(folder(to_of(act))) }
                }
                Act::Erase => {
                    server.remove(&id);
                    Change::Leaves { to: None }
                }
            };
            asked_for.push(asked(id, in_folder, change));
        }

        let folded = AsAsked::of(asked_for);
        prop_assert_eq!(laid_over(&index, &folded), shows(&server));

        // The counts, as the sidebar adds them to what the index counts.
        let (before, inbox_before) = counts(&shows(&index));
        let (truth, inbox_truth) = counts(&shows(&server));
        let held: Vec<Held> = index
            .iter()
            .map(|(id, (in_folder, unread))| held(*id, *in_folder, *unread))
            .collect();
        let moved = folded.counted(&held, inbox);

        let mut after = before.clone();
        for (id, (total, unread)) in moved.folders {
            let one = after.entry(id.get()).or_insert((0, 0));
            one.0 += total;
            one.1 += unread;
        }
        after.retain(|_, (total, _)| *total > 0);

        prop_assert_eq!(after, truth);
        prop_assert_eq!(inbox_before + moved.inbox_unread, inbox_truth);

        // And laid over an index a pass has already brought level with the server, before the
        // instructions are settled, it changes nothing: the list and the counts are the server's.
        prop_assert_eq!(laid_over(&server, &folded), shows(&server));
        let level: Vec<Held> = server
            .iter()
            .map(|(id, (in_folder, unread))| self::held(*id, *in_folder, *unread))
            .collect();
        let moved = folded.counted(&level, inbox);
        prop_assert!(
            moved.folders.is_empty() && moved.inbox_unread == 0,
            "laid over what it already holds, the fold moved {:?}",
            moved
        );
    }
}

fn to_of(act: Act) -> u64 {
    match act {
        Act::File(to) => to,
        _ => TRASH,
    }
}
