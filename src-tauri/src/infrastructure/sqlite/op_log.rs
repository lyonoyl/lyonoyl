//! The operation log, in the authoritative database.
//!
//! Every `rusqlite` type stops here. What leaves is the domain's own vocabulary, and a message is
//! named the way the domain names it — there is no column here that could hold a position in a
//! mailbox, so there is nothing to map.

use std::time::Duration;

use rusqlite::{Transaction, TransactionBehavior};

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::ports::op_log::{
    OperationId, OperationLog, OperationLogError, OwedRead, RecordedOperation, WentTo,
};
use lyonoyl_domain::message::{Flags, FolderId, MessageId};
use lyonoyl_domain::operation::{
    DeleteIntent, MailOperation, OperationKind, OperationState, OperationTarget, Stopped,
};

/// How long an instruction the server has already heard is kept.
///
/// Long enough that "did that go through" has an answer the next day, short enough that the table
/// the list reads on every window does not grow for the life of the profile.
const KEPT_AFTER_DOING: Duration = Duration::from_secs(2 * 24 * 60 * 60);

/// How long a round holds the instructions it took.
///
/// Long enough that twenty commands to one server finish inside it, short enough that a round
/// killed halfway — the machine slept, the process was stopped — does not leave the work stuck
/// until somebody notices. Nothing is lost either way: the hold expires and the instruction is due
/// again.
const HELD_FOR: Duration = Duration::from_secs(300);

/// What `due` hands out.
///
/// `pending` only: something waiting to be found again cannot be carried out until a pass has
/// found it, and asking the server where it is is exactly the mistake this prevents.
///
/// Oldest first, because the order things were asked in is the order they make sense in: a message
/// filed and then flagged must not be flagged where it is not.
///
/// And only the head of each message's line: while anything asked earlier about the same message
/// is still waiting — for its own retry, or to be found again — what came after it waits too. Its
/// own wait having run out is not enough; a way back carried out before the move it undoes leaves
/// the message where the person took it from. The inner question is written word for word as the
/// partial index kept for it, which is what lets SQLite answer it from that index.
const DUE: &str = "SELECT id, message_id, folder_id, kind, flags_add, flags_remove, move_to,
            delete_intent, state, attempts, last_error, sent_at
     FROM op_log
     WHERE account_id = ?1 AND state = 'pending' AND try_after <= unixepoch()
       AND NOT EXISTS (
           SELECT 1 FROM op_log earlier
           WHERE earlier.account_id = op_log.account_id
             AND earlier.message_id = op_log.message_id
             AND earlier.id < op_log.id
             AND earlier.state IN ('pending', 'needs_reresolve'))
     ORDER BY id ASC
     LIMIT ?2";

/// Cloneable because it is a handle on a pool, not a connection: putting a message back builds a
/// second use case over the same queue.
#[derive(Clone)]
pub struct SqliteOperationLog {
    pool: SqlitePool,
}

impl SqliteOperationLog {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl OperationLog for SqliteOperationLog {
    fn outstanding_for(
        &self,
        account_id: &str,
        target: OperationTarget,
    ) -> Result<Vec<RecordedOperation>, OperationLogError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        let mut statement = connection
            .prepare(
                "SELECT id, message_id, folder_id, kind, flags_add, flags_remove, move_to,
                        delete_intent, state, attempts, last_error, sent_at
                 FROM op_log
                 WHERE account_id = ?1 AND message_id = ?2 AND folder_id = ?3
                   AND state IN ('pending', 'needs_reresolve') AND sent_at IS NULL
                 ORDER BY id ASC",
            )
            .map_err(|_| OperationLogError::Unavailable)?;

        // Read as plain values first and turned into domain types after: a row written by another
        // version may say something this build's rules refuse, and that is "unreadable", not "the
        // database is gone".
        let rows = statement
            .query_map(
                rusqlite::params![
                    account_id,
                    target.message.get() as i64,
                    target.folder.get() as i64
                ],
                read_row,
            )
            .map_err(|_| OperationLogError::Unavailable)?;

        let mut outstanding = Vec::new();

        for row in rows {
            let row = row.map_err(|_| OperationLogError::Unavailable)?;
            outstanding.push(recorded(row)?);
        }

        Ok(outstanding)
    }

    fn due(
        &self,
        account_id: &str,
        at_most: usize,
    ) -> Result<Vec<RecordedOperation>, OperationLogError> {
        let mut connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        // Taken, not read. Reading would let a second reader — another copy of this application,
        // or a round that started while the first was still talking — take the same instruction
        // and tell the server the same thing twice.
        //
        // What holds it is `try_after`: every row handed out here is pushed past now, so nothing
        // else sees it as due. A round that dies without finishing loses the hold when the lease
        // runs out, rather than leaving the work stuck for ever.
        let taking = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| OperationLogError::Unavailable)?;

        let mut statement = taking
            .prepare(DUE)
            .map_err(|_| OperationLogError::Unavailable)?;

        let rows = statement
            .query_map(rusqlite::params![account_id, at_most as i64], read_row)
            .map_err(|_| OperationLogError::Unavailable)?;

        let mut due = Vec::new();

        for row in rows {
            due.push(recorded(row.map_err(|_| OperationLogError::Unavailable)?)?);
        }

        drop(statement);

        for RecordedOperation { id, .. } in &due {
            taking
                .execute(
                    "UPDATE op_log SET try_after = unixepoch() + ?2 WHERE id = ?1",
                    rusqlite::params![id.0, HELD_FOR.as_secs() as i64],
                )
                .map_err(|_| OperationLogError::Unavailable)?;
        }

        // Instructions the server has heard are kept for a while and then forgotten. Kept, because
        // "did this go through" is a question worth being able to answer for a day or two;
        // forgotten, because every list window asks this table about the messages it is showing,
        // and a table that only grows makes that question slower every day a person uses the
        // application.
        taking
            .execute(
                // Not what is still owed a read: until a pass has read it back it is the only
                // thing saying where the message went, however long that takes.
                "DELETE FROM op_log
                 WHERE account_id = ?1 AND state = 'done' AND owes_a_read = 0
                   AND asked_at < unixepoch() - ?2",
                rusqlite::params![account_id, KEPT_AFTER_DOING.as_secs() as i64],
            )
            .map_err(|_| OperationLogError::Unavailable)?;

        taking
            .commit()
            .map_err(|_| OperationLogError::Unavailable)?;

        Ok(due)
    }

    /// Written before the server is told, and committed on its own.
    ///
    /// `sent_at` is set once and never moved: what matters is that this instruction has been out
    /// at all, not when it last was. One statement, so it and a person taking the instruction back
    /// cannot both win: either this finds it still waiting and it is on its way, or it finds
    /// nothing and the server is told nothing.
    fn about_to_tell(&self, id: OperationId) -> Result<(), OperationLogError> {
        let marked = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?
            .execute(
                "UPDATE op_log SET sent_at = COALESCE(sent_at, unixepoch())
                 WHERE id = ?1 AND state = 'pending'",
                rusqlite::params![id.0],
            )
            .map_err(|_| OperationLogError::Unavailable)?;

        match marked {
            1 => Ok(()),
            _ => Err(OperationLogError::ChangedUnderneath),
        }
    }

    fn attempted(
        &self,
        OperationId(id): OperationId,
        became: OperationState,
        stopped: Option<Stopped>,
        retry_after: Duration,
    ) -> Result<(), OperationLogError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        // The count goes up for every answer that was not what was asked for, so a message that
        // keeps failing waits longer each time; `try_after` only means anything while it is still
        // waiting.
        let changed = connection
            .execute(
                // The reason is cleared when there is none, so an instruction that stopped and was
                // then tried again does not go on showing why it stopped the last time.
                //
                // A move or a delete the server carried out owes a read: the index still has the
                // message where it was, and only a pass can say where it is now. How long it has
                // been owed is counted from now, the moment it was carried out.
                //
                // Waiting for this device's own read-back is not an attempt: nothing failed, and
                // counted as one it would read as late the moment it was written down.
                "UPDATE op_log
                 SET state = ?2, last_error = ?4,
                     attempts = attempts + (?2 <> 'needs_reresolve'),
                     try_after = unixepoch() + ?3,
                     owes_a_read = (?2 = 'done' AND kind IN ('move', 'delete')),
                     done_at = CASE WHEN ?2 = 'done' THEN unixepoch() END
                 WHERE id = ?1",
                rusqlite::params![
                    id,
                    stored_state(became),
                    retry_after.as_secs() as i64,
                    stopped.map(|reason| reason.as_str()),
                ],
            )
            .map_err(|_| OperationLogError::Unavailable)?;

        match changed {
            1 => Ok(()),
            // Taken back while it was being decided about without the server — only something
            // never sent can be — or the account went with its queue. There is no row left to
            // say anything on.
            _ => Err(OperationLogError::ChangedUnderneath),
        }
    }

    fn release(&self, ids: &[OperationId]) -> Result<(), OperationLogError> {
        if ids.is_empty() {
            return Ok(());
        }

        let mut connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        let releasing = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| OperationLogError::Unavailable)?;

        // The hold `due` put on them, and nothing else: the count, the reason and whether it was
        // ever sent are what they were before this round took them.
        for OperationId(id) in ids {
            releasing
                .execute(
                    "UPDATE op_log SET try_after = 0 WHERE id = ?1 AND state = 'pending'",
                    [id],
                )
                .map_err(|_| OperationLogError::Unavailable)?;
        }

        releasing
            .commit()
            .map_err(|_| OperationLogError::Unavailable)
    }

    fn cancel(
        &self,
        account_id: &str,
        target: OperationTarget,
    ) -> Result<usize, OperationLogError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        connection
            .execute(
                // Only what has not been sent. Something already carried out, or being carried
                // out right now, is undone by asking for the opposite; and something a person has
                // been shown is theirs to decide about — removing either here would be deciding
                // for them, and removing the first would lose the undo to the server that heard.
                "DELETE FROM op_log
                 WHERE account_id = ?1 AND message_id = ?2 AND folder_id = ?3
                   AND state IN ('pending', 'needs_reresolve') AND sent_at IS NULL",
                rusqlite::params![
                    account_id,
                    target.message.get() as i64,
                    target.folder.get() as i64
                ],
            )
            .map_err(|_| OperationLogError::Unavailable)
    }

    fn try_again(
        &self,
        account_id: &str,
        target: OperationTarget,
    ) -> Result<usize, OperationLogError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        connection
            .execute(
                // Only what stopped, and only about this message in this folder. The count and the
                // wait go back to nothing: asking again is a decision, not the next retry.
                "UPDATE op_log
                 SET state = 'pending', attempts = 0, try_after = 0, last_error = NULL
                 WHERE account_id = ?1 AND message_id = ?2 AND folder_id = ?3
                   AND state = 'conflicted'",
                rusqlite::params![
                    account_id,
                    target.message.get() as i64,
                    target.folder.get() as i64
                ],
            )
            .map_err(|_| OperationLogError::Unavailable)
    }

    fn settled_for(
        &self,
        account_id: &str,
        target: OperationTarget,
    ) -> Result<Option<RecordedOperation>, OperationLogError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        connection
            .query_row(
                // The newest, because a message may have been moved twice and only the last move
                // is the one a way back is about. One on its way to the server counts, because
                // taking it back is no longer possible: what it says may be happening now. One
                // that stopped counts too, so that it is what the caller sees rather than the
                // older instruction beneath it.
                "SELECT id, message_id, folder_id, kind, flags_add, flags_remove, move_to,
                        delete_intent, state, attempts, last_error, sent_at
                 FROM op_log
                 WHERE account_id = ?1 AND message_id = ?2 AND folder_id = ?3
                   AND (state IN ('done', 'conflicted')
                        OR (state IN ('pending', 'needs_reresolve') AND sent_at IS NOT NULL))
                 ORDER BY id DESC
                 LIMIT 1",
                rusqlite::params![
                    account_id,
                    target.message.get() as i64,
                    target.folder.get() as i64,
                ],
                read_row,
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(OperationLogError::Unavailable),
            })?
            .map(recorded)
            .transpose()
    }

    fn folder_reset(&self, account_id: &str, folder: FolderId) -> Result<usize, OperationLogError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        connection
            .execute(
                // Only what is still waiting. What has happened, happened; what a person has to
                // look at is already theirs to decide about.
                "UPDATE op_log
                 SET state = 'needs_reresolve'
                 WHERE account_id = ?1 AND folder_id = ?2 AND state = 'pending'",
                rusqlite::params![account_id, folder.get() as i64],
            )
            .map_err(|_| OperationLogError::Unavailable)
    }

    fn folder_found(&self, account_id: &str, folder: FolderId) -> Result<usize, OperationLogError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        connection
            .execute(
                // Back to the front of the queue: waiting for a pass is not a reason to wait
                // again. That holds for one sent back because its folder renumbered and for one
                // that was waiting for this device's own move to be read back alike — what each
                // was waiting for is what this pass has just done.
                "UPDATE op_log
                 SET state = 'pending', try_after = 0
                 WHERE account_id = ?1 AND folder_id = ?2 AND state = 'needs_reresolve'",
                rusqlite::params![account_id, folder.get() as i64],
            )
            .map_err(|_| OperationLogError::Unavailable)
    }

    fn owed_a_read(&self, account_id: &str) -> Result<Vec<OwedRead>, OperationLogError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        let mut statement = connection
            .prepare(
                "SELECT id, message_id, folder_id, kind, move_to, delete_intent,
                        COALESCE(done_at, asked_at) < unixepoch() - ?2
                 FROM op_log
                 WHERE account_id = ?1 AND owes_a_read = 1
                 ORDER BY id ASC",
            )
            .map_err(|_| OperationLogError::Unavailable)?;

        let rows = statement
            .query_map(
                rusqlite::params![account_id, KEPT_AFTER_DOING.as_secs() as i64],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, bool>(6)?,
                    ))
                },
            )
            .map_err(|_| OperationLogError::Unavailable)?;

        let mut owed = Vec::new();
        for row in rows {
            let (id, message, from, kind, move_to, intent, overdue) =
                row.map_err(|_| OperationLogError::Unavailable)?;
            let folder =
                |value: i64| FolderId::new(value as u64).map_err(|_| OperationLogError::Corrupted);

            owed.push(OwedRead {
                id: OperationId(id),
                message: MessageId::new(message as u64)
                    .map_err(|_| OperationLogError::Corrupted)?,
                overdue,
                from: folder(from)?,
                to: match (kind.as_str(), move_to, intent.as_deref()) {
                    ("move", Some(to), _) => WentTo::Folder(folder(to)?),
                    ("delete", _, Some("to_trash")) => WentTo::Trash,
                    ("delete", _, Some("permanently")) => WentTo::Nowhere,
                    _ => return Err(OperationLogError::Corrupted),
                },
            });
        }

        Ok(owed)
    }

    fn read_back(&self, ids: &[OperationId]) -> Result<usize, OperationLogError> {
        if ids.is_empty() {
            return Ok(0);
        }

        let mut connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        // One change, however many there are: a pass settles what it read in one go, and a crash
        // halfway would leave some of it laid over an index that already says the same thing.
        let settling = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| OperationLogError::Unavailable)?;

        let mut settled = 0;
        for OperationId(id) in ids {
            settled += settling
                .execute(
                    "UPDATE op_log SET owes_a_read = 0 WHERE id = ?1 AND owes_a_read = 1",
                    [id],
                )
                .map_err(|_| OperationLogError::Unavailable)?;
        }

        settling
            .commit()
            .map_err(|_| OperationLogError::Unavailable)?;

        Ok(settled)
    }

    fn owes_a_read(&self, account_id: &str, message: MessageId) -> Result<bool, OperationLogError> {
        self.pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM op_log
                                WHERE account_id = ?1 AND message_id = ?2 AND owes_a_read = 1)",
                rusqlite::params![account_id, message.get() as i64],
                |row| row.get(0),
            )
            .map_err(|_| OperationLogError::Unavailable)
    }

    fn still_named(&self, account_id: &str) -> Result<Vec<MessageId>, OperationLogError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        let mut statement = connection
            .prepare(
                "SELECT DISTINCT message_id FROM op_log
                 WHERE account_id = ?1
                   AND (state IN ('pending', 'needs_reresolve', 'conflicted') OR owes_a_read = 1)",
            )
            .map_err(|_| OperationLogError::Unavailable)?;

        let rows = statement
            .query_map([account_id], |row| row.get::<_, i64>(0))
            .map_err(|_| OperationLogError::Unavailable)?;

        let mut named = Vec::new();
        for row in rows {
            let message = row.map_err(|_| OperationLogError::Unavailable)?;
            named.push(MessageId::new(message as u64).map_err(|_| OperationLogError::Corrupted)?);
        }

        Ok(named)
    }

    fn folders_gone(
        &self,
        account_id: &str,
        still_there: &[FolderId],
    ) -> Result<usize, OperationLogError> {
        let mut connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        let putting_back = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| OperationLogError::Unavailable)?;

        // The folders the account still has, and what waits for one it has not. A handful of
        // folders against a handful of rows, read in Rust rather than handed to one statement as
        // a list of values of no fixed length.
        let waiting: Vec<(i64, i64)> = putting_back
            .prepare(
                "SELECT id, folder_id FROM op_log
                 WHERE account_id = ?1 AND state = 'needs_reresolve'",
            )
            .and_then(|mut statement| {
                statement
                    .query_map([account_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|_| OperationLogError::Unavailable)?;

        let mut put_back = 0;
        for (id, folder) in waiting {
            if still_there.iter().any(|there| there.get() as i64 == folder) {
                continue;
            }
            put_back += putting_back
                .execute(
                    "UPDATE op_log SET state = 'pending', try_after = 0
                     WHERE id = ?1 AND state = 'needs_reresolve'",
                    [id],
                )
                .map_err(|_| OperationLogError::Unavailable)?;
        }

        putting_back
            .commit()
            .map_err(|_| OperationLogError::Unavailable)?;

        Ok(put_back)
    }

    fn record(
        &self,
        account_id: &str,
        replacing: &[OperationId],
        operation: Option<MailOperation>,
    ) -> Result<u64, OperationLogError> {
        let mut connection = self
            .pool
            .get()
            .map_err(|_| OperationLogError::Unavailable)?;

        // `IMMEDIATE` because this will write: taking the write lock at the start is what stops two
        // writers from both reading, both deciding, and one of them losing (invariant 6).
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| OperationLogError::Unavailable)?;

        remove(&transaction, account_id, replacing)?;

        if let Some(operation) = operation {
            insert(&transaction, account_id, operation)?;
        }

        // Counted in the same transaction as the write, so no reading of the queue can see the
        // one without the other.
        let asked: i64 = transaction
            .query_row(
                "UPDATE asked_counter SET last = last + 1 WHERE id = 1 RETURNING last",
                [],
                |row| row.get(0),
            )
            .map_err(|_| OperationLogError::Unavailable)?;

        transaction
            .commit()
            .map_err(|_| OperationLogError::Unavailable)?;

        u64::try_from(asked).map_err(|_| OperationLogError::Corrupted)
    }
}

/// Removes what a fold replaced, and says so if any of it was not there to remove.
///
/// A row that has gone means somebody else has been here: what the caller folded described a
/// moment that has passed, and writing its conclusion would drop whatever they did.
fn remove(
    transaction: &Transaction<'_>,
    account_id: &str,
    replacing: &[OperationId],
) -> Result<(), OperationLogError> {
    for OperationId(id) in replacing {
        let gone = transaction
            .execute(
                // Still outstanding, still unsent, still this account's: an operation that
                // reached the server, or started to, while this was being decided is not one to
                // remove.
                "DELETE FROM op_log
                 WHERE id = ?1 AND account_id = ?2
                   AND state IN ('pending', 'needs_reresolve') AND sent_at IS NULL",
                rusqlite::params![id, account_id],
            )
            .map_err(|_| OperationLogError::Unavailable)?;

        if gone != 1 {
            return Err(OperationLogError::ChangedUnderneath);
        }
    }

    Ok(())
}

fn insert(
    transaction: &Transaction<'_>,
    account_id: &str,
    operation: MailOperation,
) -> Result<(), OperationLogError> {
    let (kind, add, remove, move_to, intent) = match operation.kind {
        OperationKind::SetFlags { add, remove } => (
            "set_flags",
            add.bits() as i64,
            remove.bits() as i64,
            None,
            None,
        ),
        OperationKind::MoveTo { folder } => ("move", 0, 0, Some(folder.get() as i64), None),
        OperationKind::Delete { intent } => (
            "delete",
            0,
            0,
            None,
            Some(match intent {
                DeleteIntent::ToTrash => "to_trash",
                DeleteIntent::Permanently => "permanently",
            }),
        ),
    };

    let outcome = transaction.execute(
        "INSERT INTO op_log
             (account_id, message_id, folder_id, kind, flags_add, flags_remove, move_to,
              delete_intent, state, asked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, unixepoch())",
        rusqlite::params![
            account_id,
            operation.target.message.get() as i64,
            operation.target.folder.get() as i64,
            kind,
            add,
            remove,
            move_to,
            intent,
            stored_state(operation.state),
        ],
    );

    match outcome {
        Ok(_) => Ok(()),
        // Only the index that allows one outstanding operation per target. Reaching *it* means
        // another writer put one there after this one read, which is the same answer as a row that
        // vanished: look again.
        //
        // Every other refusal is this build disagreeing with this database — a key naming an
        // account that is not here, a column holding what it will not hold, a value failing a
        // check — and calling that a race hides it twice over: the caller retries, the retry fails
        // the same way, and the person is told something went wrong inside the app. Which is how a
        // column left as a number by an edited migration cost an afternoon.
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            Err(OperationLogError::ChangedUnderneath)
        }
        Err(_) => Err(OperationLogError::Unavailable),
    }
}

/// The columns every query here selects, in the order `recorded` expects them.
///
/// One place, so two queries cannot drift into reading the same row differently.
fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
    ))
}

fn stored_state(state: OperationState) -> &'static str {
    match state {
        OperationState::Pending => "pending",
        OperationState::NeedsReresolve => "needs_reresolve",
        OperationState::Conflicted => "conflicted",
        OperationState::Done => "done",
    }
}

type Row = (
    i64,
    i64,
    i64,
    String,
    i64,
    i64,
    Option<i64>,
    Option<String>,
    String,
    i64,
    Option<String>,
    Option<i64>,
);

fn recorded(row: Row) -> Result<RecordedOperation, OperationLogError> {
    let (id, message, folder, kind, add, remove, move_to, intent, state, attempts, stopped, told) =
        row;

    let target = OperationTarget::new(
        MessageId::new(message as u64).map_err(|_| OperationLogError::Corrupted)?,
        FolderId::new(folder as u64).map_err(|_| OperationLogError::Corrupted)?,
    );

    let kind = match kind.as_str() {
        "set_flags" => OperationKind::SetFlags {
            add: Flags::from_bits(add as u32),
            remove: Flags::from_bits(remove as u32),
        },
        "move" => OperationKind::MoveTo {
            folder: FolderId::new(move_to.ok_or(OperationLogError::Corrupted)? as u64)
                .map_err(|_| OperationLogError::Corrupted)?,
        },
        "delete" => OperationKind::Delete {
            intent: match intent.as_deref() {
                Some("to_trash") => DeleteIntent::ToTrash,
                Some("permanently") => DeleteIntent::Permanently,
                _ => return Err(OperationLogError::Corrupted),
            },
        },
        _ => return Err(OperationLogError::Corrupted),
    };

    Ok(RecordedOperation {
        id: OperationId(id),
        operation: MailOperation {
            target,
            kind,
            state: match state.as_str() {
                "pending" => OperationState::Pending,
                "needs_reresolve" => OperationState::NeedsReresolve,
                "conflicted" => OperationState::Conflicted,
                "done" => OperationState::Done,
                _ => return Err(OperationLogError::Corrupted),
            },
        },
        attempts: attempts
            .try_into()
            .map_err(|_| OperationLogError::Corrupted)?,
        stopped: stopped.as_deref().map(Stopped::parse),
        already_told: told.is_some(),
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
                "lyonoyl-oplog-{name}-{}-{:?}",
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
                     VALUES ('acc-1', 'user@example.com', 'User', 'imap', 1)",
                    [],
                )
                .expect("an account to act on");
            drop(connection);

            Self { dir, pool }
        }

        fn log(&self) -> SqliteOperationLog {
            SqliteOperationLog::new(self.pool.clone())
        }
    }

    impl Drop for TempProfile {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn target(message: u64, folder: u64) -> OperationTarget {
        OperationTarget::new(
            MessageId::new(message).expect("non-zero"),
            FolderId::new(folder).expect("non-zero"),
        )
    }

    fn asked(target: OperationTarget, kind: OperationKind) -> MailOperation {
        MailOperation::asked(target, kind).expect("a change was asked for")
    }

    #[test]
    fn every_kind_of_instruction_comes_back_as_it_went_in() {
        let db = TempProfile::new("roundtrip");
        let log = db.log();

        let instructions = [
            asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN.with(Flags::FLAGGED),
                    remove: Flags::ANSWERED,
                },
            ),
            asked(
                target(2, 1),
                OperationKind::MoveTo {
                    folder: FolderId::new(9).expect("non-zero"),
                },
            ),
            asked(
                target(3, 1),
                OperationKind::Delete {
                    intent: DeleteIntent::Permanently,
                },
            ),
        ];

        for instruction in instructions {
            log.record("acc-1", &[], Some(instruction))
                .expect("it is written");

            let back = log
                .outstanding_for("acc-1", instruction.target)
                .expect("readable");

            assert_eq!(back.len(), 1);
            assert_eq!(back[0].operation, instruction);
            assert_eq!(back[0].attempts, 0);
        }
    }

    #[test]
    fn a_note_that_the_server_was_told_survives_being_read_back() {
        // The only run it exists for is the one that stopped before an outcome was written, so
        // holding it in memory would be holding it exactly where it is no use.
        let db = TempProfile::new("already-told");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        let before = log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable");
        assert_eq!(before.len(), 1);
        assert!(!before[0].already_told, "nothing has been told yet");

        log.about_to_tell(before[0].id).expect("the note is taken");

        let after = log
            .settled_for("acc-1", target(1, 1))
            .expect("readable")
            .expect("the one on its way is what a way back is built from");
        assert!(
            after.already_told,
            "the note did not survive being read back"
        );
    }

    #[test]
    fn what_a_way_back_is_built_from_is_the_newest_instruction_even_one_that_stopped() {
        // Opened, so marked read and carried out; then archived, and the server refused. The
        // archive is what a person pressing undo is taking back, and the mark as read beneath it
        // is not: answered with that, undo marks the message unread and says it worked.
        let db = TempProfile::new("way-back-stopped");
        let log = db.log();
        carried(
            &log,
            OperationKind::SetFlags {
                add: Flags::SEEN,
                remove: Flags::NONE,
            },
            target(1, 1),
        );
        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::MoveTo { folder: folder(2) },
            )),
        )
        .expect("it is written");
        let archived = log.due("acc-1", 10).expect("readable")[0].id;
        log.about_to_tell(archived).expect("on its way");
        log.attempted(
            archived,
            OperationState::Conflicted,
            Some(Stopped::TheServerRefused),
            Duration::ZERO,
        )
        .expect("written down");

        let newest = log
            .settled_for("acc-1", target(1, 1))
            .expect("readable")
            .expect("something was told");
        assert_eq!(newest.id, archived);
        assert_eq!(newest.operation.state, OperationState::Conflicted);
    }

    #[test]
    fn only_what_is_still_waiting_is_offered_to_fold_against() {
        let db = TempProfile::new("outstanding");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        db.pool
            .get()
            .expect("connection")
            .execute("UPDATE op_log SET state = 'done'", [])
            .expect("it reaches the server");

        assert!(
            log.outstanding_for("acc-1", target(1, 1))
                .expect("readable")
                .is_empty(),
            "something already carried out was offered for folding"
        );
    }

    #[test]
    fn what_a_person_has_to_look_at_is_not_offered_either() {
        let db = TempProfile::new("conflicted");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        db.pool
            .get()
            .expect("connection")
            .execute("UPDATE op_log SET state = 'conflicted'", [])
            .expect("it conflicts");

        assert!(
            log.outstanding_for("acc-1", target(1, 1))
                .expect("readable")
                .is_empty(),
            "a conflict a person has to see was offered for folding"
        );
    }

    #[test]
    fn removing_what_was_folded_and_writing_the_result_is_one_change() {
        let db = TempProfile::new("atomic");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        let first = log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable")[0]
            .id;

        log.record(
            "acc-1",
            &[first],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::NONE,
                    remove: Flags::SEEN,
                },
            )),
        )
        .expect("it is written");

        let waiting = log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable");

        assert_eq!(waiting.len(), 1, "one instruction, not two");
        assert_eq!(
            waiting[0].operation.kind,
            OperationKind::SetFlags {
                add: Flags::NONE,
                remove: Flags::SEEN
            }
        );
    }

    #[test]
    fn a_fold_of_something_that_has_since_gone_is_refused_rather_than_written() {
        // The caller folded a picture of a moment that has passed. Writing its conclusion would
        // drop whatever the other writer did.
        let db = TempProfile::new("vanished");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        let first = log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable")[0]
            .id;

        db.pool
            .get()
            .expect("connection")
            .execute("UPDATE op_log SET state = 'done'", [])
            .expect("it reaches the server meanwhile");

        let outcome = log.record(
            "acc-1",
            &[first],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::NONE,
                    remove: Flags::SEEN,
                },
            )),
        );

        assert_eq!(outcome, Err(OperationLogError::ChangedUnderneath));
    }

    #[test]
    fn nothing_is_left_behind_when_a_write_is_refused() {
        // The removal and the writing are one change: a refusal in the middle must leave the queue
        // exactly as it was, or a person's instruction disappears without anything failing.
        let db = TempProfile::new("rollback");
        let log = db.log();

        let first_kind = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };
        log.record("acc-1", &[], Some(asked(target(1, 1), first_kind)))
            .expect("it is written");

        let missing = OperationId(9_999);

        let outcome = log.record(
            "acc-1",
            &[missing],
            Some(asked(
                target(2, 1),
                OperationKind::SetFlags {
                    add: Flags::FLAGGED,
                    remove: Flags::NONE,
                },
            )),
        );

        assert_eq!(outcome, Err(OperationLogError::ChangedUnderneath));

        let waiting = log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable");
        assert_eq!(waiting.len(), 1, "the queue was left as it was");
        assert_eq!(waiting[0].operation.kind, first_kind);
        assert!(
            log.outstanding_for("acc-1", target(2, 1))
                .expect("readable")
                .is_empty(),
            "half of a refused change was written anyway"
        );
    }

    #[test]
    fn a_second_instruction_of_the_same_kind_appearing_underneath_is_refused() {
        // The index that allows one outstanding instruction per target. Reaching it means another
        // writer got there after this one read, which is the same answer: look again.
        let db = TempProfile::new("raced");
        let log = db.log();

        let kind = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };

        log.record("acc-1", &[], Some(asked(target(1, 1), kind)))
            .expect("it is written");

        let outcome = log.record("acc-1", &[], Some(asked(target(1, 1), kind)));

        assert_eq!(outcome, Err(OperationLogError::ChangedUnderneath));
    }

    #[test]
    fn every_write_answers_one_more_than_the_one_before_it_a_cancelling_one_too() {
        // What the window orders a change against a read of the counts by. Read and then unread
        // before the server hears either removes the one row and writes none, and a count taken
        // from the rows would hand the next write the number the first one had.
        let db = TempProfile::new("asked-counted");
        let log = db.log();
        let read = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };

        let first = log
            .record("acc-1", &[], Some(asked(target(1, 1), read)))
            .expect("it is written");
        let written = log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable")[0]
            .id;
        let cancelled = log
            .record("acc-1", &[written], None)
            .expect("the pair cancels out");
        let again = log
            .record("acc-1", &[], Some(asked(target(1, 1), read)))
            .expect("it is written again");

        assert_eq!((first, cancelled, again), (first, first + 1, first + 2));
    }

    #[test]
    fn one_account_never_reads_or_removes_another_ones_instructions() {
        let db = TempProfile::new("accounts");
        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                 VALUES ('acc-2', 'other@example.com', 'Other', 'imap', 2)",
                [],
            )
            .expect("a second account");
        let log = db.log();

        let kind = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };
        log.record("acc-1", &[], Some(asked(target(1, 1), kind)))
            .expect("it is written");

        assert!(
            log.outstanding_for("acc-2", target(1, 1))
                .expect("readable")
                .is_empty(),
            "one account read another's queue"
        );

        let theirs = log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable")[0]
            .id;

        assert_eq!(
            log.record("acc-2", &[theirs], None),
            Err(OperationLogError::ChangedUnderneath),
            "one account removed another's instruction"
        );
    }

    #[test]
    fn a_write_waits_for_another_writer_instead_of_failing_at_it() {
        // SQLite lets one writer at a time, and a background pass writes whenever mail arrives.
        // Without a busy timeout the second one is refused the moment it asks, so marking a
        // message read would fail with "the database is unavailable" purely because a pass
        // happened to be running — which is a failure a person cannot make sense of and cannot do
        // anything about.
        //
        // The timeout is set per connection, on every connection the pool hands out. This is what
        // says so: dropping it from the pragmas fails here.
        //
        // What this does *not* check is `BEGIN IMMEDIATE`. It cannot: nothing in this transaction
        // reads before it writes, so a deferred one takes the same lock at the same moment and
        // behaves identically. That invariant is held by a rule in `scripts/check-layering.sh`
        // instead, precisely because whether a transaction reads first is the kind of thing a
        // later edit changes without anyone noticing.
        let db = TempProfile::new("waits");

        let mut holder = db.pool.get().expect("connection");
        let held = holder
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("the other writer takes the lock");

        let pool = db.pool.clone();
        let writing = std::thread::spawn(move || {
            SqliteOperationLog::new(pool).record(
                "acc-1",
                &[],
                Some(asked(
                    target(1, 1),
                    OperationKind::SetFlags {
                        add: Flags::SEEN,
                        remove: Flags::NONE,
                    },
                )),
            )
        });

        // Long enough that the other writer is certainly inside the lock, short enough to stay
        // well under the five-second busy timeout.
        std::thread::sleep(std::time::Duration::from_millis(250));
        held.commit().expect("the other writer finishes");

        assert_eq!(
            writing
                .join()
                .expect("the writing thread did not panic")
                .map(|_| ()),
            Ok(()),
            "a write behind another writer was refused instead of waiting its turn"
        );

        assert_eq!(
            db.log()
                .outstanding_for("acc-1", target(1, 1))
                .expect("readable")
                .len(),
            1
        );
    }

    #[test]
    fn what_is_due_is_what_is_waiting_and_whose_turn_has_come() {
        let db = TempProfile::new("due");
        let log = db.log();
        let kind = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };

        log.record("acc-1", &[], Some(asked(target(1, 1), kind)))
            .expect("it is written");

        // Taken once: a second round does not get the same work, so the identifier comes from the
        // round that took it.
        let taken = log.due("acc-1", 10).expect("readable");
        assert_eq!(taken.len(), 1);

        // Told the server, could not be reached, waits a quarter of a minute.
        let waiting = taken[0].id;
        log.attempted(
            waiting,
            OperationState::Pending,
            None,
            Duration::from_secs(15),
        )
        .expect("recorded");

        assert!(
            log.due("acc-1", 10).expect("readable").is_empty(),
            "something that failed a moment ago was tried again at once"
        );
    }

    #[test]
    fn what_has_to_be_found_again_is_never_offered_to_the_server() {
        // Asking the server where a message is, when the folder has reset underneath it, is
        // exactly the mistake that must never happen.
        let db = TempProfile::new("reresolve");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        assert_eq!(
            log.folder_reset("acc-1", FolderId::new(1).expect("non-zero"))
                .expect("readable"),
            1
        );
        assert!(log.due("acc-1", 10).expect("readable").is_empty());
        // But it is still outstanding: it has to happen, once the message has been found.
        assert_eq!(
            log.outstanding_for("acc-1", target(1, 1))
                .expect("readable")
                .len(),
            1
        );
    }

    #[test]
    fn a_reset_leaves_other_folders_accounts_and_settled_work_alone() {
        let db = TempProfile::new("reset-scope");
        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                 VALUES ('acc-2', 'other@example.com', 'Other', 'imap', 2)",
                [],
            )
            .expect("a second account");
        let log = db.log();
        let kind = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };

        log.record("acc-1", &[], Some(asked(target(1, 1), kind)))
            .expect("the one that resets");
        log.record("acc-1", &[], Some(asked(target(2, 2), kind)))
            .expect("another folder");
        log.record("acc-2", &[], Some(asked(target(3, 1), kind)))
            .expect("another account");

        assert_eq!(
            log.folder_reset("acc-1", FolderId::new(1).expect("non-zero"))
                .expect("readable"),
            1,
            "a reset reached past the folder it happened in"
        );
        assert_eq!(log.due("acc-1", 10).expect("readable").len(), 1);
        assert_eq!(log.due("acc-2", 10).expect("readable").len(), 1);
    }

    #[test]
    fn a_reset_does_not_reopen_what_happened_or_bury_what_conflicted() {
        // Moving a finished instruction back into the queue tells the server to do it twice, and
        // moving a conflicted one there takes it off the screen of the person who has to decide
        // about it. A folder resetting says nothing about either.
        let db = TempProfile::new("reset-settled");
        let log = db.log();
        let kind = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };

        for message in [1, 2] {
            log.record("acc-1", &[], Some(asked(target(message, 1), kind)))
                .expect("it is written");
        }

        let waiting = log.due("acc-1", 10).expect("readable");
        log.attempted(waiting[0].id, OperationState::Done, None, Duration::ZERO)
            .expect("one reaches the server");
        log.attempted(
            waiting[1].id,
            OperationState::Conflicted,
            Some(Stopped::TheServerRefused),
            Duration::ZERO,
        )
        .expect("the other cannot be carried out");

        assert_eq!(
            log.folder_reset("acc-1", FolderId::new(1).expect("non-zero"))
                .expect("readable"),
            0,
            "a reset moved something that was already settled"
        );

        let states: Vec<String> = db
            .pool
            .get()
            .expect("connection")
            .prepare("SELECT state FROM op_log ORDER BY id")
            .expect("readable")
            .query_map([], |row| row.get(0))
            .expect("readable")
            .collect::<rusqlite::Result<Vec<_>>>()
            .expect("readable");

        assert_eq!(states, vec!["done".to_owned(), "conflicted".to_owned()]);
    }

    #[test]
    fn what_one_round_took_is_not_handed_to_the_next() {
        // Two rounds at once — a second copy of the application, or a round that started while
        // the first was still talking to the server — would otherwise both take the same
        // instruction and tell the server the same thing twice.
        let db = TempProfile::new("taken");
        let log = db.log();

        for message in 1..=3 {
            log.record(
                "acc-1",
                &[],
                Some(asked(
                    target(message, 1),
                    OperationKind::SetFlags {
                        add: Flags::SEEN,
                        remove: Flags::NONE,
                    },
                )),
            )
            .expect("it is written");
        }

        let first = log.due("acc-1", 10).expect("readable");
        let second = log.due("acc-1", 10).expect("readable");

        assert_eq!(first.len(), 3);
        assert!(
            second.is_empty(),
            "the same work was handed out twice: {second:?}"
        );
    }

    #[test]
    fn the_hold_is_long_enough_to_finish_and_short_enough_to_recover() {
        // The number itself is a judgement call and not worth pinning. What is worth pinning is
        // that it has both bounds: a hold shorter than a round means two rounds racing over the
        // same instruction, and one measured in days means a round killed by a sleeping laptop
        // leaves the work stuck until somebody notices.
        assert!(
            HELD_FOR >= Duration::from_secs(60),
            "a round of twenty commands would not finish inside {HELD_FOR:?}"
        );
        assert!(
            HELD_FOR <= Duration::from_secs(3600),
            "work nobody finished would sit for {HELD_FOR:?}"
        );
    }

    #[test]
    fn work_a_round_never_finished_becomes_due_again() {
        // The machine slept, the process was stopped. Nothing is lost: the hold runs out and the
        // instruction is due again rather than stuck until somebody notices.
        let db = TempProfile::new("hold-expires");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        assert_eq!(log.due("acc-1", 10).expect("readable").len(), 1);
        assert!(log.due("acc-1", 10).expect("readable").is_empty());

        // As if the hold had run out, without waiting five minutes for it.
        db.pool
            .get()
            .expect("connection")
            .execute("UPDATE op_log SET try_after = 0", [])
            .expect("the hold runs out");

        assert_eq!(
            log.due("acc-1", 10).expect("readable").len(),
            1,
            "work nobody finished stayed held for ever"
        );
    }

    #[test]
    fn the_order_things_were_asked_in_is_the_order_they_are_carried_out() {
        // A message filed and then flagged must not be flagged where it no longer is.
        let db = TempProfile::new("order");
        let log = db.log();

        for message in [3, 1, 2] {
            log.record(
                "acc-1",
                &[],
                Some(asked(
                    target(message, 1),
                    OperationKind::SetFlags {
                        add: Flags::SEEN,
                        remove: Flags::NONE,
                    },
                )),
            )
            .expect("it is written");
        }

        let due = log.due("acc-1", 10).expect("readable");
        let messages: Vec<u64> = due
            .iter()
            .map(|one| one.operation.target.message.get())
            .collect();

        assert_eq!(messages, vec![3, 1, 2], "carried out in some other order");
    }

    #[test]
    fn no_more_than_asked_for_is_handed_out_at_once() {
        let db = TempProfile::new("at-most");
        let log = db.log();

        for message in 1..=5 {
            log.record(
                "acc-1",
                &[],
                Some(asked(
                    target(message, 1),
                    OperationKind::SetFlags {
                        add: Flags::SEEN,
                        remove: Flags::NONE,
                    },
                )),
            )
            .expect("it is written");
        }

        assert_eq!(log.due("acc-1", 2).expect("readable").len(), 2);
    }

    #[test]
    fn what_the_server_did_is_written_down_and_stops_being_offered() {
        let db = TempProfile::new("carried");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        let one = log.due("acc-1", 10).expect("readable")[0].id;
        log.attempted(one, OperationState::Done, None, Duration::ZERO)
            .expect("recorded");

        assert!(log.due("acc-1", 10).expect("readable").is_empty());
        assert!(log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable")
            .is_empty());
    }

    #[test]
    fn every_attempt_is_counted_so_the_waits_can_grow() {
        let db = TempProfile::new("attempts");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        let one = log.due("acc-1", 10).expect("readable")[0].id;
        log.attempted(one, OperationState::Pending, None, Duration::ZERO)
            .expect("recorded");
        log.attempted(one, OperationState::Pending, None, Duration::ZERO)
            .expect("recorded");

        assert_eq!(log.due("acc-1", 10).expect("readable")[0].attempts, 2);
    }

    #[test]
    fn recording_an_outcome_on_something_that_has_gone_says_so() {
        // Folded away while the server was being told. What the server did stands; there is simply
        // no row left to say so on, and pretending otherwise would write a row nobody asked for.
        let db = TempProfile::new("vanished-outcome");
        let log = db.log();

        assert_eq!(
            log.attempted(
                OperationId(9_999),
                OperationState::Done,
                None,
                Duration::ZERO
            ),
            Err(OperationLogError::ChangedUnderneath)
        );
    }

    #[test]
    fn taking_back_an_instruction_leaves_nothing_of_it() {
        // What "undo" means before a server has heard: the instruction never happened.
        let db = TempProfile::new("cancel");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        assert_eq!(log.cancel("acc-1", target(1, 1)).expect("readable"), 1);
        assert!(log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable")
            .is_empty());
        assert!(log.due("acc-1", 10).expect("readable").is_empty());
    }

    #[test]
    fn what_the_server_already_did_is_not_taken_back_by_undoing() {
        // It happened. Undoing it means asking for the opposite, which is a new instruction — and
        // deleting the record of what was done would be the application forgetting it did it.
        let db = TempProfile::new("cancel-done");
        let log = db.log();
        let kind = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };

        log.record("acc-1", &[], Some(asked(target(1, 1), kind)))
            .expect("it is written");
        let one = log.due("acc-1", 10).expect("readable")[0].id;
        log.attempted(one, OperationState::Done, None, Duration::ZERO)
            .expect("it reaches the server");

        assert_eq!(log.cancel("acc-1", target(1, 1)).expect("readable"), 0);

        let left: i64 = db
            .pool
            .get()
            .expect("connection")
            .query_row("SELECT count(*) FROM op_log", [], |row| row.get(0))
            .expect("readable");

        assert_eq!(left, 1, "the record of what was done was removed");
    }

    #[test]
    fn what_a_person_has_been_shown_is_not_taken_back_behind_their_back() {
        // A conflict is theirs to decide about. Undo removing it would answer for them.
        let db = TempProfile::new("cancel-conflicted");
        let log = db.log();
        let kind = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };

        log.record("acc-1", &[], Some(asked(target(1, 1), kind)))
            .expect("it is written");
        let one = log.due("acc-1", 10).expect("readable")[0].id;
        log.attempted(
            one,
            OperationState::Conflicted,
            Some(Stopped::TheServerRefused),
            Duration::ZERO,
        )
        .expect("it stops");

        assert_eq!(log.cancel("acc-1", target(1, 1)).expect("readable"), 0);
    }

    #[test]
    fn undoing_one_message_leaves_the_others_alone() {
        let db = TempProfile::new("cancel-scope");
        let log = db.log();
        let kind = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };

        for (message, folder) in [(1, 1), (2, 1), (1, 2)] {
            log.record("acc-1", &[], Some(asked(target(message, folder), kind)))
                .expect("it is written");
        }

        assert_eq!(log.cancel("acc-1", target(1, 1)).expect("readable"), 1);
        assert_eq!(log.due("acc-1", 10).expect("readable").len(), 2);
    }

    #[test]
    fn asking_again_puts_a_stopped_instruction_back_at_the_front_of_the_queue() {
        // A person asking again is a new decision, not the next retry of an old one: whatever
        // stopped it may have been fixed in between, and making them wait an hour for the backoff
        // would be answering a decision with a timer.
        let db = TempProfile::new("try-again");
        let log = db.log();
        let kind = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };

        log.record("acc-1", &[], Some(asked(target(1, 1), kind)))
            .expect("it is written");
        let one = log.due("acc-1", 10).expect("readable")[0].id;
        log.attempted(
            one,
            OperationState::Conflicted,
            Some(Stopped::MessageIsNotThere),
            Duration::from_secs(3600),
        )
        .expect("it stops");

        assert!(log.due("acc-1", 10).expect("readable").is_empty());

        assert_eq!(
            log.try_again("acc-1", target(1, 1)).expect("readable"),
            1,
            "nothing was put back"
        );

        let waiting = log.due("acc-1", 10).expect("readable");

        assert_eq!(waiting.len(), 1, "it is due now rather than in an hour");
        assert_eq!(waiting[0].attempts, 0, "the count went back to nothing");
        assert_eq!(waiting[0].stopped, None, "it still says why it stopped");
    }

    #[test]
    fn asking_again_touches_only_what_stopped_about_that_message() {
        let db = TempProfile::new("try-again-scope");
        let log = db.log();
        let kind = OperationKind::SetFlags {
            add: Flags::SEEN,
            remove: Flags::NONE,
        };

        for (message, folder) in [(1, 1), (2, 1), (1, 2)] {
            log.record("acc-1", &[], Some(asked(target(message, folder), kind)))
                .expect("it is written");
        }
        for waiting in log.due("acc-1", 10).expect("readable") {
            log.attempted(
                waiting.id,
                OperationState::Conflicted,
                Some(Stopped::TheServerRefused),
                Duration::ZERO,
            )
            .expect("they stop");
        }

        assert_eq!(log.try_again("acc-1", target(1, 1)).expect("readable"), 1);
        assert_eq!(
            log.due("acc-1", 10).expect("readable").len(),
            1,
            "asking again about one message put others back too"
        );
    }

    #[test]
    fn asking_again_about_something_that_did_not_stop_changes_nothing() {
        // Not an error and not a second attempt: there is nothing to ask again about.
        let db = TempProfile::new("try-again-nothing");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        assert_eq!(log.try_again("acc-1", target(1, 1)).expect("readable"), 0);
        assert_eq!(log.due("acc-1", 10).expect("readable")[0].attempts, 0);
    }

    #[test]
    fn a_fold_that_cancelled_out_leaves_the_queue_empty() {
        let db = TempProfile::new("cancelled");
        let log = db.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        let first = log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable")[0]
            .id;

        log.record("acc-1", &[first], None)
            .expect("nothing to tell the server");

        assert!(log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable")
            .is_empty());
    }

    #[test]
    fn an_instruction_the_server_heard_long_ago_is_forgotten() {
        let profile = TempProfile::new("forgetting");
        let log = profile.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        profile
            .pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE op_log SET state = 'done', asked_at = unixepoch() - ?1",
                [(KEPT_AFTER_DOING.as_secs() + 60) as i64],
            )
            .expect("it is aged");

        // Taking the queue is the once-a-round moment this belongs in.
        log.due("acc-1", 20).expect("the queue is readable");

        assert_eq!(rows_in(&profile), 0);
    }

    #[test]
    fn an_instruction_the_server_heard_just_now_is_still_there() {
        let profile = TempProfile::new("keeping");
        let log = profile.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        profile
            .pool
            .get()
            .expect("connection")
            .execute("UPDATE op_log SET state = 'done'", [])
            .expect("it reaches the server");

        log.due("acc-1", 20).expect("the queue is readable");

        assert_eq!(
            rows_in(&profile),
            1,
            "and with it the answer to whether that went through"
        );
    }

    #[test]
    fn an_instruction_sent_back_to_be_found_is_carried_out_once_a_pass_has_found_it() {
        let profile = TempProfile::new("found-again");
        let log = profile.log();

        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            )),
        )
        .expect("it is written");

        assert_eq!(log.folder_reset("acc-1", folder(1)).expect("readable"), 1);
        assert!(
            log.due("acc-1", 20).expect("readable").is_empty(),
            "something waiting to be found again is not carried out"
        );

        assert_eq!(log.folder_found("acc-1", folder(1)).expect("readable"), 1);

        assert_eq!(
            log.due("acc-1", 20).expect("readable").len(),
            1,
            "and without this it waits for the life of the profile"
        );
    }

    #[test]
    fn a_pass_over_one_folder_does_not_release_another_folders_instructions() {
        let profile = TempProfile::new("found-elsewhere");
        let log = profile.log();

        for at in [1, 2] {
            log.record(
                "acc-1",
                &[],
                Some(asked(
                    target(at, at),
                    OperationKind::SetFlags {
                        add: Flags::SEEN,
                        remove: Flags::NONE,
                    },
                )),
            )
            .expect("it is written");
            log.folder_reset("acc-1", folder(at)).expect("readable");
        }

        log.folder_found("acc-1", folder(1)).expect("readable");

        let due = log.due("acc-1", 20).expect("readable");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].operation.target.folder, folder(1));
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

    /// One instruction, handed out and carried out the way a round does it.
    fn carried(log: &SqliteOperationLog, kind: OperationKind, on: OperationTarget) -> OperationId {
        log.record("acc-1", &[], Some(asked(on, kind)))
            .expect("it is written");
        let handed = log.due("acc-1", 10).expect("readable");
        let id = handed
            .iter()
            .find(|one| one.operation.target == on)
            .expect("handed out")
            .id;
        log.about_to_tell(id).expect("on its way");
        log.attempted(id, OperationState::Done, None, Duration::ZERO)
            .expect("written down");
        id
    }

    #[test]
    fn a_move_or_delete_the_server_carried_out_owes_a_read_and_a_flag_does_not() {
        // A flag is written onto the index the moment the server confirms it. A move cannot be:
        // the number the message has where it went is the server's to give, and only a pass reads
        // it.
        let db = TempProfile::new("owed");
        let log = db.log();

        carried(&log, read(), target(1, 1));
        let moved = carried(
            &log,
            OperationKind::MoveTo { folder: folder(2) },
            target(2, 1),
        );
        let binned = carried(
            &log,
            OperationKind::Delete {
                intent: DeleteIntent::ToTrash,
            },
            target(3, 1),
        );
        let erased = carried(
            &log,
            OperationKind::Delete {
                intent: DeleteIntent::Permanently,
            },
            target(4, 1),
        );

        assert_eq!(
            log.owed_a_read("acc-1").expect("readable"),
            vec![
                OwedRead {
                    id: moved,
                    message: MessageId::new(2).expect("non-zero"),
                    overdue: false,
                    from: folder(1),
                    to: WentTo::Folder(folder(2))
                },
                OwedRead {
                    id: binned,
                    message: MessageId::new(3).expect("non-zero"),
                    overdue: false,
                    from: folder(1),
                    to: WentTo::Trash
                },
                OwedRead {
                    id: erased,
                    message: MessageId::new(4).expect("non-zero"),
                    overdue: false,
                    from: folder(1),
                    to: WentTo::Nowhere
                },
            ]
        );
        assert!(log
            .owes_a_read("acc-1", MessageId::new(2).expect("non-zero"))
            .expect("readable"));
        assert!(!log
            .owes_a_read("acc-1", MessageId::new(1).expect("non-zero"))
            .expect("readable"));
    }

    #[test]
    fn what_is_waiting_or_stopped_owes_nothing() {
        let db = TempProfile::new("owed-only-done");
        let log = db.log();

        for (message, became) in [
            (1, OperationState::Pending),
            (2, OperationState::Conflicted),
            (3, OperationState::NeedsReresolve),
        ] {
            let on = target(message, 1);
            log.record(
                "acc-1",
                &[],
                Some(asked(on, OperationKind::MoveTo { folder: folder(2) })),
            )
            .expect("it is written");
            // Picked by what it is about rather than by where it comes in the answer: one put
            // back as waiting with no wait is due again at once, and taking the first would
            // attempt it a second time and never the one this round is about.
            let id = log
                .due("acc-1", 10)
                .expect("readable")
                .iter()
                .find(|one| one.operation.target == on)
                .expect("handed out")
                .id;
            log.attempted(id, became, None, Duration::from_secs(60))
                .expect("written down");

            assert!(
                log.owed_a_read("acc-1").expect("readable").is_empty(),
                "owed a read once it became {became:?}"
            );
        }
    }

    #[test]
    fn waiting_for_this_devices_own_read_back_is_not_counted_as_an_attempt() {
        // Archived and put back at once: the way back cannot find the message in the archive
        // until the pass reading the archive back has run, and waits for it. Nothing failed, and
        // counted as an attempt the foot of the window says a change is late the moment it is
        // written down -- about an undo that is going exactly as it should.
        let db = TempProfile::new("waiting-not-attempted");
        let log = db.log();
        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 2),
                OperationKind::MoveTo { folder: folder(1) },
            )),
        )
        .expect("it is written");
        let id = log.due("acc-1", 10).expect("readable")[0].id;

        log.attempted(id, OperationState::NeedsReresolve, None, Duration::ZERO)
            .expect("written down");
        log.folder_found("acc-1", folder(2)).expect("found");

        let back = log.due("acc-1", 10).expect("readable");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].attempts, 0);
    }

    #[test]
    fn a_move_is_overdue_counted_from_when_it_was_carried_out() {
        // Asked for on a weekend offline and carried out on Monday: no pass has read it back yet,
        // and settling it on the first read whatever the index says puts the message back in the
        // folder it was filed out of on a server that does not name what left.
        let db = TempProfile::new("overdue-from-done");
        let log = db.log();
        let id = carried(
            &log,
            OperationKind::MoveTo { folder: folder(2) },
            target(1, 1),
        );
        db.pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE op_log SET asked_at = unixepoch() - 3 * 24 * 60 * 60 WHERE id = ?1",
                [id.0],
            )
            .expect("asked for three days ago");

        let owed = log.owed_a_read("acc-1").expect("readable");
        assert_eq!(owed.len(), 1);
        assert!(!owed[0].overdue, "overdue the moment it was carried out");

        db.pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE op_log SET done_at = unixepoch() - 3 * 24 * 60 * 60 WHERE id = ?1",
                [id.0],
            )
            .expect("carried out three days ago");
        assert!(log.owed_a_read("acc-1").expect("readable")[0].overdue);
    }

    #[test]
    fn the_messages_the_queue_still_speaks_about_are_every_one_not_settled() {
        let db = TempProfile::new("still-named");
        let log = db.log();
        let moved = OperationKind::MoveTo { folder: folder(2) };

        // Waiting, stopped, and carried out and owed a read: three the queue still speaks about.
        log.record("acc-1", &[], Some(asked(target(1, 1), moved)))
            .expect("written");
        carried(&log, moved, target(2, 1));
        log.record("acc-1", &[], Some(asked(target(3, 1), moved)))
            .expect("written");
        let stopping = log
            .due("acc-1", 10)
            .expect("readable")
            .iter()
            .find(|one| one.operation.target == target(3, 1))
            .expect("handed out")
            .id;
        log.attempted(stopping, OperationState::Conflicted, None, Duration::ZERO)
            .expect("written down");
        // And one read back, which it no longer does.
        let settled = carried(&log, moved, target(4, 1));
        log.read_back(&[settled]).expect("settled");

        let mut named: Vec<u64> = log
            .still_named("acc-1")
            .expect("readable")
            .iter()
            .map(|message| message.get())
            .collect();
        named.sort_unstable();
        assert_eq!(named, vec![1, 2, 3]);
    }

    #[test]
    fn what_waits_for_a_folder_the_account_no_longer_has_is_put_back_in_the_queue() {
        // Moved into a folder another device then renamed: the way back waits for that folder to
        // be read, and no pass will ever read it. Everything asked after it about the same
        // message would wait behind it for ever.
        let db = TempProfile::new("folder-gone");
        let log = db.log();
        for (message, folder_id) in [(1, 5), (2, 1)] {
            log.record(
                "acc-1",
                &[],
                Some(asked(
                    target(message, folder_id),
                    OperationKind::MoveTo { folder: folder(3) },
                )),
            )
            .expect("written");
        }
        for one in log.due("acc-1", 10).expect("readable") {
            log.attempted(one.id, OperationState::NeedsReresolve, None, Duration::ZERO)
                .expect("written down");
        }

        let put_back = log
            .folders_gone("acc-1", &[folder(1), folder(3)])
            .expect("readable");

        assert_eq!(put_back, 1);
        let due = log.due("acc-1", 10).expect("readable");
        assert_eq!(
            due.iter()
                .map(|one| one.operation.target)
                .collect::<Vec<_>>(),
            vec![target(1, 5)],
            "only the one whose folder has gone, and not the one still waiting for its read"
        );
    }

    #[test]
    fn reading_back_settles_only_what_it_names() {
        let db = TempProfile::new("read-back");
        let log = db.log();

        let first = carried(
            &log,
            OperationKind::MoveTo { folder: folder(2) },
            target(1, 1),
        );
        let second = carried(
            &log,
            OperationKind::MoveTo { folder: folder(2) },
            target(2, 1),
        );

        assert_eq!(log.read_back(&[first]).expect("written"), 1);
        assert_eq!(
            log.owed_a_read("acc-1")
                .expect("readable")
                .iter()
                .map(|owed| owed.id)
                .collect::<Vec<_>>(),
            vec![second]
        );
        assert_eq!(
            log.read_back(&[first]).expect("written"),
            0,
            "settling twice settles nothing the second time"
        );
    }

    #[test]
    fn what_is_still_owed_a_read_is_not_forgotten_with_the_rest() {
        // Until a pass reads it back it is the only thing that says where the message went, and
        // two days of a folder nobody reads successfully is not a reason to lose that.
        let db = TempProfile::new("owed-kept");
        let log = db.log();

        carried(
            &log,
            OperationKind::MoveTo { folder: folder(2) },
            target(1, 1),
        );
        carried(&log, read(), target(2, 1));
        db.pool
            .get()
            .expect("connection")
            .execute("UPDATE op_log SET asked_at = 0", [])
            .expect("both long ago");

        log.due("acc-1", 10).expect("readable");

        assert_eq!(rows_in(&db), 1, "the flag is forgotten, the move is kept");
        assert_eq!(log.owed_a_read("acc-1").expect("readable").len(), 1);
    }

    #[test]
    fn an_instruction_being_told_is_not_taken_back_by_removing_it() {
        // The server may be carrying it out this moment. Removed, the undo would be lost the
        // second the server answered: nothing would be left to say it happened, and nothing would
        // ask for it back.
        let db = TempProfile::new("cancel-in-flight");
        let log = db.log();
        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::MoveTo { folder: folder(2) },
            )),
        )
        .expect("it is written");
        let id = log.due("acc-1", 10).expect("readable")[0].id;
        log.about_to_tell(id).expect("on its way");

        assert_eq!(log.cancel("acc-1", target(1, 1)).expect("readable"), 0);
        assert_eq!(rows_in(&db), 1);
        assert_eq!(
            log.settled_for("acc-1", target(1, 1))
                .expect("readable")
                .map(|one| one.id),
            Some(id),
            "and the way back is built from it instead"
        );
    }

    #[test]
    fn an_instruction_taken_back_before_it_was_told_is_not_told() {
        let db = TempProfile::new("told-after-cancel");
        let log = db.log();
        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 1),
                OperationKind::MoveTo { folder: folder(2) },
            )),
        )
        .expect("it is written");
        let id = log.due("acc-1", 10).expect("readable")[0].id;

        assert_eq!(log.cancel("acc-1", target(1, 1)).expect("readable"), 1);
        assert_eq!(
            log.about_to_tell(id),
            Err(OperationLogError::ChangedUnderneath)
        );
    }

    #[test]
    fn an_instruction_on_its_way_is_not_folded_into_the_next() {
        // Marked read and then unread before the push reached the server: folded, the two would
        // cancel out and nothing would be left to say "unread" -- while the server went on to set
        // the message read.
        let db = TempProfile::new("fold-in-flight");
        let log = db.log();
        log.record("acc-1", &[], Some(asked(target(1, 1), read())))
            .expect("it is written");
        let id = log.due("acc-1", 10).expect("readable")[0].id;
        log.about_to_tell(id).expect("on its way");

        assert!(log
            .outstanding_for("acc-1", target(1, 1))
            .expect("readable")
            .is_empty());
        assert_eq!(
            log.record("acc-1", &[id], None),
            Err(OperationLogError::ChangedUnderneath),
            "and nothing can remove it by folding"
        );
        log.record("acc-1", &[], Some(asked(target(1, 1), unread())))
            .expect("the opposite waits behind it");
        assert_eq!(rows_in(&db), 2);
    }

    /// One instruction handed out, told to the server, and answered "not now".
    fn deferred(log: &SqliteOperationLog, kind: OperationKind, on: OperationTarget) -> OperationId {
        log.record("acc-1", &[], Some(asked(on, kind)))
            .expect("it is written");
        let id = log
            .due("acc-1", 10)
            .expect("readable")
            .iter()
            .find(|one| one.operation.target == on)
            .expect("handed out")
            .id;
        log.about_to_tell(id).expect("on its way");
        log.attempted(id, OperationState::Pending, None, Duration::from_secs(60))
            .expect("written down");
        id
    }

    /// Its wait over, as if the time had passed.
    fn its_turn(db: &TempProfile, id: OperationId) {
        db.pool
            .get()
            .expect("connection")
            .execute("UPDATE op_log SET try_after = 0 WHERE id = ?1", [id.0])
            .expect("written");
    }

    #[test]
    fn what_was_asked_after_an_instruction_still_waiting_waits_behind_it() {
        // Read, the push failed on its way, then unread. Handed out as soon as its own wait is
        // over, the unread reaches the server first and the read lands on top of it: the person's
        // last word lost, on the server and in every count.
        let db = TempProfile::new("in-line");
        let log = db.log();
        let first = deferred(&log, read(), target(1, 1));
        log.record("acc-1", &[], Some(asked(target(1, 1), unread())))
            .expect("the opposite waits behind it");

        assert!(
            log.due("acc-1", 10).expect("readable").is_empty(),
            "the second was handed out while the first was still waiting"
        );

        its_turn(&db, first);
        let due = log.due("acc-1", 10).expect("readable");
        assert_eq!(
            due.iter().map(|one| one.id).collect::<Vec<_>>(),
            vec![first]
        );

        log.attempted(first, OperationState::Done, None, Duration::ZERO)
            .expect("written down");
        let due = log.due("acc-1", 10).expect("readable");
        assert_eq!(due.len(), 1);
        assert_eq!(
            due[0].operation.kind,
            unread(),
            "and then the one behind it"
        );
    }

    #[test]
    fn a_way_back_waits_behind_a_move_that_has_not_been_carried_out() {
        // Archived offline and put back: the way back names the archive, and carried out first it
        // finds nothing there and stops, while the archiving goes ahead later.
        let db = TempProfile::new("way-back-in-line");
        let log = db.log();
        let archived = deferred(
            &log,
            OperationKind::MoveTo { folder: folder(2) },
            target(1, 1),
        );
        log.record(
            "acc-1",
            &[],
            Some(asked(
                target(1, 2),
                OperationKind::MoveTo { folder: folder(1) },
            )),
        )
        .expect("the way back is written");

        its_turn(&db, archived);
        let due = log.due("acc-1", 10).expect("readable");
        assert_eq!(
            due.iter().map(|one| one.id).collect::<Vec<_>>(),
            vec![archived]
        );
    }

    #[test]
    fn one_message_waiting_holds_up_no_other() {
        let db = TempProfile::new("other-lines");
        let log = db.log();
        deferred(&log, read(), target(1, 1));
        log.record("acc-1", &[], Some(asked(target(2, 1), read())))
            .expect("it is written");

        let due = log.due("acc-1", 10).expect("readable");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].operation.target, target(2, 1));
    }

    #[test]
    fn what_a_round_gives_back_is_due_at_once_and_charged_nothing() {
        // The server could not be reached. Nothing was said about these, so they are exactly
        // what they were: unsent, uncounted, and free to be taken back.
        let db = TempProfile::new("released");
        let log = db.log();
        log.record("acc-1", &[], Some(asked(target(1, 1), read())))
            .expect("it is written");
        let id = log.due("acc-1", 10).expect("readable")[0].id;
        assert!(log.due("acc-1", 10).expect("readable").is_empty(), "held");

        log.release(&[id]).expect("given back");

        let due = log.due("acc-1", 10).expect("readable");
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].attempts, 0);
        assert!(!due[0].already_told);
        assert_eq!(log.cancel("acc-1", target(1, 1)).expect("readable"), 1);
    }

    #[test]
    fn giving_back_leaves_what_is_no_longer_waiting_alone() {
        let db = TempProfile::new("released-settled");
        let log = db.log();
        let id = carried(&log, read(), target(1, 1));

        log.release(&[id]).expect("given back");

        assert!(log.due("acc-1", 10).expect("readable").is_empty());
    }

    #[test]
    fn what_waits_in_line_is_found_by_the_index_kept_for_it() {
        // Asked for every row the queue hands out. Read by scanning the queue instead, a trash of
        // a thousand being emptied is a thousand scans of a thousand rows every round.
        let db = TempProfile::new("in-line-plan");
        let plan: Vec<String> = db
            .pool
            .get()
            .expect("connection")
            .prepare(&format!("EXPLAIN QUERY PLAN {DUE}"))
            .expect("the statement is valid")
            .query_map(rusqlite::params!["acc-1", 10], |row| {
                row.get::<_, String>(3)
            })
            .expect("a plan")
            .map(|row| row.expect("row"))
            .collect();

        assert!(
            plan.iter().any(|step| step.contains("idx_op_log_in_line")),
            "{plan:?}"
        );
    }

    fn folder(id: u64) -> FolderId {
        FolderId::new(id).expect("non-zero")
    }

    fn rows_in(profile: &TempProfile) -> i64 {
        profile
            .pool
            .get()
            .expect("connection")
            .query_row("SELECT count(*) FROM op_log", [], |row| row.get(0))
            .expect("readable")
    }
}
