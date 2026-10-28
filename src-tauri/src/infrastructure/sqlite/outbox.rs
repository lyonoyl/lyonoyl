//! The outbox, in the database that is nobody's copy of anything.
//!
//! Every write here goes through the profile pool, which commits with `synchronous = FULL`: a
//! message somebody wrote and a machine that lost power a second later must not be a message that
//! was never written. That cost is paid once per message sent, which is the one place in this
//! application where it is obviously worth paying.

use rusqlite::{Connection, TransactionBehavior};
use std::time::Duration;

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::ports::outbox::{Held, Outbox, OutboxError, WhyHeld};
use lyonoyl_domain::message::Rfc822MessageId;
use lyonoyl_domain::outgoing::{EmailAddress, InThread, Outgoing, OutgoingFile, Subject};

pub struct SqliteOutbox {
    pool: SqlitePool,
}

impl SqliteOutbox {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    fn open(
        &self,
    ) -> Result<r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>, OutboxError> {
        self.pool.get().map_err(|_| OutboxError::Unavailable)
    }
}

/// The columns every query here reads, in the order `held_from` expects them.
const WHAT_A_ROW_HOLDS: &str =
    "id, message_id, sender, subject, body, attempts, answering, chain, written_at, filing_refused";

type Row = (
    i64,
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    i64,
    i64,
);

/// One row as the application has it.
///
/// Parsed on the way out, not only on the way in. A row written by a build that did not have these
/// rules holds whatever it was handed, and the outbox is the authoritative database — the one thing
/// here that is nobody's copy of anything, so a row in it cannot simply be dropped. Refusing to
/// read it says so, where sending it would put somebody else's header on a person's mail.
fn held_from(connection: &rusqlite::Connection, row: Row) -> Result<Held, OutboxError> {
    let (id, message_id, sender, subject, body, attempts, answering, chain, written_at, refused) =
        row;

    Ok(Held {
        id,
        message_id,
        written_at,
        copy_refused: u32::try_from(refused.max(0)).unwrap_or(u32::MAX),
        message: Outgoing {
            files: files_of(connection, id)?,
            from: parse(&sender)?,
            to: recipients(connection, id, "to")?,
            cc: recipients(connection, id, "cc")?,
            bcc: recipients(connection, id, "bcc")?,
            subject: Subject::parse(&subject).map_err(|_| OutboxError::Corrupted)?,
            body,
            thread: InThread {
                answering: answering
                    .as_deref()
                    .map(Rfc822MessageId::parse)
                    .transpose()
                    .map_err(|_| OutboxError::Corrupted)?,
                chain: chain
                    .unwrap_or_default()
                    .lines()
                    .filter(|name| !name.trim().is_empty())
                    .map(Rfc822MessageId::parse)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| OutboxError::Corrupted)?,
            },
        },
        attempts: attempts.max(0) as u32,
    })
}

/// Every row a query over [`WHAT_A_ROW_HOLDS`] answers with, as the application has them.
///
/// One reading of a row for every query that reads one, so no two can disagree about which column
/// is which.
fn held_rows(
    connection: &rusqlite::Connection,
    query: &str,
    parameters: impl rusqlite::Params,
) -> Result<Vec<Held>, OutboxError> {
    let mut statement = connection
        .prepare(query)
        .map_err(|_| OutboxError::Unavailable)?;

    let rows = statement
        .query_map(parameters, |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })
        .map_err(|_| OutboxError::Unavailable)?;

    let mut held = Vec::new();
    for row in rows {
        held.push(held_from(
            connection,
            row.map_err(|_| OutboxError::Unavailable)?,
        )?);
    }

    Ok(held)
}

/// The files one queued message carries, in the order they were attached.
fn files_of(
    connection: &rusqlite::Connection,
    outbox_id: i64,
) -> Result<Vec<OutgoingFile>, OutboxError> {
    let mut statement = connection
        .prepare(
            "SELECT name, content_type, bytes FROM outbox_files
                 WHERE outbox_id = ?1
                 ORDER BY position, id",
        )
        .map_err(|_| OutboxError::Unavailable)?;

    let files = statement
        .query_map([outbox_id], |row| {
            Ok(OutgoingFile {
                name: row.get(0)?,
                content_type: row.get(1)?,
                bytes: row.get(2)?,
            })
        })
        .map_err(|_| OutboxError::Unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| OutboxError::Unavailable)?;

    Ok(files)
}

impl Outbox for SqliteOutbox {
    fn queue(
        &self,
        account_id: &str,
        message_id: &str,
        message: &Outgoing,
    ) -> Result<(), OutboxError> {
        let mut connection = self.open()?;

        // The message and everybody it goes to are one change. A message stored with half its
        // recipients would be sent to half of them, which is worse than not being stored at all.
        let writing = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| OutboxError::Unavailable)?;

        let written = writing.execute(
            "INSERT INTO outbox
                 (account_id, message_id, sender, subject, body, answering, chain, written_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch())",
            rusqlite::params![
                account_id,
                message_id,
                message.from.as_str(),
                message.subject.as_str(),
                message.body,
                message
                    .thread
                    .answering
                    .as_ref()
                    .map(Rfc822MessageId::as_str),
                // A newline between names, which cannot occur inside one: RFC 5322 makes a
                // `Message-ID` a dot-atom in angle brackets.
                (!message.thread.chain.is_empty()).then(|| {
                    message
                        .thread
                        .chain
                        .iter()
                        .map(Rfc822MessageId::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                }),
            ],
        );

        match written {
            Ok(_) => {}
            // The name is unique, and a second row under it is the same message twice.
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
            {
                return Err(OutboxError::AlreadyWaiting)
            }
            Err(_) => return Err(OutboxError::Unavailable),
        }

        let id = writing.last_insert_rowid();

        for (kind, addresses) in [
            ("to", &message.to),
            ("cc", &message.cc),
            ("bcc", &message.bcc),
        ] {
            for address in addresses {
                writing
                    .execute(
                        "INSERT OR IGNORE INTO outbox_recipients (outbox_id, kind, address)
                         VALUES (?1, ?2, ?3)",
                        rusqlite::params![id, kind, address.as_str()],
                    )
                    .map_err(|_| OutboxError::Unavailable)?;
            }
        }

        // In the same transaction as the message. A message stored without the file it was going
        // to carry would be sent without it, and nobody would know until the person who received
        // it asked where the invoice was.
        for (position, file) in message.files.iter().enumerate() {
            writing
                .execute(
                    "INSERT INTO outbox_files (outbox_id, name, content_type, bytes, position)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        id,
                        file.name,
                        file.content_type,
                        file.bytes,
                        position as i64
                    ],
                )
                .map_err(|_| OutboxError::Unavailable)?;
        }

        writing.commit().map_err(|_| OutboxError::Unavailable)
    }

    fn ready(&self, account_id: &str, at_most: usize) -> Result<Vec<Held>, OutboxError> {
        held_rows(
            &*self.open()?,
            // The same columns as everywhere else, from the same place and read by the same
            // function: a column list written out twice drifts, and a query asking for eight
            // while the reader expects nine reads the wrong ones.
            &format!(
                "SELECT {WHAT_A_ROW_HOLDS}
                 FROM outbox
                 WHERE account_id = ?1 AND state = 'waiting' AND try_after <= unixepoch()
                 ORDER BY id ASC
                 LIMIT ?2"
            ),
            rusqlite::params![account_id, at_most as i64],
        )
    }

    fn claim(&self, id: i64) -> Result<bool, OutboxError> {
        // Committed on its own, through the pool that waits for the disk, before the message is
        // handed over: a machine that stops while a server is taking it leaves a row that says
        // it was being handed over, and that row is never handed over again by a round.
        self.open()?
            .execute(
                "UPDATE outbox SET state = 'sending' WHERE id = ?1 AND state = 'waiting'",
                [id],
            )
            .map(|changed| changed == 1)
            .map_err(|_| OutboxError::Unavailable)
    }

    fn interrupted(&self, account_id: &str) -> Result<Vec<Held>, OutboxError> {
        held_rows(
            &*self.open()?,
            &format!(
                "SELECT {WHAT_A_ROW_HOLDS}
                 FROM outbox
                 WHERE account_id = ?1 AND state = 'sending'
                 ORDER BY id ASC"
            ),
            [account_id],
        )
    }

    fn awaiting_filing(
        &self,
        account_id: &str,
        settled: Duration,
        at_most: usize,
    ) -> Result<Vec<Held>, OutboxError> {
        held_rows(
            &*self.open()?,
            // From when a server took it. A row written by a build that did not say when has
            // none, and is counted from when it was written. One whose copy was refused for now
            // waits out its time, and takes no place in the round meanwhile from the ones behind
            // it.
            &format!(
                "SELECT {WHAT_A_ROW_HOLDS}
                 FROM outbox
                 WHERE account_id = ?1
                   AND state = 'sent'
                   AND filed_at IS NULL
                   AND coalesce(accepted_at, written_at) <= unixepoch() - ?2
                   AND file_after <= unixepoch()
                 ORDER BY id ASC
                 LIMIT ?3"
            ),
            rusqlite::params![account_id, settled.as_secs() as i64, at_most as i64],
        )
    }

    fn refused_for_now(&self, id: i64, retry_after: u64) -> Result<(), OutboxError> {
        self.open()?
            .execute(
                "UPDATE outbox
                 SET filing_refused = filing_refused + 1, file_after = unixepoch() + ?2
                 WHERE id = ?1 AND state = 'sent'",
                rusqlite::params![id, retry_after as i64],
            )
            .map(|_| ())
            .map_err(|_| OutboxError::Unavailable)
    }

    fn filed(&self, id: i64) -> Result<(), OutboxError> {
        // Only a sent one: a message no server has taken exists here and nowhere else.
        self.open()?
            .execute(
                "UPDATE outbox SET filed_at = unixepoch() WHERE id = ?1 AND state = 'sent'",
                [id],
            )
            .map(|_| ())
            .map_err(|_| OutboxError::Unavailable)
    }

    fn forget_filed(&self, account_id: &str) -> Result<(), OutboxError> {
        // One statement, so one change: everybody it went to and every file it carried are rows
        // of their own, and the cascade the schema declares takes them with the message — on every
        // connection of the pool, each of which turns foreign keys on.
        self.open()?
            .execute(
                "DELETE FROM outbox
                 WHERE account_id = ?1 AND state = 'sent' AND filed_at IS NOT NULL",
                [account_id],
            )
            .map(|_| ())
            .map_err(|_| OutboxError::Unavailable)
    }

    fn forget_given_back(&self, account_id: &str) -> Result<(), OutboxError> {
        // One statement, and each row goes whole by the cascade, the way `filed` lets one go.
        self.open()?
            .execute(
                "DELETE FROM outbox WHERE account_id = ?1 AND came_back_as IS NOT NULL",
                [account_id],
            )
            .map(|_| ())
            .map_err(|_| OutboxError::Unavailable)
    }

    fn accepted(&self, id: i64) -> Result<(), OutboxError> {
        self.open()?
            .execute(
                "UPDATE outbox SET state = 'sent', held_because = NULL, accepted_at = unixepoch()
                 WHERE id = ?1",
                [id],
            )
            .map(|_| ())
            .map_err(|_| OutboxError::Unavailable)
    }

    fn stopped(&self, id: i64, why: WhyHeld, retry_after: Option<u64>) -> Result<(), OutboxError> {
        let connection = self.open()?;

        // Two different things. A message that will be tried again goes back to waiting with a
        // time on it; one that stopped is held, and no round hands it over again. Each changes a
        // row that is there and nothing else: about one that is gone, nothing is written back.
        match retry_after {
            Some(wait) => connection.execute(
                "UPDATE outbox
                 SET state = 'waiting', attempts = attempts + 1, held_because = ?2,
                     try_after = unixepoch() + ?3
                 WHERE id = ?1",
                rusqlite::params![id, stored(why), wait as i64],
            ),
            None => connection.execute(
                "UPDATE outbox
                 SET state = 'held', attempts = attempts + 1, held_because = ?2
                 WHERE id = ?1",
                rusqlite::params![id, stored(why)],
            ),
        }
        .map(|_| ())
        .map_err(|_| OutboxError::Unavailable)
    }
}

fn recipients(
    connection: &Connection,
    id: i64,
    kind: &str,
) -> Result<Vec<EmailAddress>, OutboxError> {
    let mut statement = connection
        .prepare(
            "SELECT address FROM outbox_recipients
             WHERE outbox_id = ?1 AND kind = ?2
             ORDER BY address",
        )
        .map_err(|_| OutboxError::Unavailable)?;

    let rows = statement
        .query_map(rusqlite::params![id, kind], |row| row.get::<_, String>(0))
        .map_err(|_| OutboxError::Unavailable)?;

    let mut addresses = Vec::new();
    for row in rows {
        addresses.push(parse(&row.map_err(|_| OutboxError::Unavailable)?)?);
    }

    Ok(addresses)
}

/// An address stored here was a valid address when it was written down.
///
/// One that will not parse now means the row was written by something other than this code, and
/// answering with a message addressed to fewer people than it says would be worse than refusing.
fn parse(value: &str) -> Result<EmailAddress, OutboxError> {
    EmailAddress::parse(value).map_err(|_| OutboxError::Unavailable)
}

fn stored(why: WhyHeld) -> &'static str {
    match why {
        WhyHeld::TheServerRefused => "theServerRefused",
        WhyHeld::SignInFailed => "signInFailed",
        WhyHeld::NobodyKnows => "nobodyKnows",
    }
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
                "lyonoyl-outbox-{name}-{}-{:?}",
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
                .expect("an account to send from");
            drop(connection);

            Self { dir, pool }
        }

        fn outbox(&self) -> SqliteOutbox {
            SqliteOutbox::new(self.pool.clone())
        }

        /// Everyone one waiting message goes to, as the database has it.
        fn recipients(&self, kind: &str) -> Vec<String> {
            let connection = self.pool.get().expect("connection");
            let mut statement = connection
                .prepare("SELECT address FROM outbox_recipients WHERE kind = ?1 ORDER BY address")
                .expect("readable");
            let rows = statement
                .query_map([kind], |row| row.get::<_, String>(0))
                .expect("rows");
            rows.map(|row| row.expect("row")).collect()
        }

        fn waiting(&self) -> i64 {
            self.pool
                .get()
                .expect("connection")
                .query_row("SELECT count(*) FROM outbox", [], |row| row.get(0))
                .expect("readable")
        }

        /// What one row says about where its message is.
        ///
        /// Read on whichever connection the pool hands out, which is most often the one the call
        /// before gave back — so it cannot tell a committed change from one still open there.
        fn state_of(&self, id: i64) -> String {
            state_on(&self.pool.get().expect("connection"), id)
        }

        /// Moves a row's clock, the way time passing would.
        fn changed(&self, statement: &str, id: i64) {
            self.pool
                .get()
                .expect("connection")
                .execute(statement, [id])
                .expect("the row is changed");
        }

        /// One message written down, and the number of its row.
        fn one_waiting(&self) -> i64 {
            let outbox = self.outbox();
            outbox
                .queue("acc-1", "<one@example.com>", &a_message())
                .expect("it is written");
            outbox.ready("acc-1", 10).expect("readable")[0].id
        }

        /// A message written down under `message_id` for `account_id` and carrying one file, and
        /// the number of its row.
        fn written(&self, account_id: &str, message_id: &str) -> i64 {
            let mut message = a_message();
            message.files = vec![a_file("invoice.pdf", "application/pdf", 78)];
            self.outbox()
                .queue(account_id, message_id, &message)
                .expect("it is written");
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

        /// One written down and taken by a server.
        fn sent(&self, account_id: &str, message_id: &str) -> i64 {
            let id = self.written(account_id, message_id);
            let outbox = self.outbox();
            outbox.claim(id).expect("taken");
            outbox.accepted(id).expect("taken by a server");
            id
        }

        /// How many rows a table holds, every message's together.
        fn rows_in(&self, table: &str) -> i64 {
            rows_on(&self.pool.get().expect("connection"), table)
        }
    }

    /// How many rows a table holds, as `connection` sees it.
    fn rows_on(connection: &Connection, table: &str) -> i64 {
        connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("readable")
    }

    impl Drop for TempProfile {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    /// What one row says about where its message is, as `connection` sees it.
    fn state_on(connection: &Connection, id: i64) -> String {
        connection
            .query_row("SELECT state FROM outbox WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .expect("readable")
    }

    fn address(value: &str) -> EmailAddress {
        EmailAddress::parse(value).expect("a valid address")
    }

    /// A file to attach, of a size a test can name.
    fn a_file(name: &str, kind: &str, size: usize) -> lyonoyl_domain::outgoing::OutgoingFile {
        lyonoyl_domain::outgoing::OutgoingFile {
            name: name.to_owned(),
            content_type: kind.to_owned(),
            // Distinct contents, so a read that returned the right count of the wrong rows is
            // caught: every byte says which file it came from.
            bytes: vec![name.as_bytes()[0]; size],
        }
    }

    fn a_message() -> Outgoing {
        Outgoing {
            files: Vec::new(),
            from: address("zeta@example.com"),
            to: vec![address("kappa@example.net")],
            cc: vec![address("sigma@example.org")],
            bcc: vec![address("records@example.com")],
            subject: Subject::parse("About Tuesday").expect("a subject"),
            body: "Shall we?".to_owned(),
            thread: Default::default(),
        }
    }

    #[test]
    fn a_message_is_committed_before_the_call_that_wrote_it_answers() {
        // What 0.7 claims is about a machine that stops the instant after somebody closed the
        // composer. The testable part of it is the order: a call that answers before its commit
        // leaves a message that a stop at that moment loses.
        //
        // A second connection is what asks, because it cannot see an uncommitted transaction on
        // another one. Seeing the row means the commit had finished before `queue` returned.
        let db = TempProfile::new("committed-first");

        db.outbox()
            .queue("acc-1", "<one@example.com>", &a_message())
            .expect("it is written down");

        let elsewhere = db.pool.get().expect("another connection");
        let (written, recipients): (i64, i64) = (
            elsewhere
                .query_row("SELECT count(*) FROM outbox", [], |row| row.get(0))
                .expect("readable"),
            elsewhere
                .query_row("SELECT count(*) FROM outbox_recipients", [], |row| {
                    row.get(0)
                })
                .expect("readable"),
        );

        assert_eq!(written, 1, "the message was not committed before answering");
        assert_eq!(
            recipients, 3,
            "half a message committed is a message sent to half of them"
        );
    }

    #[test]
    fn everyone_a_message_goes_to_is_written_down_with_it() {
        // A message stored with half its recipients is a message sent to half of them.
        let profile = TempProfile::new("whole");

        profile
            .outbox()
            .queue("acc-1", "<one@example.com>", &a_message())
            .expect("it is written");

        assert_eq!(
            profile.recipients("to"),
            vec!["kappa@example.net".to_owned()]
        );
        assert_eq!(
            profile.recipients("cc"),
            vec!["sigma@example.org".to_owned()]
        );
        assert_eq!(
            profile.recipients("bcc"),
            vec!["records@example.com".to_owned()],
            "a blind copy is somebody who receives the message"
        );
    }

    #[test]
    fn the_part_before_the_at_sign_is_stored_as_it_was_typed() {
        // It belongs to the receiving server. Lowercasing it is how a reply goes nowhere.
        let profile = TempProfile::new("case");
        let mut message = a_message();
        message.to = vec![address("Kappa.Vandel@example.net")];

        profile
            .outbox()
            .queue("acc-1", "<one@example.com>", &message)
            .expect("it is written");

        assert_eq!(
            profile.recipients("to"),
            vec!["Kappa.Vandel@example.net".to_owned()]
        );
    }

    #[test]
    fn the_same_message_twice_is_refused_rather_than_written_twice() {
        let profile = TempProfile::new("twice");
        let outbox = profile.outbox();

        outbox
            .queue("acc-1", "<one@example.com>", &a_message())
            .expect("it is written");

        assert_eq!(
            outbox.queue("acc-1", "<one@example.com>", &a_message()),
            Err(OutboxError::AlreadyWaiting)
        );
        assert_eq!(profile.waiting(), 1);
    }

    #[test]
    fn a_message_that_could_not_be_written_whole_is_not_written_at_all() {
        // The message and everybody it goes to are one change: the recipients are rows of their
        // own, and a commit that kept one and lost the other would send it to the wrong people.
        let profile = TempProfile::new("atomic");
        let mut message = a_message();
        message.from = address("nobody@example.com");

        profile
            .outbox()
            .queue("acc-1", "<one@example.com>", &message)
            .expect("it is written");

        let rows: i64 = profile
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT count(*) FROM outbox o JOIN outbox_recipients r ON r.outbox_id = o.id",
                [],
                |row| row.get(0),
            )
            .expect("readable");
        assert_eq!(
            rows, 3,
            "the message and its three recipients arrived together"
        );
    }

    #[test]
    fn a_sent_message_waits_for_the_settling_time_before_it_is_filed() {
        // Nothing had ever asked this query. It is the one that decides what goes into the sent
        // folder, and it reads the same columns as the query beside it -- which had been written
        // out a second time, and went on asking for one fewer than the reader expected.
        let profile = TempProfile::new("awaiting-filing");
        let outbox = profile.outbox();

        outbox
            .queue("acc-1", "<one@example.com>", &a_message())
            .expect("it is written");

        let waiting = outbox.ready("acc-1", 10).expect("readable");
        let queued = &waiting[0];

        // Still on its way out: a message nobody has sent is not a message to file.
        assert!(
            outbox
                .awaiting_filing("acc-1", Duration::ZERO, 10)
                .expect("readable")
                .is_empty(),
            "a message that has not been sent is waiting to be filed"
        );

        outbox.accepted(queued.id).expect("marked as sent");

        // Sent, but not long enough ago. A server that files its own copy is given this time, and
        // asking sooner leaves the person with two of everything they send.
        assert!(
            outbox
                .awaiting_filing("acc-1", Duration::from_secs(60), 10)
                .expect("readable")
                .is_empty(),
            "the copy was looked for before the server had time to file its own"
        );

        let due = outbox
            .awaiting_filing("acc-1", Duration::ZERO, 10)
            .expect("readable");
        assert_eq!(due.len(), 1, "the sent message is waiting to be filed");
        assert_eq!(due[0].message_id, "<one@example.com>");
        // And it comes back with the moment it was written down, which is the date both the
        // message that went out and the copy filed here are written under.
        assert_eq!(
            due[0].written_at, queued.written_at,
            "the copy would be filed under a different date from the one that was sent"
        );
        assert!(due[0].written_at > 0, "the row carries no date at all");

        // Filed once and not again: a second copy in the sent folder is the fault this guards.
        outbox.filed(due[0].id).expect("marked as filed");
        assert!(
            outbox
                .awaiting_filing("acc-1", Duration::ZERO, 10)
                .expect("readable")
                .is_empty(),
            "a message already filed is offered for filing again"
        );
    }

    #[test]
    fn a_message_is_taken_to_be_handed_over_once() {
        // Taken before a byte of it leaves, and only from waiting: the same row claimed a second
        // time is found already taken, and nothing is handed over twice.
        let profile = TempProfile::new("claim-once");
        let outbox = profile.outbox();
        let id = profile.one_waiting();

        assert_eq!(outbox.claim(id), Ok(true));
        assert_eq!(profile.state_of(id), "sending");
        assert_eq!(outbox.claim(id), Ok(false), "it was taken twice");
        assert!(
            outbox.ready("acc-1", 10).expect("readable").is_empty(),
            "a message being handed over is still offered as waiting"
        );
    }

    #[test]
    fn a_message_is_taken_on_the_disk_before_the_claim_answers() {
        // Read on a connection held out of the pool before the claim runs, so the claim cannot
        // have used it: a connection sees its own change before it is committed, and another one
        // sees only what was. A stop the instant after the claim answers finds the row saying the
        // message was on its way.
        let profile = TempProfile::new("claim-committed");
        let id = profile.one_waiting();
        let elsewhere = profile.pool.get().expect("a connection of its own");

        profile.outbox().claim(id).expect("taken");

        assert_eq!(state_on(&elsewhere, id), "sending");
    }

    #[test]
    fn a_message_that_is_not_waiting_cannot_be_taken() {
        let profile = TempProfile::new("claim-not-waiting");
        let outbox = profile.outbox();

        let held = profile.one_waiting();
        outbox
            .stopped(held, WhyHeld::SignInFailed, None)
            .expect("held for a person");
        assert_eq!(outbox.claim(held), Ok(false), "a held message was taken");

        outbox
            .queue("acc-1", "<two@example.com>", &a_message())
            .expect("it is written");
        let sent = outbox.ready("acc-1", 10).expect("readable")[0].id;
        outbox.claim(sent).expect("taken");
        outbox.accepted(sent).expect("taken by a server");
        assert_eq!(outbox.claim(sent), Ok(false), "a sent message was taken");

        assert_eq!(
            outbox.claim(9_999),
            Ok(false),
            "a message that is not there"
        );
    }

    #[test]
    fn a_message_taken_and_never_given_an_outcome_is_what_the_next_round_finds() {
        // The machine stopped while a server was taking it. Whole — recipients, files and all —
        // because the next round gives it back to the person who wrote it.
        let profile = TempProfile::new("interrupted");
        let outbox = profile.outbox();
        let mut message = a_message();
        message.files = vec![a_file("invoice.pdf", "application/pdf", 78)];
        outbox
            .queue("acc-1", "<one@example.com>", &message)
            .expect("it is written");
        let id = outbox.ready("acc-1", 10).expect("readable")[0].id;

        outbox.claim(id).expect("taken");

        let interrupted = outbox.interrupted("acc-1").expect("readable");
        assert_eq!(interrupted.len(), 1);
        assert_eq!(interrupted[0].id, id);
        assert_eq!(interrupted[0].message, message, "it came back changed");
        assert!(outbox.ready("acc-1", 10).expect("readable").is_empty());
        assert!(
            outbox.interrupted("acc-2").expect("readable").is_empty(),
            "one account's message was found on another"
        );

        outbox
            .stopped(id, WhyHeld::NobodyKnows, None)
            .expect("held for a person");
        assert!(outbox.interrupted("acc-1").expect("readable").is_empty());
        assert_eq!(profile.state_of(id), "held");
    }

    #[test]
    fn a_message_a_server_could_not_be_reached_for_goes_back_to_waiting() {
        let profile = TempProfile::new("claim-retry");
        let outbox = profile.outbox();
        let id = profile.one_waiting();

        outbox.claim(id).expect("taken");
        outbox
            .stopped(id, WhyHeld::TheServerRefused, Some(30))
            .expect("offered again later");

        assert_eq!(profile.state_of(id), "waiting");
        assert!(outbox.interrupted("acc-1").expect("readable").is_empty());
    }

    #[test]
    fn a_copy_is_looked_for_from_when_the_server_took_the_message() {
        // Written an hour ago and taken by a server a moment ago, after an hour without a network.
        // Counted from when it was written, the copy is looked for at once, before a server that
        // files its own has done it, and the person gets two.
        let profile = TempProfile::new("filing-from-acceptance");
        let outbox = profile.outbox();
        let id = profile.one_waiting();
        profile.changed(
            "UPDATE outbox SET written_at = unixepoch() - 3600 WHERE id = ?1",
            id,
        );

        outbox.claim(id).expect("taken");
        outbox.accepted(id).expect("taken by a server");

        assert!(
            outbox
                .awaiting_filing("acc-1", Duration::from_secs(60), 10)
                .expect("readable")
                .is_empty(),
            "the copy was looked for the moment the server took it"
        );

        profile.changed(
            "UPDATE outbox SET accepted_at = unixepoch() - 61 WHERE id = ?1",
            id,
        );
        assert_eq!(
            outbox
                .awaiting_filing("acc-1", Duration::from_secs(60), 10)
                .expect("readable")
                .len(),
            1,
            "a minute after the server took it, the copy is still not looked for"
        );
    }

    #[test]
    fn a_message_sent_by_a_build_that_did_not_say_when_is_counted_from_when_it_was_written() {
        // Nothing wrote the moment down, so the moment it was written is the only one there is.
        let profile = TempProfile::new("filing-fallback");
        let outbox = profile.outbox();
        let id = profile.one_waiting();
        profile.changed(
            "UPDATE outbox SET state = 'sent', accepted_at = NULL, written_at = unixepoch() - 3600
             WHERE id = ?1",
            id,
        );

        assert_eq!(
            outbox
                .awaiting_filing("acc-1", Duration::from_secs(60), 10)
                .expect("readable")
                .len(),
            1
        );
    }

    #[test]
    fn a_message_whose_copy_is_filed_leaves_nothing_of_itself_behind_once_read_back() {
        // Its copy is in the folder for sent mail, and once the account is read back that copy is
        // the message. Until then the row is what the list of sent mail shows; after it, who it
        // went to, blind copies included, and the bytes of what it carried would stay in the one
        // database nothing drops, read by nothing.
        let profile = TempProfile::new("filed-goes");
        let id = profile.sent("acc-1", "<one@example.com>");
        assert_eq!(
            (
                profile.rows_in("outbox"),
                profile.rows_in("outbox_recipients"),
                profile.rows_in("outbox_files")
            ),
            (1, 3, 1),
            "the message was not written down whole to begin with"
        );
        // Held out of the pool before the call, so the call cannot have used it: what it sees was
        // committed before `filed` answered.
        let elsewhere = profile.pool.get().expect("a connection of its own");

        profile.outbox().filed(id).expect("filed");
        assert_eq!(
            rows_on(&elsewhere, "outbox"),
            1,
            "a filed message went before the account was read back"
        );
        assert!(
            profile
                .outbox()
                .awaiting_filing("acc-1", Duration::ZERO, 10)
                .expect("readable")
                .is_empty(),
            "a filed message was offered for filing again"
        );

        profile.outbox().forget_filed("acc-1").expect("let go");

        for table in ["outbox", "outbox_recipients", "outbox_files"] {
            assert_eq!(
                rows_on(&elsewhere, table),
                0,
                "{table} still holds part of a message whose copy is filed and read back"
            );
        }
    }

    #[test]
    fn a_message_no_server_has_taken_is_never_let_go_as_filed() {
        // Waiting, being handed over, or held for a person: each exists here and nowhere else,
        // and a call meant for a sent one must not be able to lose it.
        let profile = TempProfile::new("filed-not-sent");
        let outbox = profile.outbox();
        let waiting = profile.written("acc-1", "<waiting@example.com>");
        let sending = profile.written("acc-1", "<sending@example.com>");
        outbox.claim(sending).expect("taken");
        let held = profile.written("acc-1", "<held@example.com>");
        outbox
            .stopped(held, WhyHeld::SignInFailed, None)
            .expect("held for a person");

        for id in [waiting, sending, held] {
            outbox.filed(id).expect("answered");
        }

        assert_eq!(
            profile.rows_in("outbox"),
            3,
            "a message no server has taken was let go"
        );
        assert_eq!(profile.rows_in("outbox_files"), 3);
        assert_eq!(profile.state_of(waiting), "waiting");
        assert_eq!(profile.state_of(sending), "sending");
        assert_eq!(profile.state_of(held), "held");
    }

    #[test]
    fn messages_a_profile_holds_as_filed_are_let_go_and_nothing_else_is() {
        // Rows whose copy is filed. They go whole, and nothing beside them goes: not a sent message
        // still waiting for its copy, not one on its way out or held, and not another account's.
        let profile = TempProfile::new("filed-swept");
        profile
            .pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                 VALUES ('acc-2', 'kappa@example.net', 'Kappa', 'imap', 2)",
                [],
            )
            .expect("a second account");
        let outbox = profile.outbox();

        let filed = profile.sent("acc-1", "<filed@example.com>");
        let elsewhere = profile.sent("acc-2", "<filed-elsewhere@example.com>");
        for id in [filed, elsewhere] {
            profile.changed("UPDATE outbox SET filed_at = unixepoch() WHERE id = ?1", id);
        }
        let unfiled = profile.sent("acc-1", "<unfiled@example.com>");
        let waiting = profile.written("acc-1", "<waiting@example.com>");
        let held = profile.written("acc-1", "<held@example.com>");
        outbox
            .stopped(held, WhyHeld::TheServerRefused, None)
            .expect("held for a person");

        outbox.forget_filed("acc-1").expect("let go");

        let left: Vec<i64> = {
            let connection = profile.pool.get().expect("connection");
            let mut statement = connection
                .prepare("SELECT id FROM outbox ORDER BY id")
                .expect("readable");
            let ids = statement
                .query_map([], |row| row.get(0))
                .expect("rows")
                .map(|row| row.expect("row"))
                .collect();
            ids
        };
        assert_eq!(left, vec![elsewhere, unfiled, waiting, held]);
        assert_eq!(
            (
                profile.rows_in("outbox_recipients"),
                profile.rows_in("outbox_files")
            ),
            (12, 4),
            "what a message carried did not go with it, or went with one that stays"
        );
    }

    #[test]
    fn what_reads_the_outbox_answers_the_same_once_filed_messages_are_let_go() {
        // Every query here reads messages waiting, on their way or waiting for their copy, and
        // none reads one whose copy is filed. Letting those go changes no answer but the one it is
        // about: the message just filed is not offered for filing again.
        let profile = TempProfile::new("filed-unseen");
        let outbox = profile.outbox();
        let filed_before = profile.sent("acc-1", "<filed-before@example.com>");
        profile.changed(
            "UPDATE outbox SET filed_at = unixepoch() WHERE id = ?1",
            filed_before,
        );
        let filing_now = profile.sent("acc-1", "<filing-now@example.com>");
        profile.sent("acc-1", "<unfiled@example.com>");
        let interrupted = profile.written("acc-1", "<interrupted@example.com>");
        outbox.claim(interrupted).expect("taken");
        profile.written("acc-1", "<waiting@example.com>");
        let held = profile.written("acc-1", "<held@example.com>");
        outbox
            .stopped(held, WhyHeld::NobodyKnows, None)
            .expect("held for a person");

        let answers = |outbox: &SqliteOutbox| {
            (
                outbox.ready("acc-1", 10).expect("readable"),
                outbox.interrupted("acc-1").expect("readable"),
                outbox
                    .awaiting_filing("acc-1", Duration::ZERO, 10)
                    .expect("readable"),
            )
        };
        let (ready, interrupted, mut awaiting) = answers(&outbox);
        assert_eq!(
            (ready.len(), interrupted.len(), awaiting.len()),
            (1, 1, 2),
            "the outbox was not set up the way this test reads it"
        );
        awaiting.retain(|message| message.id != filing_now);

        outbox.filed(filing_now).expect("filed");
        outbox.forget_filed("acc-1").expect("let go");

        assert_eq!(answers(&outbox), (ready, interrupted, awaiting));
    }

    #[test]
    fn a_copy_refused_for_now_waits_out_its_time_and_comes_back_counted() {
        // The count is what bounds how often a copy is asked about, and the wait is what keeps the
        // asking from being every round: both on the row, so a round, and a restart, read them
        // back.
        let profile = TempProfile::new("filing-refused");
        let outbox = profile.outbox();
        let id = profile.sent("acc-1", "<one@example.com>");
        let due = || {
            outbox
                .awaiting_filing("acc-1", Duration::ZERO, 10)
                .expect("readable")
        };
        assert_eq!(
            due()[0].copy_refused,
            0,
            "a copy nobody refused was counted"
        );

        for refused in 1..=2 {
            outbox.refused_for_now(id, 600).expect("counted");
            assert!(
                due().is_empty(),
                "a copy refused for now was offered again before its wait was over"
            );

            profile.changed(
                "UPDATE outbox SET file_after = unixepoch() WHERE id = ?1",
                id,
            );
            let again = due();
            assert_eq!(again.len(), 1, "a copy whose wait is over was not offered");
            assert_eq!(
                again[0].copy_refused, refused,
                "the refusals were not counted"
            );
        }

        assert_eq!(
            (
                profile.rows_in("outbox"),
                profile.rows_in("outbox_recipients"),
                profile.rows_in("outbox_files")
            ),
            (1, 3, 1),
            "a message whose copy may still be taken lost part of itself"
        );
    }

    #[test]
    fn a_copy_waiting_out_a_refusal_takes_no_place_from_the_ones_behind_it() {
        // Oldest first, and a round's worth at most: a copy that is not to be asked about yet is
        // not one of them, or a round's worth of refused copies would keep every later one out.
        let profile = TempProfile::new("filing-refused-first");
        let outbox = profile.outbox();
        let first = profile.sent("acc-1", "<first@example.com>");
        let second = profile.sent("acc-1", "<second@example.com>");

        outbox.refused_for_now(first, 600).expect("counted");

        let due: Vec<i64> = outbox
            .awaiting_filing("acc-1", Duration::ZERO, 1)
            .expect("readable")
            .iter()
            .map(|message| message.id)
            .collect();
        assert_eq!(due, vec![second]);
    }

    #[test]
    fn only_a_message_a_server_took_has_a_copy_to_count_as_refused() {
        // Waiting, being handed over or held for a person, it has no copy anywhere, and nothing
        // that counts refusals of one may write on its row.
        let profile = TempProfile::new("filing-refused-unsent");
        let outbox = profile.outbox();
        let waiting = profile.written("acc-1", "<waiting@example.com>");
        let sending = profile.written("acc-1", "<sending@example.com>");
        outbox.claim(sending).expect("taken");

        for id in [waiting, sending] {
            outbox.refused_for_now(id, 600).expect("answered");
        }

        let counted: i64 = profile
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT count(*) FROM outbox WHERE filing_refused > 0 OR file_after > 0",
                [],
                |row| row.get(0),
            )
            .expect("readable");
        assert_eq!(
            counted, 0,
            "a message no server has taken was counted as refused"
        );
        assert_eq!(profile.state_of(waiting), "waiting");
        assert_eq!(profile.state_of(sending), "sending");
    }

    #[test]
    fn a_message_gets_its_files_back_in_the_order_they_were_attached() {
        // Until this ran, nothing had ever put a file in the outbox: every fixture here carried
        // none, so the insert that writes them and the read that orders them were both unvisited.
        // A message that came back without its files would be sent without them, and the person
        // who received it would be the one to find out.
        let profile = TempProfile::new("files-come-back");
        let mut message = a_message();
        message.files = vec![
            a_file("invoice.pdf", "application/pdf", 78),
            a_file("photo.jpg", "image/jpeg", 331),
        ];

        profile
            .outbox()
            .queue("acc-1", "<one@example.com>", &message)
            .expect("it is written");

        let waiting = profile.outbox().ready("acc-1", 10).expect("readable");
        assert_eq!(waiting.len(), 1);

        let files = &waiting[0].message.files;
        assert_eq!(files.len(), 2, "both files came back");

        assert_eq!(files[0].name, "invoice.pdf");
        assert_eq!(files[0].content_type, "application/pdf");
        assert_eq!(files[0].bytes, vec![b'i'; 78]);

        assert_eq!(files[1].name, "photo.jpg");
        assert_eq!(files[1].content_type, "image/jpeg");
        assert_eq!(files[1].bytes, vec![b'p'; 331]);
    }

    #[test]
    fn a_message_whose_second_file_could_not_be_written_leaves_nothing_behind() {
        // The same change as the recipients, and for a sharper reason: a build that wrote the
        // message and then its files on a second connection would leave, after a stop between the
        // two, a message that goes out without its invoice.
        //
        // Counting rows after a write that succeeded cannot tell those two builds apart — both end
        // with everything present. What tells them apart is a write that fails partway, so the
        // database itself is made to refuse the second file: if the message survives that, the
        // files were never in its transaction.
        let profile = TempProfile::new("files-atomic");
        let mut message = a_message();
        message.files = vec![
            a_file("invoice.pdf", "application/pdf", 78),
            a_file("photo.jpg", "image/jpeg", 331),
        ];

        profile
            .pool
            .get()
            .expect("connection")
            .execute_batch(
                "CREATE TRIGGER refuse_the_second_file
                 BEFORE INSERT ON outbox_files WHEN NEW.position = 1
                 BEGIN SELECT RAISE(ABORT, 'the disk gave out'); END;",
            )
            .expect("the refusal is in place");

        let written = profile
            .outbox()
            .queue("acc-1", "<one@example.com>", &message);

        assert!(written.is_err(), "the write was allowed to half-happen");
        assert_eq!(profile.waiting(), 0, "the message outlived its files");

        let files: i64 = profile
            .pool
            .get()
            .expect("connection")
            .query_row("SELECT count(*) FROM outbox_files", [], |row| row.get(0))
            .expect("readable");
        assert_eq!(files, 0, "the first file outlived the message");
    }

    #[test]
    fn a_message_and_the_files_it_carries_arrive_together() {
        let profile = TempProfile::new("files-together");
        let mut message = a_message();
        message.files = vec![
            a_file("invoice.pdf", "application/pdf", 78),
            a_file("photo.jpg", "image/jpeg", 331),
        ];

        profile
            .outbox()
            .queue("acc-1", "<one@example.com>", &message)
            .expect("it is written");

        let rows: i64 = profile
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT count(*) FROM outbox o JOIN outbox_files f ON f.outbox_id = o.id",
                [],
                |row| row.get(0),
            )
            .expect("readable");
        assert_eq!(rows, 2, "the message and both its files arrived together");
    }

    #[test]
    fn a_message_taken_back_takes_its_files_with_it() {
        // The bytes of somebody's file outliving the message they attached it to is a copy of
        // their invoice in a database that is never dropped.
        let profile = TempProfile::new("files-removed");
        let mut message = a_message();
        message.files = vec![a_file("invoice.pdf", "application/pdf", 78)];

        profile
            .outbox()
            .queue("acc-1", "<one@example.com>", &message)
            .expect("it is written");

        let files_now = |profile: &TempProfile| -> i64 {
            profile
                .pool
                .get()
                .expect("connection")
                .query_row("SELECT count(*) FROM outbox_files", [], |row| row.get(0))
                .expect("readable")
        };
        assert_eq!(files_now(&profile), 1, "the file was written down");

        profile
            .pool
            .get()
            .expect("connection")
            .execute("DELETE FROM mail_accounts WHERE id = 'acc-1'", [])
            .expect("the account is removed");

        assert_eq!(profile.waiting(), 0);
        assert_eq!(
            files_now(&profile),
            0,
            "the bytes outlived the message they belonged to"
        );
    }

    #[test]
    fn removing_an_account_takes_its_unsent_messages_with_it() {
        let profile = TempProfile::new("removed");

        profile
            .outbox()
            .queue("acc-1", "<one@example.com>", &a_message())
            .expect("it is written");

        profile
            .pool
            .get()
            .expect("connection")
            .execute("DELETE FROM mail_accounts WHERE id = 'acc-1'", [])
            .expect("the account is removed");

        assert_eq!(profile.waiting(), 0);
        assert!(
            profile.recipients("to").is_empty(),
            "the addresses outlived the account they belonged to"
        );
    }

    #[test]
    fn messages_a_profile_holds_as_given_back_are_let_go_and_nothing_else_is() {
        // Rows that name the draft their message came back as, left held or on their way: that
        // draft holds everything in them, files included, and they go whole. A held row that names
        // no draft stays with the file it carried, because the file may be nowhere else; and so
        // does every message still to go, one sent, and another account's.
        let profile = TempProfile::new("given-back-swept");
        profile
            .pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                 VALUES ('acc-2', 'kappa@example.net', 'Kappa', 'imap', 2)",
                [],
            )
            .expect("a second account");
        let outbox = profile.outbox();

        let named_held = profile.written("acc-1", "<named-held@example.com>");
        let named_sending = profile.written("acc-1", "<named-sending@example.com>");
        outbox.claim(named_sending).expect("taken");
        let named_elsewhere = profile.written("acc-2", "<named-elsewhere@example.com>");
        for id in [named_held, named_sending, named_elsewhere] {
            profile.changed("UPDATE outbox SET came_back_as = 1 WHERE id = ?1", id);
        }
        let unnamed_held = profile.written("acc-1", "<unnamed-held@example.com>");
        for id in [named_held, unnamed_held] {
            outbox
                .stopped(id, WhyHeld::SignInFailed, None)
                .expect("held for a person");
        }
        let waiting = profile.written("acc-1", "<waiting@example.com>");
        let sent = profile.sent("acc-1", "<sent@example.com>");

        outbox.forget_given_back("acc-1").expect("let go");

        let left: Vec<i64> = {
            let connection = profile.pool.get().expect("connection");
            let mut statement = connection
                .prepare("SELECT id FROM outbox ORDER BY id")
                .expect("readable");
            let ids = statement
                .query_map([], |row| row.get(0))
                .expect("rows")
                .map(|row| row.expect("row"))
                .collect();
            ids
        };
        assert_eq!(left, vec![named_elsewhere, unnamed_held, waiting, sent]);
        assert_eq!(
            (
                profile.rows_in("outbox_recipients"),
                profile.rows_in("outbox_files")
            ),
            (12, 4),
            "what a message carried did not go with it, or went with one that stays"
        );
    }

    #[test]
    fn a_message_given_back_is_in_no_answer_and_nothing_done_about_it_fails_or_brings_it_back() {
        // Its row goes in the change that writes its draft. Whatever a round still does with the
        // number it read — says it stopped, takes it, says a server took it, files it — changes
        // nothing and fails nothing, and nothing that reads the outbox finds it.
        use crate::infrastructure::sqlite::drafts::SqliteDrafts;
        use crate::ports::drafts::Drafts;

        let profile = TempProfile::new("given-back-gone");
        let outbox = profile.outbox();
        let id = profile.written("acc-1", "<given-back@example.com>");
        outbox.claim(id).expect("taken");
        SqliteDrafts::new(profile.pool.clone())
            .came_back("acc-1", id, &Default::default())
            .expect("readable")
            .expect("it is back");

        let nothing_of_it = |outbox: &SqliteOutbox, after: &str| {
            assert!(
                outbox.ready("acc-1", 10).expect("readable").is_empty()
                    && outbox.interrupted("acc-1").expect("readable").is_empty()
                    && outbox
                        .awaiting_filing("acc-1", Duration::ZERO, 10)
                        .expect("readable")
                        .is_empty(),
                "a message given back is still offered, after {after}"
            );
            assert_eq!(
                (
                    profile.rows_in("outbox"),
                    profile.rows_in("outbox_recipients"),
                    profile.rows_in("outbox_files")
                ),
                (0, 0, 0),
                "the outbox holds part of a message the drafts hold, after {after}"
            );
        };
        nothing_of_it(&outbox, "it came back");

        assert_eq!(outbox.stopped(id, WhyHeld::NobodyKnows, None), Ok(()));
        assert_eq!(
            outbox.stopped(id, WhyHeld::TheServerRefused, Some(30)),
            Ok(())
        );
        assert_eq!(outbox.claim(id), Ok(false));
        assert_eq!(outbox.accepted(id), Ok(()));
        assert_eq!(outbox.filed(id), Ok(()));
        nothing_of_it(&outbox, "a round went on with the number it read");
    }
}
