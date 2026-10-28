//! Whole-stack checks at the size the product is meant for: a scripted provider, the real sync
//! use case and the real SQLite index.
//!
//! The hundred-thousand-message run is marked `#[ignore]` so a normal test run stays quick; it is
//! started explicitly with `cargo test --workspace -- --ignored --nocapture`.

use std::time::Instant;

use crate::application::sync::SyncFolder;
use crate::infrastructure::fake_provider::{batch, ScriptedProvider};
use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::infrastructure::sqlite::connection::{open_index, SqlitePool};
use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
use crate::infrastructure::sqlite::migrations::migrate_index;
use crate::ports::mail_index::MailIndex;
use crate::ports::mail_provider::MailProviderError;
use crate::read_model::list::{ListReadModel, ViewQuery, ViewScope};
use lyonoyl_domain::message::{FolderId, Uid, UidValidity};
use lyonoyl_domain::sync::VanishedReport;

const BATCH: u32 = 1_000;

/// How many rows the list asks for at a time, `PAGE_SIZE` in the window's message list, and so
/// the size every window here is timed at.
const A_WINDOW: u32 = 100;

struct Workspace {
    dir: std::path::PathBuf,
    pool: SqlitePool,
    /// The authoritative database beside the index. A list reads both, so a measurement taken
    /// against one of them is a measurement of half a list.
    profile: SqlitePool,
    path: std::path::PathBuf,
}

impl Workspace {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-scale-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("temp dir");

        let path = dir.join("index.sqlite3");
        let pool = open_index(&path, &DatabaseKey::for_a_test()).expect("pool opens");
        let mut connection = pool.get().expect("connection");
        migrate_index(&mut connection, &path.display().to_string()).expect("schema");
        connection
            .execute_batch(
                "INSERT INTO folders (id, account_id, name, role) VALUES (1, 1, 'INBOX', 1);",
            )
            .expect("folder");
        drop(connection);

        let profile_path = dir.join("profile.sqlite3");
        let profile = crate::infrastructure::sqlite::connection::open_profile(
            &profile_path,
            &DatabaseKey::for_a_test(),
        )
        .expect("pool opens");
        {
            let mut profile_connection = profile.get().expect("connection");
            crate::infrastructure::sqlite::migrations::migrate_profile(
                &mut profile_connection,
                &profile_path.display().to_string(),
            )
            .expect("schema");

            // The sidebar reads the profile for its accounts and the index for their folders. A
            // workspace with no account measures neither.
            profile_connection
                .execute(
                    "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                     VALUES ('acc-1', 'user@example.com', 'User', 'imap', 1)",
                    [],
                )
                .expect("an account for the sidebar to show");
        }

        Self {
            dir,
            pool,
            profile,
            path,
        }
    }

    fn index(&self) -> SqliteMailIndex {
        SqliteMailIndex::new(self.pool.clone())
    }

    fn count(&self, sql: &str) -> i64 {
        self.pool
            .get()
            .expect("connection")
            .query_row(sql, [], |row| row.get(0))
            .expect("count is readable")
    }

    fn megabytes(&self) -> f64 {
        let checkpoint = self.pool.get().expect("connection");
        checkpoint
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .expect("checkpoint");
        drop(checkpoint);

        let bytes = std::fs::metadata(&self.path)
            .map(|meta| meta.len())
            .unwrap_or(0);
        bytes as f64 / (1024.0 * 1024.0)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn folder() -> FolderId {
    FolderId::new(1).expect("non-zero")
}

fn generation(value: u32) -> UidValidity {
    UidValidity::new(value).expect("non-zero")
}

/// Runs `passes` passes of `BATCH` arrivals each, and returns how long applying them took.
fn seed(workspace: &Workspace, passes: u32) -> std::time::Duration {
    let mut elapsed = std::time::Duration::ZERO;

    for pass in 0..passes {
        let first_uid = pass * BATCH + 1;
        let cursor_before = if pass == 0 {
            None
        } else {
            Some(format!("{}", pass))
        };
        let provider = ScriptedProvider::new(vec![Ok(batch(
            folder(),
            generation(7),
            first_uid,
            BATCH,
            cursor_before.as_deref(),
            &format!("{}", pass + 1),
        ))]);

        let started = Instant::now();
        SyncFolder::new(provider, workspace.index())
            .execute(folder())
            .expect("the pass applies");
        elapsed += started.elapsed();
    }

    elapsed
}

/// A second, third, hundredth account: its own mail file, its own folder, its own row in the
/// profile.
///
/// The mail is one file per account, so spanning a workspace is a loop over files — and every
/// figure in [response-budgets.md](../../docs/performance/response-budgets.md) until now was taken on a workspace of
/// one, where that loop runs once and costs nothing. This is what makes it run as many times as a
/// person has accounts.
fn another_account(workspace: &Workspace, number: i64) -> SqlitePool {
    let path = workspace.dir.join(format!("{number}.sqlite3"));
    let pool = open_index(&path, &DatabaseKey::for_a_test()).expect("pool opens");
    {
        let mut connection = pool.get().expect("connection");
        migrate_index(&mut connection, &path.display().to_string()).expect("schema");
        connection
            .execute_batch(
                "INSERT INTO folders (id, account_id, name, role) VALUES (1, 1, 'INBOX', 1);",
            )
            .expect("folder");
    }

    workspace
        .profile
        .get()
        .expect("connection")
        .execute(
            "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
             VALUES (?1, ?2, 'User', 'imap', ?3)",
            rusqlite::params![
                format!("acc-{number}"),
                format!("user{number}@example.com"),
                number
            ],
        )
        .expect("an account for the sidebar to show");

    pool
}

/// Fills one account's file with `passes * BATCH` messages.
fn seed_pool(pool: &SqlitePool, passes: u32) {
    for pass in 0..passes {
        let first_uid = pass * BATCH + 1;
        let cursor_before = (pass > 0).then(|| format!("{pass}"));
        let provider = ScriptedProvider::new(vec![Ok(batch(
            folder(),
            generation(7),
            first_uid,
            BATCH,
            cursor_before.as_deref(),
            &format!("{}", pass + 1),
        ))]);

        SyncFolder::new(provider, SqliteMailIndex::new(pool.clone()))
            .execute(folder())
            .expect("the pass applies");
    }
}

#[test]
#[ignore = "measures a workspace of many accounts; started explicitly"]
fn what_many_accounts_cost() {
    // The budget says a hundred accounts must not stall the interface, and nothing had ever asked
    // a workspace of more than one. The mail is one file per account, so the questions that span a
    // workspace ask every file for the rows above the window and merge them — a loop that runs once
    // at one account and a hundred times at a hundred, and its cost is what this measures.
    //
    // The same total of mail each time, split more ways. What changes between the rows is the
    // number of files, not the amount of mail, so what the numbers show is the cost of the split.
    const ALTOGETHER: u32 = 100;

    for accounts in [1_u32, 10, 100] {
        let workspace = Workspace::new(&format!("spanning-{accounts}"));
        let each = ALTOGETHER / accounts;

        seed(&workspace, each);
        let mut indexes = vec![(1_i64, workspace.pool.clone())];
        for number in 2..=i64::from(accounts) {
            let pool = another_account(&workspace, number);
            seed_pool(&pool, each);
            indexes.push((number, pool));
        }

        let list = ListReadModel::spanning(indexes.clone(), workspace.profile.clone());
        let everywhere = ViewQuery {
            scope: ViewScope::Everything,
            unread_only: false,
            as_conversations: true,
        };
        // The sidebar the window draws: every account's file, as `get_mailbox_tree` reads it.
        let sidebar = crate::read_model::mailboxes::MailboxReadModel::spanning(
            workspace.profile.clone(),
            indexes.clone(),
        );

        // Warm, then the middle of three: one cold sample measures whether a page was resident.
        let timed = |run: &dyn Fn()| {
            run();
            let mut took: Vec<std::time::Duration> = (0..3)
                .map(|_| {
                    let started = Instant::now();
                    run();
                    started.elapsed()
                })
                .collect();
            took.sort();
            took[1]
        };

        let top = timed(&|| {
            list.get_view_window(&everywhere, 0, A_WINDOW)
                .expect("readable");
        });
        let deep = timed(&|| {
            list.get_view_window(&everywhere, 5_000 - A_WINDOW, A_WINDOW)
                .expect("readable");
        });
        let tree = timed(&|| {
            sidebar.tree().expect("readable");
        });

        println!(
            "{accounts:>4} accounts, {} messages altogether:  top {:>10?}   deep {:>10?}   sidebar {:>10?}",
            ALTOGETHER * BATCH,
            top,
            deep,
            tree
        );
    }

    println!("\nWhat this is for is deciding — see docs/performance/response-budgets.md.");
}

#[test]
fn five_thousand_messages_arrive_and_are_all_stored() {
    let workspace = Workspace::new("small");
    seed(&workspace, 5);

    assert_eq!(workspace.count("SELECT count(*) FROM messages"), 5_000);
    assert_eq!(
        workspace.count("SELECT count(*) FROM message_placements"),
        5_000
    );
}

#[test]
fn a_dropped_connection_leaves_the_previous_pass_intact() {
    let workspace = Workspace::new("dropped");
    seed(&workspace, 2);

    let provider = ScriptedProvider::new(vec![Err(MailProviderError::Offline)]);
    let error = SyncFolder::new(provider, workspace.index())
        .execute(folder())
        .expect_err("the pass fails");

    assert!(matches!(
        error,
        crate::application::error::UseCaseError::Offline
    ));
    assert_eq!(workspace.count("SELECT count(*) FROM messages"), 2_000);

    let state = workspace
        .index()
        .folder_state(folder())
        .expect("state is readable");
    assert_eq!(
        state.cursor.expect("cursor survives").as_str(),
        "2",
        "a failed pass must not move the resume point"
    );
}

#[test]
fn a_reset_uid_space_mid_run_costs_the_placements_but_not_the_messages() {
    let workspace = Workspace::new("reset");
    seed(&workspace, 3);
    assert_eq!(workspace.count("SELECT count(*) FROM messages"), 3_000);

    // The next pass reports a different generation: every stored UID now addresses nothing.
    let provider = ScriptedProvider::new(vec![Ok(batch(
        folder(),
        generation(8),
        1,
        BATCH,
        Some("3"),
        "1",
    ))]);
    let outcome = SyncFolder::new(provider, workspace.index())
        .execute(folder())
        .expect("the pass applies");
    // What the pass does once every folder is read.
    workspace
        .index()
        .forget_the_unplaced(1, &[])
        .expect("the pass sweeps");

    assert!(outcome.resynced);
    assert_eq!(
        workspace.count("SELECT count(*) FROM message_placements"),
        1_000,
        "only what the new pass brought is placed"
    );
    assert_eq!(
        workspace.count("SELECT count(*) FROM messages"),
        1_000,
        "messages that no longer sit anywhere are not kept"
    );

    let state = workspace
        .index()
        .folder_state(folder())
        .expect("state is readable");
    assert_eq!(state.uid_validity, Some(generation(8)));
}

#[test]
fn an_interrupted_pass_never_deletes_at_any_size() {
    let workspace = Workspace::new("interrupted");
    seed(&workspace, 5);

    // A pass that knows nothing about disappearances, over a folder holding five thousand messages.
    let mut incomplete = batch(folder(), generation(7), 6_000, 1, Some("5"), "6");
    incomplete.per_mailbox[0].vanished = VanishedReport::Unknown;

    let provider = ScriptedProvider::new(vec![Ok(incomplete)]);
    let outcome = SyncFolder::new(provider, workspace.index())
        .execute(folder())
        .expect("the pass applies");

    assert_eq!(outcome.summary.removed, 0);
    assert_eq!(workspace.count("SELECT count(*) FROM messages"), 5_001);
}

#[test]
fn a_server_that_forgets_half_the_mailbox_needs_a_human() {
    let workspace = Workspace::new("mass-loss");
    seed(&workspace, 5);

    let mut mass_loss = batch(folder(), generation(7), 6_000, 1, Some("5"), "6");
    mass_loss.per_mailbox[0].vanished = VanishedReport::Exact(
        (1..=2_500)
            .map(|uid| Uid::new(uid).expect("non-zero"))
            .collect(),
    );

    let provider = ScriptedProvider::new(vec![Ok(mass_loss)]);
    let outcome = SyncFolder::new(provider, workspace.index())
        .execute(folder())
        .expect("the pass applies");

    assert_eq!(outcome.deletions_awaiting_confirmation, 2_500);
    assert_eq!(outcome.summary.removed, 0);
    assert_eq!(
        workspace.count("SELECT count(*) FROM messages"),
        5_001,
        "half a mailbox does not vanish without a person agreeing"
    );
}

#[test]
#[ignore = "runs a hundred thousand messages; started explicitly"]
fn a_hundred_thousand_messages() {
    let workspace = Workspace::new("full");
    let passes = 100;

    let applying = seed(&workspace, passes);
    let stored = workspace.count("SELECT count(*) FROM messages");
    assert_eq!(stored, i64::from(passes * BATCH));

    let size = workspace.megabytes();

    let connection = workspace.pool.get().expect("connection");
    let window_started = Instant::now();
    let window: Vec<i64> = connection
        .prepare(&format!(
            "SELECT p.message_id FROM message_placements p
             WHERE p.folder_id = 1
             ORDER BY p.received_at DESC
             LIMIT {A_WINDOW}"
        ))
        .expect("statement prepares")
        .query_map([], |row| row.get(0))
        .expect("rows are readable")
        .collect::<Result<_, _>>()
        .expect("rows are readable");
    let window_time = window_started.elapsed();
    assert_eq!(window.len(), A_WINDOW as usize);

    let unread_started = Instant::now();
    let unread: i64 = connection
        .query_row(
            "SELECT count(DISTINCT message_id) FROM message_placements
             WHERE folder_id = 1 AND (flags & 1) = 0",
            [],
            |row| row.get(0),
        )
        .expect("count is readable");
    let unread_time = unread_started.elapsed();

    println!("messages stored: {stored}");
    println!("unread: {unread}");
    println!(
        "applying {passes} passes of {BATCH}: {:.2} s ({:.0} messages/second)",
        applying.as_secs_f64(),
        f64::from(passes * BATCH) / applying.as_secs_f64()
    );
    println!(
        "database size: {size:.1} MB ({:.2} KB per message)",
        size * 1024.0 / stored as f64
    );
    println!(
        "first list window of {A_WINDOW}: {:.2} ms",
        window_time.as_secs_f64() * 1000.0
    );
    println!("unread count: {:.2} ms", unread_time.as_secs_f64() * 1000.0);
}

/// How much of this build is measured rather than believed, at a size somebody chooses.
///
/// The size to design against is fifty to a hundred accounts of a hundred thousand messages
/// or more, and a number that large cannot be seeded in every run. So the size is a knob, and what
/// this prints is a table of what each interaction costs against the budget it has in
/// `docs/performance/response-budgets.md`.
///
/// It reports and never fails on a time. A machine measuring itself under a full test run is not
/// the machine somebody reads their mail on, and a gate that goes red on a busy laptop teaches
/// people to ignore it, so nothing here fails on a time. What it is for is deciding: a design chosen against these numbers is chosen, and one
/// chosen without them is guessed.
///
///     cargo test --release -p lyonoyl --lib what_this_build_costs -- --ignored --nocapture
///     LYONOYL_SCALE_MESSAGES=500000 cargo test --release -p lyonoyl --lib what_this_build_costs -- --ignored --nocapture
///
/// Without `--release`, SQLite is compiled unoptimised and every figure is about three times what the
/// application gives.
#[test]
#[ignore = "seeds a large mailbox; started explicitly"]
fn what_this_build_costs() {
    let wanted: u32 = std::env::var("LYONOYL_SCALE_MESSAGES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(100_000);
    let passes = wanted.div_ceil(BATCH).max(1);

    let workspace = Workspace::new("costs");
    let seeding = seed(&workspace, passes);
    let stored = workspace.count("SELECT count(*) FROM messages");
    let connection = workspace.pool.get().expect("connection");

    /// One measurement: what it is, what it costs cold and warm, and what it is allowed.
    ///
    /// Both, because they are different questions and a document with one of them answers the
    /// other by accident. Cold is the first time this question is asked of a file the operating
    /// system has not got in memory — somebody opening the application and searching. Warm is
    /// every time after, which is the rest of a session.
    fn said(what: &str, took: (std::time::Duration, std::time::Duration), budget: &str) {
        let cold = format!("{:?}", took.0);
        let warm = format!("{:?}", took.1);
        println!("  {what:<44} {cold:>12} cold  {warm:>12} warm   budget {budget}");
    }

    /// How many times each question is asked before one of the answers is kept.
    ///
    /// One cold call is not a measurement of this product: the first read of a page pulls it off
    /// the disk and into the operating system's cache, and the second reads it from memory. Two
    /// runs of this tool minutes apart, same machine and same binary, disagreed by up to
    /// thirty-nine times — which makes every figure in response-budgets.md a figure about whether that
    /// page happened to be resident, and none of them about the build.
    const ASKED: usize = 5;

    let timed = |run: &dyn Fn()| {
        // The first ask, kept: it is the one a person makes, and the pages it pulls off the disk
        // are what makes it different from every ask after it.
        let started = Instant::now();
        run();
        let cold = started.elapsed();

        // Then the middle of five. The middle rather than the best, because the best is the run
        // where the machine happened to be idle; and rather than the mean, because one pause drags
        // a mean and moves the middle by nothing. Taking one sample of each was what made two runs
        // of this tool disagree by thirty-nine times.
        let mut took: Vec<std::time::Duration> = (0..ASKED)
            .map(|_| {
                let started = Instant::now();
                run();
                started.elapsed()
            })
            .collect();
        took.sort();

        (cold, took[ASKED / 2])
    };

    // Through the read model, not a query written beside it. A hand-written one-column read of
    // one table said 0.22 ms for what the list actually does in 222: no join to `messages`, no
    // folding of a message a folder holds twice, no reading of what a person has asked for since.
    // A tool measuring a query the product does not run measures nothing about the product, and
    // this one drifted into doing exactly that.
    let list =
        ListReadModel::spanning(vec![(1, workspace.pool.clone())], workspace.profile.clone());
    let folder = ViewQuery {
        scope: ViewScope::Folder(FolderId::new(1).expect("non-zero")),
        unread_only: false,
        as_conversations: false,
    };
    let unread_only = ViewQuery {
        scope: ViewScope::Folder(FolderId::new(1).expect("non-zero")),
        unread_only: true,
        as_conversations: false,
    };

    let top = timed(&|| {
        list.get_view_window(&folder, 0, A_WINDOW)
            .expect("readable");
    });

    let deep_offset = stored.saturating_sub(i64::from(A_WINDOW)).max(0) as u32;
    let deep = timed(&|| {
        list.get_view_window(&folder, deep_offset, A_WINDOW)
            .expect("readable");
    });

    let unread = timed(&|| {
        list.get_view_window(&unread_only, 0, A_WINDOW)
            .expect("readable");
    });

    // The view the application opens on when there is more than one account: it spans folders, so
    // none of the folder work above touches it.
    //
    // `as_conversations` as the command sets it, which is always. Measured with it false, this read
    // a view nobody opens — and missed that the fetch behind the real one had no index to use.
    let everything = ViewQuery {
        scope: ViewScope::Everything,
        unread_only: false,
        as_conversations: true,
    };
    let across_top = timed(&|| {
        list.get_view_window(&everything, 0, A_WINDOW)
            .expect("readable");
    });
    let across_deep = timed(&|| {
        list.get_view_window(&everything, deep_offset, A_WINDOW)
            .expect("readable");
    });

    // Unread across every account: the one view the folder work above cannot reach, and the only
    // reader of the partial indexes migration 020 adds. Unmeasured until now.
    let unread_everywhere = ViewQuery {
        scope: ViewScope::Everything,
        unread_only: true,
        as_conversations: true,
    };
    let unread_across_top = timed(&|| {
        list.get_view_window(&unread_everywhere, 0, A_WINDOW)
            .expect("readable");
    });
    let unread_across_deep = timed(&|| {
        list.get_view_window(&unread_everywhere, deep_offset, A_WINDOW)
            .expect("readable");
    });

    // The sidebar, which is rebuilt on every change a synchronisation reports — up to once per
    // account per round, so its cost is paid far more often than any window's.
    let mailboxes = crate::read_model::mailboxes::MailboxReadModel::new(
        workspace.profile.clone(),
        workspace.pool.clone(),
    );
    let sidebar = timed(&|| {
        mailboxes.tree().expect("readable");
    });

    // The same sidebar with a person's changes waiting to reach the server: fifty messages marked
    // read and twenty filed away and not read back. It is read again the moment each change is
    // written down, so this is the whole of what a click costs before every count agrees.
    {
        let profile = workspace.profile.get().expect("connection");
        for id in 1..=50 {
            profile
                .execute(
                    "INSERT INTO op_log
                         (account_id, message_id, folder_id, kind, flags_add, asked_at)
                     VALUES ('acc-1', ?1, 1, 'set_flags', 1, unixepoch())",
                    [id],
                )
                .expect("a change written down");
        }
        for id in 51..=70 {
            profile
                .execute(
                    "INSERT INTO op_log
                         (account_id, message_id, folder_id, kind, move_to, state, sent_at,
                          owes_a_read, asked_at)
                     VALUES ('acc-1', ?1, 1, 'move', 2, 'done', 1, 1, unixepoch())",
                    [id],
                )
                .expect("a move carried out and not read back");
        }
    }
    let sidebar_asked = timed(&|| {
        mailboxes.tree().expect("readable");
    });
    let icon_asked = timed(&|| {
        mailboxes.unread_everywhere().expect("readable");
    });

    // Search, at both ends of what it can be asked. The seeded fixture gives every message a
    // subject of the same shape, so one word of it matches everything there is — which is the
    // worst a search box can be handed and the easiest thing for somebody to type by accident.
    let everything_matches = ViewQuery {
        scope: ViewScope::Looking("message".to_owned()),
        unread_only: false,
        as_conversations: false,
    };
    let one_sender = ViewQuery {
        scope: ViewScope::Looking("sender17@fake.invalid".to_owned()),
        unread_only: false,
        as_conversations: false,
    };

    let search_wide = timed(&|| {
        list.get_view_window(&everything_matches, 0, A_WINDOW)
            .expect("readable");
    });
    let search_narrow = timed(&|| {
        list.get_view_window(&one_sender, 0, A_WINDOW)
            .expect("readable");
    });

    // A word out of what the messages say rather than out of who sent them. The seeded opening
    // carries each message's own number, so this matches one message in the mailbox — the shape a
    // person is in when they remember a phrase and not a sender.
    let one_phrase = ViewQuery {
        scope: ViewScope::Looking(format!("number {}", wanted / 2)),
        unread_only: false,
        as_conversations: false,
    };
    let search_text = timed(&|| {
        list.get_view_window(&one_phrase, 0, A_WINDOW)
            .expect("readable");
    });

    // A window is rows, a total and an epoch, and which of the three costs the time is not a thing
    // to reason about from the shape of the SQL. The total is measured on its own because it is the
    // one that reads the whole unread set.
    let unread_total = timed(&|| {
        let _: i64 = connection
            .query_row(
                "SELECT count(DISTINCT message_id) FROM message_placements
                 WHERE folder_id = 1 AND (flags & 1) = 0",
                [],
                |row| row.get(0),
            )
            .expect("readable");
    });

    // The bottom as well as the top. Reporting only the top is how this table said the list was
    // the fastest thing in the build while the top of every folder was over budget.
    let unread_deep_offset = list
        .get_view_window(&unread_only, 0, 1)
        .expect("readable")
        .total
        .saturating_sub(A_WINDOW);
    let unread_deep = timed(&|| {
        list.get_view_window(&unread_only, unread_deep_offset, A_WINDOW)
            .expect("readable");
    });

    // A fast wrong answer is not a measurement. The deepest window has to hold the rows a walk
    // reaches, or the number above it is the cost of missing them.
    {
        let window = list
            .get_view_window(&folder, deep_offset, A_WINDOW)
            .expect("readable");
        let walked: Vec<i64> = connection
            .prepare(&format!(
                "SELECT p.message_id FROM message_placements p
                 WHERE p.folder_id = 1
                   AND NOT EXISTS (
                       SELECT 1 FROM message_placements q
                       WHERE q.folder_id = p.folder_id AND q.message_id = p.message_id
                         AND (q.received_at > p.received_at
                          OR (q.received_at = p.received_at AND q.uid > p.uid)))
                 ORDER BY p.received_at DESC, p.uid DESC
                 LIMIT {A_WINDOW} OFFSET ?1"
            ))
            .expect("prepares")
            .query_map([i64::from(deep_offset)], |row| row.get(0))
            .expect("readable")
            .collect::<Result<_, _>>()
            .expect("readable");

        let read: Vec<i64> = window
            .rows
            .iter()
            .map(|row| row.message_id.get() as i64)
            .collect();
        assert_eq!(read, walked, "the deepest window is not what walking finds");
        assert_eq!(
            i64::from(window.total),
            stored,
            "the total the map answers is not what the folder holds"
        );
    }

    /// What SQLite says it will do, which is the difference between a slow query and a slow disk.
    /// What SQLite says it will do, for a statement that may carry parameters.
    ///
    /// The plan does not depend on what is bound, but preparing does need the right number of
    /// them: asking the read model for its own statement brought the first one that has any.
    fn the_plan(connection: &rusqlite::Connection, sql: &str) -> String {
        let mut statement = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .expect("prepares");
        let unbound: Vec<rusqlite::types::Value> = (0..statement.parameter_count())
            .map(|_| rusqlite::types::Value::Null)
            .collect();
        let rows: Vec<String> = statement
            .query_map(rusqlite::params_from_iter(unbound), |row| {
                row.get::<_, String>(3)
            })
            .expect("readable")
            .collect::<Result<_, _>>()
            .expect("readable");

        rows.join(" | ")
    }

    println!();
    println!(
        "{stored} messages, {:.1} MB, seeded in {seeding:?}",
        workspace.megabytes()
    );
    said("list window at the top", top, "≤150 ms");
    said("list window at the bottom", deep, "≤150 ms");
    said("unread list window at the top", unread, "≤150 ms");
    said("unread list window at the bottom", unread_deep, "≤150 ms");
    said(
        "  of which the unread total",
        unread_total,
        "counted in both",
    );
    said("search matching every message", search_wide, "≤120 ms");
    said("search matching one sender", search_narrow, "≤120 ms");
    said(
        "search matching a phrase in the text",
        search_text,
        "≤120 ms",
    );
    said(
        "everything on the device, at the top",
        across_top,
        "≤150 ms",
    );
    said(
        "everything on the device, at the bottom",
        across_deep,
        "≤150 ms",
    );
    said(
        "unread everywhere, at the top",
        unread_across_top,
        "≤150 ms",
    );
    said(
        "unread everywhere, at the bottom",
        unread_across_deep,
        "≤150 ms",
    );
    said("the sidebar, folders and counts", sidebar, "≤150 ms");
    said(
        "the sidebar, with 70 changes on their way",
        sidebar_asked,
        "one IPC round trip",
    );
    said("the icon, with the same", icon_asked, "one IPC round trip");
    println!();
    println!("What SQLite says it will do:");
    for (what, sql) in [
        (
            "position to day",
            "SELECT day, newer FROM (
                 SELECT day, COALESCE(sum(messages) OVER (ORDER BY day DESC
                          ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING), 0) AS newer
                 FROM folder_days WHERE folder_id = 1)
             WHERE newer <= 100 ORDER BY day ASC LIMIT 1",
        ),
        (
            "list window",
            &format!(
                "SELECT w.message_id FROM (
                 SELECT p.folder_id, p.message_id, p.received_at, p.uid
                 FROM message_placements p
                 WHERE p.folder_id = 1 AND p.received_at < 1800000000
                   AND NOT EXISTS (
                       SELECT 1 FROM message_placements q
                       WHERE q.folder_id = p.folder_id AND q.message_id = p.message_id
                         AND (q.received_at > p.received_at
                          OR (q.received_at = p.received_at AND q.uid > p.uid)))
                 ORDER BY p.received_at DESC, p.uid DESC LIMIT {A_WINDOW} OFFSET 0) w
             JOIN messages m ON m.id = w.message_id
             ORDER BY w.received_at DESC, w.uid DESC"
            ),
        ),
        (
            "folder total",
            "SELECT COALESCE(sum(messages), 0) FROM folder_days WHERE folder_id = 1",
        ),
        // Asked of the read model rather than rewritten here, so the plan is the one that runs.
        (
            "unread window",
            &crate::read_model::list::the_statement_behind(&unread_only),
        ),
        (
            "everything",
            &crate::read_model::list::the_statement_behind(&everything),
        ),
        (
            "unread everywhere",
            &crate::read_model::list::the_statement_behind(&unread_everywhere),
        ),
    ] {
        println!("  {what:<14} {}", the_plan(&connection, sql));
    }

    println!();
    println!(
        "Nothing here fails on a time. What it is for is deciding — see docs/performance/response-budgets.md."
    );
}

/// What a deep window costs, and what it would cost addressed differently.
///
/// The size to design against is fifty to a hundred accounts of a hundred thousand messages
/// or more, so the list has to hold up at a depth `OFFSET` cannot reach: it walks every row it
/// skips, and that is linear in how far down somebody has scrolled. A cursor on the sort key asks
/// "what comes after this message" and walks none of them.
///
/// Measured here at a hundred thousand rather than at millions on purpose: what is being settled is
/// the shape of the two costs, and a shape visible at 100k is worse at 10M by exactly as much as
/// the depth grew.
///
///     cargo test --release -p lyonoyl --lib what_a_deep_window_costs -- --ignored --nocapture
#[test]
#[ignore = "runs a hundred thousand messages; started explicitly"]
fn what_a_deep_window_costs() {
    let workspace = Workspace::new("deep-window");
    seed(&workspace, 100);

    let connection = workspace.pool.get().expect("connection");
    let deep = 90_000;

    let by_offset = |offset: i64| {
        let started = Instant::now();
        let rows: Vec<i64> = connection
            .prepare(&format!(
                "SELECT p.message_id FROM message_placements p
                 WHERE p.folder_id = 1
                 ORDER BY p.received_at DESC, p.uid DESC
                 LIMIT {A_WINDOW} OFFSET ?1"
            ))
            .expect("statement prepares")
            .query_map([offset], |row| row.get(0))
            .expect("rows are readable")
            .collect::<Result<_, _>>()
            .expect("rows are readable");
        (rows.len(), started.elapsed())
    };

    let (shallow_rows, shallow) = by_offset(0);
    let (deep_rows, deepest) = by_offset(deep);
    assert_eq!(shallow_rows, A_WINDOW as usize);
    assert_eq!(deep_rows, A_WINDOW as usize);

    // Where that window starts, as a cursor would name it: the row itself rather than its number.
    let (at_received, at_uid): (i64, i64) = connection
        .query_row(
            "SELECT p.received_at, p.uid FROM message_placements p
             WHERE p.folder_id = 1
             ORDER BY p.received_at DESC, p.uid DESC
             LIMIT 1 OFFSET ?1",
            [deep],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("a row that deep exists");

    let started = Instant::now();
    let after_that_row: Vec<i64> = connection
        .prepare(&format!(
            "SELECT p.message_id FROM message_placements p
             WHERE p.folder_id = 1
               AND (p.received_at, p.uid) < (?1, ?2)
             ORDER BY p.received_at DESC, p.uid DESC
             LIMIT {A_WINDOW}"
        ))
        .expect("statement prepares")
        .query_map([at_received, at_uid], |row| row.get(0))
        .expect("rows are readable")
        .collect::<Result<_, _>>()
        .expect("rows are readable");
    let by_cursor = started.elapsed();
    assert_eq!(after_that_row.len(), A_WINDOW as usize);

    println!("window at the top, by offset:        {shallow:?}");
    println!("window {deep} down, by offset:      {deepest:?}");
    println!("the same window, by cursor:          {by_cursor:?}");
}

#[test]
#[ignore = "deletes twenty thousand placements; started explicitly"]
fn removing_a_fifth_of_a_large_folder() {
    let workspace = Workspace::new("removal");
    seed(&workspace, 100);

    let mut removal = batch(folder(), generation(7), 100_001, 1, Some("100"), "101");
    removal.per_mailbox[0].vanished = VanishedReport::Exact(
        (1..=20_000)
            .map(|uid| Uid::new(uid).expect("non-zero"))
            .collect(),
    );

    let provider = ScriptedProvider::new(vec![Ok(removal)]);
    let started = Instant::now();
    let outcome = SyncFolder::new(provider, workspace.index())
        .execute(folder())
        .expect("the pass applies");
    let elapsed = started.elapsed();

    assert_eq!(outcome.summary.removed, 20_000);
    assert_eq!(workspace.count("SELECT count(*) FROM messages"), 80_001);

    println!(
        "removing 20,000 placements of 100,000: {:.2} s",
        elapsed.as_secs_f64()
    );
}

#[test]
#[ignore = "measures list windows over a hundred thousand messages; started explicitly"]
fn list_windows_at_every_depth() {
    let workspace = Workspace::new("windows");
    seed(&workspace, 100);

    let list =
        ListReadModel::spanning(vec![(1, workspace.pool.clone())], workspace.profile.clone());
    let everything = ViewQuery {
        scope: ViewScope::Folder(folder()),
        unread_only: false,
        as_conversations: false,
    };
    let unread = ViewQuery {
        scope: ViewScope::Folder(folder()),
        unread_only: true,
        as_conversations: false,
    };

    // Warm the page cache so the first measurement is not the only slow one.
    list.get_view_window(&everything, 0, A_WINDOW)
        .expect("readable");

    for offset in [0, 25_000, 50_000, 100_000 - A_WINDOW] {
        let started = Instant::now();
        let window = list
            .get_view_window(&everything, offset, A_WINDOW)
            .expect("window is readable");
        let elapsed = started.elapsed();

        assert_eq!(window.total, 100_000);
        println!(
            "window of {A_WINDOW} at offset {offset}: {:.2} ms ({} rows)",
            elapsed.as_secs_f64() * 1000.0,
            window.rows.len()
        );
    }

    let started = Instant::now();
    let filtered = list
        .get_view_window(&unread, 0, A_WINDOW)
        .expect("window is readable");
    println!(
        "unread window of {A_WINDOW}: {:.2} ms (total {})",
        started.elapsed().as_secs_f64() * 1000.0,
        filtered.total
    );

    let started = Instant::now();
    let ids = list
        .get_view_ids_in_range(&everything, 10_000, 10_499)
        .expect("range is readable");
    println!(
        "500 identifiers from position 10,000: {:.2} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    assert_eq!(ids.len(), 500);
}

#[test]
#[ignore = "breaks a list window into its parts; started explicitly"]
fn what_a_list_window_actually_costs() {
    let workspace = Workspace::new("window-parts");
    seed(&workspace, 100);
    let connection = workspace.pool.get().expect("connection");

    let measure = |label: &str, sql: &str| {
        let mut statement = connection.prepare(sql).expect("statement prepares");
        // Warm, then measure.
        let _ = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map(|rows| rows.count());
        let started = Instant::now();
        let count = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .expect("rows are readable")
            .count();
        println!(
            "{label}: {:.2} ms ({count} rows out)",
            started.elapsed().as_secs_f64() * 1000.0
        );
    };

    measure(
        "rows only, index scan",
        &format!(
            "SELECT p.message_id FROM message_placements p
         WHERE p.folder_id = 1 ORDER BY p.received_at DESC LIMIT {A_WINDOW} OFFSET 50000"
        ),
    );
    measure(
        "rows joined to messages",
        &format!(
            "SELECT p.message_id FROM message_placements p JOIN messages m ON m.id = p.message_id
         WHERE p.folder_id = 1 ORDER BY p.received_at DESC LIMIT {A_WINDOW} OFFSET 50000"
        ),
    );
    measure(
        "total with join",
        "SELECT count(*) FROM message_placements p JOIN messages m ON m.id = p.message_id
         WHERE p.folder_id = 1",
    );
    measure(
        "total without join",
        "SELECT count(*) FROM message_placements WHERE folder_id = 1",
    );
    measure("epoch", "SELECT view_epoch FROM folders WHERE id = 1");
}
