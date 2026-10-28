// The composition root builds this once there is a provider to sync from. Until then the module
// is reached only by its tests.
#![allow(dead_code)]

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior};

use super::threads;
use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::infrastructure::sqlite::message_bodies::going_by;
use crate::ports::mail_index::{
    AppliedSummary, FolderRecord, FolderSyncState, MailIndex, MailIndexError, Reading, UidScope,
};
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{FlagScope, Flags, FolderId, MessageId, ModSeq, Uid, UidValidity};
use lyonoyl_domain::sync::{ArrivedMessage, FolderDelta, SyncCursor};

/// A folder row as it is stored: name, generation, cursor, how far back it has been read, whether
/// that is all of it, whether the last pass stopped short of its newest mail, and whether the
/// server refused the last reading of its new mail, and of older mail, for a moment.
type StoredFolderRow = (
    String,
    Option<i64>,
    Option<String>,
    Option<i64>,
    i64,
    i64,
    i64,
    i64,
);

/// A stored UID, or nothing when the row holds a value this build cannot represent.
fn stored_uid(value: i64) -> Option<Uid> {
    u32::try_from(value).ok().and_then(|uid| Uid::new(uid).ok())
}

/// How many UIDs are asked about in one statement. SQLite caps the number of bound parameters,
/// and a report may name more UIDs than that.
const LOOKUP_CHUNK: usize = 500;

/// Cloned freely: what it holds is a connection pool, which is a handle to the same pool.
#[derive(Clone)]
pub struct SqliteMailIndex {
    pool: SqlitePool,
}

impl SqliteMailIndex {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl MailIndex for SqliteMailIndex {
    fn record_window_floor(
        &self,
        folder_id: FolderId,
        floor: Uid,
        complete: bool,
    ) -> Result<(), MailIndexError> {
        self.pool
            .get()
            .map_err(|_| MailIndexError::Unavailable)?
            .execute(
                // The floor only ever moves down. A pass that reads a shallower range than an
                // earlier one has not forgotten what the earlier one read.
                "UPDATE folders
                 SET sync_window_floor_uid = min(COALESCE(sync_window_floor_uid, ?2), ?2),
                     full_history_synced = max(full_history_synced, ?3)
                 WHERE id = ?1",
                (
                    folder_id.get() as i64,
                    i64::from(floor.get()),
                    i64::from(complete),
                ),
            )
            .map(|_| ())
            .map_err(|_| MailIndexError::Unavailable)
    }

    /// One transaction: a listing is one answer from the server, and half of it stored is a folder
    /// tree that shows a mailbox the account does not have or hides one it does.
    fn record_folders(
        &self,
        account: i64,
        folders: &[FolderRecord],
    ) -> Result<Vec<(String, FolderId)>, MailIndexError> {
        let mut connection = self.pool.get().map_err(|_| MailIndexError::Unavailable)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| MailIndexError::Unavailable)?;

        let mut recorded = Vec::with_capacity(folders.len());
        for folder in folders {
            // The role is refreshed: a server may start declaring SPECIAL-USE for a mailbox it
            // previously described as an ordinary one. Nothing else about a stored folder is
            // touched, so its cursor and its generation survive.
            transaction
                .execute(
                    "INSERT INTO folders (account_id, name, role) VALUES (?1, ?2, ?3)
                     ON CONFLICT(account_id, name) DO UPDATE SET role = excluded.role",
                    (account, folder.name.as_str(), folder.role.stored()),
                )
                .map_err(|_| MailIndexError::Unavailable)?;

            let id: i64 = transaction
                .query_row(
                    "SELECT id FROM folders WHERE account_id = ?1 AND name = ?2",
                    (account, folder.name.as_str()),
                    |row| row.get(0),
                )
                .map_err(|_| MailIndexError::Unavailable)?;

            recorded.push((
                folder.name.clone(),
                u64::try_from(id)
                    .ok()
                    .and_then(|value| FolderId::new(value).ok())
                    .ok_or(MailIndexError::Corrupted)?,
            ));
        }

        transaction
            .commit()
            .map_err(|_| MailIndexError::Unavailable)?;

        Ok(recorded)
    }

    fn folder_state(&self, folder_id: FolderId) -> Result<FolderSyncState, MailIndexError> {
        let connection = self.pool.get().map_err(|_| MailIndexError::Unavailable)?;

        let row: Option<StoredFolderRow> = connection
            .query_row(
                "SELECT name, uid_validity, sync_cursor, sync_window_floor_uid, full_history_synced,
                        stopped_short, new_mail_refused_for_now, further_back_refused_for_now
                 FROM folders WHERE id = ?1",
                [folder_id.get() as i64],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| MailIndexError::Unavailable)?;

        let (name, uid_validity, cursor, floor, complete, stopped_short, new_mail, further_back) =
            row.ok_or(MailIndexError::UnknownFolder)?;

        let bounds: (Option<i64>, Option<i64>, Option<i64>) = connection
            .query_row(
                "SELECT min(uid), max(uid), max(modseq)
                 FROM message_placements WHERE folder_id = ?1",
                [folder_id.get() as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| MailIndexError::Unavailable)?;

        // What has actually been applied, so it cannot drift from what is stored.
        let highest_modseq = bounds
            .2
            .and_then(|value| u64::try_from(value).ok())
            .map(ModSeq::new);

        let known_window = match (bounds.0, bounds.1) {
            (Some(lowest), Some(highest)) => stored_uid(lowest)
                .zip(stored_uid(highest))
                .map(|(lowest, highest)| lowest..=highest),
            _ => None,
        };

        Ok(FolderSyncState {
            name,
            known_window,
            window_floor: floor.and_then(stored_uid),
            history_complete: complete != 0,
            highest_modseq,
            // A generation this build cannot represent is treated as "nothing stored", which makes
            // the next pass resync the folder instead of trusting an unreadable value.
            uid_validity: uid_validity
                .and_then(|value| u32::try_from(value).ok())
                .and_then(|value| UidValidity::new(value).ok()),
            cursor: cursor.map(SyncCursor::new),
            stopped_short: stopped_short != 0,
            new_mail_refused_for_now: new_mail != 0,
            further_back_refused_for_now: further_back != 0,
        })
    }

    fn inbox_of(&self, account: i64) -> Result<Option<String>, MailIndexError> {
        let connection = self.pool.get().map_err(|_| MailIndexError::Unavailable)?;

        connection
            .query_row(
                "SELECT name FROM folders WHERE account_id = ?1 AND role = ?2",
                (account, FolderRole::Inbox.stored()),
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| MailIndexError::Unavailable)
    }

    fn known_uids(
        &self,
        folder_id: FolderId,
        scope: &UidScope,
    ) -> Result<Vec<Uid>, MailIndexError> {
        let connection = self.pool.get().map_err(|_| MailIndexError::Unavailable)?;
        let folder = folder_id.get() as i64;
        let mut found = Vec::new();

        match scope {
            UidScope::Window(window) => {
                let mut statement = connection
                    .prepare(
                        "SELECT uid FROM message_placements
                         WHERE folder_id = ?1 AND uid BETWEEN ?2 AND ?3",
                    )
                    .map_err(|_| MailIndexError::Unavailable)?;
                let rows = statement
                    .query_map(
                        [
                            folder,
                            i64::from(window.start().get()),
                            i64::from(window.end().get()),
                        ],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|_| MailIndexError::Unavailable)?;
                collect_uids(rows, &mut found)?;
            }
            UidScope::Listed(uids) => {
                for chunk in uids.chunks(LOOKUP_CHUNK) {
                    let placeholders = (0..chunk.len())
                        .map(|position| format!("?{}", position + 2))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let sql = format!(
                        "SELECT uid FROM message_placements
                         WHERE folder_id = ?1 AND uid IN ({placeholders})"
                    );

                    let mut parameters: Vec<i64> = Vec::with_capacity(chunk.len() + 1);
                    parameters.push(folder);
                    parameters.extend(chunk.iter().map(|uid| i64::from(uid.get())));

                    let mut statement = connection
                        .prepare(&sql)
                        .map_err(|_| MailIndexError::Unavailable)?;
                    let rows = statement
                        .query_map(rusqlite::params_from_iter(parameters), |row| {
                            row.get::<_, i64>(0)
                        })
                        .map_err(|_| MailIndexError::Unavailable)?;
                    collect_uids(rows, &mut found)?;
                }
            }
        }

        Ok(found)
    }

    fn not_read_whole(
        &self,
        folder_id: FolderId,
        at_most: u32,
    ) -> Result<Vec<(MessageId, Uid)>, MailIndexError> {
        self.pool
            .get()
            .map_err(|_| MailIndexError::Unavailable)?
            .prepare(
                // What has not been read whole, rather than what has no stored body. The two part
                // company above the size the store keeps: such a message has no body here however
                // often it is read, and answering with it named it every round for ever.
                //
                // The size still bounds one thing, and it is the size the reading path already
                // refuses at rather than the size the store keeps at. Above that nothing can be
                // made of a message — it is not opened, and no text is taken out of it — so
                // asking for it buys nothing and costs the whole of it in memory. A message that
                // arrived without a size is read once and says its own from then on.
                "SELECT p.message_id, p.uid
                 FROM message_placements p
                 JOIN messages m ON m.id = p.message_id
                 WHERE p.folder_id = ?1
                   AND m.read_whole_at IS NULL
                   AND (m.rfc822_size IS NULL OR m.rfc822_size <= ?3)
                 ORDER BY p.received_at DESC, p.uid DESC
                 LIMIT ?2",
            )
            .map_err(|_| MailIndexError::Unavailable)?
            .query_map(
                rusqlite::params![
                    folder_id.get() as i64,
                    at_most,
                    crate::infrastructure::html::LARGEST_MESSAGE as i64
                ],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|_| MailIndexError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| MailIndexError::Unavailable)?
            .into_iter()
            .map(|(message, uid)| {
                Ok((
                    MessageId::new(message as u64).map_err(|_| MailIndexError::Unavailable)?,
                    Uid::new(uid as u32).map_err(|_| MailIndexError::Unavailable)?,
                ))
            })
            .collect()
    }

    fn placement_count(&self, folder_id: FolderId) -> Result<usize, MailIndexError> {
        let connection = self.pool.get().map_err(|_| MailIndexError::Unavailable)?;
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM message_placements WHERE folder_id = ?1",
                [folder_id.get() as i64],
                |row| row.get(0),
            )
            .map_err(|_| MailIndexError::Unavailable)?;

        usize::try_from(count).map_err(|_| MailIndexError::Corrupted)
    }

    fn apply(
        &self,
        delta: &FolderDelta,
        deletions: &[Uid],
        cursor: &SyncCursor,
        flag_scope: FlagScope,
    ) -> Result<AppliedSummary, MailIndexError> {
        let mut connection = self.pool.get().map_err(|_| MailIndexError::Unavailable)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| MailIndexError::Unavailable)?;

        let folder = delta.folder_id.get() as i64;
        let account = account_of(&transaction, folder)?;

        let mut summary = AppliedSummary::default();

        for arrival in &delta.arrived {
            let message_id = link_message(&transaction, account, arrival, flag_scope)?;
            place(&transaction, folder, arrival, message_id, flag_scope)?;
            summary.added += 1;
        }

        for update in &delta.flag_updates {
            summary.updated += apply_flags(
                &transaction,
                folder,
                update.uid,
                update.flags,
                update.modseq,
                flag_scope,
            )?;
        }

        for uid in deletions {
            summary.removed += remove_placement(&transaction, folder, *uid)?;
        }

        // Mail stored before conversations existed is given one a few thousand at a time, so an
        // upgrade never holds this transaction open for as long as a whole mailbox takes.
        threads::catch_up(&transaction, account)?;

        // A changed set moves the epoch. Flag changes count: a filtered list shows a different
        // set of messages after them.
        let set_changed = summary.added + summary.updated + summary.removed > 0;

        // The cursor is written last and inside the same transaction: stored ahead of the data it
        // describes, it would make the next pass skip changes that were never applied. Whether the
        // pass stopped short of the newest mail goes with it, because it is about that cursor.
        transaction
            .execute(
                "UPDATE folders
                 SET sync_cursor = ?2, uid_validity = ?3, view_epoch = view_epoch + ?4,
                     stopped_short = ?5
                 WHERE id = ?1",
                (
                    folder,
                    cursor.as_str(),
                    i64::from(delta.uid_validity.get()),
                    i64::from(set_changed),
                    i64::from(delta.stopped_short),
                ),
            )
            .map_err(|_| MailIndexError::Unavailable)?;

        transaction
            .commit()
            .map_err(|_| MailIndexError::Unavailable)?;

        Ok(summary)
    }

    fn reset_folder(
        &self,
        folder_id: FolderId,
        uid_validity: UidValidity,
    ) -> Result<(), MailIndexError> {
        let mut connection = self.pool.get().map_err(|_| MailIndexError::Unavailable)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| MailIndexError::Unavailable)?;

        let folder = folder_id.get() as i64;
        account_of(&transaction, folder)?;

        transaction
            .execute(
                "DELETE FROM message_placements WHERE folder_id = ?1",
                [folder],
            )
            .map_err(|_| MailIndexError::Unavailable)?;

        // How far back the folder was read is forgotten with its placements and its cursor, in the
        // same transaction. A floor and a "read to the beginning" kept across a renumbering
        // describe numbers the server has thrown away: the first pass after it would record its
        // own window under the old floor, the folder would read as complete, and everything older
        // than that window would never be read again. So does a reading refused for a moment: it
        // was refused over numbers that now name other mail.
        transaction
            .execute(
                "UPDATE folders
                 SET uid_validity = ?2, sync_cursor = NULL, sync_window_floor_uid = NULL,
                     full_history_synced = 0, stopped_short = 0, new_mail_refused_for_now = 0,
                     further_back_refused_for_now = 0, view_epoch = view_epoch + 1
                 WHERE id = ?1",
                (folder, i64::from(uid_validity.get())),
            )
            .map_err(|_| MailIndexError::Unavailable)?;

        transaction
            .commit()
            .map_err(|_| MailIndexError::Unavailable)
    }

    fn forget_it_stopped_short(&self, folder_id: FolderId) -> Result<(), MailIndexError> {
        self.pool
            .get()
            .map_err(|_| MailIndexError::Unavailable)?
            .execute(
                "UPDATE folders SET stopped_short = 0 WHERE id = ?1",
                [folder_id.get() as i64],
            )
            .map(|_| ())
            .map_err(|_| MailIndexError::Unavailable)
    }

    fn refused_for_now(
        &self,
        folder_id: FolderId,
        reading: Reading,
        refused: bool,
    ) -> Result<(), MailIndexError> {
        // A statement for each reading, written out whole: nothing here assembles one.
        let statement = match reading {
            Reading::NewMail => "UPDATE folders SET new_mail_refused_for_now = ?2 WHERE id = ?1",
            Reading::FurtherBack => {
                "UPDATE folders SET further_back_refused_for_now = ?2 WHERE id = ?1"
            }
        };

        self.pool
            .get()
            .map_err(|_| MailIndexError::Unavailable)?
            .execute(statement, (folder_id.get() as i64, i64::from(refused)))
            .map(|_| ())
            .map_err(|_| MailIndexError::Unavailable)
    }

    fn holds(&self, folder_id: FolderId, message: MessageId) -> Result<bool, MailIndexError> {
        let connection = self.pool.get().map_err(|_| MailIndexError::Unavailable)?;

        // Asked about what the queue names, and the queue may name a row that has since handed
        // its copies to the message it turned out to be: whether the folder still holds that
        // message is the answer.
        let message =
            going_by(&connection, message.get() as i64).map_err(|_| MailIndexError::Unavailable)?;

        connection
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM message_placements
                                WHERE folder_id = ?1 AND message_id = ?2)",
                (folder_id.get() as i64, message),
                |row| row.get(0),
            )
            .map_err(|_| MailIndexError::Unavailable)
    }

    fn unplaced(&self, account: i64) -> Result<Vec<MessageId>, MailIndexError> {
        let connection = self.pool.get().map_err(|_| MailIndexError::Unavailable)?;

        unplaced_in(&connection, account)?
            .into_iter()
            .map(|id| {
                u64::try_from(id)
                    .ok()
                    .and_then(|value| MessageId::new(value).ok())
                    .ok_or(MailIndexError::Corrupted)
            })
            .collect()
    }

    fn forget_the_unplaced(&self, account: i64, named: &[MessageId]) -> Result<(), MailIndexError> {
        let mut connection = self.pool.get().map_err(|_| MailIndexError::Unavailable)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| MailIndexError::Unavailable)?;

        drop_orphans(&transaction, account, named)?;
        threads::forget_empty_conversations(&transaction, account)?;

        transaction
            .commit()
            .map_err(|_| MailIndexError::Unavailable)
    }
}

fn collect_uids<I>(rows: I, into: &mut Vec<Uid>) -> Result<(), MailIndexError>
where
    I: Iterator<Item = rusqlite::Result<i64>>,
{
    for row in rows {
        let stored = row.map_err(|_| MailIndexError::Unavailable)?;
        let uid = u32::try_from(stored)
            .ok()
            .and_then(|value| Uid::new(value).ok())
            .ok_or(MailIndexError::Corrupted)?;
        into.push(uid);
    }
    Ok(())
}

fn account_of(transaction: &Transaction<'_>, folder: i64) -> Result<i64, MailIndexError> {
    transaction
        .query_row(
            "SELECT account_id FROM folders WHERE id = ?1",
            [folder],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| MailIndexError::Unavailable)?
        .ok_or(MailIndexError::UnknownFolder)
}

/// Finds the message this arrival belongs to, or stores a new one.
///
/// The same message reaches several folders as a copy or a label. When the provider gives a
/// `Message-ID`, an existing row with that header in the same account is reused, so one message
/// keeps one identity across folders.
fn link_message(
    transaction: &Transaction<'_>,
    account: i64,
    arrival: &ArrivedMessage,
    flag_scope: FlagScope,
) -> Result<i64, MailIndexError> {
    let header = arrival
        .envelope
        .rfc822_message_id
        .as_ref()
        .map(|value| value.as_str().to_owned());

    if let Some(header) = header.as_deref() {
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT id FROM messages WHERE account_id = ?1 AND rfc822_message_id = ?2",
                (account, header),
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| MailIndexError::Unavailable)?;

        if let Some(id) = existing {
            if flag_scope == FlagScope::Message {
                set_message_flags(transaction, id, arrival.envelope.flags)?;
            }
            fill_missing_preview(transaction, id, arrival.envelope.preview.as_deref())?;
            return Ok(id);
        }
    }

    let canonical_flags = match flag_scope {
        FlagScope::Message => arrival.envelope.flags,
        FlagScope::Placement => Flags::NONE,
    };

    transaction
        .execute(
            "INSERT INTO messages
                 (account_id, rfc822_message_id, subject, sender, sender_address, recipients_to,
                  recipients_cc, received_at, rfc822_size, preview, references_chain, flags)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            (
                account,
                header,
                arrival.envelope.subject.as_deref(),
                arrival.envelope.sender.as_deref(),
                arrival.envelope.sender_address.as_deref(),
                joined(&arrival.envelope.to),
                joined(&arrival.envelope.cc),
                arrival.envelope.received_at,
                arrival.envelope.rfc822_size.map(i64::from),
                arrival.envelope.preview.as_deref(),
                // What the message said about the conversation it is in.
                joined(
                    &arrival
                        .envelope
                        .references
                        .iter()
                        .map(|name| name.as_str().to_owned())
                        .collect::<Vec<_>>(),
                ),
                i64::from(canonical_flags.bits()),
            ),
        )
        .map_err(|_| MailIndexError::Unavailable)?;

    // In the same transaction as the message itself: a row a list can show without a conversation
    // is a row the thread view has nowhere to put.
    let id = transaction.last_insert_rowid();
    threads::thread_the_message(
        transaction,
        account,
        id,
        arrival.envelope.rfc822_message_id.as_ref(),
        &arrival.envelope.references,
    )?;

    Ok(id)
}

/// A list of names or addresses as one column, or nothing when the list is empty.
///
/// A newline between entries, which can occur in neither a `Message-ID` nor an address, so the
/// separator cannot appear inside an entry and splitting is exact. `None` rather than an empty
/// string: a message that named nobody and a message read before the column existed are the same
/// thing to everything above this, and one of them cannot be repaired.
fn joined(entries: &[String]) -> Option<String> {
    (!entries.is_empty()).then(|| entries.join("\n"))
}

/// Gives a message an opening line if it has none yet.
///
/// A message stored by a build that did not read bodies, or by a pass that took only its envelope,
/// is the same message. Never an overwrite: what is already there was read from the message itself,
/// and a later pass sees no more of it than the first one did.
fn fill_missing_preview(
    transaction: &Transaction<'_>,
    message_id: i64,
    preview: Option<&str>,
) -> Result<(), MailIndexError> {
    let Some(preview) = preview.filter(|text| !text.is_empty()) else {
        return Ok(());
    };

    transaction
        .execute(
            "UPDATE messages SET preview = ?2 WHERE id = ?1 AND preview IS NULL",
            (message_id, preview),
        )
        .map(|_| ())
        .map_err(|_| MailIndexError::Unavailable)
}

fn place(
    transaction: &Transaction<'_>,
    folder: i64,
    arrival: &ArrivedMessage,
    message_id: i64,
    flag_scope: FlagScope,
) -> Result<(), MailIndexError> {
    // Which of the two places a provider owns no longer changes what is written here: a copy
    // carries its own flags either way, and where a *change* is written is decided when the change
    // arrives.
    let _ = flag_scope;

    transaction
        .execute(
            "INSERT INTO message_placements
                 (folder_id, uid, message_id, modseq, flags, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(folder_id, uid) DO UPDATE SET
                 message_id = excluded.message_id,
                 modseq = excluded.modseq,
                 flags = excluded.flags,
                 received_at = excluded.received_at",
            (
                folder,
                i64::from(arrival.uid.get()),
                message_id,
                arrival.modseq.map(|modseq| modseq.get() as i64),
                // What this copy is, whoever the provider thinks owns the flag. Answering "is this
                // copy unread" from the copy is what keeps the count off the messages table.
                i64::from(arrival.envelope.flags.bits()),
                arrival.envelope.received_at,
            ),
        )
        .map_err(|_| MailIndexError::Unavailable)?;

    Ok(())
}

/// Writes changed flags to whichever of the two places the provider owns.
fn apply_flags(
    transaction: &Transaction<'_>,
    folder: i64,
    uid: Uid,
    flags: Flags,
    modseq: Option<lyonoyl_domain::message::ModSeq>,
    flag_scope: FlagScope,
) -> Result<usize, MailIndexError> {
    let uid = i64::from(uid.get());

    // Compared against what is stored, so a pass that re-reads flags nobody changed writes nothing
    // and reports nothing. Counting an unchanged flag as a change moves the folder's epoch, and the
    // list re-anchors under the reader on every pass over a quiet mailbox.
    let changed = match flag_scope {
        FlagScope::Placement => transaction
            .execute(
                "UPDATE message_placements SET flags = ?3, modseq = ?4
                 WHERE folder_id = ?1 AND uid = ?2 AND flags <> ?3",
                (
                    folder,
                    uid,
                    i64::from(flags.bits()),
                    modseq.map(|modseq| modseq.get() as i64),
                ),
            )
            .map_err(|_| MailIndexError::Unavailable)?,
        // A provider that keeps flags per message says one thing about every copy of it, so every
        // copy is written. That is one to three rows, and it is what lets a window and a count read
        // the copy alone instead of looking the message up on every row.
        FlagScope::Message => {
            transaction
                .execute(
                    "UPDATE messages SET flags = ?3
                     WHERE id = (SELECT message_id FROM message_placements
                                 WHERE folder_id = ?1 AND uid = ?2)
                       AND flags <> ?3",
                    (folder, uid, i64::from(flags.bits())),
                )
                .map_err(|_| MailIndexError::Unavailable)?;

            transaction
                .execute(
                    "UPDATE message_placements SET flags = ?3
                     WHERE message_id = (SELECT message_id FROM message_placements
                                         WHERE folder_id = ?1 AND uid = ?2)
                       AND flags <> ?3",
                    (folder, uid, i64::from(flags.bits())),
                )
                .map_err(|_| MailIndexError::Unavailable)?
        }
    };

    Ok(changed)
}

fn set_message_flags(
    transaction: &Transaction<'_>,
    message_id: i64,
    flags: Flags,
) -> Result<(), MailIndexError> {
    transaction
        .execute(
            "UPDATE messages SET flags = ?2 WHERE id = ?1",
            (message_id, i64::from(flags.bits())),
        )
        .map(|_| ())
        .map_err(|_| MailIndexError::Unavailable)
}

/// Removes one placement. A message left with none is forgotten by `forget_the_unplaced`, once the
/// pass has been through every folder.
fn remove_placement(
    transaction: &Transaction<'_>,
    folder: i64,
    uid: Uid,
) -> Result<usize, MailIndexError> {
    transaction
        .execute(
            "DELETE FROM message_placements WHERE folder_id = ?1 AND uid = ?2",
            (folder, i64::from(uid.get())),
        )
        .map_err(|_| MailIndexError::Unavailable)
}

/// Removes messages of an account that no longer sit in any folder, except those in `named` and
/// those a row in `named` handed its copies to.
///
/// The ones no folder holds are found first and removed one by one, because the queue may name
/// thousands — an emptied trash is one instruction per message — and a statement cannot be handed
/// that many values to leave out. Found this way they are the few a pass has just taken the last
/// copy of.
///
/// A row that handed its copies over is named by the queue in place of the message that took
/// them: what was asked about the row is carried out on that message's copies, and finds them
/// again under its name when the folder they went to is read. With only the row kept, that message
/// is forgotten while that folder fails, comes back a stranger when it is read, and what was asked
/// finds nothing.
fn drop_orphans(
    transaction: &Transaction<'_>,
    account: i64,
    named: &[MessageId],
) -> Result<(), MailIndexError> {
    let unplaced = unplaced_in(transaction, account)?;

    let named: std::collections::HashSet<i64> =
        named.iter().map(|message| message.get() as i64).collect();

    let mut handed_to_it = transaction
        .prepare("SELECT message_id FROM handed_over WHERE handed_to = ?1")
        .map_err(|_| MailIndexError::Unavailable)?;
    let mut spoken_of = |message: i64| -> Result<bool, MailIndexError> {
        if named.contains(&message) {
            return Ok(true);
        }
        handed_to_it
            .query_map([message], |row| row.get::<_, i64>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<i64>>>())
            .map(|rows| rows.iter().any(|row| named.contains(row)))
            .map_err(|_| MailIndexError::Unavailable)
    };

    for message in unplaced {
        if spoken_of(message)? {
            continue;
        }
        transaction
            .execute("DELETE FROM messages WHERE id = ?1", [message])
            .map_err(|_| MailIndexError::Unavailable)?;
    }

    Ok(())
}

/// The messages of an account no folder holds, as the index numbers them.
///
/// One statement for the question a pass asks before it reads and the sweep asks after, so the two
/// cannot disagree about what "no folder holds it" means.
fn unplaced_in(
    connection: &rusqlite::Connection,
    account: i64,
) -> Result<Vec<i64>, MailIndexError> {
    connection
        .prepare(
            "SELECT id FROM messages
             WHERE account_id = ?1
               AND id NOT IN (SELECT message_id FROM message_placements)",
        )
        .and_then(|mut statement| {
            statement
                .query_map([account], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<i64>>>()
        })
        .map_err(|_| MailIndexError::Unavailable)
}

#[cfg(test)]
mod tests;
