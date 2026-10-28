use super::*;
use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::infrastructure::sqlite::{connection::open_index, migrations::migrate_index};
use lyonoyl_domain::message::{ModSeq, Rfc822MessageId};
use lyonoyl_domain::sync::{FlagUpdate, MessageEnvelope, VanishedReport};

struct TempIndex {
    dir: std::path::PathBuf,
    index: SqliteMailIndex,
    pool: SqlitePool,
}

impl TempIndex {
    fn new(name: &str) -> Self {
        // A count of its own, because a thread is not one. Two of these made inside one loop on one
        // thread were handed the same directory — and on Windows, where a file that is open cannot
        // be deleted, the first one's database was still sitting there when the second tried to
        // write its folders. On macOS the same code passes: an open file can be unlinked there, so
        // the first directory really did go.
        static MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let count = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-index-{name}-{}-{count}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("index.sqlite3");
        let pool = open_index(&path, &DatabaseKey::for_a_test()).expect("pool opens");
        let mut connection = pool.get().expect("connection");
        migrate_index(&mut connection, &path.display().to_string()).expect("schema");
        connection
            .execute_batch(
                "INSERT INTO folders (id, account_id, name, role) VALUES (1, 1, 'INBOX', 1);
                     INSERT INTO folders (id, account_id, name, role) VALUES (2, 1, 'Archive', 6);",
            )
            .expect("folders");
        drop(connection);

        Self {
            dir,
            index: SqliteMailIndex::new(pool.clone()),
            pool,
        }
    }

    fn count(&self, sql: &str) -> i64 {
        self.pool
            .get()
            .expect("connection")
            .query_row(sql, [], |row| row.get(0))
            .expect("count is readable")
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        // Swallowed, and it has to be: on Windows the pool may still hold the database open when
        // this runs, and a fixture that panicked while tidying up would report the wrong failure.
        // What must not depend on it is the next fixture — hence the count above.
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn folder(id: u64) -> FolderId {
    FolderId::new(id).expect("non-zero")
}

fn uid(value: u32) -> Uid {
    Uid::new(value).expect("non-zero")
}

fn generation(value: u32) -> UidValidity {
    UidValidity::new(value).expect("non-zero")
}

fn arrival(value: u32, header: Option<&str>, flags: Flags) -> ArrivedMessage {
    ArrivedMessage {
        uid: uid(value),
        modseq: Some(ModSeq::new(u64::from(value))),
        envelope: MessageEnvelope {
            rfc822_message_id: header
                .map(|value| Rfc822MessageId::parse(value).expect("well formed")),
            subject: Some(format!("Subject {value}")),
            sender: Some("sender@example.com".to_owned()),
            sender_address: Some("sender@example.com".to_owned()),
            to: Vec::new(),
            cc: Vec::new(),
            received_at: 1_700_000_000 + i64::from(value),
            rfc822_size: Some(1024),
            preview: None,
            references: Vec::new(),
            flags,
        },
    }
}

/// An arrival that answers another, so the two are put in one conversation.
fn reply(value: u32, header: &str, answering: &str) -> ArrivedMessage {
    let mut message = arrival(value, Some(header), Flags::NONE);
    message.envelope.references = vec![Rfc822MessageId::parse(answering).expect("well formed")];
    message
}

fn delta(folder_id: u64, arrived: Vec<ArrivedMessage>) -> FolderDelta {
    FolderDelta {
        folder_id: folder(folder_id),
        uid_validity: generation(7),
        covered_from: None,
        stopped_short: false,
        arrived,
        flag_updates: Vec::new(),
        vanished: VanishedReport::Unknown,
    }
}

#[test]
fn arrivals_are_stored_with_their_placement_and_the_cursor() {
    let db = TempIndex::new("arrivals");
    let summary = db
        .index
        .apply(
            &delta(1, vec![arrival(10, Some("<a@example.com>"), Flags::SEEN)]),
            &[],
            &SyncCursor::new("120"),
            FlagScope::Message,
        )
        .expect("the delta applies");

    assert_eq!(summary.added, 1);
    assert_eq!(db.count("SELECT count(*) FROM messages"), 1);
    assert_eq!(db.count("SELECT count(*) FROM message_placements"), 1);

    let state = db.index.folder_state(folder(1)).expect("state is readable");
    assert_eq!(state.cursor.expect("cursor stored").as_str(), "120");
    assert_eq!(state.uid_validity, Some(generation(7)));
}

#[test]
fn a_pass_that_stopped_short_is_written_down_with_its_cursor_and_the_next_says_otherwise() {
    let db = TempIndex::new("stopped-short");
    let short = FolderDelta {
        stopped_short: true,
        ..delta(2, vec![arrival(10, None, Flags::NONE)])
    };

    db.index
        .apply(&short, &[], &SyncCursor::new("6997"), FlagScope::Message)
        .expect("the delta applies");
    assert!(
        db.index
            .folder_state(folder(2))
            .expect("state is readable")
            .stopped_short
    );
    assert!(
        !db.index
            .folder_state(folder(1))
            .expect("state is readable")
            .stopped_short,
        "only the folder the pass was about"
    );

    db.index
        .apply(
            &delta(2, Vec::new()),
            &[],
            &SyncCursor::new("8999"),
            FlagScope::Message,
        )
        .expect("the delta applies");
    assert!(
        !db.index
            .folder_state(folder(2))
            .expect("state is readable")
            .stopped_short,
        "read to its end this time"
    );
}

#[test]
fn a_folder_no_longer_stopped_short_once_taken_back_or_renumbered() {
    let db = TempIndex::new("stopped-short-taken-back");
    let short = || FolderDelta {
        stopped_short: true,
        ..delta(2, Vec::new())
    };
    let stopped_short = |db: &TempIndex| {
        db.index
            .folder_state(folder(2))
            .expect("state is readable")
            .stopped_short
    };

    db.index
        .apply(&short(), &[], &SyncCursor::new("6997"), FlagScope::Message)
        .expect("the delta applies");
    db.index
        .forget_it_stopped_short(folder(2))
        .expect("taken back");
    assert!(!stopped_short(&db));
    assert_eq!(
        db.index
            .folder_state(folder(2))
            .expect("state is readable")
            .cursor
            .expect("the cursor stays")
            .as_str(),
        "6997",
        "and nothing else changes"
    );

    db.index
        .apply(&short(), &[], &SyncCursor::new("6997"), FlagScope::Message)
        .expect("the delta applies");
    db.index
        .reset_folder(folder(2), generation(9))
        .expect("the folder is reset");
    assert!(
        !stopped_short(&db),
        "a renumbered folder has no position to have stopped short of"
    );
}

#[test]
fn a_reading_refused_for_a_moment_is_written_down_apart_from_the_other_and_gone_once_renumbered() {
    use crate::ports::mail_index::Reading;

    let db = TempIndex::new("refused-for-now");
    let refused = |db: &TempIndex| {
        let state = db.index.folder_state(folder(2)).expect("state is readable");
        (
            state.new_mail_refused_for_now,
            state.further_back_refused_for_now,
        )
    };
    assert_eq!(refused(&db), (false, false), "nothing was refused yet");

    db.index
        .refused_for_now(folder(2), Reading::NewMail, true)
        .expect("written down");
    assert_eq!(refused(&db), (true, false));

    db.index
        .refused_for_now(folder(2), Reading::FurtherBack, true)
        .expect("written down");
    db.index
        .refused_for_now(folder(2), Reading::NewMail, false)
        .expect("taken back");
    assert_eq!(
        refused(&db),
        (false, true),
        "taking one back leaves the other where it was"
    );

    db.index
        .refused_for_now(folder(2), Reading::NewMail, true)
        .expect("written down");
    db.index
        .reset_folder(folder(2), generation(9))
        .expect("the folder is reset");
    assert_eq!(
        refused(&db),
        (false, false),
        "a renumbered folder was refused nothing in its new numbering"
    );
}

#[test]
fn the_same_message_in_two_folders_keeps_one_identity() {
    let db = TempIndex::new("dedup");
    let header = Some("<shared@example.com>");

    db.index
        .apply(
            &delta(1, vec![arrival(10, header, Flags::NONE)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("first folder");
    db.index
        .apply(
            &delta(2, vec![arrival(77, header, Flags::NONE)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("second folder");

    assert_eq!(
        db.count("SELECT count(*) FROM messages"),
        1,
        "one message, not a copy per folder"
    );
    assert_eq!(db.count("SELECT count(*) FROM message_placements"), 2);
}

#[test]
fn a_message_without_a_header_is_never_merged_with_another() {
    let db = TempIndex::new("no-header");
    db.index
        .apply(
            &delta(
                1,
                vec![
                    arrival(10, None, Flags::NONE),
                    arrival(11, None, Flags::NONE),
                ],
            ),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("both arrivals apply");

    assert_eq!(db.count("SELECT count(*) FROM messages"), 2);
}

#[test]
fn flags_go_to_the_message_when_the_provider_owns_them() {
    let db = TempIndex::new("flags-message");
    db.index
        .apply(
            &delta(1, vec![arrival(10, None, Flags::SEEN)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");

    assert_eq!(db.count("SELECT flags FROM messages"), 1);
    // And the copy carries them too, whoever the provider thinks owns them: what a window and
    // a count read is the copy, and reading it must never mean looking the message up.
    assert_eq!(db.count("SELECT flags FROM message_placements"), 1);
}

#[test]
fn flags_go_to_the_placement_when_the_provider_keeps_them_per_mailbox() {
    let db = TempIndex::new("flags-placement");
    db.index
        .apply(
            &delta(1, vec![arrival(10, None, Flags::SEEN)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Placement,
        )
        .expect("applies");

    // The message's own value is left alone: this provider says nothing about the message,
    // only about this copy of it.
    assert_eq!(db.count("SELECT flags FROM messages"), 0);
    assert_eq!(db.count("SELECT flags FROM message_placements"), 1);
}

#[test]
fn a_flag_update_reaches_the_place_the_provider_owns() {
    let db = TempIndex::new("flag-update");
    db.index
        .apply(
            &delta(1, vec![arrival(10, None, Flags::NONE)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");

    let mut changed = delta(1, Vec::new());
    changed.flag_updates = vec![FlagUpdate {
        uid: uid(10),
        flags: Flags::SEEN.with(Flags::FLAGGED),
        modseq: Some(ModSeq::new(9)),
    }];

    let summary = db
        .index
        .apply(&changed, &[], &SyncCursor::new("2"), FlagScope::Message)
        .expect("applies");

    assert_eq!(summary.updated, 1);
    assert_eq!(
        db.count("SELECT flags FROM messages"),
        i64::from(Flags::SEEN.with(Flags::FLAGGED).bits())
    );
}

#[test]
fn a_message_stored_without_its_opening_line_gets_one_when_it_is_read_again() {
    // A message stored by a pass that took only its envelope is the same message.
    let db = TempIndex::new("preview-fill");
    let mut first = arrival(10, Some("<a@example.com>"), Flags::NONE);
    first.envelope.preview = None;

    db.index
        .apply(
            &delta(1, vec![first]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");

    let mut again = arrival(11, Some("<a@example.com>"), Flags::NONE);
    again.envelope.preview = Some("The opening line.".to_owned());

    db.index
        .apply(
            &delta(1, vec![again]),
            &[],
            &SyncCursor::new("2"),
            FlagScope::Message,
        )
        .expect("applies");

    let stored: Option<String> = db
        .pool
        .get()
        .expect("connection")
        .query_row("SELECT preview FROM messages", [], |row| row.get(0))
        .expect("the message is there");
    assert_eq!(stored.as_deref(), Some("The opening line."));
}

#[test]
fn an_opening_line_already_read_is_not_replaced() {
    // What is there was read from the message itself; a later pass sees no more of it.
    let db = TempIndex::new("preview-keep");
    let mut first = arrival(10, Some("<a@example.com>"), Flags::NONE);
    first.envelope.preview = Some("What the message says.".to_owned());

    db.index
        .apply(
            &delta(1, vec![first]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");

    let mut again = arrival(11, Some("<a@example.com>"), Flags::NONE);
    again.envelope.preview = Some("A truncated ver".to_owned());

    db.index
        .apply(
            &delta(1, vec![again]),
            &[],
            &SyncCursor::new("2"),
            FlagScope::Message,
        )
        .expect("applies");

    let stored: Option<String> = db
        .pool
        .get()
        .expect("connection")
        .query_row("SELECT preview FROM messages", [], |row| row.get(0))
        .expect("the message is there");
    assert_eq!(stored.as_deref(), Some("What the message says."));
}

#[test]
fn a_flag_that_did_not_change_is_not_a_change() {
    // A pass that re-reads the flags of a quiet folder must write nothing and report nothing.
    // Counting it moves the folder's epoch, and the list re-anchors under the reader.
    for scope in [FlagScope::Message, FlagScope::Placement] {
        let db = TempIndex::new("flag-unchanged");
        db.index
            .apply(
                &delta(1, vec![arrival(10, None, Flags::SEEN)]),
                &[],
                &SyncCursor::new("1"),
                scope,
            )
            .expect("applies");

        let mut same = delta(1, Vec::new());
        same.flag_updates = vec![FlagUpdate {
            uid: uid(10),
            flags: Flags::SEEN,
            modseq: None,
        }];

        let epoch_before = db.count("SELECT view_epoch FROM folders WHERE id = 1");
        let summary = db
            .index
            .apply(&same, &[], &SyncCursor::new("2"), scope)
            .expect("applies");

        assert_eq!(summary.updated, 0, "scope: {scope:?}");
        assert_eq!(
            db.count("SELECT view_epoch FROM folders WHERE id = 1"),
            epoch_before,
            "an unchanged flag must not move the epoch, scope: {scope:?}"
        );
    }
}

#[test]
fn a_flag_that_did_change_is_still_a_change() {
    for scope in [FlagScope::Message, FlagScope::Placement] {
        let db = TempIndex::new("flag-changed");
        db.index
            .apply(
                &delta(1, vec![arrival(10, None, Flags::NONE)]),
                &[],
                &SyncCursor::new("1"),
                scope,
            )
            .expect("applies");

        let mut changed = delta(1, Vec::new());
        changed.flag_updates = vec![FlagUpdate {
            uid: uid(10),
            flags: Flags::SEEN,
            modseq: None,
        }];

        let summary = db
            .index
            .apply(&changed, &[], &SyncCursor::new("2"), scope)
            .expect("applies");

        assert_eq!(summary.updated, 1, "scope: {scope:?}");
    }
}

/// A message whose last copy went is kept by its name until the pass forgets what nothing holds,
/// and forgotten then.
///
/// Kept, because the pass may be about to read the folder the message went to, and a message
/// found again by its `Message-ID` keeps its name — which every instruction about it names.
#[test]
fn a_message_whose_last_copy_went_is_kept_until_the_pass_forgets_it() {
    let db = TempIndex::new("delete-last");
    db.index
        .apply(
            &delta(1, vec![arrival(10, None, Flags::NONE)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");

    let summary = db
        .index
        .apply(
            &delta(1, Vec::new()),
            &[uid(10)],
            &SyncCursor::new("2"),
            FlagScope::Message,
        )
        .expect("applies");

    assert_eq!(summary.removed, 1);
    assert_eq!(db.count("SELECT count(*) FROM message_placements"), 0);
    assert_eq!(
        db.count("SELECT count(*) FROM messages"),
        1,
        "forgotten before the pass had read every folder"
    );

    db.index
        .forget_the_unplaced(1, &[])
        .expect("the sweep applies");

    assert_eq!(db.count("SELECT count(*) FROM messages"), 0);
}

/// What no folder holds is what the sweep would forget, asked without forgetting it.
///
/// A pass asks before it reads anything and again after, and a pass that could not read every
/// folder forgets only what both answers name. Once forgotten, a message is gone from search too:
/// its words stayed findable for as long as its row did.
#[test]
fn what_no_folder_holds_is_named_before_it_is_forgotten_and_gone_from_search_after() {
    let db = TempIndex::new("unplaced");
    db.index
        .apply(
            &delta(
                1,
                vec![
                    arrival(10, None, Flags::NONE),
                    arrival(11, None, Flags::NONE),
                ],
            ),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");
    assert_eq!(db.index.unplaced(1).expect("answers"), Vec::new());

    db.index
        .apply(
            &delta(1, Vec::new()),
            &[uid(10)],
            &SyncCursor::new("2"),
            FlagScope::Message,
        )
        .expect("the copy leaves");
    let gone = db.count(
        "SELECT id FROM messages WHERE id NOT IN (SELECT message_id FROM message_placements)",
    );

    assert_eq!(
        db.index.unplaced(1).expect("answers"),
        vec![MessageId::new(gone as u64).expect("non-zero")]
    );
    assert_eq!(
        db.index.unplaced(2).expect("answers"),
        Vec::new(),
        "another account's question"
    );

    db.index
        .forget_the_unplaced(1, &[])
        .expect("the sweep applies");

    assert_eq!(db.index.unplaced(1).expect("answers"), Vec::new());
    assert_eq!(
        db.count("SELECT count(*) FROM message_search WHERE message_search MATCH 'Subject'"),
        1,
        "only the message a folder still holds is found"
    );
}

/// A message the queue still speaks about outlives its last copy.
///
/// A folder that renumbered drops every copy it held, and the pass reads back only the newest of
/// them; an instruction sent back to be found there finds its message again only under the name it
/// was asked about.
#[test]
fn a_message_the_queue_still_names_is_kept_when_no_folder_holds_it() {
    let db = TempIndex::new("named-kept");
    db.index
        .apply(
            &delta(
                1,
                vec![
                    arrival(10, None, Flags::NONE),
                    arrival(11, None, Flags::NONE),
                ],
            ),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");
    let named: i64 = db.count("SELECT min(id) FROM messages");

    db.index
        .apply(
            &delta(1, Vec::new()),
            &[uid(10), uid(11)],
            &SyncCursor::new("2"),
            FlagScope::Message,
        )
        .expect("applies");
    db.index
        .forget_the_unplaced(
            1,
            &[lyonoyl_domain::message::MessageId::new(named as u64).expect("non-zero")],
        )
        .expect("the sweep applies");

    assert_eq!(db.count("SELECT count(*) FROM messages"), 1);
    assert_eq!(db.count("SELECT min(id) FROM messages"), named);
}

/// A message moved from one folder to another keeps its name, whichever of the two is read first.
#[test]
fn a_message_moved_between_folders_keeps_its_name_whichever_is_read_first() {
    let db = TempIndex::new("moved-keeps-name");
    db.index
        .apply(
            &delta(
                1,
                vec![arrival(10, Some("<moved@example.com>"), Flags::NONE)],
            ),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");
    let before = db.count("SELECT id FROM messages");

    // The inbox read first: its copy is gone.
    db.index
        .apply(
            &delta(1, Vec::new()),
            &[uid(10)],
            &SyncCursor::new("2"),
            FlagScope::Message,
        )
        .expect("the copy leaves");
    // Then the archive, where it arrived under a number of its own.
    db.index
        .apply(
            &delta(
                2,
                vec![arrival(3, Some("<moved@example.com>"), Flags::NONE)],
            ),
            &[],
            &SyncCursor::new("3"),
            FlagScope::Message,
        )
        .expect("the copy arrives");
    db.index
        .forget_the_unplaced(1, &[])
        .expect("the sweep applies");

    assert_eq!(db.count("SELECT count(*) FROM messages"), 1);
    assert_eq!(db.count("SELECT id FROM messages"), before);
    assert_eq!(
        db.count("SELECT message_id FROM message_placements WHERE folder_id = 2"),
        before
    );
}

#[test]
fn the_index_says_which_folders_hold_a_message() {
    let db = TempIndex::new("holds");
    db.index
        .apply(
            &delta(1, vec![arrival(10, None, Flags::NONE)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");
    let message = MessageId::new(db.count("SELECT id FROM messages") as u64).expect("non-zero");

    assert!(db.index.holds(folder(1), message).expect("readable"));
    assert!(!db.index.holds(folder(2), message).expect("readable"));
}

#[test]
fn a_message_still_placed_elsewhere_survives_a_removal() {
    let db = TempIndex::new("delete-one-of-two");
    let header = Some("<kept@example.com>");
    db.index
        .apply(
            &delta(1, vec![arrival(10, header, Flags::NONE)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("first folder");
    db.index
        .apply(
            &delta(2, vec![arrival(77, header, Flags::NONE)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("second folder");

    db.index
        .apply(
            &delta(1, Vec::new()),
            &[uid(10)],
            &SyncCursor::new("2"),
            FlagScope::Message,
        )
        .expect("removal applies");

    assert_eq!(
        db.count("SELECT count(*) FROM messages"),
        1,
        "the message is still in the other folder"
    );
    assert_eq!(db.count("SELECT count(*) FROM message_placements"), 1);
}

#[test]
fn removing_a_placement_that_is_not_there_changes_nothing() {
    let db = TempIndex::new("delete-missing");
    let summary = db
        .index
        .apply(
            &delta(1, Vec::new()),
            &[uid(999)],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");

    assert_eq!(summary.removed, 0);
}

#[test]
fn a_delta_for_an_unknown_folder_writes_nothing() {
    let db = TempIndex::new("unknown-folder");
    let error = db
        .index
        .apply(
            &delta(9, vec![arrival(10, None, Flags::NONE)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect_err("an unknown folder is refused");

    assert_eq!(error, MailIndexError::UnknownFolder);
    assert_eq!(db.count("SELECT count(*) FROM messages"), 0);
}

#[test]
fn known_uids_returns_only_what_is_stored() {
    let db = TempIndex::new("known");
    db.index
        .apply(
            &delta(
                1,
                vec![
                    arrival(10, None, Flags::NONE),
                    arrival(20, None, Flags::NONE),
                    arrival(30, None, Flags::NONE),
                ],
            ),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");

    let listed = db
        .index
        .known_uids(folder(1), &UidScope::Listed(vec![uid(20), uid(999)]))
        .expect("lookup succeeds");
    assert_eq!(listed, vec![uid(20)]);

    let window = db
        .index
        .known_uids(folder(1), &UidScope::Window(uid(15)..=uid(25)))
        .expect("lookup succeeds");
    assert_eq!(window, vec![uid(20)]);
}

#[test]
fn a_lookup_longer_than_one_statement_allows_is_split() {
    let db = TempIndex::new("chunked");
    let arrivals: Vec<_> = (1..=3).map(|n| arrival(n, None, Flags::NONE)).collect();
    db.index
        .apply(
            &delta(1, arrivals),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");

    // More UIDs than a single statement may bind, so the lookup has to run in chunks.
    let asked: Vec<Uid> = (1..=(LOOKUP_CHUNK as u32 * 2 + 7)).map(uid).collect();
    let found = db
        .index
        .known_uids(folder(1), &UidScope::Listed(asked))
        .expect("lookup succeeds");

    assert_eq!(found, vec![uid(1), uid(2), uid(3)]);
}

#[test]
fn resetting_a_folder_drops_its_placements_and_the_cursor() {
    let db = TempIndex::new("reset");
    db.index
        .apply(
            &delta(1, vec![arrival(10, None, Flags::NONE)]),
            &[],
            &SyncCursor::new("50"),
            FlagScope::Message,
        )
        .expect("applies");

    db.index
        .reset_folder(folder(1), generation(8))
        .expect("reset applies");
    db.index
        .forget_the_unplaced(1, &[])
        .expect("the pass sweeps");

    assert_eq!(db.count("SELECT count(*) FROM message_placements"), 0);
    assert_eq!(
        db.count("SELECT count(*) FROM messages"),
        0,
        "a message left in no folder is not kept past the pass"
    );

    let state = db.index.folder_state(folder(1)).expect("state is readable");
    assert_eq!(state.uid_validity, Some(generation(8)));
    assert!(
        state.cursor.is_none(),
        "the old cursor belongs to a UID space that no longer exists"
    );
}

#[test]
fn resetting_a_folder_forgets_how_far_back_it_was_read() {
    // A floor and a "read to the beginning" count in the numbering the server just threw away.
    // Kept, the first pass after the renumbering records its window under the old floor and the
    // folder reads as complete — and nothing older than that window is ever read again.
    let db = TempIndex::new("reset-floor");
    db.index
        .apply(
            &delta(1, vec![arrival(10, None, Flags::NONE)]),
            &[],
            &SyncCursor::new("10"),
            FlagScope::Placement,
        )
        .expect("applies");
    db.index
        .record_window_floor(folder(1), uid(1), true)
        .expect("the folder is read to its first message");

    db.index
        .reset_folder(folder(1), generation(8))
        .expect("reset applies");

    let state = db.index.folder_state(folder(1)).expect("state is readable");
    assert_eq!(
        state.window_floor, None,
        "a floor in a numbering that no longer exists"
    );
    assert!(
        !state.history_complete,
        "a renumbered folder still reads as read to its beginning"
    );
}

/// A mailbox as a server keeps it, answering a pass the way the IMAP adapter does: a first pass
/// takes the newest window, a later one continues from the cursor, and reading further back takes
/// the range it is given.
///
/// Each message is named by its place in the mailbox rather than by its number, which is what a
/// renumbering keeps and the numbers do not.
struct Renumbering {
    /// The generation, and the UID the oldest message has in it.
    numbering: std::sync::Mutex<(UidValidity, u32)>,
    holds: u32,
}

impl Renumbering {
    /// The adapter's first window, so a first pass here reads what a first pass there reads.
    const WINDOW: u32 = 500;

    fn renumber(&self, generation: UidValidity, oldest: u32) {
        *self.numbering.lock().expect("test lock") = (generation, oldest);
    }
}

impl crate::ports::mail_provider::MailSync for Renumbering {
    fn flag_scope(&self) -> FlagScope {
        FlagScope::Placement
    }

    fn fetch(
        &self,
        request: &crate::ports::mail_provider::SyncRequest,
    ) -> Result<lyonoyl_domain::sync::SyncBatch, crate::ports::mail_provider::MailProviderError>
    {
        use crate::ports::mail_provider::SyncIntent;

        let (generation, oldest) = *self.numbering.lock().expect("test lock");
        let newest = oldest + self.holds - 1;
        let stored = request
            .cursor
            .as_ref()
            .and_then(|cursor| cursor.as_str().parse::<u32>().ok())
            .filter(|_| request.uid_validity == Some(generation));

        let (range, cursor_after) = match &request.intent {
            SyncIntent::Forward => {
                let from = stored.map_or_else(
                    || (newest + 1).saturating_sub(Self::WINDOW).max(1),
                    |last| last + 1,
                );
                (from..=newest, newest.max(stored.unwrap_or(0)))
            }
            SyncIntent::Older(range) => {
                (range.start().get()..=range.end().get(), stored.unwrap_or(0))
            }
        };

        let arrived = range
            .clone()
            .filter(|number| (oldest..=newest).contains(number))
            .map(|number| {
                let place = number - oldest + 1;
                arrival(
                    number,
                    Some(&format!("<message-{place}@example.com>")),
                    Flags::SEEN,
                )
            })
            .collect();

        Ok(lyonoyl_domain::sync::SyncBatch {
            scope: lyonoyl_domain::sync::SyncScope::Mailbox,
            cursor_before: request.cursor.clone(),
            cursor_after: SyncCursor::new(cursor_after.to_string()),
            per_mailbox: vec![FolderDelta {
                folder_id: request.folder_id,
                uid_validity: generation,
                covered_from: Uid::new(*range.start()).ok(),
                stopped_short: false,
                arrived,
                flag_updates: Vec::new(),
                vanished: VanishedReport::Unknown,
            }],
        })
    }
}

#[test]
fn a_folder_renumbered_after_it_was_read_to_its_beginning_is_read_back_whole() {
    use crate::application::sync::{BackfillFolder, SyncFolder};

    let db = TempIndex::new("renumbered-whole");
    let server = Renumbering {
        numbering: std::sync::Mutex::new((generation(1), 1)),
        holds: 1_200,
    };

    // A pass and then rounds of reading further back, until the folder says it reached its first
    // message — which is what passes every two minutes do.
    let read_to_the_beginning = || {
        SyncFolder::new(&server, db.index.clone())
            .execute(folder(1))
            .expect("the pass applies");

        for _ in 0..10 {
            if BackfillFolder::new(&server, db.index.clone())
                .execute(folder(1))
                .expect("the round applies")
                .complete
            {
                return;
            }
        }
        panic!("ten rounds did not reach the first message");
    };

    read_to_the_beginning();
    assert_eq!(
        db.index.placement_count(folder(1)).expect("countable"),
        1_200
    );

    // The same twelve hundred messages, numbered again from 1001 under a new generation.
    server.renumber(generation(2), 1_001);
    read_to_the_beginning();

    assert_eq!(
        db.index.placement_count(folder(1)).expect("countable"),
        1_200,
        "a renumbered folder came back as its newest window and nothing older"
    );
    let state = db.index.folder_state(folder(1)).expect("state is readable");
    assert_eq!(state.uid_validity, Some(generation(2)));
    assert_eq!(state.window_floor, Some(uid(1)));
    assert!(state.history_complete);
}

#[test]
fn resetting_a_folder_keeps_messages_that_live_in_another_one() {
    let db = TempIndex::new("reset-shared");
    let header = Some("<shared@example.com>");
    db.index
        .apply(
            &delta(1, vec![arrival(10, header, Flags::NONE)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("first folder");
    db.index
        .apply(
            &delta(2, vec![arrival(77, header, Flags::NONE)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("second folder");

    db.index
        .reset_folder(folder(1), generation(8))
        .expect("reset applies");

    assert_eq!(db.count("SELECT count(*) FROM messages"), 1);
    assert_eq!(db.count("SELECT count(*) FROM message_placements"), 1);
}

#[test]
fn an_unknown_folder_has_no_state() {
    let db = TempIndex::new("state-missing");
    assert_eq!(
        db.index
            .folder_state(folder(9))
            .expect_err("no such folder"),
        MailIndexError::UnknownFolder
    );
}

fn record(name: &str, role: lyonoyl_domain::folder::FolderRole) -> FolderRecord {
    FolderRecord {
        name: name.to_owned(),
        role,
    }
}

#[test]
fn a_mailbox_the_account_did_not_have_is_added() {
    use lyonoyl_domain::folder::FolderRole;

    let db = TempIndex::new("record-new");
    let recorded = db
        .index
        .record_folders(1, &[record("Drafts", FolderRole::Drafts)])
        .expect("the listing is recorded");

    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, "Drafts");
    assert_eq!(
        db.count("SELECT count(*) FROM folders WHERE account_id = 1"),
        3
    );
}

#[test]
fn a_mailbox_that_is_already_there_keeps_its_identifier() {
    // The identifier is what every placement of every message in that folder points at.
    use lyonoyl_domain::folder::FolderRole;

    let db = TempIndex::new("record-existing");
    let recorded = db
        .index
        .record_folders(1, &[record("INBOX", FolderRole::Inbox)])
        .expect("the listing is recorded");

    assert_eq!(recorded[0].1, folder(1));
    assert_eq!(
        db.count("SELECT count(*) FROM folders WHERE account_id = 1"),
        2
    );
}

#[test]
fn recording_a_mailbox_again_leaves_its_place_in_the_sync_untouched() {
    // Losing the cursor would make the next pass re-read the whole mailbox; losing the
    // generation would make it drop every placement and read them back.
    use lyonoyl_domain::folder::FolderRole;

    let db = TempIndex::new("record-keeps-cursor");
    db.pool
        .get()
        .expect("connection")
        .execute(
            "UPDATE folders SET sync_cursor = '4242', uid_validity = 7 WHERE id = 1",
            [],
        )
        .expect("a folder that has been synced before");

    db.index
        .record_folders(1, &[record("INBOX", FolderRole::Inbox)])
        .expect("the listing is recorded");

    let state = db.index.folder_state(folder(1)).expect("state is readable");
    assert_eq!(
        state.cursor.map(|cursor| cursor.as_str().to_owned()),
        Some("4242".to_owned())
    );
    assert_eq!(state.uid_validity, UidValidity::new(7).ok());
}

#[test]
fn a_role_the_server_started_declaring_is_taken_up() {
    use lyonoyl_domain::folder::FolderRole;

    let db = TempIndex::new("record-role");
    db.index
        .record_folders(1, &[record("Archive", FolderRole::Junk)])
        .expect("the listing is recorded");

    let role: i64 = db
        .pool
        .get()
        .expect("connection")
        .query_row("SELECT role FROM folders WHERE id = 2", [], |row| {
            row.get(0)
        })
        .expect("role is readable");
    assert_eq!(FolderRole::from_stored(role), FolderRole::Junk);
}

#[test]
fn two_accounts_may_both_have_a_mailbox_of_the_same_name() {
    use lyonoyl_domain::folder::FolderRole;

    let db = TempIndex::new("record-two-accounts");
    let recorded = db
        .index
        .record_folders(2, &[record("INBOX", FolderRole::Inbox)])
        .expect("the listing is recorded");

    assert_ne!(
        recorded[0].1,
        folder(1),
        "the second account's inbox is not the first account's inbox"
    );
}

#[test]
fn a_mailbox_missing_from_a_listing_is_left_alone() {
    // A partial answer from a server is not proof that a mailbox is gone, and the mail in it
    // is the only copy on this device.
    use lyonoyl_domain::folder::FolderRole;

    let db = TempIndex::new("record-no-removal");
    db.index
        .record_folders(1, &[record("INBOX", FolderRole::Inbox)])
        .expect("the listing is recorded");

    assert_eq!(
        db.count("SELECT count(*) FROM folders WHERE account_id = 1"),
        2,
        "the archive that was not listed is still there"
    );
}

/// A conversation stored by a synchronisation carries the date its message arrived.
///
/// `arrived_at` is written by a trigger on the copy in the folder, which is a later statement than
/// the one storing the message, so the conversation was cached before the column held anything.
/// The list reads `newest_at > 0`, so what that produced was not a wrong date but no row at all.
///
/// Driven through `apply` rather than by placing rows, because the fault is in the order the real
/// path writes in: a fixture that fills `arrived_at` in itself cannot produce it.
#[test]
fn a_conversation_stored_by_a_synchronisation_can_be_seen() {
    let db = TempIndex::new("conversation-seen");
    db.index
        .apply(
            &delta(
                1,
                vec![
                    arrival(10, Some("<one@example.com>"), Flags::NONE),
                    arrival(11, Some("<two@example.com>"), Flags::NONE),
                ],
            ),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("both arrivals apply");

    assert_eq!(db.count("SELECT count(*) FROM conversations"), 2);
    assert_eq!(
        db.count("SELECT count(*) FROM conversations WHERE newest_at > 0"),
        2,
        "a conversation with a zero date is a row no view can show"
    );
}

/// The conversation names the message that arrived last, not the one stored last.
#[test]
fn a_conversation_names_the_message_that_arrived_last() {
    let db = TempIndex::new("conversation-newest");
    db.index
        .apply(
            &delta(
                1,
                vec![
                    arrival(10, Some("<first@example.com>"), Flags::NONE),
                    reply(11, "<second@example.com>", "<first@example.com>"),
                ],
            ),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("both arrivals apply");

    assert_eq!(db.count("SELECT count(*) FROM conversations"), 1);
    assert_eq!(db.count("SELECT messages FROM conversations"), 2);
    assert_eq!(
        db.count("SELECT newest_at FROM conversations"),
        1_700_000_011
    );
    assert_eq!(
        db.count(
            "SELECT (SELECT newest_message FROM conversations)
                  = (SELECT message_id FROM message_placements WHERE uid = 11)"
        ),
        1,
        "the row speaks for the reply, which is what the list shows"
    );
}

/// Losing the newest message makes the conversation older rather than leaving it where it was.
#[test]
fn a_conversation_gets_older_when_its_newest_message_goes() {
    let db = TempIndex::new("conversation-older");
    db.index
        .apply(
            &delta(
                1,
                vec![
                    arrival(10, Some("<first@example.com>"), Flags::NONE),
                    reply(11, "<second@example.com>", "<first@example.com>"),
                ],
            ),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("both arrivals apply");

    db.index
        .apply(
            &delta(1, Vec::new()),
            &[uid(11)],
            &SyncCursor::new("2"),
            FlagScope::Message,
        )
        .expect("the reply is removed");
    db.index
        .forget_the_unplaced(1, &[])
        .expect("the pass sweeps");

    assert_eq!(db.count("SELECT messages FROM conversations"), 1);
    assert_eq!(
        db.count("SELECT newest_at FROM conversations"),
        1_700_000_010
    );
    assert_eq!(
        db.count(
            "SELECT (SELECT newest_message FROM conversations)
                  = (SELECT message_id FROM message_placements WHERE uid = 10)"
        ),
        1
    );
}

/// What is waiting is what has not been read, and the one size that still counts is the reading's.
///
/// Answering with "what has no stored body" named a message over the size the store keeps on every
/// round for ever; excluding it by that size took its text out of the index, because a message
/// never fetched is never read. So the store's size decides nothing here. The size that does is
/// the one the reading path already refuses at: above it a message is not opened and no text comes
/// out of it, so asking for it buys nothing and costs the whole of it in memory.
#[test]
fn what_is_waiting_is_what_has_not_been_read_whole() {
    use crate::infrastructure::html::LARGEST_MESSAGE;

    let db = TempIndex::new("waiting-is-unread-whole");

    // Past what the store keeps, well inside what the reader will take.
    let mut not_kept = arrival(10, Some("<large@example.com>"), Flags::NONE);
    not_kept.envelope.rfc822_size = Some(9 * 1024 * 1024);
    // Past what anything can make sense of.
    let mut unreadable = arrival(11, Some("<enormous@example.com>"), Flags::NONE);
    unreadable.envelope.rfc822_size = Some(u32::try_from(LARGEST_MESSAGE + 1).expect("fits"));
    let mut ordinary = arrival(12, Some("<ordinary@example.com>"), Flags::NONE);
    ordinary.envelope.rfc822_size = Some(4096);
    let mut unnamed = arrival(13, Some("<unnamed@example.com>"), Flags::NONE);
    unnamed.envelope.rfc822_size = None;

    db.index
        .apply(
            &delta(1, vec![not_kept, unreadable, ordinary, unnamed]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("all four apply");

    let waiting = |db: &TempIndex| -> Vec<u32> {
        db.index
            .not_read_whole(folder(1), 50)
            .expect("readable")
            .into_iter()
            .map(|(_, uid)| uid.get())
            .collect()
    };

    let first = waiting(&db);
    assert!(
        first.contains(&10),
        "a message too large to keep is still read once, or its text reaches no index"
    );
    assert!(
        !first.contains(&11),
        "a message nothing can read is not worth the whole of it in memory"
    );
    assert!(first.contains(&12));
    assert!(
        first.contains(&13),
        "a size nobody named is not a size at all"
    );

    // Marked the way reading it marks it.
    db.index
        .pool
        .get()
        .expect("connection")
        .execute(
            "UPDATE messages SET read_whole_at = 1
             WHERE id = (SELECT message_id FROM message_placements WHERE uid = 10)",
            [],
        )
        .expect("marked");

    let again = waiting(&db);
    assert!(
        !again.contains(&10),
        "read once means read once, whatever its size"
    );
    assert_eq!(again.len(), 2, "the other two are untouched");
}

/// Bytes that are not a message were still read, and are not asked for again.
///
/// The parser refuses some of what a server can send — an empty body for a message expunged
/// between the listing and the fetch is the ordinary case. Returning early without recording the
/// reading put the message back at the head of every round, which is the fault this column exists
/// to end, arriving through a different door.
#[test]
fn bytes_that_are_not_a_message_are_still_read_only_once() {
    use crate::infrastructure::secure_storage::message_key::MessageKey;
    use crate::infrastructure::sqlite::message_bodies::SqliteMessageBodies;
    use crate::ports::message_bodies::MessageBodies;

    let db = TempIndex::new("unparseable-read-once");
    db.index
        .apply(
            &delta(1, vec![arrival(10, Some("<odd@example.com>"), Flags::NONE)]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");

    let waiting = |db: &TempIndex| {
        db.index
            .not_read_whole(folder(1), 50)
            .expect("readable")
            .len()
    };
    assert_eq!(waiting(&db), 1);

    let (message, _) = db.index.not_read_whole(folder(1), 1).expect("readable")[0];
    let store = SqliteMessageBodies::new(
        db.pool.clone(),
        MessageKey::from(&[5u8; 32]).expect("a key of the right size"),
    );
    store
        .fill_in_missing(message, b"just some text\r\n")
        .expect("nothing to fill in is not a failure");

    assert_eq!(
        waiting(&db),
        0,
        "reading bytes that are not a message is still reading them"
    );
}

/// The same, for a body with no octets at all.
///
/// A message expunged between the listing and the fetch: the server answers the section with a
/// zero-length literal rather than an error, so this arrives as a successful reading of nothing.
#[test]
fn a_message_that_answered_with_nothing_is_read_only_once() {
    use crate::infrastructure::secure_storage::message_key::MessageKey;
    use crate::infrastructure::sqlite::message_bodies::SqliteMessageBodies;
    use crate::ports::message_bodies::MessageBodies;

    let db = TempIndex::new("empty-body-read-once");
    db.index
        .apply(
            &delta(
                1,
                vec![arrival(10, Some("<gone@example.com>"), Flags::NONE)],
            ),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("applies");

    let (message, _) = db.index.not_read_whole(folder(1), 1).expect("readable")[0];
    SqliteMessageBodies::new(
        db.pool.clone(),
        MessageKey::from(&[5u8; 32]).expect("a key of the right size"),
    )
    .fill_in_missing(message, b"")
    .expect("nothing to fill in is not a failure");

    assert_eq!(
        db.index
            .not_read_whole(folder(1), 50)
            .expect("readable")
            .len(),
        0
    );
}

/// Mail does not arrive in the order it was written, and the conversation follows the date.
///
/// The arriving side is arithmetic — one row found by its key, never a look at the conversation —
/// so what it decides has to be right without seeing the others. Stored newest-in-the-middle, so a
/// rule that simply took the message it was given would end on the wrong one.
#[test]
fn a_conversation_out_of_order_still_names_the_message_that_arrived_last() {
    let db = TempIndex::new("conversation-out-of-order");
    db.index
        .apply(
            &delta(
                1,
                vec![
                    arrival(10, Some("<root@example.com>"), Flags::NONE),
                    reply(14, "<latest@example.com>", "<root@example.com>"),
                    reply(12, "<middle@example.com>", "<root@example.com>"),
                ],
            ),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("all three apply");

    assert_eq!(db.count("SELECT count(*) FROM conversations"), 1);
    assert_eq!(db.count("SELECT messages FROM conversations"), 3);
    assert_eq!(
        db.count("SELECT newest_at FROM conversations"),
        1_700_000_014
    );
    assert_eq!(
        db.count(
            "SELECT (SELECT newest_message FROM conversations)
                  = (SELECT message_id FROM message_placements WHERE uid = 14)"
        ),
        1,
        "the conversation speaks for the message with the latest date, not the last one stored"
    );
}

/// Two messages sharing a second are told apart by the number, the way the list orders them.
///
/// A conversation naming one of them while sitting in the place of the other is a row that opens
/// something else than it shows.
#[test]
fn two_messages_in_one_second_are_settled_by_the_number() {
    let db = TempIndex::new("conversation-same-second");
    let mut first = arrival(10, Some("<root@example.com>"), Flags::NONE);
    first.envelope.received_at = 1_700_000_500;
    let mut second = reply(11, "<same@example.com>", "<root@example.com>");
    second.envelope.received_at = 1_700_000_500;

    db.index
        .apply(
            &delta(1, vec![first, second]),
            &[],
            &SyncCursor::new("1"),
            FlagScope::Message,
        )
        .expect("both apply");

    assert_eq!(
        db.count("SELECT newest_at FROM conversations"),
        1_700_000_500
    );
    assert_eq!(
        db.count(
            "SELECT (SELECT newest_message FROM conversations)
                  = (SELECT max(message_id) FROM message_placements)"
        ),
        1,
        "the higher number speaks, which is the tie-break the list is ordered by"
    );
}
