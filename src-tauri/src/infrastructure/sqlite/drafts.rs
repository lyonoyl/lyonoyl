//! Drafts in the authoritative database.
//!
//! The profile, not the index: a message being written exists nowhere else, so losing it loses
//! what somebody typed. Everything here goes through the connection that waits for the disk.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior};

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::ports::drafts::{Draft, DraftError, DraftFile, Drafts, Kept};
use lyonoyl_domain::outgoing::{OutgoingFile, HEAVIEST_DRAFTS, MOST_DRAFTS};

pub struct SqliteDrafts {
    pool: SqlitePool,
    room: Room,
}

/// How much one account's drafts may hold: how many there may be, and what they may weigh
/// together, text and files, in bytes.
struct Room {
    drafts: i64,
    weight: i64,
}

impl SqliteDrafts {
    /// Drafts held to the room the domain gives an account, which no person's own use reaches.
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            room: Room {
                drafts: MOST_DRAFTS as i64,
                weight: HEAVIEST_DRAFTS as i64,
            },
        }
    }

    /// Drafts held to a room of a test's own, which it can fill without writing gigabytes.
    #[cfg(test)]
    fn holding(pool: SqlitePool, drafts: i64, weight: i64) -> Self {
        Self {
            pool,
            room: Room { drafts, weight },
        }
    }

    fn open(
        &self,
    ) -> Result<r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>, DraftError> {
        self.pool.get().map_err(|_| DraftError::Unavailable)
    }
}

impl Drafts for SqliteDrafts {
    fn keep(
        &self,
        account_id: &str,
        under: Option<i64>,
        draft: &Draft,
        files: &[DraftFile],
    ) -> Result<i64, DraftError> {
        let mut connection = self.open()?;

        // The text and the files are one change. A keep that wrote the text and not the file
        // attached a second ago is a draft that comes back without it, saying nothing.
        //
        // Immediate, so the room below is read and taken in one change: no other keep can write
        // between the two, and two keeps at once cannot both take the last of it.
        let writing = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| DraftError::Unavailable)?;

        // What the draft kept under that number weighs before this keep, and nothing for one that
        // is not there: what the keep adds is measured against it.
        let was = match under {
            Some(id) => weight_of(&writing, id).map_err(|_| DraftError::Unavailable)?,
            None => None,
        }
        .unwrap_or(0);

        // Replaced rather than added to. A row per keystroke would be a folder of half-written
        // messages nobody asked for, and the one a person means is always the last.
        //
        // A row somebody threw away while this was being written is gone, and writing it back
        // would bring back a message they discarded: what is in the window is written down as a
        // draft of its own instead, with every file it carries.
        let id = match under {
            Some(id) if replaced(&writing, id, draft).map_err(|_| DraftError::Unavailable)? => id,
            _ => {
                // A draft of its own is a row more, and the one keep that can be refused for how
                // many there are.
                if !room_for_another(&writing, account_id, self.room.drafts)
                    .map_err(|_| DraftError::Unavailable)?
                {
                    return Err(DraftError::NoRoom);
                }
                written_down(&writing, account_id, draft, false)
                    .map_err(|_| DraftError::Unavailable)?
            }
        };

        carrying(&writing, id, files).map_err(|_| DraftError::Unavailable)?;

        // Heavier than it was, with the drafts of its account heavier together than they may be:
        // refused whole, and the change rolls back with everything above it. Measured on the
        // account the draft is kept in rather than the one named, which a keep under another
        // account's number would otherwise slip past. A keep that adds nothing is never refused:
        // taking a file off is how a person makes room.
        let weighs = weight_of(&writing, id)
            .map_err(|_| DraftError::Unavailable)?
            .unwrap_or(0);
        if weighs > was
            && weight_beside(&writing, id).map_err(|_| DraftError::Unavailable)? > self.room.weight
        {
            return Err(DraftError::NoRoom);
        }

        writing.commit().map_err(|_| DraftError::Unavailable)?;

        Ok(id)
    }

    fn files(&self, id: i64) -> Result<Vec<DraftFile>, DraftError> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT kept_as, name, content_type, bytes FROM draft_files
                 WHERE draft_id = ?1
                 ORDER BY position, id",
            )
            .map_err(|_| DraftError::Unavailable)?;

        let files = statement
            .query_map([id], |row| {
                Ok(DraftFile {
                    kept_as: row.get(0)?,
                    file: Arc::new(OutgoingFile {
                        name: row.get(1)?,
                        content_type: row.get(2)?,
                        bytes: row.get(3)?,
                    }),
                })
            })
            .map_err(|_| DraftError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| DraftError::Unavailable)?;

        Ok(files)
    }

    fn came_back(
        &self,
        account_id: &str,
        from_outbox: i64,
        draft: &Draft,
    ) -> Result<Option<i64>, DraftError> {
        let mut connection = self.open()?;
        let writing = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| DraftError::Unavailable)?;

        match held_back_as(&writing, account_id, from_outbox)
            .map_err(|_| DraftError::Unavailable)?
        {
            // Not in the outbox: given back already, by this round or by another that read the
            // row before it went. Its files went with that draft, so a second would be the message
            // without them, and one written after the first was sent or thrown away would be a
            // message somebody has already dealt with.
            None => return Ok(None),
            // Back already, as the draft the row names, which holds everything the row does and
            // the files besides: the row goes, and nothing is written.
            Some(Some(_)) => {
                let_go(&writing, from_outbox).map_err(|_| DraftError::Unavailable)?;
                writing.commit().map_err(|_| DraftError::Unavailable)?;
                return Ok(None);
            }
            Some(None) => {}
        }

        let id =
            written_down(&writing, account_id, draft, true).map_err(|_| DraftError::Unavailable)?;

        // Moved rather than copied, in the change that writes the draft: a file left in the outbox
        // as well is a second copy of somebody's document that nothing reads, and one taken out
        // of it before the draft was committed is a document a crash loses. Each is given a name
        // of its own for the composer that opens the draft to keep it by.
        writing
            .execute(
                "INSERT INTO draft_files (draft_id, kept_as, name, content_type, bytes, position)
                 SELECT ?1, lower(hex(randomblob(16))), name, content_type, bytes, position
                 FROM outbox_files
                 WHERE outbox_id = ?2
                 ORDER BY position, id",
                rusqlite::params![id, from_outbox],
            )
            .map_err(|_| DraftError::Unavailable)?;

        // And the row goes in the same change, with everybody it was addressed to, blind copies
        // included: the draft holds all of it now, and a copy nothing reads is a copy of
        // somebody's writing kept for nothing in the database nothing clears.
        let_go(&writing, from_outbox).map_err(|_| DraftError::Unavailable)?;

        writing.commit().map_err(|_| DraftError::Unavailable)?;

        Ok(Some(id))
    }

    fn all(&self, account_id: &str) -> Result<Vec<Kept>, DraftError> {
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT id, recipients, copies, subject, body, answering, chain, blind
                 FROM drafts
                 WHERE account_id = ?1
                 ORDER BY written_at DESC, id DESC",
            )
            .map_err(|_| DraftError::Unavailable)?;

        let kept = statement
            .query_map([account_id], kept_from)
            .map_err(|_| DraftError::Unavailable)?
            .collect::<Result<Vec<Kept>, _>>()
            .map_err(|_| DraftError::Unavailable)?;

        Ok(kept)
    }

    fn latest(&self, account_id: &str) -> Result<Option<Kept>, DraftError> {
        self.open()?
            .query_row(
                "SELECT id, recipients, copies, subject, body, answering, chain, blind
                 FROM drafts
                 WHERE account_id = ?1
                 ORDER BY written_at DESC, id DESC
                 LIMIT 1",
                [account_id],
                kept_from,
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(DraftError::Unavailable),
            })
    }

    fn discard(&self, id: i64) -> Result<(), DraftError> {
        // One statement: the files it carried go with it by the cascade the schema declares, on
        // every connection of the pool, each of which turns foreign keys on.
        self.open()?
            .execute("DELETE FROM drafts WHERE id = ?1", [id])
            .map(|_| ())
            .map_err(|_| DraftError::Unavailable)
    }
}

/// Writes what is in the window over the draft kept under `id`, and says whether there was one.
fn replaced(writing: &Transaction, id: i64, draft: &Draft) -> rusqlite::Result<bool> {
    writing
        .execute(
            "UPDATE drafts
             SET recipients = ?2, copies = ?3, subject = ?4, body = ?5,
                 answering = ?6, chain = ?7, blind = ?8, written_at = unixepoch()
             WHERE id = ?1",
            rusqlite::params![
                id,
                draft.recipients,
                draft.copies,
                draft.subject,
                draft.body,
                draft.answering,
                draft.chain.join("\n"),
                draft.blind,
            ],
        )
        .map(|changed| changed > 0)
}

/// Whether the outbox holds a message of the account under `from_outbox`, and if it does, the
/// draft its row names it as having come back as.
///
/// Nothing for a row that is not there, or is another account's.
fn held_back_as(
    writing: &Transaction,
    account_id: &str,
    from_outbox: i64,
) -> rusqlite::Result<Option<Option<i64>>> {
    writing
        .query_row(
            "SELECT came_back_as FROM outbox WHERE id = ?1 AND account_id = ?2",
            rusqlite::params![from_outbox, account_id],
            |row| row.get(0),
        )
        .optional()
}

/// Deletes the outbox's row of a message, and with it, by the cascade the schema declares on
/// every connection of the pool, everybody it was addressed to and every file still in it.
fn let_go(writing: &Transaction, from_outbox: i64) -> rusqlite::Result<()> {
    writing
        .execute("DELETE FROM outbox WHERE id = ?1", [from_outbox])
        .map(|_| ())
}

/// Whether the account has room for a draft more.
fn room_for_another(
    writing: &Transaction,
    account_id: &str,
    at_most: i64,
) -> rusqlite::Result<bool> {
    writing.query_row(
        "SELECT count(*) < ?2 FROM drafts WHERE account_id = ?1",
        rusqlite::params![account_id, at_most],
        |row| row.get(0),
    )
}

/// What one row of `drafts` weighs, in bytes: its text, and the files it carries.
///
/// Read from the lengths SQLite keeps beside each value rather than from the values, so a field
/// or a file that spills onto pages of its own is weighed without those pages being read.
const WEIGHS: &str = "octet_length(recipients) + octet_length(copies) + octet_length(blind)
     + octet_length(subject) + octet_length(body)
     + coalesce(octet_length(answering), 0) + coalesce(octet_length(chain), 0)
     + coalesce((SELECT sum(length(bytes)) FROM draft_files WHERE draft_id = drafts.id), 0)";

/// What the draft kept under `id` weighs, and nothing for one that is not there.
fn weight_of(writing: &Transaction, id: i64) -> rusqlite::Result<Option<i64>> {
    writing
        .query_row(
            &format!("SELECT {WEIGHS} FROM drafts WHERE id = ?1"),
            [id],
            |row| row.get(0),
        )
        .optional()
}

/// What every draft of the account that keeps the one under `id` weighs together.
fn weight_beside(writing: &Transaction, id: i64) -> rusqlite::Result<i64> {
    writing.query_row(
        &format!(
            "SELECT coalesce(sum({WEIGHS}), 0) FROM drafts
             WHERE account_id = (SELECT account_id FROM drafts WHERE id = ?1)"
        ),
        [id],
        |row| row.get(0),
    )
}

/// Writes a draft down as one of its own, and answers its number.
fn written_down(
    writing: &Transaction,
    account_id: &str,
    draft: &Draft,
    came_back: bool,
) -> rusqlite::Result<i64> {
    writing.execute(
        "INSERT INTO drafts
             (account_id, recipients, copies, subject, body, answering, chain, blind, written_at,
              came_back)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, unixepoch(), ?9)",
        rusqlite::params![
            account_id,
            draft.recipients,
            draft.copies,
            draft.subject,
            draft.body,
            draft.answering,
            draft.chain.join("\n"),
            draft.blind,
            came_back,
        ],
    )?;

    Ok(writing.last_insert_rowid())
}

/// Makes the files a draft holds exactly the ones named, in the order named.
///
/// One it already holds under the same name is moved rather than written again: the bytes are the
/// expensive part, and the composer names every file it carries at every keep.
fn carrying(writing: &Transaction, draft_id: i64, files: &[DraftFile]) -> rusqlite::Result<()> {
    let held: HashMap<String, i64> = writing
        .prepare("SELECT kept_as, position FROM draft_files WHERE draft_id = ?1")?
        .query_map([draft_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;

    let named: HashSet<&str> = files.iter().map(|one| one.kept_as.as_str()).collect();
    for kept_as in held
        .keys()
        .filter(|kept_as| !named.contains(kept_as.as_str()))
    {
        writing.execute(
            "DELETE FROM draft_files WHERE draft_id = ?1 AND kept_as = ?2",
            rusqlite::params![draft_id, kept_as],
        )?;
    }

    for (position, one) in files.iter().enumerate() {
        let position = position as i64;

        match held.get(&one.kept_as) {
            Some(&already) if already == position => {}
            Some(_) => {
                writing.execute(
                    "UPDATE draft_files SET position = ?3 WHERE draft_id = ?1 AND kept_as = ?2",
                    rusqlite::params![draft_id, one.kept_as, position],
                )?;
            }
            None => {
                writing.execute(
                    "INSERT INTO draft_files
                         (draft_id, kept_as, name, content_type, bytes, position)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        draft_id,
                        one.kept_as,
                        one.file.name,
                        one.file.content_type,
                        one.file.bytes,
                        position,
                    ],
                )?;
            }
        }
    }

    Ok(())
}

/// One row of the drafts table, in the shape the port answers with.
///
/// One reader for two statements: the newest one and all of them are the same columns in the same
/// order, and two readers of the same row eventually disagree about one of them.
fn kept_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<Kept> {
    Ok(Kept {
        id: row.get(0)?,
        draft: Draft {
            recipients: row.get(1)?,
            copies: row.get(2)?,
            subject: row.get(3)?,
            body: row.get(4)?,
            answering: row.get(5)?,
            chain: row
                .get::<_, Option<String>>(6)?
                .unwrap_or_default()
                .lines()
                .filter(|name| !name.trim().is_empty())
                .map(str::to_owned)
                .collect(),
            blind: row.get(7)?,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::secure_storage::database_key::DatabaseKey;
    use crate::infrastructure::sqlite::{connection::open_profile, migrations::migrate_profile};

    struct TempProfile {
        dir: std::path::PathBuf,
        pool: SqlitePool,
    }

    impl TempProfile {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "lyonoyl-drafts-{name}-{}-{:?}",
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

        fn how_many(&self) -> i64 {
            self.pool
                .get()
                .expect("connection")
                .query_row("SELECT count(*) FROM drafts", [], |row| row.get(0))
                .expect("readable")
        }
    }

    impl Drop for TempProfile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn half_written() -> Draft {
        Draft {
            // Not an address yet, and that is the point: a message being written is allowed to be
            // half of one, and refusing to keep it would lose what somebody typed.
            recipients: "kap".to_owned(),
            copies: String::new(),
            blind: "quiet@example.com".to_owned(),
            subject: String::new(),
            body: "about Tues".to_owned(),
            answering: None,
            chain: Vec::new(),
        }
    }

    #[test]
    fn what_was_typed_comes_back_as_it_was_typed() {
        let db = TempProfile::new("roundtrip");
        let drafts = db.drafts();

        let id = drafts
            .keep("acc-1", None, &half_written(), &[])
            .expect("kept");
        let back = drafts
            .latest("acc-1")
            .expect("readable")
            .expect("it is there");

        assert_eq!(back.id, id);
        assert_eq!(back.draft, half_written());
    }

    #[test]
    fn keeping_it_again_replaces_it_rather_than_leaving_two() {
        // A row per keystroke would be a folder of half-written messages nobody asked for.
        let db = TempProfile::new("replaces");
        let drafts = db.drafts();

        let id = drafts
            .keep("acc-1", None, &half_written(), &[])
            .expect("kept");
        let mut more = half_written();
        more.body = "about Tuesday".to_owned();
        let again = drafts
            .keep("acc-1", Some(id), &more, &[])
            .expect("kept again");

        assert_eq!(again, id);
        assert_eq!(db.how_many(), 1);
        assert_eq!(
            drafts
                .latest("acc-1")
                .expect("readable")
                .expect("it is there")
                .draft
                .body,
            "about Tuesday"
        );
    }

    #[test]
    fn keeping_one_that_was_thrown_away_writes_it_down_rather_than_failing() {
        // Somebody throws a message away and the window, a moment behind, keeps what is in it. The
        // row is gone, so there is nothing to update — and losing the keystrokes because of that
        // would be losing them at the moment they were typed.
        let db = TempProfile::new("discarded");
        let drafts = db.drafts();

        let id = drafts
            .keep("acc-1", None, &half_written(), &[])
            .expect("kept");
        drafts.discard(id).expect("thrown away");
        assert_eq!(db.how_many(), 0);

        let mut later = half_written();
        later.body = "about Tuesday".to_owned();
        drafts
            .keep("acc-1", Some(id), &later, &[])
            .expect("it is written down anyway");

        assert_eq!(db.how_many(), 1, "one message, not none and not two");
        assert_eq!(
            drafts
                .latest("acc-1")
                .expect("readable")
                .expect("there")
                .draft
                .body,
            "about Tuesday"
        );
    }

    #[test]
    fn what_is_being_written_is_committed_before_the_window_is_told() {
        // The window closes on this answer, so an answer that arrives before the commit is a
        // message lost by anything that stops the machine in between. A second connection cannot
        // see an uncommitted transaction on another one, so seeing the row settles the order.
        let db = TempProfile::new("committed-first");

        db.drafts()
            .keep("acc-1", None, &half_written(), &[])
            .expect("it is written down");

        let elsewhere = db.pool.get().expect("another connection");
        let written: i64 = elsewhere
            .query_row("SELECT count(*) FROM drafts", [], |row| row.get(0))
            .expect("readable");

        assert_eq!(written, 1, "it was not committed before answering");
    }

    #[test]
    fn an_account_with_nothing_being_written_says_so() {
        let db = TempProfile::new("nothing");

        assert_eq!(db.drafts().latest("acc-1").expect("readable"), None);
    }

    #[test]
    fn the_conversation_it_belongs_to_survives_the_round_trip() {
        let db = TempProfile::new("chain");
        let drafts = db.drafts();
        let answering = Draft {
            answering: Some("first@example.com".to_owned()),
            chain: vec![
                "first@example.com".to_owned(),
                "second@example.com".to_owned(),
            ],
            ..half_written()
        };

        drafts.keep("acc-1", None, &answering, &[]).expect("kept");

        assert_eq!(
            drafts
                .latest("acc-1")
                .expect("readable")
                .expect("there")
                .draft,
            answering
        );
    }

    /// A file to carry, whose every byte says which file it is.
    fn a_file(kept_as: &str, name: &str, size: usize) -> DraftFile {
        DraftFile {
            kept_as: kept_as.to_owned(),
            file: Arc::new(OutgoingFile {
                name: name.to_owned(),
                content_type: "application/pdf".to_owned(),
                bytes: vec![name.as_bytes()[0]; size],
            }),
        }
    }

    impl TempProfile {
        /// The row each file of a draft is held in, and what it is kept as, in order.
        fn rows_of(&self, draft: i64) -> Vec<(i64, String)> {
            self.pool
                .get()
                .expect("connection")
                .prepare(
                    "SELECT id, kept_as FROM draft_files WHERE draft_id = ?1 ORDER BY position, id",
                )
                .expect("readable")
                .query_map([draft], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("rows")
                .map(|row| row.expect("row"))
                .collect()
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

        /// Whether a draft says it is a message that came back.
        fn came_back(&self, draft: i64) -> bool {
            self.pool
                .get()
                .expect("connection")
                .query_row(
                    "SELECT came_back FROM drafts WHERE id = ?1",
                    [draft],
                    |row| row.get(0),
                )
                .expect("readable")
        }

        /// Refuses, from now on, every file of that name a draft is given.
        fn refusing_a_file_named(&self, name: &str) {
            self.pool
                .get()
                .expect("connection")
                .execute_batch(&format!(
                    "CREATE TRIGGER refused BEFORE INSERT ON draft_files
                     WHEN NEW.name = '{name}'
                     BEGIN SELECT RAISE(ABORT, 'refused'); END;"
                ))
                .expect("the trigger is written");
        }

        /// A message written down in the outbox carrying these files, and the number of its row.
        fn queued_carrying(&self, message_id: &str, files: &[(&str, usize)]) -> i64 {
            use crate::infrastructure::sqlite::outbox::SqliteOutbox;
            use crate::ports::outbox::Outbox;
            use lyonoyl_domain::outgoing::{EmailAddress, Outgoing, Subject};

            let message = Outgoing {
                files: files
                    .iter()
                    .map(|(name, size)| (*a_file("", name, *size).file).clone())
                    .collect(),
                from: EmailAddress::parse("zeta@example.com").expect("an address"),
                to: vec![EmailAddress::parse("kappa@example.net").expect("an address")],
                cc: vec![EmailAddress::parse("sigma@example.org").expect("an address")],
                bcc: vec![EmailAddress::parse("records@example.com").expect("an address")],
                subject: Subject::parse("About Tuesday").expect("a subject"),
                body: "Shall we?".to_owned(),
                thread: Default::default(),
            };
            SqliteOutbox::new(self.pool.clone())
                .queue("acc-1", message_id, &message)
                .expect("it is written down");
            self.pool
                .get()
                .expect("connection")
                .query_row(
                    "SELECT id FROM outbox WHERE message_id = ?1",
                    [message_id],
                    |row| row.get(0),
                )
                .expect("readable")
        }

        /// How many files an outbox row still carries.
        fn left_in_the_outbox(&self, outbox_id: i64) -> i64 {
            self.pool
                .get()
                .expect("connection")
                .query_row(
                    "SELECT count(*) FROM outbox_files WHERE outbox_id = ?1",
                    [outbox_id],
                    |row| row.get(0),
                )
                .expect("readable")
        }

        /// What the outbox holds of one message: its row, everybody it is addressed to, and the
        /// files it carries.
        fn held_in_the_outbox(&self, outbox_id: i64) -> (i64, i64, i64) {
            held_on(&self.pool.get().expect("connection"), outbox_id)
        }

        /// Says on an outbox row, as a build that kept the row did, that its message came back
        /// as `draft`, and holds it for a person.
        fn naming(&self, outbox_id: i64, draft: i64) {
            self.pool
                .get()
                .expect("connection")
                .execute(
                    "UPDATE outbox
                     SET came_back_as = ?2, state = 'held', held_because = 'signInFailed'
                     WHERE id = ?1",
                    [outbox_id, draft],
                )
                .expect("the row says so");
        }

        fn another_account(&self, id: &str) {
            self.pool
                .get()
                .expect("connection")
                .execute(
                    "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                     VALUES (?1, 'kappa@example.net', 'Kappa', 'imap', 2)",
                    [id],
                )
                .expect("a second account");
        }

        /// As many drafts as asked, written straight into the table in one change: a keep each
        /// would wait for the disk ten thousand times.
        fn filled_with(&self, account_id: &str, how_many: usize) {
            let mut connection = self.pool.get().expect("connection");
            let writing = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("a change");
            for _ in 0..how_many {
                writing
                    .execute(
                        "INSERT INTO drafts (account_id, written_at) VALUES (?1, 0)",
                        [account_id],
                    )
                    .expect("written");
            }
            writing.commit().expect("committed");
        }

        /// Drafts that may weigh `weight` bytes together, held to the domain's bound on how many.
        fn drafts_weighing_at_most(&self, weight: i64) -> SqliteDrafts {
            SqliteDrafts::holding(self.pool.clone(), MOST_DRAFTS as i64, weight)
        }

        /// Refuses, from now on, every change of the kind named: `DELETE ON outbox_files`, say.
        fn refusing(&self, change: &str) {
            self.pool
                .get()
                .expect("connection")
                .execute_batch(&format!(
                    "CREATE TRIGGER refusing BEFORE {change}
                     BEGIN SELECT RAISE(ABORT, 'refused'); END;"
                ))
                .expect("the trigger is written");
        }
    }

    #[test]
    fn a_draft_keeps_the_files_it_carries_whole_and_in_order() {
        let db = TempProfile::new("files");
        let drafts = db.drafts();
        let carried = [
            a_file("t1", "invoice.pdf", 300),
            a_file("t2", "photo.jpg", 500),
        ];

        let id = drafts
            .keep("acc-1", None, &half_written(), &carried)
            .expect("kept");

        assert_eq!(drafts.files(id).expect("readable"), carried.to_vec());
    }

    #[test]
    fn a_file_the_draft_already_holds_is_not_written_again() {
        // The composer names every file it carries at every keep, a second or so after the typing
        // stops. Written each time, a photograph attached ten minutes ago is ten minutes of
        // eighteen megabytes a second to a disk this database waits for.
        let db = TempProfile::new("files-once");
        let drafts = db.drafts();
        let id = drafts
            .keep(
                "acc-1",
                None,
                &half_written(),
                &[a_file("t1", "invoice.pdf", 300)],
            )
            .expect("kept");
        let first = db.rows_of(id);

        // The same name with other bytes behind it: bytes written again would be these.
        let mut more = half_written();
        more.body = "about Tuesday".to_owned();
        drafts
            .keep("acc-1", Some(id), &more, &[a_file("t1", "other.pdf", 7)])
            .expect("kept again");

        assert_eq!(db.rows_of(id), first, "the file was written again");
        assert_eq!(
            drafts.files(id).expect("readable"),
            vec![a_file("t1", "invoice.pdf", 300)]
        );
        assert_eq!(
            drafts
                .latest("acc-1")
                .expect("readable")
                .expect("there")
                .draft
                .body,
            "about Tuesday",
            "the text was not kept with it"
        );
    }

    #[test]
    fn a_file_taken_off_leaves_the_draft_at_the_next_keep() {
        let db = TempProfile::new("files-off");
        let drafts = db.drafts();
        let id = drafts
            .keep(
                "acc-1",
                None,
                &half_written(),
                &[
                    a_file("t1", "invoice.pdf", 300),
                    a_file("t2", "photo.jpg", 500),
                ],
            )
            .expect("kept");

        drafts
            .keep(
                "acc-1",
                Some(id),
                &half_written(),
                &[a_file("t2", "photo.jpg", 500)],
            )
            .expect("kept without it");

        assert_eq!(
            drafts.files(id).expect("readable"),
            vec![a_file("t2", "photo.jpg", 500)]
        );
        assert_eq!(
            db.rows_in("draft_files"),
            1,
            "the file taken off is still held"
        );
    }

    #[test]
    fn the_files_keep_the_order_they_are_named_in_without_being_written_again() {
        let db = TempProfile::new("files-order");
        let drafts = db.drafts();
        let (invoice, photo) = (
            a_file("t1", "invoice.pdf", 300),
            a_file("t2", "photo.jpg", 500),
        );
        let id = drafts
            .keep(
                "acc-1",
                None,
                &half_written(),
                &[invoice.clone(), photo.clone()],
            )
            .expect("kept");
        let rows: Vec<i64> = db.rows_of(id).into_iter().map(|(row, _)| row).collect();

        drafts
            .keep(
                "acc-1",
                Some(id),
                &half_written(),
                &[photo.clone(), invoice.clone()],
            )
            .expect("kept in the other order");

        assert_eq!(drafts.files(id).expect("readable"), vec![photo, invoice]);
        assert_eq!(
            db.rows_of(id)
                .into_iter()
                .map(|(row, _)| row)
                .collect::<Vec<_>>(),
            vec![rows[1], rows[0]],
            "moving a file wrote it again"
        );
    }

    #[test]
    fn the_files_go_with_the_draft() {
        // Sent or thrown away, a draft is gone, and a file left behind is somebody's document that
        // nothing will ever send and nothing will ever clear.
        let db = TempProfile::new("files-go");
        let drafts = db.drafts();
        let id = drafts
            .keep(
                "acc-1",
                None,
                &half_written(),
                &[a_file("t1", "invoice.pdf", 300)],
            )
            .expect("kept");

        drafts.discard(id).expect("thrown away");

        assert_eq!(db.rows_in("draft_files"), 0);
        assert_eq!(drafts.files(id).expect("readable"), Vec::new());
    }

    #[test]
    fn a_draft_thrown_away_while_it_was_written_is_written_down_again_with_its_files() {
        // The row is gone, and the files with it; what is in the window is a draft of its own now,
        // and it carries everything the window shows.
        let db = TempProfile::new("files-again");
        let drafts = db.drafts();
        let carried = [a_file("t1", "invoice.pdf", 300)];
        let id = drafts
            .keep("acc-1", None, &half_written(), &carried)
            .expect("kept");
        drafts.discard(id).expect("thrown away");

        let again = drafts
            .keep("acc-1", Some(id), &half_written(), &carried)
            .expect("written down anyway");

        assert_eq!(drafts.files(again).expect("readable"), carried.to_vec());
    }

    #[test]
    fn the_text_and_the_files_are_kept_in_one_change() {
        // A keep that wrote the new text and not the file attached with it is a draft that comes
        // back without the file and says nothing about it.
        let db = TempProfile::new("files-atomic");
        let drafts = db.drafts();
        let id = drafts
            .keep(
                "acc-1",
                None,
                &half_written(),
                &[a_file("t1", "invoice.pdf", 300)],
            )
            .expect("kept");
        db.refusing_a_file_named("refused.pdf");

        let mut more = half_written();
        more.body = "about Tuesday".to_owned();
        let refused = drafts.keep("acc-1", Some(id), &more, &[a_file("t2", "refused.pdf", 9)]);

        assert_eq!(refused, Err(DraftError::Unavailable));
        assert_eq!(
            drafts
                .latest("acc-1")
                .expect("readable")
                .expect("there")
                .draft,
            half_written(),
            "the text was kept without the file"
        );
        assert_eq!(
            drafts.files(id).expect("readable"),
            vec![a_file("t1", "invoice.pdf", 300)],
            "the file taken off went without the rest of the keep"
        );
    }

    #[test]
    fn a_file_is_committed_with_its_draft_before_the_window_is_told() {
        // Asked on a connection taken before the keep, so it cannot be the one the keep used: a
        // connection sees another's change only once it is committed.
        let db = TempProfile::new("files-committed");
        let elsewhere = db.pool.get().expect("another connection");

        db.drafts()
            .keep(
                "acc-1",
                None,
                &half_written(),
                &[a_file("t1", "invoice.pdf", 300)],
            )
            .expect("it is written down");

        let written: i64 = elsewhere
            .query_row("SELECT count(*) FROM draft_files", [], |row| row.get(0))
            .expect("readable");
        assert_eq!(written, 1, "the file was not committed before answering");
    }

    /// What `connection` sees of one message in the outbox: its row, everybody it is addressed to,
    /// and the files it carries.
    fn held_on(connection: &rusqlite::Connection, outbox_id: i64) -> (i64, i64, i64) {
        connection
            .query_row(
                "SELECT (SELECT count(*) FROM outbox WHERE id = ?1),
                        (SELECT count(*) FROM outbox_recipients WHERE outbox_id = ?1),
                        (SELECT count(*) FROM outbox_files WHERE outbox_id = ?1)",
                [outbox_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("readable")
    }

    #[test]
    fn a_message_that_came_back_brings_the_files_it_carried() {
        let db = TempProfile::new("came-back");
        let drafts = db.drafts();
        let refused = db.queued_carrying(
            "<one@example.com>",
            &[("invoice.pdf", 300), ("photo.jpg", 500)],
        );
        let other = db.queued_carrying("<two@example.com>", &[("notes.txt", 40)]);

        let id = drafts
            .came_back(
                "acc-1",
                refused,
                &Draft {
                    recipients: "kappa@example.net".to_owned(),
                    subject: "About Tuesday".to_owned(),
                    body: "Shall we?".to_owned(),
                    ..Draft::default()
                },
            )
            .expect("readable")
            .expect("it is back");

        let carried = drafts.files(id).expect("readable");
        assert_eq!(
            carried
                .iter()
                .map(|one| (one.file.name.as_str(), one.file.bytes.len()))
                .collect::<Vec<_>>(),
            vec![("invoice.pdf", 300), ("photo.jpg", 500)]
        );
        assert_eq!(
            *carried[0].file,
            (*a_file("", "invoice.pdf", 300).file).clone()
        );
        assert_ne!(
            carried[0].kept_as, carried[1].kept_as,
            "two files kept under one name"
        );
        assert_eq!(db.left_in_the_outbox(refused), 0, "the outbox kept a copy");
        assert_eq!(
            db.left_in_the_outbox(other),
            1,
            "another message lost its file"
        );
        assert!(db.came_back(id), "the draft does not say it came back");
        assert_eq!(
            drafts
                .latest("acc-1")
                .expect("readable")
                .expect("there")
                .draft
                .subject,
            "About Tuesday"
        );
    }

    #[test]
    fn a_message_that_came_back_leaves_nothing_of_itself_in_the_outbox() {
        // The draft holds it now: the text, everybody it was addressed to and the files it
        // carried. Its row kept as well would be a second copy of all of it, blind copies
        // included, in the database nothing clears, and nothing would ever read it.
        let db = TempProfile::new("came-back-alone");
        let drafts = db.drafts();
        let refused = db.queued_carrying("<one@example.com>", &[("invoice.pdf", 300)]);
        let other = db.queued_carrying("<two@example.com>", &[("notes.txt", 40)]);
        // Taken out of the pool before the call, so the call cannot have used it: what it sees
        // was committed before `came_back` answered.
        let elsewhere = db.pool.get().expect("a connection of its own");

        drafts
            .came_back("acc-1", refused, &half_written())
            .expect("readable")
            .expect("it is back");

        assert_eq!(
            held_on(&elsewhere, refused),
            (0, 0, 0),
            "the outbox kept part of a message the drafts hold"
        );
        assert_eq!(
            held_on(&elsewhere, other),
            (1, 3, 1),
            "another message lost part of itself"
        );
    }

    #[test]
    fn a_message_comes_back_whole_or_not_at_all() {
        // A draft written and its files left behind, or the files taken out and no draft written:
        // either is a document a crash between the two loses.
        let db = TempProfile::new("came-back-atomic");
        let drafts = db.drafts();
        let refused = db.queued_carrying("<one@example.com>", &[("refused.pdf", 300)]);
        db.refusing_a_file_named("refused.pdf");

        let outcome = drafts.came_back("acc-1", refused, &half_written());

        assert_eq!(outcome, Err(DraftError::Unavailable));
        assert_eq!(db.how_many(), 0, "a draft came back without its file");
        assert_eq!(
            db.held_in_the_outbox(refused),
            (1, 3, 1),
            "the message left the outbox for nowhere"
        );
    }

    #[test]
    fn a_message_whose_files_cannot_leave_the_outbox_does_not_come_back() {
        // A draft committed with its files still in the outbox as well is a second copy of
        // somebody's document that nothing reads.
        let db = TempProfile::new("came-back-files-stay");
        let drafts = db.drafts();
        let refused = db.queued_carrying("<one@example.com>", &[("invoice.pdf", 300)]);
        db.refusing("DELETE ON outbox_files");

        let outcome = drafts.came_back("acc-1", refused, &half_written());

        assert_eq!(outcome, Err(DraftError::Unavailable));
        assert_eq!(db.how_many(), 0, "it came back with its file left behind");
        assert_eq!(db.held_in_the_outbox(refused), (1, 3, 1));
    }

    #[test]
    fn a_message_the_outbox_cannot_let_go_of_does_not_come_back() {
        // A draft committed with the row still in the outbox is the message in two places:
        // everybody it was addressed to, blind copies included, kept a second time for nothing.
        let db = TempProfile::new("came-back-row-stays");
        let drafts = db.drafts();
        let refused = db.queued_carrying("<one@example.com>", &[("invoice.pdf", 300)]);
        db.refusing("DELETE ON outbox");

        let outcome = drafts.came_back("acc-1", refused, &half_written());

        assert_eq!(outcome, Err(DraftError::Unavailable));
        assert_eq!(db.how_many(), 0, "it came back with its row left behind");
        assert_eq!(
            db.held_in_the_outbox(refused),
            (1, 3, 1),
            "part of the message left the outbox for a draft that is not there"
        );
    }

    #[test]
    fn a_message_given_back_twice_comes_back_once_with_its_files() {
        // A round that read the message before another gave it back gives it back again. Its
        // files went with the first draft: a second would be the message without them, counted
        // as come back twice and there to be sent again without its attachment.
        let db = TempProfile::new("came-back-twice");
        let drafts = db.drafts();
        let refused = db.queued_carrying("<one@example.com>", &[("invoice.pdf", 300)]);

        let first = drafts
            .came_back("acc-1", refused, &half_written())
            .expect("readable")
            .expect("it is back");
        let again = drafts
            .came_back("acc-1", refused, &half_written())
            .expect("asked again");

        assert_eq!(again, None, "it came back a second time");
        assert_eq!(db.how_many(), 1, "it came back twice");
        assert_eq!(
            drafts.files(first).expect("readable").len(),
            1,
            "the draft lost its file"
        );
    }

    #[test]
    fn a_message_whose_draft_is_gone_does_not_come_back_again() {
        // Sent again from its draft, or thrown away: given back once more, it would be a message
        // somebody has already dealt with, back without its files.
        let db = TempProfile::new("came-back-gone");
        let drafts = db.drafts();
        let refused = db.queued_carrying("<one@example.com>", &[("invoice.pdf", 300)]);
        let id = drafts
            .came_back("acc-1", refused, &half_written())
            .expect("readable")
            .expect("it is back");
        drafts.discard(id).expect("thrown away");

        let again = drafts
            .came_back("acc-1", refused, &half_written())
            .expect("asked again");

        assert_eq!(again, None);
        assert_eq!(db.how_many(), 0, "a message thrown away came back");
    }

    #[test]
    fn nothing_comes_back_of_a_message_the_outbox_does_not_hold_for_the_account() {
        // Not there at all, or another account's: a draft written for it would be a message
        // nobody wrote, or somebody's message in the wrong account's drafts.
        let db = TempProfile::new("came-back-not-held");
        db.another_account("acc-2");
        let drafts = db.drafts();
        let refused = db.queued_carrying("<one@example.com>", &[("invoice.pdf", 300)]);

        assert_eq!(drafts.came_back("acc-1", 9_999, &half_written()), Ok(None));
        assert_eq!(
            drafts.came_back("acc-2", refused, &half_written()),
            Ok(None)
        );

        assert_eq!(db.how_many(), 0, "a draft came back from nothing");
        assert_eq!(
            db.held_in_the_outbox(refused),
            (1, 3, 1),
            "another account let go of a message that was not its own"
        );
    }

    #[test]
    fn a_row_that_names_the_draft_its_message_came_back_as_goes_and_brings_nothing_back() {
        // The draft it names holds the message and the files it carried, and the row a copy of
        // the rest. Given back again it goes, and writes nothing, whether or not that draft is
        // still there.
        let db = TempProfile::new("came-back-named");
        let drafts = db.drafts();
        let named = db.queued_carrying("<one@example.com>", &[]);
        let draft = drafts
            .keep("acc-1", None, &half_written(), &[])
            .expect("kept");
        db.naming(named, draft);
        let gone = db.queued_carrying("<two@example.com>", &[]);
        db.naming(gone, draft + 1);

        assert_eq!(drafts.came_back("acc-1", named, &half_written()), Ok(None));
        assert_eq!(drafts.came_back("acc-1", gone, &half_written()), Ok(None));

        assert_eq!(db.how_many(), 1, "it came back as a second draft");
        for row in [named, gone] {
            assert_eq!(
                db.held_in_the_outbox(row),
                (0, 0, 0),
                "the row outlived the draft that holds its message"
            );
        }
    }

    #[test]
    fn a_draft_that_came_back_still_says_so_once_it_is_kept_again() {
        // It says so for as long as it exists: opened and written into, it is still the message
        // somebody pressed send on.
        let db = TempProfile::new("came-back-kept");
        let drafts = db.drafts();
        let refused = db.queued_carrying("<one@example.com>", &[("invoice.pdf", 300)]);
        let id = drafts
            .came_back("acc-1", refused, &half_written())
            .expect("readable")
            .expect("it is back");
        let carried = drafts.files(id).expect("readable");

        let mut more = half_written();
        more.body = "about Tuesday".to_owned();
        drafts
            .keep("acc-1", Some(id), &more, &carried)
            .expect("kept again");

        assert!(db.came_back(id));
        assert_eq!(drafts.files(id).expect("readable"), carried);
    }

    #[test]
    fn a_draft_kept_from_the_window_did_not_come_back() {
        let db = TempProfile::new("not-came-back");

        let id = db
            .drafts()
            .keep("acc-1", None, &half_written(), &[])
            .expect("kept");

        assert!(!db.came_back(id));
    }

    #[test]
    fn an_account_keeps_as_many_drafts_as_it_may_and_writes_no_more() {
        // A window keeping under no number writes a draft of its own each time. Past the bound it
        // is refused and writes nothing, file and all; a draft already there is kept again,
        // because that writes no row; and another account has room of its own.
        let db = TempProfile::new("room-count");
        db.another_account("acc-2");
        let drafts = db.drafts();
        db.filled_with("acc-1", MOST_DRAFTS - 1);

        let last = drafts
            .keep("acc-1", None, &half_written(), &[])
            .expect("the last place is taken");
        let past = drafts.keep(
            "acc-1",
            None,
            &half_written(),
            &[a_file("t1", "invoice.pdf", 300)],
        );

        assert_eq!(past, Err(DraftError::NoRoom));
        assert_eq!(
            db.how_many(),
            MOST_DRAFTS as i64,
            "a draft past the bound was written"
        );
        assert_eq!(
            db.rows_in("draft_files"),
            0,
            "the file of a refused draft was written"
        );
        let mut more = half_written();
        more.body = "about Tuesday".to_owned();
        assert_eq!(
            drafts.keep("acc-1", Some(last), &more, &[]),
            Ok(last),
            "a draft already there was refused"
        );
        drafts
            .keep("acc-2", None, &half_written(), &[])
            .expect("another account has room of its own");
    }

    #[test]
    fn two_keeps_at_once_cannot_both_take_the_last_place() {
        // Each reads the room and writes into it in one change, so the second waits for the first
        // and finds the room taken.
        let db = TempProfile::new("room-race");
        db.filled_with("acc-1", MOST_DRAFTS - 1);
        let together = std::sync::Barrier::new(2);

        let outcomes: Vec<Result<i64, DraftError>> = std::thread::scope(|scope| {
            let keeps: Vec<_> = (0..2)
                .map(|_| {
                    scope.spawn(|| {
                        let drafts = db.drafts();
                        together.wait();
                        drafts.keep("acc-1", None, &half_written(), &[])
                    })
                })
                .collect();
            keeps
                .into_iter()
                .map(|keep| keep.join().expect("the keep ran"))
                .collect()
        });

        assert_eq!(
            outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
            1,
            "{outcomes:?}"
        );
        assert!(outcomes.contains(&Err(DraftError::NoRoom)), "{outcomes:?}");
        assert_eq!(db.how_many(), MOST_DRAFTS as i64);
    }

    /// A draft whose text weighs `bytes`, every one of them in its body.
    fn weighing(bytes: usize) -> Draft {
        Draft {
            body: "w".repeat(bytes),
            ..Draft::default()
        }
    }

    #[test]
    fn a_draft_that_would_take_the_drafts_past_what_they_may_weigh_is_refused_whole() {
        // Each kept as a draft of its own and each carrying the same file, the way a window gone
        // wrong keeps them: kept up to what they may weigh together, and refused past it.
        let db = TempProfile::new("room-weight");
        let drafts = db.drafts_weighing_at_most(1_000);
        for _ in 0..2 {
            drafts
                .keep(
                    "acc-1",
                    None,
                    &weighing(100),
                    &[a_file("t1", "invoice.pdf", 400)],
                )
                .expect("within the room, and at its edge");
        }

        let past = drafts.keep("acc-1", None, &weighing(1), &[]);

        assert_eq!(past, Err(DraftError::NoRoom));
        assert_eq!(
            (db.how_many(), db.rows_in("draft_files")),
            (2, 2),
            "what was refused was written"
        );
    }

    #[test]
    fn a_draft_kept_again_heavier_than_the_room_left_is_refused_and_stays_as_it_was() {
        // Grown keep by keep, every draft a window keeps could hold what a draft may: the room is
        // weighed at every keep that adds to one, not only at a draft of its own.
        let db = TempProfile::new("room-grown");
        let drafts = db.drafts_weighing_at_most(1_000);
        drafts
            .keep("acc-1", None, &weighing(600), &[])
            .expect("kept");
        let id = drafts
            .keep("acc-1", None, &weighing(300), &[])
            .expect("kept");

        let grown = drafts.keep(
            "acc-1",
            Some(id),
            &weighing(300),
            &[a_file("t1", "invoice.pdf", 101)],
        );

        assert_eq!(grown, Err(DraftError::NoRoom));
        assert_eq!(drafts.files(id).expect("readable"), Vec::new());
        drafts
            .keep("acc-1", Some(id), &weighing(400), &[])
            .expect("as much as is left is kept");
    }

    #[test]
    fn a_keep_that_adds_nothing_goes_through_however_heavy_the_drafts_are() {
        // Past the room, a person makes room by taking a file off or writing less, and a keep
        // refused for that would keep nothing they did. Only one that adds is refused.
        let db = TempProfile::new("room-lighter");
        let roomy = db.drafts();
        let heavy = roomy
            .keep(
                "acc-1",
                None,
                &weighing(1_000),
                &[a_file("t1", "invoice.pdf", 1_000)],
            )
            .expect("kept");
        let light = roomy
            .keep("acc-1", None, &weighing(100), &[])
            .expect("kept");
        let tight = db.drafts_weighing_at_most(1_000);

        tight
            .keep("acc-1", Some(heavy), &weighing(1_000), &[])
            .expect("a file taken off");
        tight
            .keep(
                "acc-1",
                Some(light),
                &Draft {
                    body: "x".repeat(100),
                    ..Draft::default()
                },
                &[],
            )
            .expect("written over, no heavier");

        assert_eq!(
            tight.keep("acc-1", Some(light), &weighing(101), &[]),
            Err(DraftError::NoRoom)
        );
        assert_eq!(
            tight.files(heavy).expect("readable"),
            Vec::new(),
            "the file taken off came back"
        );
    }

    #[test]
    fn what_a_draft_weighs_is_every_field_a_window_fills_and_every_file() {
        // A field left out of the weighing is a way past the room.
        let db = TempProfile::new("room-fields");
        let drafts = db.drafts_weighing_at_most(9);
        let ten = || "x".repeat(10);
        let filled: [Draft; 7] = [
            Draft {
                recipients: ten(),
                ..Draft::default()
            },
            Draft {
                copies: ten(),
                ..Draft::default()
            },
            Draft {
                blind: ten(),
                ..Draft::default()
            },
            Draft {
                subject: ten(),
                ..Draft::default()
            },
            Draft {
                body: ten(),
                ..Draft::default()
            },
            Draft {
                answering: Some(ten()),
                ..Draft::default()
            },
            Draft {
                chain: vec![ten()],
                ..Draft::default()
            },
        ];

        for draft in filled {
            assert_eq!(
                drafts.keep("acc-1", None, &draft, &[]),
                Err(DraftError::NoRoom),
                "{draft:?}"
            );
        }
        assert_eq!(
            drafts.keep(
                "acc-1",
                None,
                &Draft::default(),
                &[a_file("t1", "invoice.pdf", 10)]
            ),
            Err(DraftError::NoRoom)
        );
        assert_eq!(db.how_many(), 0);
    }

    #[test]
    fn a_draft_is_weighed_with_the_account_it_is_kept_in() {
        // A keep naming another account under this one's number writes over this one's draft.
        // Weighed with the account it names, it would grow a draft past its own account's room
        // by naming one with room to spare; and another account's drafts weigh nothing here.
        let db = TempProfile::new("room-whose");
        db.another_account("acc-2");
        let drafts = db.drafts_weighing_at_most(1_000);
        drafts
            .keep("acc-1", None, &weighing(900), &[])
            .expect("kept");
        let id = drafts
            .keep("acc-1", None, &weighing(100), &[])
            .expect("kept");

        assert_eq!(
            drafts.keep("acc-2", Some(id), &weighing(101), &[]),
            Err(DraftError::NoRoom)
        );
        drafts
            .keep("acc-2", None, &weighing(1_000), &[])
            .expect("another account's room is its own");
    }

    #[test]
    fn a_message_that_comes_back_is_never_refused_for_room() {
        // Refused, it would stay in the outbox, where nothing shows it, and stop every round of
        // its account: the outbox lets go of it only in the change that writes its draft.
        let db = TempProfile::new("room-came-back");
        db.filled_with("acc-1", MOST_DRAFTS);
        let refused = db.queued_carrying("<one@example.com>", &[("invoice.pdf", 300)]);
        let drafts = db.drafts_weighing_at_most(0);

        let id = drafts
            .came_back("acc-1", refused, &half_written())
            .expect("readable")
            .expect("it is back");

        assert_eq!(drafts.files(id).expect("readable").len(), 1);
        assert_eq!(db.held_in_the_outbox(refused), (0, 0, 0));
    }

    #[test]
    fn the_drafts_the_application_opens_are_held_to_the_room_the_domain_gives() {
        // Every other test of the weight holds its drafts to a room of its own, a few bytes wide,
        // because the domain's is gigabytes. This one is about the figures: the drafts the
        // application opens are held to the domain's, and the heaviest draft a window can keep —
        // as much text as a message may carry in its body, its subject and its addresses, the
        // longest names of a conversation, and files as heavy as a message may be — fits in them.
        use lyonoyl_domain::message::LONGEST_MESSAGE_ID;
        use lyonoyl_domain::outgoing::{HEAVIEST_MESSAGE, LONGEST_BODY};
        use lyonoyl_domain::thread::NAMES_CONSULTED;

        let db = TempProfile::new("room-domain");
        let drafts = db.drafts();
        let name = format!("<{}>", "n".repeat(LONGEST_MESSAGE_ID));
        let heaviest = Draft {
            recipients: "r".repeat(LONGEST_BODY),
            subject: "s".repeat(LONGEST_BODY),
            body: "b".repeat(LONGEST_BODY),
            answering: Some(name.clone()),
            chain: vec![name; NAMES_CONSULTED],
            ..Draft::default()
        };

        assert_eq!(
            (drafts.room.drafts, drafts.room.weight),
            (MOST_DRAFTS as i64, HEAVIEST_DRAFTS as i64)
        );
        drafts
            .keep(
                "acc-1",
                None,
                &heaviest,
                &[a_file("t1", "photo.jpg", HEAVIEST_MESSAGE)],
            )
            .expect("the heaviest draft a window can keep fits in an account's room");
    }
}
