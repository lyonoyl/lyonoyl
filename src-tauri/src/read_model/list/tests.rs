use super::*;
use crate::application::sync::SyncFolder;
use crate::infrastructure::fake_provider::{batch, ScriptedProvider, SeededIndex};
use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
use crate::ports::mail_index::MailIndex;
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::UidValidity;

fn folder() -> FolderId {
    FolderId::new(1).expect("non-zero")
}

/// The queue is read by an index, not by reading all of it.
///
/// This runs for every window the list draws, so a scan of it is paid on every frame of a
/// scroll. Measured at fifty thousand waiting instructions: 4.35 ms scanning against 0.06 ms
/// once the index exists. The statement is asked for by name rather than written again here,
/// because the plan of a query nothing runs proves nothing about the one that does.
#[test]
fn what_is_waiting_is_found_by_an_index_rather_than_by_reading_the_queue() {
    let mut connection = rusqlite::Connection::open_in_memory().expect("opens");
    crate::infrastructure::sqlite::migrations::migrate_profile(&mut connection, ":memory:")
        .expect("the profile schema applies");

    let statement = crate::read_model::as_asked::what_is_waiting_for("?1, ?2, ?3");
    let mut explained = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {statement}"))
        .expect("prepares");
    let plan = explained
        .query_map([1i64, 2, 3], |row| row.get::<_, String>(3))
        .expect("readable")
        .collect::<Result<Vec<_>, _>>()
        .expect("readable")
        .join(" | ");

    assert!(
        plan.contains("idx_op_log_by_message"),
        "the queue is read without the index written for this question: {plan}"
    );
    assert!(
        !plan.contains("SCAN op_log"),
        "the queue is read in full for every window the list draws: {plan}"
    );
}

fn seeded(name: &str, count: u32) -> SeededIndex {
    SeededIndex::with(name, folder(), count)
}

fn list(db: &SeededIndex) -> ListReadModel {
    ListReadModel::spanning(vec![(1, db.pool.clone())], db.profile.clone())
}

fn whole_folder() -> ViewQuery {
    ViewQuery {
        scope: ViewScope::Folder(folder()),
        unread_only: false,
        as_conversations: false,
    }
}

fn the_account() -> ViewQuery {
    ViewQuery {
        scope: ViewScope::Account(1),
        unread_only: false,
        as_conversations: false,
    }
}

fn unread() -> ViewQuery {
    ViewQuery {
        scope: ViewScope::Folder(folder()),
        unread_only: true,
        as_conversations: false,
    }
}

/// A folder spread over several days, two messages to a second, holding two messages twice:
/// one whose second copy is newer, and one whose second copy is older but numbered higher.
///
/// Both shapes are the point. The inversion tells an order taken from the copy that speaks
/// from one taken from the highest number the message has anywhere; the shared second is what
/// an order that stops at the date cannot resolve.
fn a_folder_over_several_days(name: &str, count: u32) -> SeededIndex {
    let db = seeded(name, count);
    db.spread_over_a_week();
    db.another_copy_of(folder().get(), 3, 3 * 86_400);
    db.another_copy_of(folder().get(), 5, -2 * 86_400);
    db
}

/// Every message the folder holds, in the order the list puts them in, worked out without the
/// map and without skipping: the copy that speaks for each message, sorted.
fn walking_the_whole_folder(db: &SeededIndex) -> Vec<i64> {
    let connection = db.pool.get().expect("connection");
    let mut statement = connection
        .prepare(
            "SELECT p.message_id FROM message_placements p
                 WHERE p.folder_id = ?1
                   AND NOT EXISTS (
                       SELECT 1 FROM message_placements q
                       WHERE q.folder_id = p.folder_id AND q.message_id = p.message_id
                         AND (q.received_at > p.received_at
                          OR (q.received_at = p.received_at AND q.uid > p.uid)))
                 ORDER BY p.received_at DESC, p.uid DESC",
        )
        .expect("prepares");

    let walked = statement
        .query_map([folder().get() as i64], |row| row.get(0))
        .expect("readable")
        .map(|row| row.expect("row"))
        .collect();
    walked
}

#[test]
fn every_position_reads_the_row_that_walking_to_it_reads() {
    // The whole point of the map. A position is turned into a day and the window seeks to it,
    // rather than counting rows from the top -- so the thing to prove is that the row it lands
    // on is the row that counting would have reached, at every position and not only at the
    // ones a page happens to start at.
    //
    // This is also what holds the order of two messages that share a second. A test asserting
    // that directly could not be made to fail: with the number taken out of both the sort and
    // the index, SQLite still handed back the same order, so the test passed on a build where
    // nothing asked for it. This one fails there, because the row a position lands on moves.
    let db = a_folder_over_several_days("every-position", 40);
    let walked = walking_the_whole_folder(&db);
    assert_eq!(walked.len(), 40, "the copies fold into forty messages");

    for limit in [1_u32, 5] {
        for offset in 0..walked.len() as u32 {
            let rows = list(&db)
                .get_view_window(&whole_folder(), offset, limit)
                .expect("window is readable")
                .rows;

            let expected: Vec<i64> = walked
                .iter()
                .skip(offset as usize)
                .take(limit as usize)
                .copied()
                .collect();
            let read: Vec<i64> = rows.iter().map(|row| row.message_id.get() as i64).collect();

            assert_eq!(read, expected, "at position {offset}, {limit} at a time");
        }
    }
}

#[test]
fn the_last_message_in_the_folder_is_reachable() {
    // A map counting copies rather than messages puts every position ahead of the row it
    // names, and the error is largest at the bottom: the last rows become unreachable while
    // the count beside them still says they are there.
    let db = a_folder_over_several_days("the-last-one", 40);
    let walked = walking_the_whole_folder(&db);

    let window = list(&db)
        .get_view_window(&whole_folder(), walked.len() as u32 - 1, 10)
        .expect("window is readable");

    assert_eq!(window.rows.len(), 1);
    assert_eq!(
        window.rows[0].message_id.get() as i64,
        *walked.last().expect("a last message")
    );
    assert_eq!(window.total as usize, walked.len());
}

#[test]
fn the_total_is_what_a_full_walk_yields() {
    // The total now comes from the map. That makes drift visible instead of silent: a map
    // that had drifted would disagree with the folder here.
    let db = a_folder_over_several_days("the-total", 40);

    let window = list(&db)
        .get_view_window(&whole_folder(), 0, 1)
        .expect("window is readable");

    assert_eq!(window.total as usize, walking_the_whole_folder(&db).len());
}

#[test]
fn a_position_past_the_end_answers_with_no_rows() {
    let db = a_folder_over_several_days("past-the-end", 10);

    let window = list(&db)
        .get_view_window(&whole_folder(), 10_000, 20)
        .expect("window is readable");

    assert!(window.rows.is_empty());
}

#[test]
fn a_folder_holding_nothing_answers_with_no_rows() {
    let db = seeded("holds-nothing", 4);
    db.take_everything_out();

    let window = list(&db)
        .get_view_window(&whole_folder(), 0, 20)
        .expect("window is readable");

    assert!(window.rows.is_empty());
    assert_eq!(window.total, 0);
}

#[test]
fn a_message_opened_in_its_newest_copy_is_still_unread_by_an_older_one() {
    // The flag belongs to the copy. A folder holding a message twice, the newer copy opened
    // and the older not, still holds unread mail, and the row standing for it has to be the
    // unread copy — otherwise the message is represented by a copy the view filters out and
    // vanishes from a list it belongs in.
    let db = seeded("read-newer-unread-older", 20);

    // A message the folder holds once, and holds read.
    let (opened, at): (i64, i64) = {
        let connection = db.pool.get().expect("connection");
        connection
            .query_row(
                "SELECT message_id, received_at FROM message_placements
                     WHERE folder_id = 1 AND (flags & 1) = 1
                     ORDER BY received_at DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("the seed leaves some read")
    };

    assert!(
        !shown_in_the_unread_list(&db).contains(&opened),
        "a message read in its only copy is unread mail"
    );

    db.another_copy_of(folder().get(), opened as u64, -2 * 86_400);

    let unread_rows = list(&db)
        .get_view_window(&unread(), 0, 20)
        .expect("window is readable")
        .rows;
    let row = unread_rows
        .iter()
        .find(|row| row.message_id.get() as i64 == opened)
        .expect("the message is unread again, by its older copy");

    assert_eq!(
        row.received_at,
        at - 2 * 86_400,
        "the row is dated by the copy the view is showing"
    );
}

fn shown_in_the_unread_list(db: &SeededIndex) -> Vec<i64> {
    list(db)
        .get_view_window(&unread(), 0, 100)
        .expect("window is readable")
        .rows
        .iter()
        .map(|row| row.message_id.get() as i64)
        .collect()
}

#[test]
fn the_unread_list_does_not_consult_the_map() {
    // Days hold no flags, so the unread view is answered by walking and must stay that way.
    // Wrecking the map is how that is proved: if the unread window changed, it was reading it.
    let db = a_folder_over_several_days("unread-without-the-map", 40);
    let before = list(&db)
        .get_view_window(&unread(), 3, 5)
        .expect("window is readable");

    db.make_the_map_of_days_wrong();

    let after = list(&db)
        .get_view_window(&unread(), 3, 5)
        .expect("window is readable");

    assert_eq!(
        before
            .rows
            .iter()
            .map(|row| row.message_id)
            .collect::<Vec<_>>(),
        after
            .rows
            .iter()
            .map(|row| row.message_id)
            .collect::<Vec<_>>()
    );
    assert_eq!(before.total, after.total);
}

fn looking_for(typed: &str) -> ViewQuery {
    ViewQuery {
        scope: ViewScope::Looking(typed.to_owned()),
        unread_only: false,
        as_conversations: false,
    }
}

fn what_a_search_shows(db: &SeededIndex, typed: &str) -> Vec<i64> {
    list(db)
        .get_view_window(&looking_for(typed), 0, 50)
        .expect("window is readable")
        .rows
        .iter()
        .map(|row| row.message_id.get() as i64)
        .collect()
}

#[test]
fn a_search_finds_a_message_by_a_word_of_its_subject() {
    let db = seeded("search-by-subject", 6);
    let one = list(&db)
        .get_view_window(&whole_folder(), 0, 6)
        .expect("window is readable")
        .rows[0]
        .message_id;
    db.say(one.get(), "Quarterly report for the board");

    assert_eq!(
        what_a_search_shows(&db, "quarterly"),
        vec![one.get() as i64]
    );
}

#[test]
fn a_search_answers_while_the_word_is_still_being_typed() {
    // A search that waits for a whole word answers after somebody has stopped looking.
    let db = seeded("search-as-typed", 6);
    let one = list(&db)
        .get_view_window(&whole_folder(), 0, 6)
        .expect("window is readable")
        .rows[0]
        .message_id;
    db.say(one.get(), "Quarterly report for the board");

    assert_eq!(what_a_search_shows(&db, "quar"), vec![one.get() as i64]);
}

#[test]
fn a_search_finds_mail_that_was_sent_as_well_as_mail_that_arrived() {
    // The difference between a search and every other view. The account and device views show
    // what arrived, because a row that also held drafts and sent mail would mean nothing. What
    // somebody looks for is as likely to be something they wrote.
    let db = seeded("search-sent", 6);
    db.add_folder(2, "Sent", FolderRole::Sent);
    let one = list(&db)
        .get_view_window(&whole_folder(), 0, 6)
        .expect("window is readable")
        .rows[0]
        .message_id;
    db.say(one.get(), "Quarterly report for the board");
    db.only_in(one.get(), 2);

    let across_the_device = list(&db)
        .get_view_window(
            &ViewQuery {
                scope: ViewScope::Everything,
                unread_only: false,
                as_conversations: false,
            },
            0,
            50,
        )
        .expect("window is readable")
        .rows;
    assert!(
        !across_the_device.iter().any(|row| row.message_id == one),
        "mail that never arrived is not in a view of what arrived"
    );

    assert_eq!(
        what_a_search_shows(&db, "quarterly"),
        vec![one.get() as i64],
        "and a search finds it anyway"
    );
}

#[test]
fn a_search_that_matches_nothing_answers_with_no_rows() {
    let db = seeded("search-nothing", 6);
    let window = list(&db)
        .get_view_window(&looking_for("nothingatallmatchesthis"), 0, 50)
        .expect("window is readable");

    assert!(window.rows.is_empty());
    assert_eq!(window.total, 0);
}

#[test]
fn a_search_counts_what_it_shows() {
    let db = seeded("search-total", 8);
    let window = list(&db)
        .get_view_window(&looking_for("message"), 0, 3)
        .expect("window is readable");

    assert_eq!(window.rows.len(), 3, "a page of it");
    assert_eq!(window.total, 8, "and how much there is");
}

#[test]
fn what_the_index_would_read_as_an_operator_is_only_something_typed() {
    // Everything here is a syntax error or an operator in the index's own language. A search
    // box that answers one of them with an error is a search box that does not work, and text
    // going into a query language as itself is the shape of a whole class of defect.
    let db = seeded("search-operators", 4);
    let one = list(&db)
        .get_view_window(&whole_folder(), 0, 4)
        .expect("window is readable")
        .rows[0]
        .message_id;
    db.say(one.get(), "Lunch with sigma@example.org");

    for typed in [
        "\"",
        "a\" OR \"b",
        "(",
        "*",
        "^lunch",
        "subject:lunch",
        "-lunch",
        "NEAR",
        "lunch AND NOT lunch",
    ] {
        list(&db)
            .get_view_window(&looking_for(typed), 0, 10)
            .unwrap_or_else(|error| panic!("typing {typed:?} answered with {error:?}"));
    }

    assert_eq!(
        what_a_search_shows(&db, "sigma@example.org"),
        vec![one.get() as i64],
        "an address is found as the words it is stored as"
    );
}

#[test]
fn a_row_names_the_folder_it_stands_for() {
    // What anything done to the row is done in. Without it the interface would have to guess
    // which copy of a message it is looking at.
    let db = seeded("row-folder", 3);
    let window = list(&db)
        .get_view_window(&whole_folder(), 0, 3)
        .expect("window is readable");

    for row in &window.rows {
        assert_eq!(row.folder_id, Some(folder()));
    }
}

#[test]
fn a_message_marked_read_shows_read_before_the_server_has_heard() {
    // The click is written down at once and the server told later. Showing the index's answer
    // in between would mean a click that visibly does nothing, which is exactly what an
    // offline mail client must not look like.
    let db = seeded("optimistic-read", 10);
    let before = list(&db)
        .get_view_window(&unread(), 0, 10)
        .expect("window is readable");
    let unread = before.rows.first().expect("the seed leaves some unread");

    db.asked_to_mark(unread.message_id.get(), folder().get(), true);

    let after = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");
    let same = after
        .rows
        .iter()
        .find(|row| row.message_id == unread.message_id)
        .expect("still in the list");

    assert!(!same.unread, "the list still showed it unread");
}

#[test]
fn a_message_marked_unread_shows_unread_the_same_way() {
    let db = seeded("optimistic-unread", 10);
    let all = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");
    let read = all
        .rows
        .iter()
        .find(|row| !row.unread)
        .expect("the seed leaves some read");

    db.asked_to_mark(read.message_id.get(), folder().get(), false);

    let after = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");
    let same = after
        .rows
        .iter()
        .find(|row| row.message_id == read.message_id)
        .expect("still in the list");

    assert!(same.unread);
}

#[test]
fn an_instruction_about_another_copy_does_not_change_this_one() {
    // A message sits in more than one folder, and marking it read in one is not marking it
    // read in the other.
    let db = seeded("optimistic-elsewhere", 10);
    let before = list(&db)
        .get_view_window(&unread(), 0, 10)
        .expect("window is readable");
    let unread = before.rows.first().expect("the seed leaves some unread");

    db.asked_to_mark(unread.message_id.get(), 999, true);

    let after = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");
    let same = after
        .rows
        .iter()
        .find(|row| row.message_id == unread.message_id)
        .expect("still in the list");

    assert!(same.unread, "another folder's instruction changed this row");
}

#[test]
fn an_instruction_that_has_been_carried_out_is_not_laid_over_anything() {
    // Once the server knows, the index is the truth. Keeping the overlay would show a state
    // that has already stopped being pending, and would never stop showing it.
    let db = seeded("optimistic-done", 10);
    let before = list(&db)
        .get_view_window(&unread(), 0, 10)
        .expect("window is readable");
    let unread = before.rows.first().expect("the seed leaves some unread");

    db.asked_to_mark(unread.message_id.get(), folder().get(), true);
    db.everything_reached_the_server();

    let after = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");
    let same = after
        .rows
        .iter()
        .find(|row| row.message_id == unread.message_id)
        .expect("still in the list");

    assert!(same.unread, "a finished instruction was still being shown");
}

#[test]
fn an_instruction_that_stopped_is_shown_on_the_row_it_was_about() {
    // The application promises that what it could not carry out reaches a person rather than
    // being dropped. This is where the promise is kept.
    let db = seeded("stopped", 5);
    let one = list(&db)
        .get_view_window(&whole_folder(), 0, 5)
        .expect("window is readable")
        .rows[0]
        .message_id;

    db.stopped_asking(one.get(), folder().get(), "messageIsNotThere");

    let rows = list(&db)
        .get_view_window(&whole_folder(), 0, 5)
        .expect("window is readable")
        .rows;

    let same = rows
        .iter()
        .find(|row| row.message_id == one)
        .expect("still in the list");

    assert_eq!(same.stopped, Some(Stopped::MessageIsNotThere));
    assert!(
        rows.iter().filter(|row| row.stopped.is_some()).count() == 1,
        "a stop was shown on rows it was not about"
    );
}

#[test]
fn a_reason_this_build_has_no_words_for_is_still_shown_as_a_stop() {
    // Written by a newer version. Saying nothing would hide an instruction a person has to
    // decide about; saying the wrong specific thing would send them somewhere pointless.
    let db = seeded("stopped-unknown", 3);
    let one = list(&db)
        .get_view_window(&whole_folder(), 0, 3)
        .expect("window is readable")
        .rows[0]
        .message_id;

    db.stopped_asking(one.get(), folder().get(), "some-newer-reason");

    let rows = list(&db)
        .get_view_window(&whole_folder(), 0, 3)
        .expect("window is readable")
        .rows;

    assert_eq!(rows[0].stopped, Some(Stopped::TheServerRefused));
}

#[test]
fn a_message_told_to_go_elsewhere_is_shown_on_its_way_out() {
    let db = seeded("on-its-way", 6);
    let rows = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows;
    let one = rows.first().expect("a message").message_id;

    db.asked_to_file(one.get(), folder().get(), folder().get() + 1);

    let rows = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows;

    let leaving: Vec<_> = rows.iter().filter(|row| row.leaving).collect();
    assert_eq!(leaving.len(), 1, "only the one that was asked about");
    assert_eq!(leaving[0].message_id, one);
    assert_eq!(
        rows.len(),
        6,
        "and it keeps its place, because the list is paged by position"
    );
}

#[test]
fn a_message_told_to_go_somewhere_is_shown_there_before_any_pass_moves_it() {
    // The other half of being on its way out. Marked as leaving the folder it is in and not yet
    // written into the one it was sent to, a message was in neither list until a pass got round to
    // both -- gone from one, absent from the other, which reads as a message that was lost.
    let db = seeded("arriving", 6);
    let elsewhere = FolderId::new(folder().get() + 1).expect("non-zero");
    db.add_folder(
        elsewhere.get() as i64,
        "Junk",
        lyonoyl_domain::folder::FolderRole::Junk,
    );
    let into = ViewQuery {
        scope: ViewScope::Folder(elsewhere),
        unread_only: false,
        as_conversations: false,
    };

    let before = list(&db)
        .get_view_window(&into, 0, 10)
        .expect("window is readable");
    assert_eq!(before.rows.len(), 0, "nothing is there to begin with");

    let one = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows
        .first()
        .expect("a message")
        .message_id;

    db.asked_to_file(one.get(), folder().get(), elsewhere.get());

    let after = list(&db)
        .get_view_window(&into, 0, 10)
        .expect("window is readable");

    assert_eq!(
        after.rows.len(),
        1,
        "it is not in the folder it was sent to"
    );
    assert_eq!(after.rows[0].message_id, one);
    assert!(
        after.rows[0].arriving,
        "shown as though it had arrived, which is a claim about the server"
    );
    assert_eq!(
        after.rows[0].folder_id,
        Some(elsewhere),
        "anything else done to it must be done where it is going"
    );
    assert_eq!(after.total, 1, "and it is counted, or the paging is short");
}

#[test]
fn a_message_a_pass_has_already_moved_is_not_shown_twice() {
    // Between the instruction and the pass it is in one list; after the pass it is in the other.
    // Counted in both, one message would be two rows.
    let db = seeded("arrived-already", 6);
    let one = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows
        .first()
        .expect("a message")
        .message_id;

    // Told to go where it already is: the pass has been and the queue has not been cleared yet.
    db.asked_to_file(one.get(), folder().get(), folder().get());

    let rows = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows;

    assert_eq!(
        rows.iter().filter(|row| row.message_id == one).count(),
        1,
        "one message, two rows"
    );
}

#[test]
fn a_message_thrown_away_is_shown_on_its_way_out_too() {
    let db = seeded("thrown-away", 6);
    let rows = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows;
    let one = rows.first().expect("a message").message_id;

    db.asked_to_throw_away(one.get(), folder().get());

    let rows = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows;

    let row = rows
        .iter()
        .find(|row| row.message_id == one)
        .expect("the row is still there");
    assert!(row.leaving);
}

#[test]
fn a_message_that_stopped_on_its_way_is_not_shown_as_still_going() {
    let db = seeded("stopped-going", 6);
    let rows = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows;
    let one = rows.first().expect("a message").message_id;

    db.asked_to_file(one.get(), folder().get(), folder().get() + 1);
    db.stopped_asking(one.get(), folder().get(), "messageIsNotThere");

    let rows = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows;
    let row = rows
        .iter()
        .find(|row| row.message_id == one)
        .expect("the row");

    assert_eq!(row.stopped, Some(Stopped::MessageIsNotThere));
}

#[test]
fn a_message_nobody_touched_is_not_shown_as_going_anywhere() {
    let db = seeded("untouched", 6);

    let rows = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows;

    assert!(rows.iter().all(|row| !row.leaving));
}

#[test]
fn something_still_on_its_way_is_not_shown_as_stopped() {
    // Ten, because the generator marks every fifth message unread and three of them may hold
    // none at all.
    let db = seeded("stopped-not-yet", 10);
    let before = list(&db)
        .get_view_window(&unread(), 0, 10)
        .expect("window is readable");
    let one = before.rows.first().expect("some are unread").message_id;

    db.asked_to_mark(one.get(), folder().get(), true);

    let rows = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows;

    assert!(rows.iter().all(|row| row.stopped.is_none()));
}

#[test]
fn a_message_the_inbox_holds_twice_reads_the_same_wherever_it_is_shown() {
    // The flag belongs to the copy, and so does the arrival time. Letting each view speak for
    // whichever copy its grouping happened to keep made the same message read in the folder and
    // unread beside the account, and put it in a different place in each.
    let db = seeded("two-copies-two-views", 5);
    let inbox = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");
    let twice = inbox.rows[2].message_id;

    // The copy already there is read; a later one arrives that nobody has opened.
    db.every_copy_read(folder().get(), twice.get());
    db.another_copy_of(folder().get(), twice.get(), 100_000);

    let by_folder = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");
    let by_account = list(&db)
        .get_view_window(&the_account(), 0, 10)
        .expect("window is readable");

    let ids = |window: &ViewWindow| -> Vec<u64> {
        window.rows.iter().map(|row| row.message_id.get()).collect()
    };
    assert_eq!(
        ids(&by_folder),
        ids(&by_account),
        "the same mail in a different order is the same mail nobody can find"
    );

    let unread = |window: &ViewWindow| -> bool {
        window
            .rows
            .iter()
            .find(|row| row.message_id == twice)
            .expect("the message")
            .unread
    };
    assert_eq!(unread(&by_folder), unread(&by_account));
    assert!(
        unread(&by_folder),
        "a copy nobody has opened is unread mail, whatever the other copy says"
    );
}

#[test]
fn a_message_a_folder_holds_twice_is_one_row_and_counted_once() {
    // A server keeps two copies apart by number; the same `Message-ID` makes them one message
    // here. Two rows would open the same thing, and the count beside the folder would say
    // there is more mail than there is.
    let db = seeded("two-copies", 5);
    let before = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");

    db.a_second_copy_of(folder().get(), before.rows[0].message_id.get());

    let after = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");

    assert_eq!(
        after.rows.len(),
        before.rows.len(),
        "the message showed twice"
    );
    assert_eq!(after.total, before.total, "it was counted twice");

    let same: Vec<_> = after
        .rows
        .iter()
        .filter(|row| row.message_id == before.rows[0].message_id)
        .collect();
    assert_eq!(same.len(), 1);
}

#[test]
fn the_newest_message_comes_first() {
    let db = seeded("order", 10);
    let window = list(&db)
        .get_view_window(&whole_folder(), 0, 3)
        .expect("window is readable");

    assert_eq!(window.rows.len(), 3);
    assert!(
        window.rows[0].received_at > window.rows[1].received_at,
        "the list opens on the newest message"
    );
    assert_eq!(window.total, 10);
}

#[test]
fn a_window_reads_the_positions_it_was_asked_for() {
    let db = seeded("offset", 10);
    let all = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");
    let middle = list(&db)
        .get_view_window(&whole_folder(), 4, 2)
        .expect("window is readable");

    assert_eq!(middle.rows.len(), 2);
    assert_eq!(middle.rows[0], all.rows[4]);
    assert_eq!(middle.rows[1], all.rows[5]);
    assert_eq!(middle.total, 10, "the total counts the set, not the window");
}

#[test]
fn a_window_past_the_end_is_empty_but_still_reports_the_total() {
    let db = seeded("past-end", 10);
    let window = list(&db)
        .get_view_window(&whole_folder(), 100, 50)
        .expect("window is readable");

    assert!(window.rows.is_empty());
    assert_eq!(window.total, 10);
}

#[test]
fn an_empty_folder_reads_as_an_empty_list() {
    let db = seeded("empty", 0);
    let window = list(&db)
        .get_view_window(&whole_folder(), 0, 50)
        .expect("window is readable");

    assert!(window.rows.is_empty());
    assert_eq!(window.total, 0);
}

#[test]
fn the_unread_filter_narrows_the_rows_and_the_total_together() {
    // The generator marks every fifth message unread.
    let db = seeded("unread", 100);
    let window = list(&db)
        .get_view_window(&unread(), 0, 50)
        .expect("window is readable");

    assert_eq!(window.total, 20);
    assert_eq!(window.rows.len(), 20);
    assert!(window.rows.iter().all(|row| row.unread));
}

#[test]
fn the_epoch_moves_when_the_set_changes() {
    let db = seeded("epoch", 10);
    let before = list(&db)
        .get_view_window(&whole_folder(), 0, 1)
        .expect("window is readable")
        .view_epoch;

    let provider = ScriptedProvider::new(vec![Ok(batch(
        folder(),
        UidValidity::new(7).expect("non-zero"),
        11,
        5,
        Some("1"),
        "2",
    ))]);
    SyncFolder::new(provider, SqliteMailIndex::new(db.pool.clone()))
        .execute(folder())
        .expect("the pass applies");

    let after = list(&db)
        .get_view_window(&whole_folder(), 0, 1)
        .expect("window is readable");

    assert!(
        after.view_epoch > before,
        "arrivals move the epoch: {before} then {}",
        after.view_epoch
    );
    assert_eq!(after.total, 15);
}

#[test]
fn a_range_returns_exactly_the_positions_it_covers() {
    let db = seeded("range", 10);
    let all = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");

    let ids = list(&db)
        .get_view_ids_in_range(&whole_folder(), 2, 5)
        .expect("range is readable");

    assert_eq!(ids.len(), 4);
    assert_eq!(
        ids,
        all.rows[2..=5]
            .iter()
            .map(|row| row.message_id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_range_a_folder_is_emptied_by_holds_every_message() {
    // `empty_folder` asks for `0..=u32::MAX`. Counting that span as `to - from + 1` wraps to zero,
    // which is a `LIMIT 0`: the folder reports success and keeps every message. Named here as the
    // range the command actually sends rather than as a large number.
    let db = seeded("emptying", 10);

    let held = list(&db)
        .get_view_ids_in_range(&whole_folder(), 0, u32::MAX)
        .expect("the range is readable");

    assert_eq!(
        held.len(),
        10,
        "the whole folder, not a wrapped-around nothing"
    );
}

#[test]
fn emptying_a_folder_takes_what_it_shows_rather_than_what_the_index_holds() {
    // A message rescued from the trash and not yet read into the inbox is still held in the trash
    // by the index. Emptied with the rest, its way home is folded into a delete for good, and the
    // message the person had just saved is erased on the server.
    let db = seeded("emptying-as-shown", 4);
    let inbox = FolderId::new(folder().get() + 1).expect("non-zero");
    db.add_folder(inbox.get() as i64, "Kept", FolderRole::Custom);

    let held = list(&db)
        .get_view_ids_in_range(&whole_folder(), 0, u32::MAX)
        .expect("the range is readable");
    let rescued = held[0];
    db.asked_to_file(rescued.get(), folder().get(), inbox.get());

    let taken = list(&db).what_emptying_takes(folder()).expect("readable");

    assert!(
        !taken.contains(&rescued),
        "the message on its way out of the folder is emptied with it"
    );
    assert_eq!(taken.len(), 3, "and every other message it holds is");
}

#[test]
fn emptying_a_folder_takes_what_is_on_its_way_into_it() {
    // Thrown away a moment ago and not yet read into the trash: the trash shows it, and a person
    // emptying the trash has emptied it of that too.
    let db = seeded("emptying-arriving", 3);
    let trash = FolderId::new(folder().get() + 1).expect("non-zero");
    db.add_folder(trash.get() as i64, "Trash", FolderRole::Trash);

    let thrown = list(&db)
        .get_view_ids_in_range(&whole_folder(), 0, u32::MAX)
        .expect("the range is readable")[0];
    db.asked_to_file(thrown.get(), folder().get(), trash.get());

    let taken = list(&db).what_emptying_takes(trash).expect("readable");

    assert_eq!(taken, vec![thrown]);
}

#[test]
fn a_backwards_range_is_empty_rather_than_an_error() {
    let db = seeded("backwards", 10);
    assert!(list(&db)
        .get_view_ids_in_range(&whole_folder(), 5, 2)
        .expect("range is readable")
        .is_empty());
}

/// The account, asked for the way the window asks for it.
fn the_account_as_the_window_asks() -> ViewQuery {
    ViewQuery {
        scope: ViewScope::Account(1),
        unread_only: false,
        as_conversations: true,
    }
}

#[test]
fn the_account_answers_with_the_inbox_and_not_with_the_bin() {
    // What the row above an account's folders means, and it is not "everything this account
    // holds": mail in the bin and mail in the junk are in the same file and are not in it. What
    // keeps them out is the date a message arrived in an inbox, which mail that never reached one
    // does not have -- so the rule is one column rather than a list of folders to exclude, and a
    // folder somebody invents later is covered by it without anybody remembering to.
    let db = seeded("account-bin", 4);
    db.add_folder(3, "Trash", FolderRole::Trash);
    db.add_folder(4, "Junk", FolderRole::Junk);
    db.fill_folder(FolderId::new(3).expect("non-zero"), 300, 3);
    db.fill_folder(FolderId::new(4).expect("non-zero"), 400, 2);

    let connection = db.pool.get().expect("connection");
    let held: i64 = connection
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .expect("counted");
    assert_eq!(held, 9, "the bin and the junk really are in the file");

    let shown = list(&db)
        .get_view_window(&the_account_as_the_window_asks(), 0, 50)
        .expect("readable")
        .rows
        .len();

    assert_eq!(shown, 4, "the four that reached the inbox, and those only");
}

#[test]
fn a_message_taken_out_of_the_inbox_leaves_the_account_row_too() {
    // The two rows a person compares: the account and the inbox under it. Archiving takes a
    // message out of one, and it has to take it out of the other, or the account row goes on
    // showing mail that is no longer in the inbox and the two lists disagree for good.
    let db = seeded("account-archived", 4);
    db.add_folder(2, "Archive", FolderRole::Archive);

    db.move_newest_message(2);

    let account = list(&db)
        .get_view_window(&the_account_as_the_window_asks(), 0, 50)
        .expect("readable")
        .rows
        .len();
    let inbox = list(&db)
        .get_view_window(&whole_folder(), 0, 50)
        .expect("readable")
        .rows
        .len();

    assert_eq!((inbox, account), (3, 3), "both rows lost the same message");
}

fn account(number: i64) -> ViewQuery {
    ViewQuery {
        scope: ViewScope::Account(number),
        unread_only: false,
        as_conversations: false,
    }
}

fn messages_of(_db: &SeededIndex) -> ViewQuery {
    ViewQuery {
        scope: ViewScope::Everything,
        unread_only: false,
        as_conversations: false,
    }
}

fn conversations_of(_db: &SeededIndex) -> ViewQuery {
    ViewQuery {
        scope: ViewScope::Everything,
        unread_only: false,
        as_conversations: true,
    }
}

fn everywhere() -> ViewQuery {
    ViewQuery {
        scope: ViewScope::Everything,
        unread_only: false,
        as_conversations: false,
    }
}

/// Two accounts: the seeded one with an inbox and an archive, and a second with its own inbox.
fn two_accounts(name: &str) -> SeededIndex {
    let db = seeded(name, 10);
    db.add_folder(2, "Archive", FolderRole::Archive);
    db.fill_folder(FolderId::new(2).expect("non-zero"), 100, 4);

    db.add_folder_of_another_account(9, 2);
    db.fill_folder(FolderId::new(9).expect("non-zero"), 200, 6);

    db
}

/// A search reaches every account's mail, and there is more than one file to reach into.
///
/// A search belongs to no folder and no account, so no single file holds its answer: it has to be
/// asked of all of them and their answers added up. Whether that happens turns on one function
/// saying a search spans accounts — and if it stopped saying so, a search would look for a file to
/// address, find none, and every search on a device with two mailboxes would answer with an error
/// rather than with mail.
///
/// Nothing said so. The whole suite stayed green with that function changed, because every other
/// test that puts two files on the device asks about a folder, an account or everything.
#[test]
fn a_search_reaches_the_mail_of_every_account() {
    let first = seeded("searching-two-files-one", 6);
    let second = seeded("searching-two-files-two", 4);

    let list = ListReadModel::spanning(
        vec![(1, first.pool.clone()), (7, second.pool.clone())],
        first.profile.clone(),
    );

    // Every message the generator writes is called "Message <uid>", so this matches all of both.
    let found = list
        .get_view_window(
            &ViewQuery {
                scope: ViewScope::Looking("message".to_owned()),
                unread_only: false,
                as_conversations: false,
            },
            0,
            50,
        )
        .expect("a search answers on a device with two accounts");

    assert_eq!(
        found.total, 10,
        "a search that reaches one file reaches half the mail"
    );
    assert_eq!(found.rows.len(), 10);
    assert!(
        found.not_answering.is_empty(),
        "both files answered, so neither is named as silent"
    );
}

/// One account's file that will not open leaves the other accounts' mail readable.
///
/// This is the whole reason the mail was put in separate files. A window that failed because one
/// of them failed would be the single file all over again, with more steps — and a window that
/// quietly came back shorter would be worse than either, because nobody would know an account
/// was missing.
#[test]
fn a_file_that_will_not_open_costs_its_own_account_and_no_other() {
    let working = seeded("one-file-broken", 6);
    let broken = seeded("the-broken-one", 0);

    // A file that answers nothing, however it got that way.
    broken.stops_answering();

    let list = ListReadModel::spanning(
        vec![(1, working.pool.clone()), (7, broken.pool.clone())],
        working.profile.clone(),
    );

    let window = list
        .get_view_window(&everywhere(), 0, 50)
        .expect("the list still answers");

    assert!(
        !window.rows.is_empty(),
        "the mail of the account that is fine went missing with the one that is not"
    );
    assert_eq!(
        window.not_answering,
        vec![7],
        "the account whose file would not open is not named"
    );
}

/// A folder still answers once there is a second account on the device.
///
/// A folder belongs to one account, so one file holds it. Asking every file for it, or
/// refusing because there is more than one, both turn every click in the sidebar into an
/// error the moment somebody adds their second mailbox.
#[test]
fn a_folder_answers_with_a_second_account_on_the_device() {
    // Numbered the way each file numbers its own: a counter seeded at `account << 40`, so the
    // folder of account one and the folder of account seven are different numbers carrying
    // the account that minted them.
    let mine = FolderId::new((1u64 << 40) | 1).expect("non-zero");
    let theirs = FolderId::new((7u64 << 40) | 1).expect("non-zero");
    let ours = SeededIndex::with("a-folder-with-two-accounts", mine, 6);
    let others = SeededIndex::with("the-other-account", theirs, 4);

    let list = ListReadModel::spanning(
        vec![(1, ours.pool.clone()), (7, others.pool.clone())],
        ours.profile.clone(),
    );

    let count_in = |folder: FolderId| {
        list.get_view_window(
            &ViewQuery {
                scope: ViewScope::Folder(folder),
                unread_only: false,
                as_conversations: false,
            },
            0,
            50,
        )
        .expect("a folder is readable with a second account on the device")
        .total
    };

    // Each answers from its own file, which is the whole of the assertion: a folder that
    // answered from whichever file happened to be first would give one of these twice.
    assert_eq!(count_in(mine), 6);
    assert_eq!(count_in(theirs), 4);

    // An account is the same question asked another way, and takes the same turning.
    let theirs_as_well = FolderId::new((7u64 << 40) | 2).expect("non-zero");
    others.add_folder_of_another_account(theirs_as_well.get() as i64, 7);
    others.fill_folder(theirs_as_well, 500, 3);

    let whole_account = list
        .get_view_window(
            &ViewQuery {
                scope: ViewScope::Account(7),
                unread_only: false,
                as_conversations: false,
            },
            0,
            50,
        )
        .expect("an account is readable alongside another");
    assert_eq!(whole_account.total, 3);
}

/// Opening a conversation gives its messages, oldest first, ending with the one the list showed.
///
/// Forwards because that is how a conversation is read: the reply at the bottom is the one
/// already on screen, and what somebody opens it for is what came before.
#[test]
fn a_conversation_opens_oldest_first_and_ends_where_the_list_left_off() {
    let db = seeded("opening-a-conversation", 0);
    db.place_conversation(
        folder(),
        &[
            (11, 1_700_000_000),
            (12, 1_700_000_100),
            (13, 1_700_000_050),
        ],
        41,
    );
    db.place_conversation(folder(), &[(14, 1_700_000_200)], 42);

    let list = list(&db);
    let opened = list
        .conversation_of(MessageId::new(12).expect("non-zero"))
        .expect("readable");

    assert_eq!(
        opened
            .iter()
            .map(|row| row.message_id.get())
            .collect::<Vec<_>>(),
        vec![11, 13, 12],
        "a conversation does not open in the order it was written"
    );

    // Asked by any of its messages, not only the one the list showed.
    let from_the_oldest = list
        .conversation_of(MessageId::new(11).expect("non-zero"))
        .expect("readable");
    assert_eq!(from_the_oldest.len(), 3);

    // A message on its own is a conversation of one.
    let alone = list
        .conversation_of(MessageId::new(14).expect("non-zero"))
        .expect("readable");
    assert_eq!(alone.len(), 1);
}

/// A word from what a message says finds it, through the read model the application calls.
///
/// The index is one thing and the view is another: a search that matched in the index and then
/// asked the wrong question of `messages` would answer with nothing and look like a search that
/// found nothing.
#[test]
fn a_search_finds_a_message_by_a_word_it_opens_with() {
    let db = seeded("searching-what-a-message-says", 0);
    db.place_conversation(folder(), &[(11, 1_700_000_000)], 11);
    db.what_a_message_says(11, "Lunch", "bring the quarterly numbers");

    let looking = |typed: &str| {
        list(&db)
            .get_view_window(
                &ViewQuery {
                    scope: ViewScope::Looking(typed.to_owned()),
                    unread_only: false,
                    as_conversations: false,
                },
                0,
                50,
            )
            .expect("readable")
            .rows
            .iter()
            .map(|row| row.message_id.get())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        looking("quarterly"),
        vec![11],
        "a word from the text finds nothing"
    );
    assert_eq!(looking("Lunch"), vec![11], "the subject stopped finding it");
    assert!(
        looking("elsewhere").is_empty(),
        "a word nobody wrote finds a message"
    );
}

/// A row of conversations names the conversation, not only the message it happens to show.
///
/// The row shows the newest message, so a reply arriving makes it a different message — and an
/// interface holding a conversation open by what the row showed would lose it exactly when
/// something happened in it. The name it keeps is the earliest message of the thread.
#[test]
fn a_conversation_row_keeps_its_name_when_a_reply_makes_it_a_different_row() {
    let db = seeded("a-conversation-that-keeps-its-name", 0);
    db.place_conversation(folder(), &[(11, 1_700_000_000), (12, 1_700_000_100)], 11);

    let named = |list: &ListReadModel| {
        let window = list
            .get_view_window(&conversations_of(&db), 0, 50)
            .expect("readable");
        let row = window.rows.first().expect("one conversation").clone();
        (
            row.message_id.get(),
            row.conversation_id.map(|id| id.get()),
            row.in_conversation,
        )
    };

    let list = list(&db);
    assert_eq!(named(&list), (12, Some(11), 2));

    // The reply lands, and the row is now a different message under the same name.
    db.place_conversation(folder(), &[(13, 1_700_000_200)], 11);
    assert_eq!(named(&list), (13, Some(11), 3));
}

/// A conversation holding a message that reached no inbox opens like any other.
///
/// The reply somebody sent is in Sent and nowhere else, so it arrived in no inbox: the date it
/// arrived and the flag saying whether it was read both hold nothing. Read as a number and as
/// a flag, nothing is not a value — and it was the whole conversation that failed, not the one
/// message, which on screen is a row that opens and shuts again saying nothing.
#[test]
fn a_conversation_opens_though_one_of_its_messages_reached_no_inbox() {
    let db = seeded("a-conversation-holding-a-sent-reply", 0);
    db.add_folder(2, "Sent", FolderRole::Sent);
    db.place_conversation(folder(), &[(11, 1_700_000_000)], 41);
    db.place_message_outside_the_inbox(
        FolderId::new(2).expect("non-zero"),
        9_001,
        12,
        1_700_000_100,
        41,
    );

    let opened = list(&db)
        .conversation_of(MessageId::new(11).expect("non-zero"))
        .expect("a conversation holding a sent reply is readable");

    assert_eq!(
        opened
            .iter()
            .map(|row| row.message_id.get())
            .collect::<Vec<_>>(),
        vec![11, 12],
        "the sent reply is not in the conversation, or not after what it answers"
    );
    assert!(
        !opened[1].unread,
        "a message somebody sent is offered as unread mail"
    );
}

/// A conversation is one row, whatever it holds, and the row is its newest message.
///
/// Ten replies in one thread are one line, not ten pushing everything else off the screen.
/// The row shows the newest of them, because that is what a person is looking for when they
/// come back to a conversation.
#[test]
fn a_conversation_is_one_row_showing_its_newest_message() {
    let db = seeded("one-row-per-conversation", 0);
    let inbox = folder();

    // Four messages: three answering each other, one on its own.
    db.place_conversation(
        inbox,
        &[
            (11, 1_700_000_000),
            (12, 1_700_000_100),
            (13, 1_700_000_050),
        ],
        41,
    );
    db.place_conversation(inbox, &[(14, 1_700_000_200)], 42);

    let list = list(&db);
    let flat = list
        .get_view_window(&messages_of(&db), 0, 50)
        .expect("readable");
    let grouped = list
        .get_view_window(&conversations_of(&db), 0, 50)
        .expect("readable");

    assert_eq!(
        flat.rows.len(),
        4,
        "the fixture does not hold four messages"
    );
    assert_eq!(grouped.rows.len(), 2, "a conversation is not one row");
    assert_eq!(
        grouped.total, 2,
        "the length of the list is not the number of conversations"
    );

    let three = grouped
        .rows
        .iter()
        .find(|row| row.in_conversation == 3)
        .expect("the conversation of three");
    assert_eq!(
        three.message_id.get(),
        12,
        "the row shows a message that is not the newest of its conversation"
    );

    let alone = grouped
        .rows
        .iter()
        .find(|row| row.in_conversation == 1)
        .expect("the message on its own");
    assert_eq!(alone.message_id.get(), 14);
}

/// A window part-way down the list across accounts holds what walking to it reaches.
///
/// Written because a mutation went unnoticed: every window spanning accounts was read at the
/// top, so the merge could skip twice — once in each file and once again while merging — and
/// nothing said so. Depth is where a merge is wrong, and depth is what nothing asked for.
#[test]
fn a_window_part_way_down_the_list_across_accounts_holds_what_walking_reaches() {
    let db = two_accounts("everywhere-deep");
    let list = list(&db);

    // The whole list, read at the top, is what walking to any position in it means.
    let all = list
        .get_view_window(&everywhere(), 0, 200)
        .expect("readable");
    assert!(
        all.rows.len() > 12,
        "the fixture must be deep enough for a window to sit inside it"
    );

    for offset in [1u32, 3, 7, 12] {
        let window = list
            .get_view_window(&everywhere(), offset, 5)
            .expect("readable");

        let walked: Vec<_> = all
            .rows
            .iter()
            .skip(offset as usize)
            .take(5)
            .map(|row| row.message_id)
            .collect();
        let read: Vec<_> = window.rows.iter().map(|row| row.message_id).collect();

        assert_eq!(
            read, walked,
            "the window at {offset} is not what walking to it reaches"
        );
        assert_eq!(
            window.total, all.total,
            "the total moved between two reads of the same list"
        );
    }
}

#[test]
fn an_account_shows_the_mail_that_arrived_in_it() {
    // The account row and the inbox under it are the same mail. A row that also held drafts,
    // sent mail and the contents of the bin would be a different thing from the folder beneath
    // it, and the word "inbox" would stop meaning anything.
    let db = two_accounts("account-scope");

    let window = list(&db)
        .get_view_window(&account(1), 0, 100)
        .expect("window is readable");

    assert_eq!(
        window.total, 10,
        "the ten in the inbox, not the four archived"
    );
    assert_eq!(window.rows.len(), 10);

    let inbox = list(&db)
        .get_view_window(&whole_folder(), 0, 100)
        .expect("window is readable");
    assert_eq!(
        window
            .rows
            .iter()
            .map(|row| row.message_id)
            .collect::<Vec<_>>(),
        inbox
            .rows
            .iter()
            .map(|row| row.message_id)
            .collect::<Vec<_>>(),
        "the same mail, in the same order"
    );
}

#[test]
fn an_account_and_its_inbox_agree_on_what_has_been_read() {
    // IMAP keeps flags per mailbox, so what a person did to a message is recorded on its
    // placement. An account row reading `messages.flags` calls everything unread, and then
    // disagrees with the folder under it about the very same mail.
    let db = seeded("account-flags", 10);
    let store = SqliteMailIndex::new(db.pool.clone());

    let mut read_one = lyonoyl_domain::sync::FolderDelta {
        folder_id: folder(),
        uid_validity: UidValidity::new(7).expect("non-zero"),
        covered_from: None,
        stopped_short: false,
        arrived: Vec::new(),
        flag_updates: vec![lyonoyl_domain::sync::FlagUpdate {
            uid: lyonoyl_domain::message::Uid::new(5).expect("non-zero"),
            flags: lyonoyl_domain::message::Flags::SEEN,
            modseq: None,
        }],
        vanished: lyonoyl_domain::sync::VanishedReport::Unknown,
    };
    read_one.arrived.clear();

    store
        .apply(
            &read_one,
            &[],
            &lyonoyl_domain::sync::SyncCursor::new("10"),
            lyonoyl_domain::message::FlagScope::Placement,
        )
        .expect("the flag applies");

    let unread_of = |query: &ViewQuery| -> Vec<(u64, bool)> {
        list(&db)
            .get_view_window(query, 0, 100)
            .expect("window is readable")
            .rows
            .iter()
            .map(|row| (row.message_id.get(), row.unread))
            .collect()
    };

    assert_eq!(
        unread_of(&account(1)),
        unread_of(&whole_folder()),
        "the account row and the inbox describe the same mail the same way"
    );
    assert!(
        unread_of(&account(1)).iter().any(|(_, unread)| !unread),
        "and the one that was read shows as read"
    );
}

#[test]
fn a_message_kept_in_an_archive_as_well_still_counts_once() {
    let db = seeded("account-archived", 5);
    db.add_folder(2, "Archive", FolderRole::Archive);
    db.place_existing_message(2, 500, 1);

    let window = list(&db)
        .get_view_window(&account(1), 0, 100)
        .expect("window is readable");

    assert_eq!(window.total, 5);
}

#[test]
fn one_message_in_two_folders_is_one_row_for_the_account() {
    // Counting placements instead of messages would show a labelled message once per label,
    // and every position in the list after it would be wrong.
    let db = seeded("account-labels", 5);
    db.add_folder(2, "Archive", FolderRole::Archive);
    db.place_existing_message(2, 500, 1);

    let window = list(&db)
        .get_view_window(&account(1), 0, 100)
        .expect("window is readable");

    assert_eq!(window.total, 5);
    assert_eq!(window.rows.len(), 5);
}

#[test]
fn an_account_does_not_show_another_accounts_mail() {
    let db = two_accounts("account-isolation");

    let second = list(&db)
        .get_view_window(&account(2), 0, 100)
        .expect("window is readable");

    assert_eq!(second.total, 6, "only what arrived in the second account");
}

#[test]
fn everything_shows_the_mail_that_arrived_in_every_account() {
    let db = two_accounts("everything-scope");

    let window = list(&db)
        .get_view_window(&everywhere(), 0, 100)
        .expect("window is readable");

    assert_eq!(window.total, 16, "ten and six, both inboxes, no archive");
    assert_eq!(window.rows.len(), 16);
}

#[test]
fn the_wider_scopes_open_on_the_newest_message() {
    let db = two_accounts("wide-order");

    for query in [account(1), everywhere()] {
        let window = list(&db)
            .get_view_window(&query, 0, 5)
            .expect("window is readable");

        let dates: Vec<i64> = window.rows.iter().map(|row| row.received_at).collect();
        let mut sorted = dates.clone();
        sorted.sort_by(|left, right| right.cmp(left));
        assert_eq!(dates, sorted, "rows must come newest first");
    }
}

#[test]
fn the_unread_filter_works_at_every_scope() {
    // The generator marks every fifth message unread.
    let db = two_accounts("wide-unread");

    let unread_account = ViewQuery {
        scope: ViewScope::Account(1),
        unread_only: true,
        as_conversations: false,
    };
    let unread_everywhere = ViewQuery {
        scope: ViewScope::Everything,
        unread_only: true,
        as_conversations: false,
    };

    let account_window = list(&db)
        .get_view_window(&unread_account, 0, 100)
        .expect("window is readable");
    let everything_window = list(&db)
        .get_view_window(&unread_everywhere, 0, 100)
        .expect("window is readable");

    // Every subject the generator writes is "Message <uid>", so this matches the whole mailbox
    // and what is left is the filter alone.
    let unread_search = ViewQuery {
        scope: ViewScope::Looking("message".to_owned()),
        unread_only: true,
        as_conversations: false,
    };
    let every_search = ViewQuery {
        scope: ViewScope::Looking("message".to_owned()),
        unread_only: false,
        as_conversations: false,
    };
    let unread_folder = ViewQuery {
        scope: ViewScope::Folder(folder()),
        unread_only: true,
        as_conversations: false,
    };

    let search_window = list(&db)
        .get_view_window(&unread_search, 0, 100)
        .expect("window is readable");
    let whole_search = list(&db)
        .get_view_window(&every_search, 0, 100)
        .expect("window is readable");
    let folder_window = list(&db)
        .get_view_window(&unread_folder, 0, 100)
        .expect("window is readable");

    assert!(account_window.rows.iter().all(|row| row.unread));
    assert!(everything_window.rows.iter().all(|row| row.unread));
    assert!(folder_window.rows.iter().all(|row| row.unread));
    assert!(
        search_window.rows.iter().all(|row| row.unread),
        "a search that was asked for unread mail must not answer with read mail"
    );
    assert!(
        everything_window.total >= account_window.total,
        "the wider scope cannot hold less"
    );
    assert!(account_window.total > 0, "the fixture has unread mail");
    assert!(
        search_window.total > 0,
        "the fixture has unread mail to find"
    );
    assert!(
        search_window.total < whole_search.total,
        "the fixture has read mail too, or the filter is being asked to prove nothing"
    );
    assert_eq!(
        search_window.total as usize,
        search_window.rows.len(),
        "the length of the list is the length of what it shows"
    );
}

/// A search answers with messages, whatever the caller asked about conversations.
///
/// The command asks for conversations everywhere they can be answered, and for a search they
/// deliberately cannot: one row standing for three matching messages hides which of them matched,
/// and finding that out means opening the row. A search is asked to name a message.
#[test]
fn a_search_answers_with_messages_rather_than_conversations() {
    let db = seeded("searching-is-not-conversations", 0);
    db.place_conversation(folder(), &[(11, 1_700_000_000), (12, 1_700_000_100)], 41);
    // Both of them match, which is the case grouping would hide: one row saying "2" where two
    // separate hits are what was asked for.
    db.what_a_message_says(11, "Invoice for March", "the total is attached");
    db.what_a_message_says(12, "Re: Invoice for March", "thank you");

    let found = list(&db)
        .get_view_window(
            &ViewQuery {
                scope: ViewScope::Looking("invoice".to_owned()),
                unread_only: false,
                as_conversations: true,
            },
            0,
            100,
        )
        .expect("window is readable");

    assert_eq!(found.total, 2, "both messages of the conversation are hits");
    assert_eq!(found.rows.len(), 2);
    assert!(
        found.rows.iter().all(|row| row.in_conversation == 1),
        "a hit stands for itself rather than for the conversation it is in"
    );
}

#[test]
fn a_change_in_a_quiet_folder_still_moves_the_epoch_of_the_scopes_above_it() {
    // Every folder counts its own changes. A maximum over them would sit at whatever the
    // busiest folder reached and never move for a change anywhere else, and the list would go
    // on showing rows that are no longer there.
    let db = seeded("wide-epoch", 5);
    let inbox = folder();
    let archive = FolderId::new(2).expect("non-zero");
    db.add_folder(2, "Archive", FolderRole::Archive);
    db.fill_folder(archive, 100, 1);

    // The inbox counts several changes; the archive stays at one.
    for pass in 0..3 {
        db.fill_folder(inbox, 500 + pass * 10, 2);
    }

    let epoch_of = |query: &ViewQuery| {
        list(&db)
            .get_view_window(query, 0, 1)
            .expect("window is readable")
            .view_epoch
    };

    let quiet_before = epoch_of(&ViewQuery {
        scope: ViewScope::Folder(archive),
        unread_only: false,
        as_conversations: false,
    });
    let busiest_before = epoch_of(&ViewQuery {
        scope: ViewScope::Folder(inbox),
        unread_only: false,
        as_conversations: false,
    });
    assert!(
        quiet_before < busiest_before,
        "the fixture needs a folder that is not the busiest: {quiet_before} and {busiest_before}"
    );

    let account_before = epoch_of(&account(1));
    let everything_before = epoch_of(&everywhere());

    db.fill_folder(archive, 900, 3);

    let busiest_after = epoch_of(&ViewQuery {
        scope: ViewScope::Folder(inbox),
        unread_only: false,
        as_conversations: false,
    });
    assert_eq!(
        busiest_after, busiest_before,
        "the busiest folder did not change, which is what makes this test worth having"
    );
    assert!(epoch_of(&account(1)) > account_before);
    assert!(epoch_of(&everywhere()) > everything_before);
}

#[test]
fn a_range_covers_the_same_positions_at_every_scope() {
    let db = two_accounts("wide-range");

    for query in [account(1), everywhere()] {
        let all = list(&db)
            .get_view_window(&query, 0, 100)
            .expect("window is readable");
        let ids = list(&db)
            .get_view_ids_in_range(&query, 1, 3)
            .expect("range is readable");

        assert_eq!(
            ids,
            all.rows[1..=3]
                .iter()
                .map(|row| row.message_id)
                .collect::<Vec<_>>()
        );
    }
}

/// A group shows the mail of the accounts in it, and of no others.
///
/// This is the whole of what a group does to the list, and it is worth pinning at three files
/// rather than two: with two, a narrowing that accidentally kept "everything but the last" would
/// look exactly like one that kept "the ones named".
#[test]
fn a_group_reads_the_accounts_in_it_and_no_others() {
    let first = seeded("group-window-one", 6);
    let second = seeded("group-window-two", 4);
    let third = seeded("group-window-three", 5);

    let list = ListReadModel::spanning(
        vec![
            (1, first.pool.clone()),
            (7, second.pool.clone()),
            (9, third.pool.clone()),
        ],
        first.profile.clone(),
    );

    let in_the_group = list
        .get_view_window(
            &ViewQuery {
                scope: ViewScope::Group(vec![1, 9]),
                unread_only: false,
                as_conversations: false,
            },
            0,
            50,
        )
        .expect("a group answers");

    let everywhere = list
        .get_view_window(&everywhere(), 0, 50)
        .expect("everything answers");

    assert_eq!(
        in_the_group.total, 11,
        "the group holds the first and third accounts' mail"
    );
    assert_eq!(in_the_group.rows.len(), 11);
    assert_eq!(
        everywhere.total, 15,
        "and the device holds four more that the group does not show"
    );
}

/// A group holding nothing shows nothing, rather than showing everything.
///
/// The difference matters because "no accounts named" and "no narrowing asked for" are one value
/// apart, and reading an empty set as "all of them" would show a person somebody else's mail under
/// a group they had just emptied.
#[test]
fn an_empty_group_shows_no_mail_rather_than_all_of_it() {
    let first = seeded("group-empty-one", 6);
    let second = seeded("group-empty-two", 4);

    let list = ListReadModel::spanning(
        vec![(1, first.pool.clone()), (7, second.pool.clone())],
        first.profile.clone(),
    );

    let empty = list
        .get_view_window(
            &ViewQuery {
                scope: ViewScope::Group(Vec::new()),
                unread_only: false,
                as_conversations: false,
            },
            0,
            50,
        )
        .expect("an empty group answers");

    assert_eq!(empty.total, 0, "a group with nothing in it holds no mail");
    assert!(empty.rows.is_empty());
}

/// A group naming an account this device no longer holds reads the rest.
///
/// Removing an account does not walk the arrangements it was in, so a group can name a file that
/// is not here. That is not a failure to report — the mailbox is gone and the group outlived it —
/// and a group that answered with an error would be a sidebar entry nobody could open again.
#[test]
fn a_group_naming_an_account_that_is_gone_shows_the_rest() {
    let first = seeded("group-gone-one", 6);

    let list = ListReadModel::spanning(vec![(1, first.pool.clone())], first.profile.clone());

    let found = list
        .get_view_window(
            &ViewQuery {
                scope: ViewScope::Group(vec![1, 4242]),
                unread_only: false,
                as_conversations: false,
            },
            0,
            50,
        )
        .expect("a group naming a missing account still answers");

    assert_eq!(found.total, 6, "the account that is here is read");
    assert!(
        found.not_answering.is_empty(),
        "an account that is gone is not a file that failed to answer"
    );
}

/// A group naming every account answers exactly what "everything" answers.
///
/// The compiler checks the exhaustive matches over the scope, and it checked them when this
/// variant was added. It does not check the six `_ =>` arms in the statement builder, and each of
/// those decides something — whether an account predicate is added, which parameter carries the
/// limit, whether the window seeks by day. A variant that slipped into the wrong one there would
/// be a list that is subtly not the list it claims to be, and nothing would say so.
///
/// So this asks the question from outside: over every shape the list is asked in, a group holding
/// all of them and everything on the device are the same list.
#[test]
fn a_group_of_every_account_is_the_same_list_as_everything() {
    let first = seeded("group-equals-everything-one", 8);
    let second = seeded("group-equals-everything-two", 5);

    let list = ListReadModel::spanning(
        vec![(1, first.pool.clone()), (7, second.pool.clone())],
        first.profile.clone(),
    );

    for unread_only in [false, true] {
        for as_conversations in [false, true] {
            let whole = list
                .get_view_window(
                    &ViewQuery {
                        scope: ViewScope::Everything,
                        unread_only,
                        as_conversations,
                    },
                    0,
                    50,
                )
                .expect("everything answers");

            let grouped = list
                .get_view_window(
                    &ViewQuery {
                        scope: ViewScope::Group(vec![1, 7]),
                        unread_only,
                        as_conversations,
                    },
                    0,
                    50,
                )
                .expect("a group of all of them answers");

            let shape = format!("unread_only={unread_only}, as_conversations={as_conversations}");

            assert_eq!(grouped.total, whole.total, "totals differ at {shape}");
            assert_eq!(
                grouped.rows.len(),
                whole.rows.len(),
                "row counts differ at {shape}"
            );
            assert_eq!(
                grouped
                    .rows
                    .iter()
                    .map(|row| row.message_id)
                    .collect::<Vec<_>>(),
                whole
                    .rows
                    .iter()
                    .map(|row| row.message_id)
                    .collect::<Vec<_>>(),
                "the same rows in the same order at {shape}"
            );

            // A fixture where every shape answers nothing would pass all of the above and prove
            // nothing at all.
            if !unread_only {
                assert!(
                    whole.total > 0,
                    "the fixture must have mail to compare at {shape}"
                );
            }
        }
    }
}

/// The folder a seeded message can be filed into, and the query that shows it.
fn a_second_folder(db: &SeededIndex, role: FolderRole) -> (FolderId, ViewQuery) {
    let elsewhere = FolderId::new(folder().get() + 1).expect("non-zero");
    db.add_folder(elsewhere.get() as i64, "Elsewhere", role);
    (
        elsewhere,
        ViewQuery {
            scope: ViewScope::Folder(elsewhere),
            unread_only: false,
            as_conversations: false,
        },
    )
}

fn the_first(db: &SeededIndex) -> MessageId {
    list(db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows
        .first()
        .expect("a message")
        .message_id
}

#[test]
fn a_move_the_server_carried_out_stays_shown_where_it_went_until_it_is_read_back() {
    // Pushed within a second of the click and read back a second later. In between, the queue
    // says "done" and the index still has the message where it was -- read from the index alone,
    // the row would slide back into the inbox and vanish from the archive for that second.
    let db = seeded("carried-not-read-back", 6);
    let (archive, into) = a_second_folder(&db, FolderRole::Archive);
    let one = the_first(&db);

    db.asked_to_file(one.get(), folder().get(), archive.get());
    db.carried_out_and_not_read_back();

    let left = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");
    let arrived = list(&db)
        .get_view_window(&into, 0, 10)
        .expect("window is readable");

    assert!(left
        .rows
        .iter()
        .find(|row| row.message_id == one)
        .is_some_and(|row| row.leaving));
    assert_eq!(arrived.rows.len(), 1);
    assert!(arrived.rows[0].arriving);
}

#[test]
fn a_move_that_has_been_read_back_is_the_index_alone() {
    let db = seeded("read-back", 6);
    let (archive, _) = a_second_folder(&db, FolderRole::Archive);
    let one = the_first(&db);

    db.asked_to_file(one.get(), folder().get(), archive.get());
    db.everything_reached_the_server();

    assert!(list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable")
        .rows
        .iter()
        .all(|row| !row.leaving));
}

#[test]
fn a_message_thrown_away_arrives_in_the_trash_before_any_pass_moves_it() {
    // The trash is the folder somebody opens to check. Shown there only once a pass has read
    // both folders, a message thrown away is in neither list in between.
    let db = seeded("into-the-trash", 6);
    let (_, trash) = a_second_folder(&db, FolderRole::Trash);
    let one = the_first(&db);

    db.asked_to_throw_away(one.get(), folder().get());

    let arrived = list(&db)
        .get_view_window(&trash, 0, 10)
        .expect("window is readable");

    assert_eq!(arrived.rows.len(), 1);
    assert_eq!(arrived.rows[0].message_id, one);
    assert!(arrived.rows[0].arriving);
}

#[test]
fn a_message_asked_back_before_its_move_was_read_back_is_home_again() {
    // Archived, carried out, and undone a second later. The way back is asked of the archive,
    // where the index does not have it yet -- and the inbox row must stop leaving at once.
    let db = seeded("asked-back", 6);
    let (archive, into) = a_second_folder(&db, FolderRole::Archive);
    let one = the_first(&db);

    db.asked_to_file(one.get(), folder().get(), archive.get());
    db.carried_out_and_not_read_back();
    db.asked_back(one.get(), archive.get(), folder().get());

    let home = list(&db)
        .get_view_window(&whole_folder(), 0, 10)
        .expect("window is readable");
    let away = list(&db)
        .get_view_window(&into, 0, 10)
        .expect("window is readable");

    assert!(home
        .rows
        .iter()
        .find(|row| row.message_id == one)
        .is_some_and(|row| !row.leaving));
    assert!(
        away.rows.is_empty(),
        "and it is not arriving in the archive"
    );
}

#[test]
fn a_message_read_on_its_way_arrives_read() {
    let db = seeded("read-then-filed", 10);
    let (archive, into) = a_second_folder(&db, FolderRole::Archive);
    // Message 5 is one the generator leaves unread.
    db.asked_to_mark(5, folder().get(), true);
    db.asked_to_file(5, folder().get(), archive.get());

    let arrived = list(&db)
        .get_view_window(&into, 0, 10)
        .expect("window is readable");

    assert_eq!(arrived.rows.len(), 1);
    assert!(!arrived.rows[0].unread);
}
