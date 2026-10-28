use super::*;
use crate::infrastructure::secure_storage::database_key::DatabaseKey;

#[test]
fn migration_set_is_valid() {
    profile_migrations()
        .validate()
        .expect("the profile migrations are internally consistent");

    // The larger ledger of the two, and the one nothing checked: twenty-four entries against
    // seventeen, and every account's mail behind them.
    index_migrations()
        .validate()
        .expect("the index migrations are internally consistent");
}

#[test]
fn applies_to_an_empty_database_and_is_idempotent() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");

    migrate_profile(&mut connection, ":memory:").expect("first run applies the schema");
    migrate_profile(&mut connection, ":memory:").expect("a second run is a no-op");

    let count: i64 = connection
        .query_row("SELECT count(*) FROM mail_accounts", [], |row| row.get(0))
        .expect("the table exists after migration");
    assert_eq!(count, 0);
}

/// A folder with two folders and a dozen messages, migrated and ready to be filled.
/// The schema while the search index held a subject and the addresses and nothing else.
const WHILE_THE_INDEX_HELD_NO_TEXT: usize = 22;

fn an_index_to_fill() -> Connection {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_index(&mut connection, ":memory:").expect("schema applies");
    fill_in_the_names(&connection);
    connection
}

fn fill_in_the_names(connection: &Connection) {
    connection
        .execute_batch(
            "INSERT INTO folders (id, account_id, name, role) VALUES (1, 1, 'INBOX', 1);
                 INSERT INTO folders (id, account_id, name, role) VALUES (2, 1, 'Archive', 6);",
        )
        .expect("two folders");

    for message in 1..=12 {
        connection
            .execute(
                "INSERT INTO messages (id, account_id, received_at) VALUES (?1, 1, 0)",
                [message],
            )
            .expect("a message");
    }
}

/// The day a second holds, counted the way the map counts it.
fn day(at: i64) -> i64 {
    at.div_euclid(86_400)
}

/// What the map says a folder holds, day by day, ignoring days it holds nothing on.
fn what_the_map_says(connection: &Connection, folder: i64) -> Vec<(i64, i64)> {
    connection
        .prepare(
            "SELECT day, messages FROM folder_days
                 WHERE folder_id = ?1 AND messages > 0 ORDER BY day",
        )
        .expect("prepares")
        .query_map([folder], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("readable")
        .map(|row| row.expect("row"))
        .collect()
}

/// The same thing worked out from the placements themselves: one message is one entry, filed
/// under the day of its newest copy in that folder.
fn what_the_folder_holds(connection: &Connection, folder: i64) -> Vec<(i64, i64)> {
    let newest: Vec<i64> = connection
        .prepare(
            "SELECT max(received_at) FROM message_placements
                 WHERE folder_id = ?1 GROUP BY message_id",
        )
        .expect("prepares")
        .query_map([folder], |row| row.get(0))
        .expect("readable")
        .map(|row| row.expect("row"))
        .collect();

    let mut counted: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    for at in newest {
        *counted.entry(day(at)).or_default() += 1;
    }

    counted.into_iter().collect()
}

/// What asking the placements directly would answer, worked out in Rust.
///
/// Deliberately not the expression the triggers run: an oracle sharing the code under test is
/// blind wherever that code is. Returns one row per message, newest table state.
fn what_the_placements_say(connection: &Connection) -> Vec<(i64, Option<i64>, Option<i64>)> {
    use std::collections::HashMap;

    let mut roles: HashMap<i64, i64> = HashMap::new();
    let mut folders = connection
        .prepare("SELECT id, role FROM folders")
        .expect("folders read");
    for row in folders
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .expect("folder rows")
    {
        let (id, role) = row.expect("a folder");
        roles.insert(id, role);
    }

    let mut copies: HashMap<i64, Vec<(i64, i64)>> = HashMap::new();
    let mut placements = connection
        .prepare("SELECT message_id, folder_id, flags FROM message_placements")
        .expect("placements read");
    for row in placements
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .expect("placement rows")
    {
        let (message, folder, flags) = row.expect("a placement");
        copies.entry(message).or_default().push((folder, flags));
    }

    let mut messages = connection
        .prepare("SELECT id FROM messages ORDER BY id")
        .expect("messages read");
    let ids: Vec<i64> = messages
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("message rows")
        .map(|row| row.expect("a message"))
        .collect();

    ids.into_iter()
        .map(|message| {
            let held = copies.get(&message).cloned().unwrap_or_default();
            let in_an_inbox: Vec<(i64, i64)> = held
                .iter()
                .copied()
                .filter(|(folder, _)| roles.get(folder).copied() == Some(1))
                .collect();

            let speaks_for_it = in_an_inbox
                .iter()
                .map(|(folder, _)| *folder)
                .min()
                .or_else(|| held.iter().map(|(folder, _)| *folder).min());

            let unread = in_an_inbox.iter().map(|(_, flags)| flags & 1).min();

            (message, speaks_for_it, unread)
        })
        .collect()
}

fn what_the_columns_hold(connection: &Connection) -> Vec<(i64, Option<i64>, Option<i64>)> {
    let mut held = connection
        .prepare("SELECT id, inbox_folder_id, inbox_unread FROM messages ORDER BY id")
        .expect("columns read");
    let rows = held
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })
        .expect("column rows")
        .map(|row| row.expect("a row"))
        .collect();
    rows
}

/// The columns migration 020 keeps must say what the placements say, after anything.
///
/// Placing, moving, reading, unreading, removing and a folder changing what it is for, in an
/// order fixed by a seed so a failure is reproducible. The check runs after every single step,
/// because a column that is right at the end and wrong in the middle is a list that showed the
/// wrong folder for as long as it took the next message to arrive.
#[test]
fn the_inbox_columns_follow_what_the_placements_say() {
    let connection = an_index_to_fill();
    connection
        .execute_batch(
            "INSERT INTO folders (id, account_id, name, role) VALUES (3, 1, 'Later', 0);",
        )
        .expect("a third folder");

    // Enough of each kind to be sure the sequence exercised it, counted rather than assumed.
    let mut placed = 0;
    let mut moved = 0;
    let mut flagged = 0;
    let mut removed = 0;
    let mut re_roled = 0;
    let mut uid = 1_000i64;
    let mut seed = 0x5eed_1234u64;
    let mut next = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (seed >> 33) as i64
    };

    for step in 0..400 {
        let message = (next().rem_euclid(12)) + 1;
        let folder = (next().rem_euclid(3)) + 1;

        match next().rem_euclid(10) {
            0..=3 => {
                uid += 1;
                if connection
                        .execute(
                            "INSERT INTO message_placements (folder_id, uid, message_id, received_at, flags)
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            [folder, uid, message, step * 3_600, next().rem_euclid(2)],
                        )
                        .is_ok()
                    {
                        placed += 1;
                    }
            }
            4..=5 => {
                uid += 1;
                let changed = connection
                    .execute(
                        "UPDATE message_placements SET folder_id = ?1, uid = ?2
                             WHERE (folder_id, uid) IN (
                                 SELECT folder_id, uid FROM message_placements
                                 WHERE message_id = ?3 AND folder_id <> ?1 LIMIT 1)",
                        [folder, uid, message],
                    )
                    .unwrap_or(0);
                moved += changed;
            }
            6..=7 => {
                let changed = connection
                    .execute(
                        "UPDATE message_placements SET flags = ?1 WHERE message_id = ?2",
                        [next().rem_euclid(2), message],
                    )
                    .expect("a flag change");
                flagged += changed;
            }
            8 => {
                let changed = connection
                    .execute(
                        "DELETE FROM message_placements
                             WHERE (folder_id, uid) IN (
                                 SELECT folder_id, uid FROM message_placements
                                 WHERE message_id = ?1 LIMIT 1)",
                        [message],
                    )
                    .expect("a removal");
                removed += changed;
            }
            _ => {
                let role = if next().rem_euclid(2) == 0 { 1 } else { 6 };
                let changed = connection
                    .execute(
                        "UPDATE folders SET role = ?1 WHERE id = ?2 AND role <> ?1",
                        [role, folder],
                    )
                    .expect("a role change");
                re_roled += changed;
            }
        }

        assert_eq!(
            what_the_columns_hold(&connection),
            what_the_placements_say(&connection),
            "the columns and the placements disagree after step {step}"
        );
    }

    assert!(placed > 0, "nothing was ever placed");
    assert!(moved > 0, "nothing was ever moved between folders");
    assert!(flagged > 0, "nothing was ever read or unread");
    assert!(removed > 0, "nothing was ever removed");
    assert!(re_roled > 0, "no folder ever changed what it is for");

    // The three answers the column has, put there on purpose rather than hoped for: a walk
    // that never produced one of them would pass against a column nobody maintained.
    connection
        .execute_batch(
            "UPDATE folders SET role = 1 WHERE id = 1;
                 UPDATE folders SET role = 6 WHERE id = 2;
                 UPDATE folders SET role = 0 WHERE id = 3;
                 DELETE FROM message_placements WHERE message_id IN (1, 2, 3);
                 INSERT INTO message_placements (folder_id, uid, message_id, received_at, flags)
                 VALUES (1, 90001, 1, 100, 0),
                        (1, 90002, 2, 100, 1),
                        (3, 90003, 3, 100, 0);",
        )
        .expect("a state with all three answers in it");

    let ending = what_the_columns_hold(&connection);
    assert_eq!(ending, what_the_placements_say(&connection));

    let answer = |message: i64| {
        ending
            .iter()
            .find(|(id, _, _)| *id == message)
            .copied()
            .expect("the message is there")
    };
    assert_eq!(answer(1), (1, Some(1), Some(0)), "unread in the inbox");
    assert_eq!(answer(2), (2, Some(1), Some(1)), "read in the inbox");
    assert_eq!(answer(3), (3, Some(3), None), "in no inbox at all");
}

/// One number, read straight out of the database.
fn a_number(connection: &Connection, sql: &str) -> i64 {
    connection
        .query_row(sql, [], |row| row.get(0))
        .expect("the number is readable")
}

fn place(connection: &Connection, folder: i64, uid: i64, message: i64, at: i64) {
    connection
        .execute(
            "INSERT INTO message_placements (folder_id, uid, message_id, received_at)
                 VALUES (?1, ?2, ?3, ?4)",
            [folder, uid, message, at],
        )
        .expect("a placement");
}

#[test]
fn the_map_of_a_folder_by_day_follows_what_is_in_it() {
    // Kept by the database, so a writer that forgets is not a way for the list to point at the
    // wrong place. What this asserts is the forgetting: rows are put in, moved and taken out
    // through plain SQL that knows nothing about the map.
    let connection = an_index_to_fill();

    place(&connection, 1, 10, 1, 86_400);
    place(&connection, 1, 11, 2, 86_400);
    place(&connection, 1, 12, 3, 172_800);
    assert_eq!(what_the_map_says(&connection, 1), vec![(1, 2), (2, 1)]);

    // One moves to another folder: it leaves one day and joins another.
    connection
        .execute(
            "UPDATE message_placements SET folder_id = 2 WHERE uid = 11",
            [],
        )
        .expect("it moves");
    assert_eq!(what_the_map_says(&connection, 1), vec![(1, 1), (2, 1)]);
    assert_eq!(what_the_map_says(&connection, 2), vec![(1, 1)]);

    // One is taken out. The day it was the last of counts nothing, and the row is left alone:
    // removing it costs a search of the whole map on every message deleted.
    connection
        .execute("DELETE FROM message_placements WHERE uid = 12", [])
        .expect("it goes");
    assert_eq!(what_the_map_says(&connection, 1), vec![(1, 1)]);

    let emptied: i64 = connection
        .query_row(
            "SELECT messages FROM folder_days WHERE folder_id = 1 AND day = 2",
            [],
            |row| row.get(0),
        )
        .expect("the day is still a row");
    assert_eq!(emptied, 0, "an emptied day counts nothing and stays");
}

#[test]
fn a_folder_that_holds_one_message_twice_counts_it_once() {
    // What the list shows is one row, dated by the newer copy. A map counting both copies
    // answers a position ahead of the row it names.
    let connection = an_index_to_fill();

    place(&connection, 1, 10, 1, 200 * 86_400);
    place(&connection, 1, 11, 1, 198 * 86_400);

    assert_eq!(
        what_the_map_says(&connection, 1),
        vec![(200, 1)],
        "one message, filed under its newer copy and nowhere else"
    );
}

#[test]
fn removing_the_newest_copy_files_the_message_under_the_older_one() {
    let connection = an_index_to_fill();

    place(&connection, 1, 10, 1, 200 * 86_400);
    place(&connection, 1, 11, 1, 198 * 86_400);

    connection
        .execute("DELETE FROM message_placements WHERE uid = 10", [])
        .expect("the newer copy goes");

    assert_eq!(what_the_map_says(&connection, 1), vec![(198, 1)]);
}

#[test]
fn removing_an_older_copy_leaves_the_message_where_it_was() {
    let connection = an_index_to_fill();

    place(&connection, 1, 10, 1, 200 * 86_400);
    place(&connection, 1, 11, 1, 198 * 86_400);

    connection
        .execute("DELETE FROM message_placements WHERE uid = 11", [])
        .expect("the older copy goes");

    assert_eq!(what_the_map_says(&connection, 1), vec![(200, 1)]);
}

#[test]
fn re_pointing_a_copy_to_another_message_is_seen() {
    // The only writer of this table re-points `message_id` when a uid it already holds turns
    // out to be a different message. A map that watched the folder and the date but not this
    // drifted, and nothing failed.
    let connection = an_index_to_fill();

    place(&connection, 1, 10, 1, 200 * 86_400);
    place(&connection, 1, 11, 1, 198 * 86_400);
    assert_eq!(what_the_map_says(&connection, 1), vec![(200, 1)]);

    connection
        .execute(
            "UPDATE message_placements SET message_id = 2 WHERE uid = 11",
            [],
        )
        .expect("the older copy turns out to be another message");

    assert_eq!(
        what_the_map_says(&connection, 1),
        vec![(198, 1), (200, 1)],
        "two messages now, each under its own day"
    );
}

#[test]
fn a_message_dated_before_1970_lands_in_its_own_day() {
    // Integer division truncates toward zero, which puts the last day before the epoch in the
    // same bucket as the first day after it -- and makes `day * 86400` not that bucket's first
    // second, which is arithmetic the list depends on to find a window.
    let connection = an_index_to_fill();

    place(&connection, 1, 10, 1, -1);
    place(&connection, 1, 11, 2, 1);

    assert_eq!(
        what_the_map_says(&connection, 1),
        vec![(-1, 1), (0, 1)],
        "the day before the epoch is a day of its own"
    );
}

#[test]
fn the_backfill_counts_a_message_a_folder_holds_twice_once() {
    // The one case a run of random changes cannot reach: it starts from an empty folder, so
    // the map is built empty whatever the backfill says and the triggers repair the rest. A
    // backfill counting copies ships silently, and every database already holding a duplicate
    // is wrong until its next write to that folder.
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    index_migrations()
        .to_version(&mut connection, WHILE_THE_MAP_COUNTED_COPIES)
        .expect("the schema while the map counted copies");
    fill_in_the_names(&connection);

    place(&connection, 1, 10, 1, 200 * 86_400);
    place(&connection, 1, 11, 1, 198 * 86_400);

    migrate_index(&mut connection, ":memory:").expect("the map is rebuilt");

    assert_eq!(
        what_the_map_says(&connection, 1),
        vec![(200, 1)],
        "a folder that already held a message twice is read as holding it once"
    );
}

#[test]
fn a_folder_emptied_by_a_uidvalidity_change_leaves_the_map_empty() {
    let connection = an_index_to_fill();

    for uid in 1..=12 {
        place(&connection, 1, uid, uid, uid * 86_400);
    }

    connection
        .execute("DELETE FROM message_placements WHERE folder_id = 1", [])
        .expect("the folder is emptied");

    assert_eq!(what_the_map_says(&connection, 1), Vec::new());
}

#[test]
fn a_folder_going_away_takes_its_days_with_it() {
    let connection = an_index_to_fill();
    connection
        .execute("PRAGMA foreign_keys = ON", [])
        .expect("cascades are on");

    place(&connection, 2, 10, 1, 200 * 86_400);
    assert_eq!(what_the_map_says(&connection, 2), vec![(200, 1)]);

    connection
        .execute("DELETE FROM folders WHERE id = 2", [])
        .expect("the folder goes");

    let left: i64 = connection
        .query_row(
            "SELECT count(*) FROM folder_days WHERE folder_id = 2",
            [],
            |row| row.get(0),
        )
        .expect("readable");
    assert_eq!(left, 0);
}

#[test]
fn the_map_follows_any_run_of_changes() {
    // The named tests above each fix one shape. This one takes the shapes it was not thought
    // of: copies arriving out of order, a copy re-pointed and then removed, a folder emptied
    // mid-run. After every single change the map must equal what the placements say.
    let mut seed: u64 = 0x5eed_1970_0827;
    let mut next = move |bound: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % bound) as i64
    };

    for _ in 0..40 {
        let connection = an_index_to_fill();
        let mut live: Vec<(i64, i64)> = Vec::new();

        for _ in 0..60 {
            let folder = next(2) + 1;
            let uid = next(20) + 1;
            let at = (next(400) - 3) * 86_400 + next(86_400);

            match next(10) {
                0..=4 => {
                    connection
                        .execute(
                            "INSERT INTO message_placements
                                     (folder_id, uid, message_id, received_at)
                                 VALUES (?1, ?2, ?3, ?4)
                                 ON CONFLICT(folder_id, uid) DO UPDATE SET
                                     message_id = excluded.message_id,
                                     received_at = excluded.received_at",
                            [folder, uid, next(12) + 1, at],
                        )
                        .expect("a placement arrives");
                    if !live.contains(&(folder, uid)) {
                        live.push((folder, uid));
                    }
                }
                5..=6 if !live.is_empty() => {
                    let (folder, uid) = live.remove((next(live.len() as u64)) as usize);
                    connection
                        .execute(
                            "DELETE FROM message_placements WHERE folder_id = ?1 AND uid = ?2",
                            [folder, uid],
                        )
                        .expect("a placement goes");
                }
                7..=8 if !live.is_empty() => {
                    let (folder, uid) = live[(next(live.len() as u64)) as usize];
                    connection
                        .execute(
                            "UPDATE message_placements SET received_at = ?3, message_id = ?4
                                 WHERE folder_id = ?1 AND uid = ?2",
                            [folder, uid, at, next(12) + 1],
                        )
                        .expect("a placement changes");
                }
                _ => {
                    connection
                        .execute(
                            "DELETE FROM message_placements WHERE folder_id = ?1",
                            [folder],
                        )
                        .expect("a folder is emptied");
                    live.retain(|(held, _)| *held != folder);
                }
            }

            for folder in 1..=2 {
                assert_eq!(
                    what_the_map_says(&connection, folder),
                    what_the_folder_holds(&connection, folder),
                    "the map and the folder disagree"
                );
            }
        }
    }
}

#[test]
fn emptying_a_folder_does_not_read_the_whole_map() {
    // A day whose count reaches zero used to be deleted, by a search with nothing to search
    // by: every message removed read every day of every folder. Emptying twenty thousand
    // messages took forty-four seconds. The bound here is wide because it is a wall clock on
    // somebody else's machine; what it forbids is three orders of magnitude away from it.
    let connection = an_index_to_fill();

    connection
        .execute_batch(
            "INSERT INTO messages (id, account_id, received_at)
                 WITH RECURSIVE n(i) AS (SELECT 13 UNION ALL SELECT i + 1 FROM n WHERE i < 45000)
                 SELECT i, 1, 0 FROM n;

                 INSERT INTO folders (id, account_id, name, role)
                 WITH RECURSIVE f(i) AS (SELECT 3 UNION ALL SELECT i + 1 FROM f WHERE i < 12)
                 SELECT i, 1, 'Kept ' || i, 6 FROM f;

                 -- Ten folders of ten years, none of which this test touches, so a search that
                 -- does not say which folder it means has something to walk.
                 INSERT INTO message_placements (folder_id, uid, message_id, received_at)
                 WITH RECURSIVE d(i) AS (SELECT 0 UNION ALL SELECT i + 1 FROM d WHERE i < 3649)
                 SELECT f.id, d.i + 1, 13 + (f.id - 3) * 3650 + d.i, d.i * 86400
                 FROM d, folders f WHERE f.id BETWEEN 3 AND 12;

                 -- One message per copy, which is what a folder normally holds: a message with
                 -- hundreds of copies would measure finding its newest one instead.
                 INSERT INTO message_placements (folder_id, uid, message_id, received_at)
                 WITH RECURSIVE u(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM u WHERE i < 5000)
                 SELECT 1, i, 39513 + i, (i % 400) * 86400 FROM u;",
        )
        .expect("ten folders of days and a folder to empty");

    let started = std::time::Instant::now();
    connection
        .execute("DELETE FROM message_placements WHERE folder_id = 1", [])
        .expect("the folder is emptied");
    let took = started.elapsed();

    let days: i64 = connection
        .query_row("SELECT count(*) FROM folder_days", [], |row| row.get(0))
        .expect("readable");
    assert!(
        days > 30_000,
        "the map has to be large for this to mean anything"
    );

    assert!(
        took < std::time::Duration::from_secs(2),
        "emptying five thousand messages took {took:?}, with {days} days in the map"
    );
}

/// Stores a message the way the index sees one, and hands back its identifier.
fn a_message_saying(
    connection: &Connection,
    subject: &str,
    sender: &str,
    address: &str,
    to: &str,
) -> i64 {
    connection
        .execute(
            "INSERT INTO messages
                     (account_id, subject, sender, sender_address, recipients_to, received_at)
                 VALUES (1, ?1, ?2, ?3, ?4, 0)",
            [subject, sender, address, to],
        )
        .expect("a message");
    connection.last_insert_rowid()
}

/// What searching for this finds, asked the way the application asks it.
fn found(connection: &Connection, typed: &str) -> Vec<i64> {
    let asked = lyonoyl_domain::search::asked_for(typed).expect("something was typed");
    connection
        .prepare("SELECT rowid FROM message_search WHERE message_search MATCH ?1")
        .expect("prepares")
        .query_map([asked], |row| row.get(0))
        .expect("readable")
        .map(|row| row.expect("row"))
        .collect()
}

/// Counting the matches in the index and counting them through the table give one number.
///
/// The list uses the first because it is six times faster — 41 ms against 261 at a hundred thousand
/// messages — and it may only do that while the two agree. They agree because the index is
/// contentless and its rows are the messages, which is a property of these triggers rather than of
/// anything the read model can see. So it is asserted here, where the triggers are, and asserted
/// through a message going and a subject changing rather than on a table nothing has touched.
#[test]
fn the_index_and_the_table_count_a_search_alike() {
    let connection = an_index_to_fill();

    let both_ways = |word: &str| -> (i64, i64) {
        let asked = lyonoyl_domain::search::asked_for(word).expect("something was typed");
        let in_the_index: i64 = connection
            .query_row(
                "SELECT count(*) FROM message_search WHERE message_search MATCH ?1",
                [&asked],
                |row| row.get(0),
            )
            .expect("the index counts");
        let through_the_table: i64 = connection
            .query_row(
                "SELECT count(*) FROM messages m
                 WHERE m.id IN (SELECT rowid FROM message_search WHERE message_search MATCH ?1)",
                [&asked],
                |row| row.get(0),
            )
            .expect("the table counts");
        (in_the_index, through_the_table)
    };

    let going = a_message_saying(
        &connection,
        "Quarterly report",
        "Kappa Vandel",
        "kappa@example.net",
        "you@x.test",
    );
    a_message_saying(
        &connection,
        "Quarterly figures",
        "Zeta Quorin",
        "zeta@example.com",
        "you@x.test",
    );
    let changing = a_message_saying(
        &connection,
        "Something else",
        "Sigma Ordrin",
        "sigma@example.org",
        "you@x.test",
    );

    assert_eq!(both_ways("quarterly"), (2, 2), "the two counts start apart");

    // A message that goes has to leave the index with it, or the index counts one that is not there.
    connection
        .execute("DELETE FROM messages WHERE id = ?1", [going])
        .expect("one message goes");
    assert_eq!(
        both_ways("quarterly"),
        (1, 1),
        "the index still counts a message that is gone"
    );

    // And a word that arrives has to arrive in both.
    connection
        .execute(
            "UPDATE messages SET subject = 'Quarterly kettle' WHERE id = ?1",
            [changing],
        )
        .expect("one subject changes");
    assert_eq!(
        both_ways("quarterly"),
        (2, 2),
        "a word added to a subject reaches one count and not the other"
    );
    assert_eq!(both_ways("kettle"), (1, 1), "the new word is not indexed");
}

#[test]
fn a_message_is_found_by_a_word_of_its_subject() {
    let connection = an_index_to_fill();
    let one = a_message_saying(
        &connection,
        "Quarterly report",
        "Kappa Vandel",
        "kappa@example.net",
        "you@x.test",
    );
    a_message_saying(
        &connection,
        "Lunch",
        "Sigma Ordrin",
        "sigma@example.org",
        "you@x.test",
    );

    assert_eq!(found(&connection, "quarterly"), vec![one]);
}

#[test]
fn a_message_is_found_by_who_sent_it_and_by_the_address() {
    let connection = an_index_to_fill();
    let one = a_message_saying(
        &connection,
        "Lunch",
        "Sigma Ordrin",
        "sigma@example.org",
        "you@x.test",
    );

    assert_eq!(found(&connection, "ordrin"), vec![one]);
    assert_eq!(found(&connection, "sigma@example.org"), vec![one]);
}

#[test]
fn the_two_spellings_of_yo_find_each_other() {
    // Nobody types the two dots, and plenty of senders do. Both spellings have to be one word
    // or half the mail in Russian is unfindable by the name it was sent under.
    let connection = an_index_to_fill();
    let written_with = a_message_saying(
        &connection,
        "Про ёлку",
        "Каппа Вандёль",
        "p@x.test",
        "you@x.test",
    );
    let written_without = a_message_saying(
        &connection,
        "Про елку",
        "Каппа Вандель",
        "q@x.test",
        "you@x.test",
    );

    assert_eq!(
        found(&connection, "ёлку"),
        vec![written_with, written_without]
    );
    assert_eq!(
        found(&connection, "елку"),
        vec![written_with, written_without]
    );
    assert_eq!(
        found(&connection, "Вандёль"),
        vec![written_with, written_without]
    );
}

#[test]
fn a_capital_yo_is_folded_as_well() {
    // A word opening a subject or a name carries the capital, and the folding is two replacements
    // rather than one: the index and the search each fold the capital on their own.
    let connection = an_index_to_fill();
    let capital = a_message_saying(
        &connection,
        "Ёлка в пятницу",
        "Kappa Vandel",
        "kappa@example.net",
        "you@x.test",
    );

    assert_eq!(found(&connection, "елка"), vec![capital]);
    assert_eq!(found(&connection, "Ёлка"), vec![capital]);
}

#[test]
fn the_short_i_is_not_the_letter_i() {
    // Short i (U+0439) and i (U+0438) are two letters rather than a letter and an accent, and only
    // yo is folded by hand: a search for one must not answer with the other.
    let connection = an_index_to_fill();
    let with_short_i = a_message_saying(
        &connection,
        "Мой май",
        "Kappa Vandel",
        "kappa@example.net",
        "you@x.test",
    );
    let plain = a_message_saying(
        &connection,
        "Мои маи",
        "Sigma Ordrin",
        "sigma@example.org",
        "you@x.test",
    );

    assert_eq!(found(&connection, "май"), vec![with_short_i]);
    assert_eq!(found(&connection, "маи"), vec![plain]);
}

#[test]
fn a_message_that_goes_leaves_the_index() {
    let connection = an_index_to_fill();
    let one = a_message_saying(
        &connection,
        "Quarterly report",
        "Kappa Vandel",
        "kappa@example.net",
        "you@x.test",
    );
    assert_eq!(found(&connection, "quarterly"), vec![one]);

    connection
        .execute("DELETE FROM messages WHERE id = ?1", [one])
        .expect("it goes");

    assert!(
        found(&connection, "quarterly").is_empty(),
        "a message nobody holds is still findable"
    );
}

#[test]
fn a_sender_filled_in_afterwards_becomes_findable() {
    let connection = an_index_to_fill();
    let one = a_message_saying(&connection, "Quarterly report", "", "", "you@x.test");
    assert!(found(&connection, "ordrin").is_empty());

    connection
        .execute(
            "UPDATE messages SET sender = 'Sigma Ordrin' WHERE id = ?1",
            [one],
        )
        .expect("the sender arrives later");

    assert_eq!(found(&connection, "ordrin"), vec![one]);
    assert_eq!(
        found(&connection, "quarterly"),
        vec![one],
        "and the message is not indexed twice"
    );
}

#[test]
fn what_a_message_says_is_findable_once_this_device_holds_it() {
    // This test used to say the opposite, and the reason it did was that message text was
    // sealed while the index beside it was not — so an index over that text would have been
    // the text again, in the clear, next to the thing that was sealed. The whole file is
    // encrypted now, and what is left is an ordinary question about what a person can find.
    let connection = an_index_to_fill();
    let one = a_message_saying(
        &connection,
        "Lunch",
        "Sigma Ordrin",
        "sigma@example.org",
        "you@x.test",
    );
    connection
        .execute(
            "UPDATE messages SET preview = 'bring the quarterly numbers' WHERE id = ?1",
            [one],
        )
        .expect("an opening is stored");

    assert_eq!(
        found(&connection, "quarterly"),
        vec![one],
        "a word the message opens with does not find it"
    );
}

#[test]
fn a_message_stored_with_its_opening_is_findable_by_it() {
    // The ordinary path: a pass reads two kilobytes of the message and stores the opening in
    // the same row as the subject. Every other test here fills the opening in afterwards,
    // which is the other trigger — this one would have been the one nothing covered.
    let connection = an_index_to_fill();
    connection
        .execute(
            "INSERT INTO messages
                     (account_id, subject, sender, sender_address, recipients_to, received_at,
                      preview)
                 VALUES (1, 'Lunch', 'Sigma Ordrin', 'sigma@example.org', 'you@x.test', 0,
                         'bring the quarterly numbers')",
            [],
        )
        .expect("a message stored with what it opens with");
    let one = connection.last_insert_rowid();

    assert_eq!(found(&connection, "quarterly"), vec![one]);
}

#[test]
fn mail_stored_before_the_index_held_text_becomes_findable_by_it() {
    // The upgrade. A mailbox synchronised by an earlier build is the ordinary case for anybody
    // who already has one, and an index rebuilt empty would leave every message they hold
    // findable by its subject and by nothing it says — for ever, because nothing re-reads a
    // message that has not changed.
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    index_migrations()
        .to_version(&mut connection, WHILE_THE_INDEX_HELD_NO_TEXT)
        .expect("the schema while the index held no text");
    fill_in_the_names(&connection);

    connection
        .execute(
            "INSERT INTO messages
                     (account_id, subject, sender, sender_address, recipients_to, received_at,
                      preview)
                 VALUES (1, 'Lunch', 'Sigma Ordrin', 'sigma@example.org', 'you@x.test', 0,
                         'bring the quarterly numbers')",
            [],
        )
        .expect("a message stored by the earlier build");
    let one = connection.last_insert_rowid();
    assert!(
        found(&connection, "quarterly").is_empty(),
        "the earlier schema already indexed the text, so this proves nothing"
    );

    migrate_index(&mut connection, ":memory:").expect("the upgrade applies");

    assert_eq!(
        found(&connection, "quarterly"),
        vec![one],
        "mail that was already here did not reach the index the upgrade built"
    );
    assert_eq!(
        found(&connection, "Lunch"),
        vec![one],
        "the upgrade lost what the index already held"
    );
}

#[test]
fn an_opening_that_arrives_after_the_message_is_indexed_with_it() {
    // A pass stores a message before it has read its opening, and fills it in afterwards. An
    // index that only followed the subject and the addresses would hold nothing of what that
    // message says, for ever.
    let connection = an_index_to_fill();
    let one = a_message_saying(
        &connection,
        "Lunch",
        "Sigma Ordrin",
        "sigma@example.org",
        "you@x.test",
    );

    assert!(found(&connection, "afterwards").is_empty());

    connection
        .execute(
            "UPDATE messages SET preview = 'it arrived afterwards' WHERE id = ?1",
            [one],
        )
        .expect("the opening is filled in");

    assert_eq!(found(&connection, "afterwards"), vec![one]);
}

#[test]
fn the_index_holds_what_it_indexes_and_not_a_second_copy_of_it() {
    // Contentless: tokens and nothing else, so a subject, an address and an opening are each
    // written once. What is searched is stated by the columns, and a column nobody gave it is
    // a column somebody has to explain.
    let connection = an_index_to_fill();

    let columns: i64 = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_info('message_search')",
            [],
            |row| row.get(0),
        )
        .expect("readable");
    assert_eq!(columns, 3, "the index has a column it was not given");

    let stored: String = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE name = 'message_search'",
            [],
            |row| row.get(0),
        )
        .expect("readable");
    assert!(stored.contains("content=''"), "{stored}");
}

#[test]
fn records_the_schema_version_in_user_version() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");

    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version is readable");
    // Pinned on purpose. The number is how SQLite knows which migrations a file has already
    // seen, so a migration quietly dropped from the list would leave older installations
    // reading a schema this build no longer writes. Adding one: bump this by one.
    assert_eq!(
        version, 28,
        "the applied migration count is tracked in user_version"
    );
}

/// The count beside the migrations must be the count of them.
///
/// `INDEX_MIGRATIONS` decides how far the upgrade walk climbs. Left behind when a migration is
/// added, it does not fail: the walk simply stops one short, and the version nobody covers is
/// the newest one. The comment above the constant says exactly this and nothing enforced it,
/// so it went stale.
#[test]
fn the_number_beside_the_index_migrations_is_how_many_there_are() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_index(&mut connection, ":memory:").expect("schema applies");

    let version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version is readable");

    assert_eq!(
        version, INDEX_MIGRATIONS as i64,
        "adding an index migration means bumping INDEX_MIGRATIONS, or the upgrade walk \
             silently stops covering the newest one"
    );
}

/// A rebuild must not take the mail that pointed at what it rebuilt.
///
/// Opened through the pool on purpose: `foreign_keys = ON` is set there and nowhere else, so a
/// bare in-memory connection cannot reach this at all. With it on, dropping `messages` performs
/// the cascade `message_placements` asked for — no error, a successful migration, an empty
/// mailbox.
#[test]
fn an_upgrade_that_rebuilds_a_table_keeps_the_mail_it_did_not_name() {
    let dir = std::env::temp_dir().join(format!("lyonoyl-rebuild-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory to work in");
    let path = dir.join("index.sqlite3");
    std::fs::remove_file(&path).ok();

    let pool = super::super::connection::open_index(&path, &DatabaseKey::for_a_test())
        .expect("the index opens");
    let mut connection = pool.get().expect("a connection");
    assert_eq!(
        connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .expect("readable"),
        1,
        "this test is only about a database whose foreign keys are enforced"
    );

    index_migrations()
        .to_version(&mut connection, BEFORE_THE_REBUILD)
        .expect("the schema before the rebuild applies");

    connection
        .execute_batch(
            "INSERT INTO folders (id, account_id, name, role) VALUES (1, 1, 'INBOX', 1);
                 INSERT INTO messages (id, account_id, received_at)
                 VALUES (1, 1, 1700000000), (2, 1, 1700000100);
                 INSERT INTO message_placements (folder_id, uid, message_id, received_at)
                 VALUES (1, 11, 1, 1700000000), (1, 12, 2, 1700000100);",
        )
        .expect("mail kept before the upgrade");

    migrate_index(&mut connection, "index.sqlite3").expect("the upgrade applies");

    let placements: i64 = connection
        .query_row("SELECT count(*) FROM message_placements", [], |row| {
            row.get(0)
        })
        .expect("readable");
    let messages: i64 = connection
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .expect("readable");

    assert_eq!(messages, 2, "the rebuild lost the messages themselves");
    assert_eq!(
        placements, 2,
        "the rebuild dropped the copies that pointed at what it rebuilt"
    );

    drop(connection);
    std::fs::remove_file(&path).ok();
}

/// An account that loses every message does not start numbering again.
///
/// Without `AUTOINCREMENT` the next insert takes the number the first message had, and an
/// identifier already sitting in the router, a cache and `op_log` starts naming a different
/// message in each of them.
#[test]
fn an_index_that_loses_every_message_does_not_start_numbering_again() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_index(&mut connection, ":memory:").expect("schema applies");

    connection
        .execute_batch(
            "INSERT INTO messages (account_id, received_at)
                 VALUES (1, 1700000000), (1, 1700000100), (1, 1700000200);",
        )
        .expect("three messages");

    let highest: i64 = connection
        .query_row("SELECT max(id) FROM messages", [], |row| row.get(0))
        .expect("readable");

    connection
        .execute_batch("DELETE FROM messages;")
        .expect("the account loses everything");

    connection
        .execute(
            "INSERT INTO messages (account_id, received_at) VALUES (1, 1700000300)",
            [],
        )
        .expect("one more arrives");

    let given: i64 = connection
        .query_row("SELECT max(id) FROM messages", [], |row| row.get(0))
        .expect("readable");

    assert!(
        given > highest,
        "an emptied index handed out {given} again, and {highest} had already been used"
    );
}

/// What the conversations table says must be what the messages say, after anything.
///
/// The scrollbar for a list of conversations is drawn from the size of this table. A number
/// that has drifted from the truth is not a number that is briefly wrong — nothing recomputes
/// it, so it is wrong for as long as the mailbox lives.
#[test]
fn the_conversations_table_says_what_the_messages_say() {
    let connection = an_index_to_fill();

    // Worked out from the messages, without touching the table under test.
    let truth = |connection: &Connection| -> Vec<(i64, i64, i64, i64)> {
        let mut held = connection
            .prepare(
                "SELECT m.thread_id, count(*), COALESCE(max(m.arrived_at), 0),
                            (SELECT n.id FROM messages n WHERE n.thread_id = m.thread_id
                             ORDER BY n.arrived_at DESC, n.id DESC LIMIT 1)
                     FROM messages m
                     WHERE m.thread_id IS NOT NULL GROUP BY m.thread_id ORDER BY m.thread_id",
            )
            .expect("readable");
        held.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows")
    };

    let kept = |connection: &Connection| -> Vec<(i64, i64, i64, i64)> {
        let mut held = connection
            .prepare(
                "SELECT thread_id, messages, newest_at, newest_message
                     FROM conversations ORDER BY thread_id",
            )
            .expect("readable");
        held.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows")
    };

    let mut seed = 0x51de_2024u64;
    let mut next = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (seed >> 33) as i64
    };

    let mut joined = 0;
    let mut moved = 0;
    let mut gone = 0;

    for step in 0..300 {
        let message = (next().rem_euclid(12)) + 1;
        let conversation = (next().rem_euclid(4)) + 1;

        match next().rem_euclid(10) {
            0..=4 => {
                joined += connection
                    .execute(
                        "UPDATE messages SET thread_id = ?2 WHERE id = ?1 AND thread_id IS NULL",
                        [message, conversation],
                    )
                    .expect("joining a conversation");
            }
            5..=7 => {
                moved += connection
                    .execute(
                        "UPDATE messages SET thread_id = ?2 WHERE id = ?1",
                        [message, conversation],
                    )
                    .expect("moving between conversations");
            }
            _ => {
                gone += connection
                    .execute("DELETE FROM messages WHERE id = ?1", [message])
                    .expect("a message going");
            }
        }

        assert_eq!(
            kept(&connection),
            truth(&connection),
            "the conversations table and the messages disagree after step {step}"
        );
    }

    assert!(joined > 0, "nothing ever joined a conversation");
    assert!(moved > 0, "nothing ever moved between conversations");
    assert!(gone > 0, "nothing ever went");
    // Two conversations put there on purpose, one of them holding more than one message: a run
    // that ended with none would agree about nothing, which is not agreement.
    connection
        .execute_batch(
            // `arrived_at` set here rather than left to the placements: this is about which
            // message speaks for a conversation, and that is decided by when it arrived.
            "INSERT INTO messages (id, account_id, received_at, arrived_at, thread_id)
                 VALUES (901, 1, 1700000000, 1700000000, 55),
                        (902, 1, 1700000100, 1700000100, 55),
                        (903, 1, 1700000200, 1700000200, 56);",
        )
        .expect("two conversations");

    assert_eq!(kept(&connection), truth(&connection));
    assert_eq!(
        kept(&connection)
            .into_iter()
            .filter(|(thread, ..)| *thread == 55 || *thread == 56)
            .collect::<Vec<_>>(),
        vec![(55, 2, 1_700_000_100, 902), (56, 1, 1_700_000_200, 903)],
        "a conversation does not know its size, its newest message, or when it arrived"
    );
}

#[test]
fn settings_tables_are_created() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");

    for table in ["preferences", "device_window_state", "device_pane_width"] {
        let exists: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("sqlite_master is readable");
        assert_eq!(exists, 1, "missing table: {table}");
    }
}

/// One pending `set_flags` on message 1 in folder 1, for the tests below to collide with.
#[cfg(test)]
fn a_pending_flag(connection: &Connection, message: i64, folder: i64) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO op_log (account_id, message_id, folder_id, kind, flags_add, asked_at)
             VALUES ('acc-1', ?1, ?2, 'set_flags', 1, 0)",
        [message, folder],
    )?;

    Ok(())
}

/// An account for operations to belong to. The log keeps a real key to it, so a row cannot
/// name an account that is not there.
#[cfg(test)]
fn an_account(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO mail_accounts (id, email, display_name, provider)
                 VALUES ('acc-1', 'user@example.com', 'User', 'imap')",
            [],
        )
        .expect("the account is written");
}

#[test]
fn an_operation_cannot_name_an_account_that_is_not_here() {
    // The account lives in this same file, so the key is a real one: an operation for an
    // account nobody added is an instruction with no server to send it to, and removing an
    // account takes its outstanding operations with it.
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("keys are enforced here as they are in the application");

    assert!(
        a_pending_flag(&connection, 1, 1).is_err(),
        "an operation was written for an account that is not here"
    );

    an_account(&connection);
    a_pending_flag(&connection, 1, 1).expect("now there is an account to act on");

    connection
        .execute("DELETE FROM mail_accounts WHERE id = 'acc-1'", [])
        .expect("the account is removed");

    let left: i64 = connection
        .query_row("SELECT count(*) FROM op_log", [], |row| row.get(0))
        .expect("the log is readable");

    assert_eq!(left, 0, "operations outlived the account they belonged to");
}

#[test]
fn the_operation_log_holds_no_way_to_write_down_a_position() {
    // The whole rule in one assertion. A uid is a position that means a different message once the
    // folder resets, so an operation carrying one would, after a reset, apply to a stranger's
    // message. The column does not exist, and this is what says so out loud.
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");

    let columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('op_log')")
        .expect("the table is there")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("columns are readable")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("columns are readable");

    assert!(columns.contains(&"message_id".to_owned()));
    for named in ["uid", "message_uid", "uid_validity", "position"] {
        assert!(
            !columns.iter().any(|column| column == named),
            "op_log gained a column that holds a position: {named}"
        );
    }
}

#[test]
fn only_one_thing_may_be_waiting_to_happen_to_one_message_in_one_folder() {
    // Coalescing is a decision, and decisions live in the domain. This is the net underneath
    // it: two writers racing cannot leave two outstanding operations on one target, so the
    // server cannot be told the same thing twice.
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("keys are enforced here as they are in the application");
    an_account(&connection);

    a_pending_flag(&connection, 1, 1).expect("the first one is written");

    assert!(
        a_pending_flag(&connection, 1, 1).is_err(),
        "a second outstanding operation on the same target was allowed"
    );

    // The same message in another folder is another thing to do, and so is another message.
    a_pending_flag(&connection, 1, 2).expect("another folder is another operation");
    a_pending_flag(&connection, 2, 1).expect("another message is another operation");
}

#[test]
fn what_has_already_happened_does_not_block_asking_again() {
    // The index covers what is still outstanding only. Marking a message read, having that
    // reach the server, and marking it read again a week later is two operations, not an error.
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("keys are enforced here as they are in the application");
    an_account(&connection);

    a_pending_flag(&connection, 1, 1).expect("the first one is written");
    connection
        .execute("UPDATE op_log SET state = 'done'", [])
        .expect("it reaches the server");

    a_pending_flag(&connection, 1, 1).expect("asking again is allowed");
}

#[test]
fn a_conflict_does_not_hold_the_queue_shut_either() {
    // A conflicted operation waits for a person, not for the server. Blocking every later
    // operation on that message behind it would turn one stuck row into a stuck mailbox.
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("keys are enforced here as they are in the application");
    an_account(&connection);

    a_pending_flag(&connection, 1, 1).expect("the first one is written");
    connection
        .execute("UPDATE op_log SET state = 'conflicted'", [])
        .expect("it conflicts");

    a_pending_flag(&connection, 1, 1).expect("a later operation is still allowed");
}

#[test]
fn an_operation_that_says_nothing_is_refused_by_the_schema() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("keys are enforced here as they are in the application");
    an_account(&connection);

    let refused = [
            // A move with nowhere to go.
            "INSERT INTO op_log (account_id, message_id, folder_id, kind, asked_at)
             VALUES ('acc-1', 1, 1, 'move', 0)",
            // A delete that does not say whether it can be undone.
            "INSERT INTO op_log (account_id, message_id, folder_id, kind, asked_at)
             VALUES ('acc-1', 1, 1, 'delete', 0)",
            // Flags that change nothing.
            "INSERT INTO op_log (account_id, message_id, folder_id, kind, asked_at)
             VALUES ('acc-1', 1, 1, 'set_flags', 0)",
            // A flag asked for and against at once, which has no answer to give a server.
            "INSERT INTO op_log (account_id, message_id, folder_id, kind, flags_add, flags_remove, asked_at)
             VALUES ('acc-1', 1, 1, 'set_flags', 1, 1, 0)",
            // A kind nothing knows how to carry out.
            "INSERT INTO op_log (account_id, message_id, folder_id, kind, asked_at)
             VALUES ('acc-1', 1, 1, 'send_to_space', 0)",
            // A state nothing knows how to read.
            "INSERT INTO op_log (account_id, message_id, folder_id, kind, flags_add, state, asked_at)
             VALUES ('acc-1', 1, 1, 'set_flags', 1, 'probably fine', 0)",
        ];

    for statement in refused {
        assert!(
            connection.execute(statement, []).is_err(),
            "the schema accepted an operation it cannot carry out: {statement}"
        );
    }
}

#[test]
fn accounts_written_before_the_index_number_existed_are_given_one() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, 4)
        .expect("the previous schema applies");

    for (id, email) in [("acc-a", "a@example.com"), ("acc-b", "b@example.com")] {
        connection
            .execute(
                "INSERT INTO mail_accounts (id, email, display_name, provider)
                     VALUES (?1, ?2, '', 'imap')",
                (id, email),
            )
            .expect("a row written by the previous version");
    }

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let mut statement = connection
        .prepare("SELECT id, index_id FROM mail_accounts ORDER BY index_id")
        .expect("the column exists");
    let numbered: Vec<(String, i64)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("rows are readable")
        .map(|row| row.expect("row"))
        .collect();

    assert_eq!(
        numbered,
        vec![("acc-a".to_owned(), 1), ("acc-b".to_owned(), 2)]
    );

    let next: i64 = connection
        .query_row("SELECT next FROM account_index_counter", [], |row| {
            row.get(0)
        })
        .expect("the counter exists");
    assert_eq!(
        next, 3,
        "the next account continues past the ones already here"
    );
}

#[test]
fn two_accounts_cannot_share_an_index_number() {
    // They would share their folders and their mail.
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");

    connection
        .execute(
            "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                 VALUES ('acc-a', 'a@example.com', '', 'imap', 4)",
            [],
        )
        .expect("the first account is stored");

    let clash = connection.execute(
        "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
             VALUES ('acc-b', 'b@example.com', '', 'imap', 4)",
        [],
    );

    assert!(clash.is_err(), "the second account must be refused");
}

#[test]
fn upgrading_from_the_previous_schema_keeps_the_stored_window() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, 2)
        .expect("the previous schema applies");

    connection
        .execute(
            "INSERT INTO device_window_state (window, x, y, width, height, maximized)
                 VALUES ('main', 40, 60, 1200, 800, 1)",
            [],
        )
        .expect("a row written by the previous version");

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let (x, y, width, height, mode): (i64, i64, i64, i64, i64) = connection
        .query_row(
            "SELECT x, y, width, height, mode FROM device_window_state WHERE window = 'main'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("the row survives the upgrade");

    assert_eq!((x, y, width, height), (40, 60, 1200, 800));
    assert_eq!(mode, 1, "a maximized window stays maximized");
}

#[test]
fn downgrading_reduces_full_screen_to_maximized() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    connection
        .execute(
            "INSERT INTO device_window_state (window, x, y, width, height, mode)
                 VALUES ('main', 0, 0, 1400, 900, 2)",
            [],
        )
        .expect("a full-screen window");

    profile_migrations()
        .to_version(&mut connection, 2)
        .expect("the down migration runs");

    let maximized: i64 = connection
        .query_row(
            "SELECT maximized FROM device_window_state WHERE window = 'main'",
            [],
            |row| row.get(0),
        )
        .expect("the row survives the downgrade");
    assert_eq!(maximized, 1);
}

fn index_database() -> Connection {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign keys are enabled for this connection");
    migrate_index(&mut connection, ":memory:").expect("schema applies");
    connection
}

fn seed_folder_and_message(connection: &Connection) {
    connection
        .execute_batch(
            "INSERT INTO folders (id, account_id, name, role) VALUES (1, 1, 'INBOX', 0);
                 INSERT INTO folders (id, account_id, name, role) VALUES (2, 1, 'Archive', 5);
                 INSERT INTO messages (id, account_id, received_at) VALUES (10, 1, 1700000000);
                 INSERT INTO message_placements (folder_id, uid, message_id) VALUES (1, 500, 10);",
        )
        .expect("seed applies");
}

/// A database that already held instructions keeps them, under the name the rest of the file
/// uses for an account.
#[test]
fn instructions_written_before_the_upgrade_keep_the_account_they_named() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, 8)
        .expect("the previous schema applies");

    connection
        .execute_batch(
            "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                 VALUES ('work', 'a@example.com', 'Work', 'imap', 7),
                        ('home', 'b@example.com', 'Home', 'imap', 9);

                 INSERT INTO op_log
                     (account_id, message_id, folder_id, kind, flags_add, flags_remove, move_to,
                      state, asked_at)
                 VALUES (7, 100, 1, 'set_flags', 1, 0, NULL, 'pending', 0),
                        (9, 200, 2, 'move', 0, 0, 5, 'conflicted', 0),
                        (42, 300, 3, 'set_flags', 1, 0, NULL, 'pending', 0);",
        )
        .expect("rows written by the build with the defect");

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let kept: Vec<(String, i64, String)> = connection
        .prepare("SELECT account_id, message_id, state FROM op_log ORDER BY message_id")
        .expect("readable")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("rows")
        .map(|row| row.expect("row"))
        .collect();

    assert_eq!(
        kept,
        vec![
            ("work".to_owned(), 100, "pending".to_owned()),
            ("home".to_owned(), 200, "conflicted".to_owned()),
        ],
        "both instructions survive, now naming their account the way this file does; \
             the third named an account this device does not have"
    );
}

/// The gate for the whole class: an upgraded database is the one people actually have, and it
/// has to accept what this build writes. It did not — 007 was corrected in place, so a fresh
/// database got the correction and every existing one kept a column that refused the value.
/// Archiving and deleting failed for months of nobody noticing, because the refusal was read
/// as two writers racing.
#[test]
fn a_database_upgraded_from_an_older_version_accepts_what_this_build_writes() {
    for from in 1..=8 {
        let mut connection = Connection::open_in_memory().expect("in-memory database opens");
        profile_migrations()
            .to_version(&mut connection, from)
            .expect("an older schema applies");
        migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

        connection
            .execute(
                "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                     VALUES ('work', 'a@example.com', 'Work', 'imap', 1)",
                [],
            )
            .expect("an account can be added");

        connection
            .execute(
                "INSERT INTO op_log
                         (account_id, message_id, folder_id, kind, flags_add, flags_remove,
                          move_to, state, asked_at)
                     VALUES ('work', 1, 2, 'move', 0, 0, 3, 'pending', 0)",
                [],
            )
            .unwrap_or_else(|error| {
                panic!("a database upgraded from version {from} refuses an instruction: {error}")
            });
    }
}

#[test]
fn every_message_kept_before_the_sealing_is_deleted_by_the_upgrade() {
    // The upgrade that made this build honest. Every body stored before it is on the disk as
    // the server sent it, and they cannot be sealed in place — the key does not exist until
    // the application asks for it, and a migration is not the place to reach into a keychain.
    // They are a cache of a copy, so they go, and the message is fetched again when somebody
    // opens it. Shipping without this would mean a build that says the mail is encrypted while
    // the mail already there is not.
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    index_migrations()
        .to_version(&mut connection, BEFORE_THE_SEALING)
        .expect("the schema before the sealing applies");

    connection
        .execute_batch(
            "INSERT INTO folders (id, account_id, name, role) VALUES (1, 1, 'INBOX', 1);
                 INSERT INTO messages (id, account_id, received_at, body_state)
                 VALUES (1, 1, 1700000000, 3);",
        )
        .expect("a message that has its body");
    connection
        .execute(
            "INSERT INTO message_bodies (message_id, raw, stored_at)
                 VALUES (1, ?1, 1700000000)",
            [b"Subject: in the clear\r\n\r\nread by anything".as_slice()],
        )
        .expect("a body written before the sealing");

    migrate_index(&mut connection, ":memory:").expect("upgrade applies");

    let left: i64 = connection
        .query_row("SELECT count(*) FROM message_bodies", [], |row| row.get(0))
        .expect("the count is readable");
    assert_eq!(left, 0, "a message is still on the disk as it arrived");

    // And the list must not go on saying the message is held here, or it would offer a body
    // that is not there.
    let state: i64 = connection
        .query_row("SELECT body_state FROM messages WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("the state is readable");
    assert_eq!(state, 0, "the index still claims to hold the message");
}

/// The same gate, for the index.
///
/// Two columns arrived on `messages` after the table existed — the conversation a message
/// belongs to, and the address an answer to it goes to — and a column added by a later
/// migration is only present in a database made this morning until somebody proves otherwise.
/// The write below is the one a pass makes, so a column missing from an upgraded index fails
/// here rather than in front of a person with mail in it.
#[test]
fn an_upgraded_index_accepts_what_this_build_writes_about_a_message() {
    for from in 1..=INDEX_MIGRATIONS {
        let mut connection = Connection::open_in_memory().expect("in-memory database opens");
        index_migrations()
            .to_version(&mut connection, from)
            .expect("an older schema applies");
        migrate_index(&mut connection, ":memory:").expect("upgrade applies");

        connection
            .execute(
                "INSERT INTO messages
                         (account_id, rfc822_message_id, subject, sender, sender_address,
                          received_at, rfc822_size, preview, references_chain, flags)
                     VALUES (1, 'a@example.com', 'About Tuesday', 'Zeta Quorin',
                             'zeta@example.com', 1700000000, 1024, NULL, 'older@example.com', 0)",
                [],
            )
            .unwrap_or_else(|error| {
                panic!("an index upgraded from version {from} refuses a message: {error}")
            });
    }
}

#[test]
fn openings_that_were_really_markup_are_cleared_on_upgrade() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    index_migrations()
        .to_version(&mut connection, 5)
        .expect("the previous schema applies");

    connection
        .execute_batch(
            "INSERT INTO messages (id, account_id, received_at, preview)
                 VALUES (1, 1, 0, '<p>Hello,</p><p>Thank you for your response.</p>'),
                        (2, 1, 0, 'Hello, thank you for your response.'),
                        (3, 1, 0, NULL),
                        (4, 1, 0, 'Rescheduled. I=E2=80=99ll be back later.'),
                        (5, 1, 0, 'The rule is a = b + c, which holds.');",
        )
        .expect("rows written by the build with the defect");

    migrate_index(&mut connection, ":memory:").expect("upgrade applies");

    let previews: Vec<Option<String>> = connection
        .prepare("SELECT preview FROM messages ORDER BY id")
        .expect("readable")
        .query_map([], |row| row.get(0))
        .expect("rows")
        .map(|row| row.expect("row"))
        .collect();

    assert_eq!(previews[0], None, "the line of tags is gone");
    assert_eq!(
        previews[1].as_deref(),
        Some("Hello, thank you for your response."),
        "and the one made of words is untouched"
    );
    assert_eq!(previews[2], None);
    assert_eq!(previews[3], None, "an escape is not a word either");
    assert_eq!(
        previews[4].as_deref(),
        Some("The rule is a = b + c, which holds."),
        "and an equals sign a person typed is not an escape"
    );
}

#[test]
fn the_index_schema_applies_and_is_idempotent() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_index(&mut connection, ":memory:").expect("first run applies the schema");
    migrate_index(&mut connection, ":memory:").expect("a second run is a no-op");

    for table in ["folders", "messages", "message_placements"] {
        let exists: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .expect("sqlite_master is readable");
        assert_eq!(exists, 1, "missing table: {table}");
    }
}

#[test]
fn the_index_schema_can_be_rolled_back() {
    let mut connection = index_database();
    index_migrations()
        .to_version(&mut connection, 0)
        .expect("the down migration runs");

    let tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN
                 ('folders', 'messages', 'message_placements', 'message_keywords')",
            [],
            |row| row.get(0),
        )
        .expect("sqlite_master is readable");
    assert_eq!(tables, 0);
}

#[test]
fn a_message_has_no_uid_column() {
    let connection = index_database();
    let columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('messages')")
        .expect("pragma is queryable")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("rows are readable")
        .collect::<Result<_, _>>()
        .expect("rows are readable");

    assert!(
        !columns.iter().any(|column| column == "uid"),
        "a UID addresses a message inside one folder and belongs in message_placements"
    );
}

#[test]
fn one_message_can_sit_in_several_folders() {
    let connection = index_database();
    seed_folder_and_message(&connection);

    connection
        .execute(
            "INSERT INTO message_placements (folder_id, uid, message_id) VALUES (2, 900, 10)",
            [],
        )
        .expect("the same message appears in another folder");

    let placements: i64 = connection
        .query_row(
            "SELECT count(*) FROM message_placements WHERE message_id = 10",
            [],
            |row| row.get(0),
        )
        .expect("count is readable");
    assert_eq!(placements, 2);
}

#[test]
fn a_uid_is_unique_within_its_folder() {
    let connection = index_database();
    seed_folder_and_message(&connection);
    connection
        .execute(
            "INSERT INTO messages (id, account_id, received_at) VALUES (11, 1, 1700000001)",
            [],
        )
        .expect("a second message");

    let conflict = connection.execute(
        "INSERT INTO message_placements (folder_id, uid, message_id) VALUES (1, 500, 11)",
        [],
    );
    assert!(conflict.is_err(), "(folder, uid) addresses one message");
}

#[test]
fn removing_a_folder_removes_its_placements_and_nothing_else() {
    let connection = index_database();
    seed_folder_and_message(&connection);

    connection
        .execute("DELETE FROM folders WHERE id = 1", [])
        .expect("folder is removed");

    let placements: i64 = connection
        .query_row("SELECT count(*) FROM message_placements", [], |row| {
            row.get(0)
        })
        .expect("count is readable");
    let messages: i64 = connection
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .expect("count is readable");

    assert_eq!(placements, 0);
    assert_eq!(messages, 1, "the message itself survives losing a folder");
}

#[test]
fn removing_a_message_removes_its_placements() {
    let connection = index_database();
    seed_folder_and_message(&connection);

    connection
        .execute("DELETE FROM messages WHERE id = 10", [])
        .expect("message is removed");

    let left: i64 = connection
        .query_row("SELECT count(*) FROM message_placements", [], |row| {
            row.get(0)
        })
        .expect("count is readable");
    assert_eq!(left, 0, "message_placements still holds rows");
}

#[test]
fn a_folder_name_is_unique_within_an_account_but_not_across_accounts() {
    let connection = index_database();
    seed_folder_and_message(&connection);

    let duplicate = connection.execute(
        "INSERT INTO folders (id, account_id, name, role) VALUES (3, 1, 'INBOX', 0)",
        [],
    );
    assert!(duplicate.is_err());

    connection
        .execute(
            "INSERT INTO folders (id, account_id, name, role) VALUES (4, 2, 'INBOX', 0)",
            [],
        )
        .expect("another account has its own INBOX");
}

#[test]
fn strict_typing_rejects_a_value_of_the_wrong_type() {
    let connection = index_database();
    let wrong = connection.execute(
        "INSERT INTO messages (id, account_id, received_at) VALUES (12, 1, 'yesterday')",
        [],
    );
    assert!(wrong.is_err(), "a date is stored as a number, not as text");
}

#[test]
fn can_be_rolled_back() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");

    profile_migrations()
        .to_version(&mut connection, 0)
        .expect("the down migration is defined and runs");

    let exists: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'mail_accounts'",
            [],
            |row| row.get(0),
        )
        .expect("sqlite_master is readable");
    assert_eq!(exists, 0, "rolling back removes the table");
}

/// A database written before the repair has its conversations built again.
///
/// The trigger added with the repair keeps new arrivals right, and does nothing for the rows
/// already holding a zero — every conversation in every database written until then. Those are
/// rows the list cannot show, so the migration builds the table again, and this is what says it
/// did. The state is produced rather than described: the message is stored, given its
/// conversation, and only then placed, which is the order the synchronisation writes in and the
/// order that leaves the date behind.
#[test]
fn conversations_written_before_the_repair_get_their_date() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    index_migrations()
        .to_version(&mut connection, WHILE_A_CONVERSATION_HAD_NO_DATE)
        .expect("the schema while a conversation had no date");
    fill_in_the_names(&connection);

    connection
        .execute("UPDATE messages SET thread_id = 1 WHERE id IN (1, 2)", [])
        .expect("two messages of one conversation");
    place(&connection, 1, 10, 1, 1_700_000_000);
    place(&connection, 1, 11, 2, 1_700_000_100);

    assert_eq!(
        a_number(
            &connection,
            "SELECT newest_at FROM conversations WHERE thread_id = 1"
        ),
        0,
        "the state this migration exists to repair"
    );

    migrate_index(&mut connection, ":memory:").expect("the conversations are built again");

    assert_eq!(
        a_number(
            &connection,
            "SELECT newest_at FROM conversations WHERE thread_id = 1"
        ),
        1_700_000_100,
        "the date of the message that arrived last"
    );
    assert_eq!(
        a_number(
            &connection,
            "SELECT newest_message FROM conversations WHERE thread_id = 1"
        ),
        2
    );
    assert_eq!(
        a_number(
            &connection,
            "SELECT messages FROM conversations WHERE thread_id = 1"
        ),
        2
    );
}

/// A message whose body is already here was read whole to get it, and says so afterwards.
///
/// Without the backfill every message on the device would look unread-whole, and the next
/// synchronisation would fetch the entire mailbox again — over the network, for something it
/// already has.
#[test]
fn a_message_already_held_is_not_read_again_after_the_upgrade() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    index_migrations()
        .to_version(&mut connection, WHILE_NOTHING_SAID_A_MESSAGE_WAS_READ)
        .expect("the schema before anything recorded a reading");
    fill_in_the_names(&connection);
    place(&connection, 1, 10, 1, 1_700_000_000);
    place(&connection, 1, 11, 2, 1_700_000_100);

    // One has its body here and its words in the index; one has the body and no words, the state
    // every message stored between 012 and 024 is in; one has never been fetched.
    connection
        .execute(
            "INSERT INTO message_bodies (message_id, raw, stored_at) VALUES (1, x'00', 1699999999)",
            [],
        )
        .expect("a body already kept");
    connection
        .execute(
            "UPDATE messages SET body_text = 'what it says' WHERE id = 1",
            [],
        )
        .expect("its words are in the index");
    connection
        .execute(
            "INSERT INTO message_bodies (message_id, raw, stored_at) VALUES (3, x'00', 1699999998)",
            [],
        )
        .expect("a body kept before the column for its words existed");

    migrate_index(&mut connection, ":memory:").expect("the column arrives and is filled in");

    assert_eq!(
        a_number(
            &connection,
            "SELECT read_whole_at FROM messages WHERE id = 1"
        ),
        1_699_999_999,
        "a message whose body is here was read whole to get it"
    );
    assert_eq!(
        a_number(
            &connection,
            "SELECT count(*) FROM messages WHERE id = 2 AND read_whole_at IS NULL"
        ),
        1,
        "a message never fetched is still waiting to be"
    );
    assert_eq!(
        a_number(
            &connection,
            "SELECT count(*) FROM messages WHERE id = 3 AND read_whole_at IS NULL"
        ),
        1,
        "a body kept before its words could be indexed is read once more, or it is never findable"
    );
}

/// A folder read before anything said whether its last pass stopped short starts as one that did
/// not, and takes what this build writes about it.
#[test]
fn a_folder_read_before_the_upgrade_has_not_stopped_short() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    index_migrations()
        .to_version(&mut connection, WHILE_NOTHING_SAID_A_FOLDER_STOPPED_SHORT)
        .expect("the schema before the column");
    connection
        .execute(
            "INSERT INTO folders (id, account_id, name, role, sync_cursor)
             VALUES (1, 1, 'Archive', 6, '6997')",
            [],
        )
        .expect("a folder read before the upgrade");

    migrate_index(&mut connection, ":memory:").expect("the column arrives");

    assert_eq!(
        a_number(
            &connection,
            "SELECT stopped_short FROM folders WHERE id = 1"
        ),
        0
    );
    connection
        .execute("UPDATE folders SET stopped_short = 1 WHERE id = 1", [])
        .expect("what this build writes");
}

/// What nothing writes and nothing reads is gone once the index is upgraded: two columns of a
/// message, two of a folder, the keywords and the index for unread over a message's own flags.
#[test]
fn what_nothing_read_is_gone_after_the_upgrade() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    index_migrations()
        .to_version(&mut connection, WHILE_IT_CARRIED_WHAT_NOTHING_READ)
        .expect("the schema that still carried them");

    migrate_index(&mut connection, ":memory:").expect("the upgrade runs");

    let left: Vec<String> = connection
        .prepare(
            "SELECT name FROM pragma_table_info('messages')
                 WHERE name IN ('bodystructure', 'subject_norm')
             UNION ALL SELECT name FROM pragma_table_info('folders')
                 WHERE name IN ('last_error_kind', 'last_successful_sync_at')
             UNION ALL SELECT name FROM sqlite_master
                 WHERE name IN ('message_keywords', 'idx_messages_unread')",
        )
        .expect("the schema is readable")
        .query_map([], |row| row.get(0))
        .expect("the schema is listed")
        .collect::<Result<_, _>>()
        .expect("every name reads");
    assert!(left.is_empty(), "still in the index: {left:?}");
}

/// The mail written while those stood comes through the upgrade whole: the message, its folder
/// and its copy there.
#[test]
fn the_mail_written_before_the_upgrade_stays() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    index_migrations()
        .to_version(&mut connection, WHILE_IT_CARRIED_WHAT_NOTHING_READ)
        .expect("the schema that still carried them");
    connection
        .execute_batch(
            "INSERT INTO folders (id, account_id, name, role, last_error_kind)
                 VALUES (1, 1, 'INBOX', 0, 'auth');
             INSERT INTO messages (id, account_id, received_at, subject, subject_norm, bodystructure)
                 VALUES (10, 1, 1700000000, 'Hello', 'hello', x'00');
             INSERT INTO message_placements (folder_id, uid, message_id) VALUES (1, 500, 10);
             INSERT INTO message_keywords (message_id, keyword) VALUES (10, 'work');",
        )
        .expect("mail written before the upgrade");

    migrate_index(&mut connection, ":memory:").expect("the upgrade runs");

    let kept: (String, String, i64) = connection
        .query_row(
            "SELECT m.subject, f.name, p.uid FROM messages m
             JOIN message_placements p ON p.message_id = m.id
             JOIN folders f ON f.id = p.folder_id
             WHERE m.id = 10",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("the message is still there");
    assert_eq!(kept, ("Hello".to_owned(), "INBOX".to_owned(), 500));
}

/// A message stored before the upgrade has handed its copies to nobody, and goes by itself.
///
/// Read otherwise, an instruction about it would look for its copies under another message.
#[test]
fn a_message_stored_before_the_upgrade_goes_by_itself() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    index_migrations()
        .to_version(&mut connection, WHILE_NOTHING_SAID_WHERE_A_ROWS_COPIES_WENT)
        .expect("the schema before the table");
    connection
        .execute_batch(
            "INSERT INTO messages (id, account_id, received_at) VALUES (5, 1, 1700000000);
             INSERT INTO messages (id, account_id, received_at) VALUES (9, 1, 1700000000);",
        )
        .expect("messages stored before the upgrade");

    migrate_index(&mut connection, ":memory:").expect("the table arrives");

    assert_eq!(a_number(&connection, "SELECT count(*) FROM handed_over"), 0);
    connection
        .execute(
            "INSERT INTO handed_over (message_id, handed_to) VALUES (5, 9)",
            [],
        )
        .expect("what this build writes");
}

/// A folder read before the upgrade was refused nothing for a moment, in either reading.
///
/// Read otherwise, its first refusal for a moment after the upgrade would be taken for the second
/// in a row, and a range a server will describe whole two minutes later kept as rows with no name.
#[test]
fn a_folder_read_before_the_upgrade_was_refused_nothing_for_now() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    index_migrations()
        .to_version(
            &mut connection,
            WHILE_NOTHING_SAID_A_READING_WAS_REFUSED_FOR_NOW,
        )
        .expect("the schema before the columns");
    connection
        .execute(
            "INSERT INTO folders (id, account_id, name, role, sync_cursor)
             VALUES (1, 1, 'Archive', 6, '6997')",
            [],
        )
        .expect("a folder read before the upgrade");

    migrate_index(&mut connection, ":memory:").expect("the columns arrive");

    assert_eq!(
        a_number(
            &connection,
            "SELECT new_mail_refused_for_now + further_back_refused_for_now
             FROM folders WHERE id = 1"
        ),
        0
    );
    connection
        .execute(
            "UPDATE folders SET new_mail_refused_for_now = 1, further_back_refused_for_now = 1
             WHERE id = 1",
            [],
        )
        .expect("what this build writes");
}

/// An account written before groups existed keeps its mail and joins nothing.
///
/// The column arrives with no default beyond NULL, so every account a person already had is
/// ungrouped — which is the top of the sidebar, where they were before anybody made a group.
#[test]
fn accounts_written_before_groups_existed_belong_to_no_group() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, 17)
        .expect("the previous schema applies");

    connection
        .execute(
            "INSERT INTO mail_accounts (id, email, display_name, provider)
                 VALUES ('acc-a', 'a@example.com', '', 'imap')",
            [],
        )
        .expect("a row written by the previous version");

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let group: Option<String> = connection
        .query_row(
            "SELECT group_id FROM mail_accounts WHERE id = 'acc-a'",
            [],
            |row| row.get(0),
        )
        .expect("the column exists");

    assert_eq!(group, None, "an account nobody grouped is in no group");
}

/// Removing a group must not remove the accounts in it.
///
/// A group is an arrangement of the sidebar. Somebody tidying it away has said nothing at all
/// about their mailboxes, and a cascade here would delete mail to undo a change to a list.
#[test]
fn removing_a_group_returns_its_accounts_rather_than_deleting_them() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");

    connection
        .execute(
            "INSERT INTO account_groups (id, name, position) VALUES ('g1', 'Work', 0)",
            [],
        )
        .expect("a group is made");
    connection
        .execute(
            "INSERT INTO mail_accounts (id, email, display_name, provider, group_id)
                 VALUES ('acc-a', 'a@example.com', '', 'imap', 'g1')",
            [],
        )
        .expect("an account joins it");

    connection
        .execute("DELETE FROM account_groups WHERE id = 'g1'", [])
        .expect("the group is removed");

    let (still_there, group): (i64, Option<String>) = connection
        .query_row(
            "SELECT count(*), max(group_id) FROM mail_accounts WHERE id = 'acc-a'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the account is readable");

    assert_eq!(still_there, 1, "the account survives its group");
    assert_eq!(group, None, "and is back at the top of the sidebar");
}

/// Removing an account takes its membership with it.
///
/// The membership is a column on the account's own row, so there is no second place holding a
/// name for a mailbox that no longer exists.
#[test]
fn removing_an_account_takes_its_membership_with_it() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");

    connection
        .execute(
            "INSERT INTO account_groups (id, name, position) VALUES ('g1', 'Work', 0)",
            [],
        )
        .expect("a group is made");
    connection
        .execute(
            "INSERT INTO mail_accounts (id, email, display_name, provider, group_id)
                 VALUES ('acc-a', 'a@example.com', '', 'imap', 'g1')",
            [],
        )
        .expect("an account joins it");

    connection
        .execute("DELETE FROM mail_accounts WHERE id = 'acc-a'", [])
        .expect("the account is removed");

    let members: i64 = connection
        .query_row(
            "SELECT count(*) FROM mail_accounts WHERE group_id = 'g1'",
            [],
            |row| row.get(0),
        )
        .expect("the group is readable");
    let groups: i64 = connection
        .query_row("SELECT count(*) FROM account_groups", [], |row| row.get(0))
        .expect("the group is readable");

    assert_eq!(members, 0, "nothing is left pointing at the group");
    assert_eq!(groups, 1, "and the group itself is untouched");
}

/// A membership naming a group that does not exist is refused by the database.
///
/// The check is worth pinning because it only holds while `PRAGMA foreign_keys` is on, and that
/// is set per connection rather than stored in the file.
#[test]
fn an_account_cannot_join_a_group_that_does_not_exist() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign keys are enforced");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");

    let refused = connection.execute(
        "INSERT INTO mail_accounts (id, email, display_name, provider, group_id)
             VALUES ('acc-a', 'a@example.com', '', 'imap', 'no-such-group')",
        [],
    );

    assert!(
        refused.is_err(),
        "a membership must name a group that exists"
    );
}

/// A message taken back takes the files it was going to carry with it.
///
/// A row pointing at an outbox entry that is gone is a file nothing will ever send and nothing
/// will ever clear — in the one database this application never drops.
#[test]
fn removing_a_queued_message_removes_the_files_it_was_carrying() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign keys are enforced");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");

    connection
        .execute(
            "INSERT INTO mail_accounts (id, email, display_name, provider)
                 VALUES ('acc-a', 'a@example.com', '', 'imap')",
            [],
        )
        .expect("an account");
    connection
        .execute(
            "INSERT INTO outbox (id, account_id, message_id, sender, subject, body, written_at)
                 VALUES (1, 'acc-a', '<m@example.com>', 'a@example.com', 'S', 'B', 0)",
            [],
        )
        .expect("a queued message");
    connection
        .execute(
            "INSERT INTO outbox_files (outbox_id, name, content_type, bytes, position)
                 VALUES (1, 'invoice.pdf', 'application/pdf', X'25504446', 0)",
            [],
        )
        .expect("a file with it");

    connection
        .execute("DELETE FROM outbox WHERE id = 1", [])
        .expect("taken back");

    let left: i64 = connection
        .query_row("SELECT count(*) FROM outbox_files", [], |row| row.get(0))
        .expect("countable");

    assert_eq!(
        left, 0,
        "nothing is left pointing at a message that is gone"
    );
}

/// The version the profile was at while nothing an instruction did was owed a read.
///
/// Named rather than counted back from the end, for the reason the ones beside the index
/// migrations give: a number written as "the last but one" stops meaning this the moment another
/// migration is added.
const WHILE_NOTHING_OWED_A_READ: usize = 19;

/// An instruction written before the upgrade is carried over as it was, owing nothing.
///
/// A queue at the version before carries nothing out without a pass reading the account first,
/// so what it holds as done has been read back. Marking any of it owed would lay a move from last
/// week over the index until some pass happened to read its folders.
#[test]
fn instructions_written_before_reads_were_owed_owe_nothing() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, WHILE_NOTHING_OWED_A_READ)
        .expect("the previous schema applies");
    an_account(&connection);
    connection
        .execute_batch(
            "INSERT INTO op_log (account_id, message_id, folder_id, kind, move_to, state, asked_at)
                 VALUES ('acc-1', 1, 1, 'move', 2, 'done', 0);
             INSERT INTO op_log (account_id, message_id, folder_id, kind, flags_add, asked_at)
                 VALUES ('acc-1', 2, 1, 'set_flags', 1, 0);",
        )
        .expect("rows written by the previous version");

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let owed: Vec<(i64, i64)> = connection
        .prepare("SELECT message_id, owes_a_read FROM op_log ORDER BY message_id")
        .expect("readable")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("rows")
        .map(|row| row.expect("row"))
        .collect();
    assert_eq!(owed, vec![(1, 0), (2, 0)]);
}

/// Something already told to the server is not what a new instruction is folded into, so the two
/// may stand one behind the other. Two unsent ones about the same thing still may not.
#[test]
fn an_instruction_on_its_way_does_not_stop_the_next_one_being_written_behind_it() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("keys are enforced here as they are in the application");
    an_account(&connection);

    a_pending_flag(&connection, 1, 1).expect("the first one is written");
    connection
        .execute("UPDATE op_log SET sent_at = 1", [])
        .expect("and the server is being told about it");

    a_pending_flag(&connection, 1, 1).expect("the next one waits behind it");
    assert!(
        a_pending_flag(&connection, 1, 1).is_err(),
        "a second unsent instruction on the same target was allowed"
    );
}

#[test]
fn the_upgrade_that_owes_reads_can_be_rolled_back() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    an_account(&connection);
    a_pending_flag(&connection, 1, 1).expect("an instruction");

    profile_migrations()
        .to_version(&mut connection, WHILE_NOTHING_OWED_A_READ)
        .expect("the down migration runs");

    let columns: i64 = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_info('op_log') WHERE name = 'owes_a_read'",
            [],
            |row| row.get(0),
        )
        .expect("readable");
    assert_eq!(columns, 0);
    assert!(
        a_pending_flag(&connection, 1, 1).is_err(),
        "the old rule is back: one waiting instruction per target"
    );
}

/// A queue holding one instruction on its way and another written behind it rolls back all the
/// same, keeping the later one.
///
/// The version before allows one waiting instruction per target, and this one allows two while
/// the first is being told. Without collapsing them the rule could not be made again, and the
/// rollback would stop halfway.
#[test]
fn the_upgrade_that_owes_reads_rolls_back_over_two_instructions_in_line() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    an_account(&connection);

    a_pending_flag(&connection, 1, 1).expect("the first one is written");
    connection
        .execute("UPDATE op_log SET sent_at = 1", [])
        .expect("and the server is being told about it");
    connection
        .execute(
            "INSERT INTO op_log (account_id, message_id, folder_id, kind, flags_remove, asked_at)
                 VALUES ('acc-1', 1, 1, 'set_flags', 1, 0)",
            [],
        )
        .expect("the opposite waits behind it");

    profile_migrations()
        .to_version(&mut connection, WHILE_NOTHING_OWED_A_READ)
        .expect("the down migration runs");

    let left: Vec<(i64, i64)> = connection
        .prepare("SELECT flags_add, flags_remove FROM op_log")
        .expect("readable")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("rows")
        .map(|row| row.expect("row"))
        .collect();
    assert_eq!(left, vec![(0, 1)], "the person's last word is the one kept");
}

/// The version the profile was at while nothing said when an instruction was carried out.
const WHILE_NOTHING_SAID_WHEN_DONE: usize = 20;

/// What was carried out before the upgrade says nothing about when, and keeps what it had.
///
/// Measured from when it was asked for instead, which is what a pass reading it back falls back
/// to: making a date up for it would either settle a move the index has not caught up with, or
/// keep laying one over the index that it long since has.
#[test]
fn instructions_carried_out_before_the_upgrade_say_nothing_about_when() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, WHILE_NOTHING_SAID_WHEN_DONE)
        .expect("the previous schema applies");
    an_account(&connection);
    connection
        .execute(
            "INSERT INTO op_log (account_id, message_id, folder_id, kind, move_to, state, asked_at,
                                 owes_a_read)
                 VALUES ('acc-1', 1, 1, 'move', 2, 'done', 7, 1)",
            [],
        )
        .expect("a row written by the previous version");

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let kept: (i64, i64, Option<i64>) = connection
        .query_row(
            "SELECT asked_at, owes_a_read, done_at FROM op_log",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("readable");
    assert_eq!(kept, (7, 1, None));
}

#[test]
fn the_upgrade_that_says_when_something_was_done_can_be_rolled_back() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    an_account(&connection);
    connection
        .execute(
            "INSERT INTO op_log (account_id, message_id, folder_id, kind, move_to, state, asked_at,
                                 done_at)
                 VALUES ('acc-1', 1, 1, 'move', 2, 'done', 7, 9)",
            [],
        )
        .expect("an instruction carried out");

    profile_migrations()
        .to_version(&mut connection, WHILE_NOTHING_SAID_WHEN_DONE)
        .expect("the down migration runs");

    let columns: i64 = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_info('op_log') WHERE name = 'done_at'",
            [],
            |row| row.get(0),
        )
        .expect("readable");
    assert_eq!(columns, 0);
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM op_log", [], |row| row.get(0))
        .expect("readable");
    assert_eq!(rows, 1, "the instruction itself survives the rollback");
}

/// The version the profile was at while nothing counted what a person had asked for.
const WHILE_NOTHING_COUNTED_WHAT_WAS_ASKED: usize = 21;

/// A queue upgraded with instructions in it starts counting from nought, and counts from there.
///
/// Nought rather than how many rows it holds: the count orders writes against reads of the
/// sidebar, and a window open across the upgrade does not exist — the application restarts.
#[test]
fn a_queue_upgraded_with_instructions_in_it_counts_from_nought() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, WHILE_NOTHING_COUNTED_WHAT_WAS_ASKED)
        .expect("the previous schema applies");
    an_account(&connection);
    a_pending_flag(&connection, 1, 1).expect("a row written by the previous version");

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let counted: (i64, i64) = connection
        .query_row(
            "SELECT (SELECT last FROM asked_counter), (SELECT count(*) FROM op_log)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("readable");
    assert_eq!(
        counted,
        (0, 1),
        "the instruction survives and the count starts"
    );
    assert!(
        connection
            .execute("INSERT INTO asked_counter (id, last) VALUES (2, 0)", [])
            .is_err(),
        "a second count would let two writes answer with the same number"
    );
}

#[test]
fn the_upgrade_that_counts_what_was_asked_can_be_rolled_back() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    an_account(&connection);
    a_pending_flag(&connection, 1, 1).expect("an instruction");

    profile_migrations()
        .to_version(&mut connection, WHILE_NOTHING_COUNTED_WHAT_WAS_ASKED)
        .expect("the down migration runs");

    let tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'asked_counter'",
            [],
            |row| row.get(0),
        )
        .expect("readable");
    assert_eq!(tables, 0);
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM op_log", [], |row| row.get(0))
        .expect("readable");
    assert_eq!(rows, 1, "the instruction itself survives the rollback");
}

/// The version the profile was at while nothing said when a server took a message.
const WHILE_NOTHING_SAID_WHEN_A_SERVER_TOOK_IT: usize = 22;

/// A message sent by the previous version, written an hour ago and not yet filed.
fn sent_an_hour_ago(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO outbox
                 (id, account_id, message_id, sender, subject, body, state, written_at)
             VALUES (1, 'acc-1', '<m@example.com>', 'user@example.com', 'S', 'B', 'sent',
                     unixepoch() - 3600)",
            [],
        )
        .expect("a message the previous version sent");
}

/// A message a server took under the previous version comes through the upgrade as it was, and
/// says nothing about when the server took it: that version wrote nothing down, and the outbox
/// counts such a row from when it was written.
#[test]
fn a_message_sent_before_the_upgrade_keeps_everything_and_claims_no_moment() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, WHILE_NOTHING_SAID_WHEN_A_SERVER_TOOK_IT)
        .expect("the previous schema applies");
    an_account(&connection);
    sent_an_hour_ago(&connection);

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let (state, accepted_at): (String, Option<i64>) = connection
        .query_row(
            "SELECT state, accepted_at FROM outbox WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the message survives the upgrade");
    assert_eq!(state, "sent");
    assert_eq!(accepted_at, None, "a moment nobody wrote down was invented");
}

#[test]
fn the_upgrade_that_says_when_a_server_took_a_message_can_be_rolled_back() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    an_account(&connection);
    sent_an_hour_ago(&connection);
    connection
        .execute(
            "UPDATE outbox SET accepted_at = unixepoch() WHERE id = 1",
            [],
        )
        .expect("a server took it a moment ago");

    profile_migrations()
        .to_version(&mut connection, WHILE_NOTHING_SAID_WHEN_A_SERVER_TOOK_IT)
        .expect("the down migration runs");

    let columns: i64 = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_info('outbox') WHERE name = 'accepted_at'",
            [],
            |row| row.get(0),
        )
        .expect("readable");
    assert_eq!(columns, 0);
    let rows: i64 = connection
        .query_row("SELECT count(*) FROM outbox", [], |row| row.get(0))
        .expect("readable");
    assert_eq!(rows, 1, "the message itself survives the rollback");
}

/// The version the profile was at while a draft held text and nothing else.
const WHILE_A_DRAFT_HELD_NO_FILES: usize = 23;

/// A draft, and a message the previous version held for a person with the file it carried, as
/// that version left them: the draft it gave back holds the text, and the file stayed behind in
/// the outbox.
fn given_back_without_its_file(connection: &Connection) {
    connection
        .execute_batch(
            "INSERT INTO drafts (id, account_id, recipients, subject, body, written_at)
                 VALUES (1, 'acc-1', 'kappa@example.net', 'About Tuesday', 'Shall we?', 0);
             INSERT INTO outbox
                 (id, account_id, message_id, sender, subject, body, state, held_because,
                  written_at)
                 VALUES (1, 'acc-1', '<m@example.com>', 'user@example.com', 'About Tuesday',
                         'Shall we?', 'held', 'signInFailed', 0);
             INSERT INTO outbox_files (outbox_id, name, content_type, bytes, position)
                 VALUES (1, 'invoice.pdf', 'application/pdf', X'25504446', 0);",
        )
        .expect("what the previous version wrote");
}

/// A draft written before the upgrade keeps everything it held, and is not a message that came
/// back: that version wrote nothing down that says so, and the held row beside it names no draft.
/// The file that version left in the outbox stays where it is, because nothing says whose it is.
#[test]
fn a_draft_kept_before_the_upgrade_keeps_its_text_carries_nothing_and_did_not_come_back() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, WHILE_A_DRAFT_HELD_NO_FILES)
        .expect("the previous schema applies");
    an_account(&connection);
    given_back_without_its_file(&connection);

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let (subject, body, came_back): (String, String, i64) = connection
        .query_row(
            "SELECT subject, body, came_back FROM drafts WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("the draft survives the upgrade");
    assert_eq!(
        (subject.as_str(), body.as_str()),
        ("About Tuesday", "Shall we?")
    );
    assert_eq!(
        came_back, 0,
        "a draft nothing marked was counted as come back"
    );
    let (carried, left): (i64, i64) = connection
        .query_row(
            "SELECT (SELECT count(*) FROM draft_files), (SELECT count(*) FROM outbox_files)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("readable");
    assert_eq!((carried, left), (0, 1));
}

#[test]
fn the_upgrade_that_lets_a_draft_carry_files_can_be_rolled_back() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    an_account(&connection);
    connection
        .execute_batch(
            "INSERT INTO drafts (id, account_id, subject, written_at, came_back)
                 VALUES (1, 'acc-1', 'About Tuesday', 0, 1);
             INSERT INTO draft_files (draft_id, kept_as, name, content_type, bytes, position)
                 VALUES (1, 't1', 'invoice.pdf', 'application/pdf', X'25504446', 0);",
        )
        .expect("a draft that came back with a file");

    profile_migrations()
        .to_version(&mut connection, WHILE_A_DRAFT_HELD_NO_FILES)
        .expect("the down migration runs");

    let (tables, columns, drafts): (i64, i64, i64) = connection
        .query_row(
            "SELECT (SELECT count(*) FROM sqlite_master WHERE name = 'draft_files'),
                    (SELECT count(*) FROM pragma_table_info('drafts') WHERE name = 'came_back'),
                    (SELECT count(*) FROM drafts)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("readable");
    assert_eq!((tables, columns), (0, 0));
    assert_eq!(drafts, 1, "the draft itself survives the rollback");
}

/// The version the profile was at while nothing said which draft a message came back as.
const WHILE_NOTHING_SAID_WHAT_A_MESSAGE_CAME_BACK_AS: usize = 24;

/// A message the previous version gave back and held comes through the upgrade as it was, and
/// says nothing about which draft it came back as: that version wrote nothing down, and no round
/// reads a held message again to need it.
#[test]
fn a_message_held_before_the_upgrade_says_nothing_about_what_it_came_back_as() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(
            &mut connection,
            WHILE_NOTHING_SAID_WHAT_A_MESSAGE_CAME_BACK_AS,
        )
        .expect("the previous schema applies");
    an_account(&connection);
    given_back_without_its_file(&connection);

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let (state, came_back_as): (String, Option<i64>) = connection
        .query_row(
            "SELECT state, came_back_as FROM outbox WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the message survives the upgrade");
    assert_eq!(state, "held");
    assert_eq!(came_back_as, None, "a draft nobody wrote down was named");
}

#[test]
fn the_upgrade_that_says_what_a_message_came_back_as_can_be_rolled_back() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    an_account(&connection);
    given_back_without_its_file(&connection);
    connection
        .execute("UPDATE outbox SET came_back_as = 1 WHERE id = 1", [])
        .expect("it came back as the draft beside it");

    profile_migrations()
        .to_version(
            &mut connection,
            WHILE_NOTHING_SAID_WHAT_A_MESSAGE_CAME_BACK_AS,
        )
        .expect("the down migration runs");

    let (columns, rows): (i64, i64) = connection
        .query_row(
            "SELECT (SELECT count(*) FROM pragma_table_info('outbox') WHERE name = 'came_back_as'),
                    (SELECT count(*) FROM outbox)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("readable");
    assert_eq!(columns, 0);
    assert_eq!(rows, 1, "the message itself survives the rollback");
}

/// The version the profile was at while nothing counted the refusals of a sent message's copy.
const WHILE_NOTHING_COUNTED_A_REFUSED_COPY: usize = 25;

/// A message sent under the previous version and not filed yet comes through the upgrade as it was:
/// refused by nobody, and due to be asked about at once. That version wrote nothing down about
/// refusals, and one taken as already refused would be let go sooner for it.
#[test]
fn a_message_sent_before_the_upgrade_is_counted_as_refused_by_nobody_and_due_at_once() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, WHILE_NOTHING_COUNTED_A_REFUSED_COPY)
        .expect("the previous schema applies");
    an_account(&connection);
    sent_an_hour_ago(&connection);

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let (state, refused, due): (String, i64, bool) = connection
        .query_row(
            "SELECT state, filing_refused, file_after <= unixepoch() FROM outbox WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("the message survives the upgrade");
    assert_eq!(state, "sent");
    assert_eq!(refused, 0, "a refusal nobody heard was counted");
    assert!(due, "a copy nobody refused was told to wait");
}

#[test]
fn the_upgrade_that_counts_refused_copies_can_be_rolled_back() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    an_account(&connection);
    sent_an_hour_ago(&connection);
    connection
        .execute(
            "UPDATE outbox SET filing_refused = 2, file_after = unixepoch() + 3600 WHERE id = 1",
            [],
        )
        .expect("refused twice, and waiting");

    profile_migrations()
        .to_version(&mut connection, WHILE_NOTHING_COUNTED_A_REFUSED_COPY)
        .expect("the down migration runs");

    let (columns, rows): (i64, i64) = connection
        .query_row(
            "SELECT (SELECT count(*) FROM pragma_table_info('outbox')
                     WHERE name IN ('filing_refused', 'file_after')),
                    (SELECT count(*) FROM outbox)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("readable");
    assert_eq!(columns, 0);
    assert_eq!(rows, 1, "the message itself survives the rollback");
}

/// The version the profile was at while no pane could be left folded.
const WHILE_NO_PANE_FOLDED: usize = 26;

/// A profile from before the sidebar could fold opens it: nothing in it says otherwise, and the
/// table that would is there and empty.
#[test]
fn a_profile_from_before_folding_opens_every_pane() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, WHILE_NO_PANE_FOLDED)
        .expect("the previous schema applies");
    connection
        .execute(
            "INSERT INTO device_pane_width (pane, width) VALUES ('sidebar', 300)",
            [],
        )
        .expect("a width left before the upgrade");

    migrate_profile(&mut connection, ":memory:").expect("upgrade applies");

    let (folded, width): (i64, i64) = connection
        .query_row(
            "SELECT (SELECT count(*) FROM device_pane_folded),
                    (SELECT width FROM device_pane_width WHERE pane = 'sidebar')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("both tables are readable");
    assert_eq!(folded, 0, "a pane came through the upgrade folded");
    assert_eq!(width, 300, "the width left before the upgrade was lost");
}

#[test]
fn the_upgrade_that_folds_panes_can_be_rolled_back() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    connection
        .execute(
            "INSERT INTO device_pane_folded (pane) VALUES ('sidebar')",
            [],
        )
        .expect("the sidebar folded");

    profile_migrations()
        .to_version(&mut connection, WHILE_NO_PANE_FOLDED)
        .expect("the down migration runs");

    let tables: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name = 'device_pane_folded'",
            [],
            |row| row.get(0),
        )
        .expect("readable");
    assert_eq!(tables, 0);
}

/// A draft thrown away or sent takes the files it was carrying with it, and a draft holds a file
/// under one name once.
///
/// A row pointing at a draft that is gone is a file nothing will ever send and nothing will ever
/// clear — in the one database this application never drops.
#[test]
fn removing_a_draft_removes_the_files_it_was_carrying() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    migrate_profile(&mut connection, ":memory:").expect("schema applies");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("keys are enforced here as they are in the application");
    an_account(&connection);
    connection
        .execute_batch(
            "INSERT INTO drafts (id, account_id, written_at) VALUES (1, 'acc-1', 0);
             INSERT INTO drafts (id, account_id, written_at) VALUES (2, 'acc-1', 0);
             INSERT INTO draft_files (draft_id, kept_as, name, content_type, bytes, position)
                 VALUES (1, 't1', 'invoice.pdf', 'application/pdf', X'25504446', 0);
             INSERT INTO draft_files (draft_id, kept_as, name, content_type, bytes, position)
                 VALUES (2, 't1', 'invoice.pdf', 'application/pdf', X'25504446', 0);",
        )
        .expect("two drafts, each with a file");
    assert!(
        connection
            .execute(
                "INSERT INTO draft_files (draft_id, kept_as, name, content_type, bytes, position)
                     VALUES (1, 't1', 'again.pdf', 'application/pdf', X'25', 1)",
                [],
            )
            .is_err(),
        "a draft held two files under one name"
    );

    connection
        .execute("DELETE FROM drafts WHERE id = 1", [])
        .expect("thrown away");

    let left: Vec<i64> = connection
        .prepare("SELECT draft_id FROM draft_files")
        .expect("readable")
        .query_map([], |row| row.get(0))
        .expect("rows")
        .map(|row| row.expect("row"))
        .collect();
    assert_eq!(
        left,
        vec![2],
        "the other draft's file went too, or this one's stayed"
    );
}

/// A file a newer build wrote is that build's, and nothing here migrates downward.
///
/// Refused before anything runs against it, saying the version it was found at, and left at that
/// version: the newer build reads it as it stands, and a migration this build does not have is not
/// one it can undo.
#[test]
fn a_file_a_newer_build_wrote_is_refused_and_left_at_its_version() {
    type Migrate = fn(&mut Connection, &str) -> Result<(), StorageError>;
    let kinds: [(&str, Migrate); 2] = [("profile", migrate_profile), ("index", migrate_index)];

    for (kind, migrate) in kinds {
        let mut connection = Connection::open_in_memory().expect("in-memory database opens");
        migrate(&mut connection, ":memory:").expect("schema applies");
        let newest = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("user_version is readable") as usize;
        connection
            .execute_batch(&format!(
                "CREATE TABLE written_later (kept INTEGER); PRAGMA user_version = {};",
                newest + 1
            ))
            .expect("what a newer build wrote");

        let refused = migrate(&mut connection, ":memory:").expect_err("a newer build's file");

        assert!(
            matches!(refused, StorageError::FromANewerBuild { found, .. } if found == newest + 1),
            "{kind}: {refused}"
        );
        let after = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .expect("user_version is readable") as usize;
        assert_eq!(
            after,
            newest + 1,
            "{kind}: the version a newer build left was changed"
        );
    }
}

/// The profile while every group was drawn above every account in no group.
const WHILE_GROUPS_CAME_FIRST: usize = 27;

#[test]
fn a_profile_keeps_its_sidebar_when_groups_and_accounts_share_one_order() {
    let mut connection = Connection::open_in_memory().expect("in-memory database opens");
    profile_migrations()
        .to_version(&mut connection, WHILE_GROUPS_CAME_FIRST)
        .expect("the schema while groups came first");

    // Two groups at places that are not their final ones, two accounts that agree on a place and
    // differ in when they arrived, one dragged further down, and one inside a group.
    connection
        .execute_batch(
            "INSERT INTO account_groups (id, name, position) VALUES
                 ('g-home', 'Home', 4),
                 ('g-work', 'Work', 1);
             INSERT INTO mail_accounts
                 (id, email, display_name, provider, index_id, created_at,
                  position_in_group, group_id)
             VALUES
                 ('acc-second', 'kappa@example.net', '', 'imap', 1, '2', 0, NULL),
                 ('acc-first', 'zeta@example.com', '', 'imap', 2, '1', 0, NULL),
                 ('acc-dragged', 'sigma@example.org', '', 'imap', 3, '0', 3, NULL),
                 ('acc-in-work', 'zeta.quorin@example.com', '', 'imap', 4, '0', 7, 'g-work');",
        )
        .expect("a sidebar arranged before");

    migrate_profile(&mut connection, ":memory:").expect("one order");

    let groups: Vec<(String, i64)> = connection
        .prepare("SELECT id, position FROM account_groups ORDER BY position")
        .expect("readable")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("readable")
        .collect::<Result<_, _>>()
        .expect("rows");
    let accounts: Vec<(String, i64, Option<String>)> = connection
        .prepare("SELECT id, position_in_group, group_id FROM mail_accounts ORDER BY id")
        .expect("readable")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("readable")
        .collect::<Result<_, _>>()
        .expect("rows");

    assert_eq!(
        groups,
        vec![("g-work".to_owned(), 0), ("g-home".to_owned(), 1)],
        "the groups first, in the order they had"
    );
    assert_eq!(
        accounts,
        vec![
            ("acc-dragged".to_owned(), 4, None),
            ("acc-first".to_owned(), 2, None),
            ("acc-in-work".to_owned(), 7, Some("g-work".to_owned())),
            ("acc-second".to_owned(), 3, None),
        ],
        "then the accounts in no group, in theirs; one inside a group keeps its place in it"
    );
}
