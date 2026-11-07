use super::*;
use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::infrastructure::sqlite::connection::{open_profile, SqlitePool};
use crate::infrastructure::sqlite::drafts::SqliteDrafts;
use crate::infrastructure::sqlite::migrations::migrate_profile;
use crate::infrastructure::sqlite::outbox::SqliteOutbox;
use crate::ports::drafts::{Draft, DraftError, Drafts};
use crate::ports::outbox::Outbox;
use lyonoyl_domain::outgoing::{EmailAddress, Outgoing, Subject};

/// A profile on disk with one account, gone with the test.
struct TempProfile {
    dir: std::path::PathBuf,
    pool: SqlitePool,
}

impl TempProfile {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-originals-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("profile.sqlite3");
        let pool = open_profile(&path, &DatabaseKey::for_a_test()).expect("pool opens");
        let mut connection = pool.get().expect("connection");
        migrate_profile(&mut connection, &path.display().to_string()).expect("schema");
        connection
            .execute(
                "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                 VALUES ('acc-1', 'zeta@example.com', 'Zeta', 'imap', 1)",
                [],
            )
            .expect("an account to write from");
        drop(connection);

        Self { dir, pool }
    }

    fn drafts(&self) -> SqliteDrafts {
        SqliteDrafts::new(self.pool.clone())
    }

    fn outbox(&self) -> SqliteOutbox {
        SqliteOutbox::new(self.pool.clone())
    }

    fn rows_in(&self, table: &str) -> i64 {
        self.pool
            .get()
            .expect("connection")
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("readable")
    }

    fn drawing_kept_for(&self, draft: i64) -> String {
        self.pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT drawing FROM draft_originals WHERE draft_id = ?1",
                [draft],
                |row| row.get(0),
            )
            .expect("readable")
    }
}

impl Drop for TempProfile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A message passed on in its own form: its markup, where its drawing begins, its text, and two
/// pictures whose every byte says which picture it is.
fn an_original(paragraph: &str) -> Original {
    Original {
        opening: "<html><head><meta charset=\"utf-8\"></head><body>".to_owned(),
        drawing: format!("<p>{paragraph}</p></body></html>"),
        text: paragraph.to_owned(),
        pictures: vec![
            OriginalPicture {
                named: "chart@example.com".to_owned(),
                content_type: "image/png".to_owned(),
                bytes: vec![b'c'; 40],
            },
            OriginalPicture {
                named: "logo@example.com".to_owned(),
                content_type: "image/gif".to_owned(),
                bytes: vec![b'l'; 20],
            },
        ],
    }
}

fn a_draft() -> Draft {
    Draft {
        body: "Kappa, look at this:".to_owned(),
        ..Draft::default()
    }
}

/// A message written down in the outbox passing `original` on, and the number of its row.
fn queued(profile: &TempProfile, message_id: &str, original: Option<Original>) -> i64 {
    let message = Outgoing {
        files: Vec::new(),
        from: EmailAddress::parse("zeta@example.com").expect("an address"),
        to: vec![EmailAddress::parse("kappa@example.net").expect("an address")],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: Subject::parse("Fwd: The chart").expect("a subject"),
        body: "Kappa, look at this:".to_owned(),
        thread: Default::default(),
        original,
    };
    profile
        .outbox()
        .queue("acc-1", message_id, &message)
        .expect("it is written down");
    profile
        .pool
        .get()
        .expect("connection")
        .query_row(
            "SELECT id FROM outbox WHERE message_id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .expect("readable")
}

#[test]
fn a_draft_keeps_what_it_passes_on_and_gives_it_back_whole() {
    let profile = TempProfile::new("whole");
    let drafts = profile.drafts();
    let original = an_original("Theirs");

    let id = drafts
        .keep_passing_on("acc-1", None, &a_draft(), &[], Some(&original))
        .expect("kept");

    assert_eq!(drafts.original(id).expect("readable"), Some(original));
}

#[test]
fn a_keep_naming_what_a_draft_already_passes_on_does_not_write_it_again() {
    let profile = TempProfile::new("once");
    let drafts = profile.drafts();
    let original = an_original("Theirs");
    let id = drafts
        .keep_passing_on("acc-1", None, &a_draft(), &[], Some(&original))
        .expect("kept");

    // Marked where it is kept: written again, the mark would be gone.
    profile
        .pool
        .get()
        .expect("connection")
        .execute(
            "UPDATE draft_originals SET drawing = 'as it was kept' WHERE draft_id = ?1",
            [id],
        )
        .expect("marked");
    let more = Draft {
        body: "Kappa, look at this, and tell me what you think:".to_owned(),
        ..Draft::default()
    };
    drafts
        .keep_passing_on("acc-1", Some(id), &more, &[], Some(&original))
        .expect("kept again");

    assert_eq!(profile.drawing_kept_for(id), "as it was kept");
}

#[test]
fn a_keep_naming_nothing_to_pass_on_leaves_it() {
    // Nothing in the window takes it off: a keep naming none is a draft that passes nothing new
    // on, and what it passes on goes with the draft.
    let profile = TempProfile::new("left");
    let drafts = profile.drafts();
    let id = drafts
        .keep_passing_on("acc-1", None, &a_draft(), &[], Some(&an_original("Theirs")))
        .expect("kept");
    assert_eq!(profile.rows_in("draft_original_pictures"), 2);

    drafts
        .keep("acc-1", Some(id), &a_draft(), &[])
        .expect("kept again");

    assert_eq!(
        drafts.original(id).expect("readable"),
        Some(an_original("Theirs"))
    );
    assert_eq!(profile.rows_in("draft_original_pictures"), 2);
}

#[test]
fn what_a_draft_passes_on_counts_towards_what_its_account_may_keep() {
    let profile = TempProfile::new("room");
    let original = an_original("Theirs");
    let draft = a_draft();
    let without = draft.body.len() as i64;
    // Room for the text and every byte of what it passes on but one.
    let drafts = SqliteDrafts::holding(
        profile.pool.clone(),
        10,
        without + original.weighs() as i64 - 1,
    );

    let refused = drafts.keep_passing_on("acc-1", None, &draft, &[], Some(&original));

    assert_eq!(refused, Err(DraftError::NoRoom));
    assert_eq!(profile.rows_in("drafts"), 0, "a refused keep wrote a draft");
    assert_eq!(profile.rows_in("draft_originals"), 0);
    assert_eq!(profile.rows_in("draft_original_pictures"), 0);
}

#[test]
fn a_draft_thrown_away_takes_what_it_passes_on_with_it() {
    let profile = TempProfile::new("discard");
    let drafts = profile.drafts();
    let id = drafts
        .keep_passing_on("acc-1", None, &a_draft(), &[], Some(&an_original("Theirs")))
        .expect("kept");

    drafts.discard(id).expect("thrown away");

    assert_eq!(profile.rows_in("draft_originals"), 0);
    assert_eq!(profile.rows_in("draft_original_pictures"), 0);
}

#[test]
fn a_queued_message_hands_over_what_it_passes_on_whole() {
    let profile = TempProfile::new("queued");
    let original = an_original("Theirs");
    queued(&profile, "<one@example.com>", Some(original.clone()));
    queued(&profile, "<two@example.com>", None);

    let ready = profile.outbox().ready("acc-1", 10).expect("readable");

    assert_eq!(ready.len(), 2);
    assert_eq!(ready[0].message.original, Some(original));
    assert_eq!(ready[1].message.original, None);
}

#[test]
fn a_message_that_comes_back_brings_what_it_passes_on() {
    let profile = TempProfile::new("back");
    let original = an_original("Theirs");
    let row = queued(&profile, "<one@example.com>", Some(original.clone()));

    let id = profile
        .drafts()
        .came_back("acc-1", row, &a_draft())
        .expect("written")
        .expect("a draft");

    assert_eq!(
        profile.drafts().original(id).expect("readable"),
        Some(original),
        "it came back without what it passes on"
    );
    assert_eq!(
        profile.rows_in("outbox_originals"),
        0,
        "a copy stayed behind"
    );
    assert_eq!(profile.rows_in("outbox_original_pictures"), 0);
}

#[test]
fn a_message_let_go_of_takes_what_it_passes_on_with_it() {
    let profile = TempProfile::new("let-go");
    let row = queued(&profile, "<one@example.com>", Some(an_original("Theirs")));

    profile
        .pool
        .get()
        .expect("connection")
        .execute("DELETE FROM outbox WHERE id = ?1", [row])
        .expect("let go of");

    assert_eq!(profile.rows_in("outbox_originals"), 0);
    assert_eq!(profile.rows_in("outbox_original_pictures"), 0);
}
