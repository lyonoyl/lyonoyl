use std::collections::HashMap;

use thiserror::Error;

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::read_model::as_asked;
use crate::read_model::mailboxes::account_that_minted;
use lyonoyl_domain::merge::{window_across, Place};
use lyonoyl_domain::message::{FolderId, MessageId};
use lyonoyl_domain::operation::Stopped;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReadModelError {
    #[error("the mail index is unavailable")]
    Unavailable,
    #[error("the mail index holds data this build cannot read")]
    Corrupted,
}

/// Which mail the list is showing.
///
/// The two wider scopes read `messages` rather than `message_placements`: a message filed in
/// several folders is one message, and a list that showed it once per folder would be counted and
/// scrolled wrong. It follows that a per-folder flag override is not visible there — outside a
/// folder there is no folder to take it from, so the account-wide value is the answer.
///
/// Both mean mail that *arrived*: what is in an inbox, and nothing else. An account row that also
/// showed drafts, sent mail and the contents of the bin would be a different thing from the inbox
/// under it, and neither the row nor the word "inbox" would mean anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewScope {
    /// One folder, in the order the provider delivered it.
    Folder(FolderId),
    /// Everything one account holds, whatever folder it is filed in.
    Account(i64),
    /// Everything on this device, across every account.
    Everything,
    /// Everything the accounts in one group hold.
    ///
    /// The accounts are carried here rather than a group's name, because narrowing to a group is
    /// choosing which files to read and nothing else: the mail is one file per account, so the
    /// question asked of each file is the same one [`ViewScope::Everything`] asks. That is why
    /// this cost nothing to add — the merge already took the files it was given.
    ///
    /// Empty means a group holding no accounts, which reads as an empty list rather than as
    /// everything.
    Group(Vec<i64>),
    /// Whatever answers what somebody typed, wherever it is filed.
    ///
    /// Unlike the three above this is not confined to mail that arrived: something looked for is
    /// as likely to be a message that was sent as one that came in. The text held here is what a
    /// person typed, and it is turned into a question for the index in one place
    /// ([`lyonoyl_domain::search::asked_for`]) and never used as a query itself.
    Looking(String),
}

/// What the list is showing.
///
/// A value rather than a handle to something held in memory: it fits in a URL, survives a restart
/// and needs no call to release it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewQuery {
    pub scope: ViewScope,
    pub unread_only: bool,
    /// Whether a row is a conversation or a single message.
    ///
    /// Only the views that span folders can answer this: a conversation is a thing of the account,
    /// not of a folder, and asking a folder which conversations touch it is a different question.
    /// Inside a folder the list stays messages.
    pub as_conversations: bool,
}

/// One row of the list. Narrow on purpose: this is fetched while the user scrolls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    /// How many messages the conversation this row stands for holds.
    ///
    /// One when the list is showing messages, and one for a conversation of one — the row does not
    /// have to know which of those it is.
    pub in_conversation: u32,
    /// The conversation this row stands for, where it stands for one.
    ///
    /// A row shows a conversation's newest message, and a reply arriving makes a different message
    /// the newest — so the message a row shows is not a name the conversation keeps. This is: it is
    /// the earliest message of the thread, it carries the account that minted it, and merging two
    /// conversations only ever moves it to the lower of the two names.
    ///
    /// Absent where the list is showing messages rather than conversations.
    pub conversation_id: Option<MessageId>,
    pub message_id: MessageId,
    /// Why an instruction about this message stopped, when one has.
    ///
    /// The application promises that something it could not carry out is shown to the person
    /// rather than dropped. This is where that promise is kept: the row that the instruction was
    /// about is the row it is shown on.
    pub stopped: Option<Stopped>,
    /// The folder this row stands for, which is what anything done to it is done in.
    ///
    /// In a folder view that is the folder being looked at. In a view spanning folders it is the
    /// inbox copy, which is also the copy whose read state the row shows — so what is acted on is
    /// what is displayed. `None` for a message that sits in no folder this device knows, which
    /// can only happen between a pass removing a placement and the next read.
    pub folder_id: Option<FolderId>,
    pub subject: Option<String>,
    pub sender: Option<String>,
    /// The opening of the message, as plain text. Absent until its body has been read.
    pub preview: Option<String>,
    pub received_at: i64,
    pub unread: bool,
    /// Whether this message has been told to go somewhere else and the server has not answered yet.
    ///
    /// The row stays where it is rather than disappearing: the list is counted and paged by
    /// position, and a window that returned fewer rows than it was asked for would put every row
    /// after it under the wrong number. So the row is marked instead, the interface shows it on its
    /// way out, and the pass that follows the instruction is what removes it.
    pub leaving: bool,
    /// Whether this message is on its way *into* the folder being looked at, and not there yet.
    ///
    /// The other half of `leaving`, and it exists for the same reason. A message told to go
    /// somewhere is marked on the list it is leaving the moment the instruction is written down;
    /// until this, it appeared in the folder it was sent to only when a pass had got round to both
    /// folders. Between those two moments it was nowhere -- gone from one list and not yet in the
    /// other -- which reads as a message that was lost.
    ///
    /// The index is not touched to arrange this. What it holds is what the server said, and saying
    /// otherwise there would mean inventing a position in a folder the message has not reached:
    /// the number that addresses it is the server's, and a made-up one is what an instruction
    /// would then be carried out against. What the person asked for lives in the operation log,
    /// and this row is the two of them read together.
    pub arriving: bool,
}

/// A slice of the list, with the size of the whole set and the epoch it was read at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewWindow {
    pub rows: Vec<ListRow>,
    pub total: u32,
    /// Counts changes to the set. A different value means rows moved, not that the window is wrong.
    pub view_epoch: u64,
    /// Accounts whose mail could not be read for this window.
    ///
    /// Their mail is not here and neither is it counted, so a list that said nothing about them
    /// would be a list quietly missing an account — which is the failure the separate files were
    /// meant to make smaller, not to hide. Empty is the ordinary case.
    pub not_answering: Vec<i64>,
}

pub struct ListReadModel {
    /// The files the mail is kept in, one per account.
    ///
    /// A window inside one folder or one account is read from the file that holds it. A window
    /// spanning accounts has no one file to ask, so it is read from each and merged — and it is
    /// read that way already, over the one file there is, rather than on the day there are more.
    indexes: Vec<(i64, SqlitePool)>,
    /// Where instructions wait until a server can be told about them.
    ///
    /// Two databases, so this cannot be a join: the window is read from the index, and what a
    /// person has asked for since is laid over it here.
    profile: SqlitePool,
}

impl ListReadModel {
    /// Reading across every file the mail is kept in.
    ///
    /// A list is the one reader that has no single file to name: a window inside a folder is
    /// answered by the file holding it, but a window across accounts is answered by all of them.
    /// So there is one constructor and it takes them all, however many there are.
    pub fn spanning(indexes: Vec<(i64, SqlitePool)>, profile: SqlitePool) -> Self {
        Self { indexes, profile }
    }

    /// Shows what a person did, rather than what a server has heard about yet.
    ///
    /// Marking a message read is written down at once and told to the server a moment later, so
    /// between those two moments the index still says unread. Showing that would mean a click that
    /// visibly does nothing — and doing nothing is exactly what an offline mail client must not
    /// look like.
    ///
    /// The copy the row stands for, and only that copy: whether it has been read as asked, whether
    /// it is on its way out, and why an instruction about it stopped. A message on its way out
    /// keeps its row until a pass has read it back, because the list is paged by position.
    fn as_the_person_left_it(&self, rows: &mut [ListRow]) -> Result<(), ReadModelError> {
        if rows.is_empty() {
            return Ok(());
        }

        let asked_about: Vec<i64> = rows.iter().map(|row| row.message_id.get() as i64).collect();
        let queued = as_asked::about(&self.profile, &asked_about)?;
        if queued.is_empty() {
            return Ok(());
        }

        let folded = as_asked::folded(&queued, |account| self.trash_of(account));

        for row in rows.iter_mut() {
            let Some(shown) = row
                .folder_id
                .and_then(|folder| folded.copy(row.message_id, folder))
            else {
                continue;
            };

            if shown.stopped.is_some() {
                row.stopped = shown.stopped;
            }
            // Told to go elsewhere and not read back yet. What makes a click visible before any
            // server has been reached — which is the whole reason instructions are written down
            // first.
            if shown.leaving {
                row.leaving = true;
            }
            if let Some(seen) = shown
                .seen
                .or(shown.arriving.and_then(|arriving| arriving.seen))
            {
                row.unread = !seen;
            }
        }

        Ok(())
    }

    /// Where an account keeps deleted mail, asked of its own file.
    fn trash_of(&self, account: i64) -> Option<FolderId> {
        let index = match self.indexes.as_slice() {
            [(_, only)] => only,
            _ => &self.indexes.iter().find(|(held, _)| *held == account)?.1,
        };
        as_asked::trash_in(index, account)
    }

    /// The messages on their way into this folder, which no pass has read into it yet.
    ///
    /// Read from the operation log, which is where what a person asked for lives, and turned into
    /// rows out of the index, which is where what the message says lives. Neither is changed: the
    /// index goes on holding what the server said, and this is the two of them read together.
    ///
    /// A delete counts as arriving in the trash, because that is what "delete" means on an account
    /// that has one -- and the trash is exactly the folder somebody opens to check.
    fn arriving_into(&self, folder: FolderId) -> Result<Vec<ListRow>, ReadModelError> {
        let Some(number) = self.account_of(folder)? else {
            return Ok(Vec::new());
        };
        let index = self
            .the_file_holding(&ViewScope::Folder(folder))?
            .get()
            .map_err(|_| ReadModelError::Unavailable)?;

        let queued = as_asked::of_account(&self.profile, number)?;
        if queued.is_empty() {
            return Ok(Vec::new());
        }

        let waiting: Vec<i64> = as_asked::folded(&queued, |account| self.trash_of(account))
            .arriving_into(folder)
            .into_iter()
            .map(|message| message.get() as i64)
            .collect();

        if waiting.is_empty() {
            return Ok(Vec::new());
        }

        let places = std::iter::repeat_n("?", waiting.len())
            .collect::<Vec<_>>()
            .join(", ");

        // Not the ones a pass has already brought in: between the instruction and the pass a
        // message is in one of these lists, never in both, or it would be two rows for one
        // message.
        let mut statement = index
            .prepare(&format!(
                "SELECT m.id, m.subject, m.sender, m.preview, m.received_at,
                        COALESCE((SELECT min(p.flags & 1) FROM message_placements p
                                  WHERE p.message_id = m.id), 1) = 0,
                        NULL, 1
                 FROM messages m
                 WHERE m.id IN ({places})
                   AND NOT EXISTS (SELECT 1 FROM message_placements p
                                   WHERE p.message_id = m.id AND p.folder_id = ?{destination})
                 ORDER BY m.received_at DESC, m.id DESC",
                places = places,
                destination = waiting.len() + 1,
            ))
            .map_err(|_| ReadModelError::Unavailable)?;

        let mut asked: Vec<i64> = waiting.clone();
        asked.push(folder.get() as i64);

        let rows = statement
            .query_map(rusqlite::params_from_iter(&asked), |row| Ok(read_row(row)))
            .map_err(|_| ReadModelError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ReadModelError::Unavailable)?
            .into_iter()
            .collect::<Result<Vec<ListRow>, _>>()?;

        Ok(rows
            .into_iter()
            .map(|row| ListRow {
                // The folder it is going to, so anything else done to it is done where it is going.
                folder_id: Some(folder),
                arriving: true,
                ..row
            })
            .collect())
    }

    /// Whose folder this is, asked of the folder itself, so the queue is read for that account
    /// alone. Nothing for a folder the file does not have.
    fn account_of(&self, folder: FolderId) -> Result<Option<i64>, ReadModelError> {
        Ok(self
            .the_file_holding(&ViewScope::Folder(folder))?
            .get()
            .map_err(|_| ReadModelError::Unavailable)?
            .query_row(
                "SELECT account_id FROM folders WHERE id = ?1",
                [folder.get() as i64],
                |row| row.get::<_, i64>(0),
            )
            .ok())
    }

    /// The file holding the mail a window is over.
    ///
    /// A folder belongs to one account and so does an account, so one file answers and the rest
    /// are not opened. Which one is read out of the number: a folder is minted in its account's
    /// file and carries that account in its top bits.
    ///
    /// One file on the device answers whatever is asked of it — there is nothing to choose
    /// between, and the numbers it minted before files were numbered by account carry no account
    /// to choose by. With several, a number naming an account nothing here holds is unanswerable
    /// rather than answered from another file, which would be somebody else's mail.
    fn the_file_holding(&self, scope: &ViewScope) -> Result<&SqlitePool, ReadModelError> {
        if let [(_, only)] = self.indexes.as_slice() {
            return Ok(only);
        }

        let account = match scope {
            ViewScope::Folder(folder) => account_that_minted(folder.get()),
            ViewScope::Account(account) => Some(*account),
            ViewScope::Everything | ViewScope::Group(_) | ViewScope::Looking(_) => None,
        };

        self.indexes
            .iter()
            .find(|(held, _)| Some(*held) == account)
            .map(|(_, pool)| pool)
            .ok_or(ReadModelError::Unavailable)
    }

    /// Reads `limit` rows starting at `offset` from one file, with its total and its epoch.
    ///
    /// The three come from one transaction, so a message arriving mid-read cannot produce a window
    /// that disagrees with its own total. Across files there is no such instant — SQLite gives a
    /// snapshot per file — and what that costs is written where the merge happens.
    fn one_file(
        pool: &SqlitePool,
        shape: &Shape<'_>,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<ListRow>, i64, i64), ReadModelError> {
        let mut connection = pool.get().map_err(|_| ReadModelError::Unavailable)?;
        let snapshot = connection
            .transaction()
            .map_err(|_| ReadModelError::Unavailable)?;

        // Somebody who has typed only punctuation has asked for nothing, and nothing is an empty
        // list rather than a failure. The index refuses an empty question, so it is never asked
        // one.
        let (rows, total) = if shape.asks_nothing() {
            (Vec::new(), 0)
        } else {
            let mut statement = snapshot
                .prepare(&shape.rows())
                .map_err(|_| ReadModelError::Unavailable)?;

            let arguments = shape.arguments(&snapshot, offset, limit)?;

            let rows = statement
                .query_map(rusqlite::params_from_iter(&arguments), |row| {
                    Ok(read_row(row))
                })
                .map_err(|_| ReadModelError::Unavailable)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ReadModelError::Unavailable)?
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            let total: i64 = snapshot
                .query_row(
                    &shape.total(),
                    rusqlite::params_from_iter(shape.key()),
                    |row| row.get(0),
                )
                .map_err(|_| ReadModelError::Unavailable)?;

            (rows, total)
        };

        let view_epoch: i64 = snapshot
            .query_row(
                &shape.epoch(),
                rusqlite::params_from_iter(shape.epoch_key()),
                |row| row.get(0),
            )
            .map_err(|_| ReadModelError::Unavailable)?;

        Ok((rows, total, view_epoch))
    }

    /// Reads `limit` rows starting at `offset`, together with the total and the epoch.
    ///
    /// A window that spans accounts is read from every file and merged. The totals are summed and
    /// so are the epochs, and neither comes from a single instant: SQLite gives one snapshot per
    /// file, and there is no instant across several. What that can cost is bounded, because the
    /// files do not overlap — a message belongs to one account and cannot move between them — so a
    /// window can be a moment stale but can neither show a row twice nor lose one. The epoch is
    /// what notices, and the list re-anchors on it rather than reloading.
    pub fn get_view_window(
        &self,
        query: &ViewQuery,
        offset: u32,
        limit: u32,
    ) -> Result<ViewWindow, ReadModelError> {
        let shape = Shape::of(query);

        // What a person asked to be put here, and no pass has moved yet. First in the window and
        // counted in its length, so every position after them still names the row it did: they
        // are what the list would show if the server had already been told, and a message that is
        // gone from one folder and not yet in the other is a message that reads as lost.
        let arriving = match &query.scope {
            ViewScope::Folder(folder) if !query.unread_only => self.arriving_into(*folder)?,
            _ => Vec::new(),
        };
        let ahead = u32::try_from(arriving.len()).unwrap_or(u32::MAX);

        let (rows, total, view_epoch, not_answering) = if shape.spans_accounts() {
            self.merged(&shape, offset.saturating_sub(ahead), limit)?
        } else {
            let (rows, total, epoch) = Self::one_file(
                self.the_file_holding(&query.scope)?,
                &shape,
                offset.saturating_sub(ahead),
                limit,
            )?;
            (rows, total, epoch, Vec::new())
        };

        // The ones that fall inside this window, and the rest of it filled from the index.
        let taken: Vec<ListRow> = arriving
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect();
        let room = (limit as usize).saturating_sub(taken.len());
        let mut rows: Vec<ListRow> = taken
            .into_iter()
            .chain(rows.into_iter().take(room))
            .collect();
        let total = total + i64::from(ahead);

        // Read from the index, then laid over with what the person has asked for since. The
        // overlay is outside the read on purpose: it is a different database, and holding one open
        // across both would be one lock waiting on another.
        self.as_the_person_left_it(&mut rows)?;

        Ok(ViewWindow {
            rows,
            total: u32::try_from(total).map_err(|_| ReadModelError::Corrupted)?,
            view_epoch: u64::try_from(view_epoch).map_err(|_| ReadModelError::Corrupted)?,
            not_answering,
        })
    }

    /// Where each row of one file sits in the order, with its total and its epoch.
    ///
    /// The places are the key the merge orders by and the identifier it ends up with; the total is
    /// how long the list is and the epoch is what notices it changing.
    ///
    /// The same transaction as `one_file` and for the same reason: a message arriving mid-read
    /// cannot produce a window that disagrees with its own total.
    fn places_in(
        pool: &SqlitePool,
        shape: &Shape<'_>,
        reach: u32,
    ) -> Result<InOrder, ReadModelError> {
        let mut connection = pool.get().map_err(|_| ReadModelError::Unavailable)?;
        let snapshot = connection
            .transaction()
            .map_err(|_| ReadModelError::Unavailable)?;

        let (places, total) = if shape.asks_nothing() {
            (Vec::new(), 0)
        } else {
            let mut statement = snapshot
                .prepare(&shape.places())
                .map_err(|_| ReadModelError::Unavailable)?;

            let mut arguments = shape.key();
            arguments.push(rusqlite::types::Value::Integer(i64::from(reach)));

            // Three columns: what the row is shown under, what the order compares, and what the
            // fetch is keyed by. The first and the last differ only for a list of conversations,
            // where the row is a message and the thing fetched is the thread.
            let places = statement
                .query_map(rusqlite::params_from_iter(&arguments), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .map_err(|_| ReadModelError::Unavailable)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ReadModelError::Unavailable)?
                .into_iter()
                .map(|(shown, arrived_at, fetch_by)| {
                    (
                        Place {
                            arrived_at,
                            id: shown as u64,
                        },
                        fetch_by,
                    )
                })
                .collect::<Vec<_>>();

            let total: i64 = snapshot
                .query_row(
                    &shape.total(),
                    rusqlite::params_from_iter(shape.key()),
                    |row| row.get(0),
                )
                .map_err(|_| ReadModelError::Unavailable)?;

            (places, total)
        };

        let view_epoch: i64 = snapshot
            .query_row(
                &shape.epoch(),
                rusqlite::params_from_iter(shape.epoch_key()),
                |row| row.get(0),
            )
            .map_err(|_| ReadModelError::Unavailable)?;

        Ok((places, total, view_epoch))
    }

    /// The rows one file holds for the identifiers the merge chose.
    fn rows_named(
        pool: &SqlitePool,
        shape: &Shape<'_>,
        wanted: &[i64],
    ) -> Result<Vec<ListRow>, ReadModelError> {
        let connection = pool.get().map_err(|_| ReadModelError::Unavailable)?;

        let mut statement = connection
            .prepare(&shape.rows_for(wanted.len()))
            .map_err(|_| ReadModelError::Unavailable)?;

        let rows = statement
            .query_map(rusqlite::params_from_iter(wanted), |row| Ok(read_row(row)))
            .map_err(|_| ReadModelError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ReadModelError::Unavailable)?
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// One window out of every file, in the order the list is kept in.
    ///
    /// Each file is asked for the rows up to the end of the window and none beyond it, because the
    /// row that belongs at a position can be in any of them. The merge then takes the window out
    /// of what they answered.
    fn merged(
        &self,
        shape: &Shape<'_>,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<ListRow>, i64, i64, Vec<i64>), ReadModelError> {
        let reach = offset.saturating_add(limit);

        let mut runs = Vec::with_capacity(self.indexes.len());
        let mut total = 0i64;
        let mut view_epoch = 0i64;

        let mut not_answering = Vec::new();

        // Which files to open *is* the narrowing. The mail is one file per account, so a group
        // asks each of its files exactly what everything on the device asks all of them — there is
        // no predicate to add and nothing to filter afterwards. A group naming an account this
        // device no longer holds is simply a file that is not here, not an error: the account was
        // removed and the arrangement outlived it.
        let only: Option<&[i64]> = match &shape.query.scope {
            ViewScope::Group(accounts) => Some(accounts.as_slice()),
            _ => None,
        };

        for (account, pool) in self
            .indexes
            .iter()
            .filter(|(held, _)| only.is_none_or(|wanted| wanted.contains(held)))
        {
            // One file that will not open is one account's mail missing, not everybody's. Reading
            // on and saying which is the whole difference the separate files were for; a `?` here
            // would blank the list of the ninety-nine that are fine.
            let Ok((places, counted, epoch)) = Self::places_in(pool, shape, reach) else {
                not_answering.push(*account);
                continue;
            };
            total += counted;
            view_epoch += epoch;
            runs.push(
                places
                    .into_iter()
                    .map(|(place, id)| (place, (*account, id)))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
        }

        // Fifty identifiers, in the order the list shows them. Everything above the window has
        // been compared and dropped without a subject, a sender or a preview ever being read.
        let chosen: Vec<(i64, i64)> = window_across(&mut runs, offset as usize, limit as usize);

        // Keyed by the file as well as by what the fetch was keyed on. A message number carries
        // its account in the top bits and is unique across the device, but a thread number does
        // not: `thread_id` is a row number in one file, so two accounts number their first thread
        // alike, and keyed by that alone one would overwrite the other.
        let mut fetched: HashMap<(i64, i64), ListRow> = HashMap::new();
        for (account, pool) in self
            .indexes
            .iter()
            .filter(|(held, _)| only.is_none_or(|wanted| wanted.contains(held)))
        {
            let wanted: Vec<i64> = chosen
                .iter()
                .filter(|(from, _)| from == account)
                .map(|(_, id)| *id)
                .collect();
            if wanted.is_empty() {
                continue;
            }
            // A file that answered with places and will not answer with rows is the same missing
            // account as above, and is said so the same way rather than silently short.
            let Ok(rows) = Self::rows_named(pool, shape, &wanted) else {
                not_answering.push(*account);
                continue;
            };
            for row in rows {
                // Under the same key the fetch was asked for, which for a list of conversations is
                // the thread the row stands for rather than the message it shows.
                let under = if shape.fetched_by_the_thread() {
                    row.conversation_id.map(|thread| thread.get() as i64)
                } else {
                    Some(row.message_id.get() as i64)
                };
                if let Some(under) = under {
                    fetched.insert((*account, under), row);
                }
            }
        }

        // Back into the order the merge decided. A row the fetch did not return is a message that
        // left between the two reads, and it is left out rather than guessed at.
        let rows: Vec<ListRow> = chosen
            .into_iter()
            .filter_map(|(account, id)| fetched.remove(&(account, id)))
            .collect();

        Ok((rows, total, view_epoch, not_answering))
    }

    /// The messages of the conversation a message belongs to, oldest first.
    ///
    /// Asked by the message the list shows rather than by the conversation's own number: that
    /// number never crosses to the interface, and a row already knows which message it stands for.
    ///
    /// Oldest first because a conversation is read forwards — the reply at the bottom is the one
    /// already shown in the list, and what a person opens it for is what came before.
    ///
    /// A message in no conversation is a conversation of one, and answers with itself.
    pub fn conversation_of(&self, message: MessageId) -> Result<Vec<ListRow>, ReadModelError> {
        let index = self.indexes.iter().find_map(|(_, pool)| {
            let held = pool.get().ok()?;
            let known: i64 = held
                .query_row(
                    "SELECT count(*) FROM messages WHERE id = ?1",
                    [message.get() as i64],
                    |row| row.get(0),
                )
                .ok()?;
            (known > 0).then_some(pool.clone())
        });

        let Some(pool) = index else {
            return Ok(Vec::new());
        };

        let connection = pool.get().map_err(|_| ReadModelError::Unavailable)?;

        let mut statement = connection
            .prepare(&format!(
                // The message's own date and a flag taken from any copy of it, the way a search
                // reads them — not the date it reached an inbox and the flag of the copy there.
                // A conversation holds everything of the thread, and what a person sent reached no
                // inbox at all: both of those columns hold nothing for it, and reading nothing as
                // a number failed the whole conversation over one message.
                "SELECT m.id, m.subject, m.sender, m.preview, m.received_at,
                        COALESCE((SELECT min(p.flags & 1) FROM message_placements p
                                  WHERE p.message_id = m.id), 1) = 0,
                        {}, 1
                 FROM messages m
                 WHERE m.thread_id IS (SELECT thread_id FROM messages WHERE id = ?1)
                   AND (m.thread_id IS NOT NULL OR m.id = ?1)
                 ORDER BY m.received_at ASC, m.id ASC",
                the_inbox_copy(),
            ))
            .map_err(|_| ReadModelError::Unavailable)?;

        let mut rows = statement
            .query_map([message.get() as i64], |row| Ok(read_row(row)))
            .map_err(|_| ReadModelError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ReadModelError::Unavailable)?
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;

        // Laid over with what the person has asked for since, the same as a window is: a message
        // they have just archived shows as leaving here too, rather than as though nothing happened.
        self.as_the_person_left_it(&mut rows)?;
        Ok(rows)
    }

    /// Every message a folder shows, which is what emptying it takes.
    ///
    /// Not what the index holds there. A message a person has asked out of the folder — rescued
    /// from the trash, on its way to the inbox — is on its way out, and emptying the folder behind
    /// it would erase what they had just saved: a delete asked after a move out of the same place
    /// is folded into one delete. A message on its way in — thrown away and not yet read into the
    /// trash — is in the folder to anybody looking at it, and goes with the rest.
    pub fn what_emptying_takes(&self, folder: FolderId) -> Result<Vec<MessageId>, ReadModelError> {
        let held = self.get_view_ids_in_range(
            &ViewQuery {
                scope: ViewScope::Folder(folder),
                unread_only: false,
                // Messages, not conversations: what is being emptied is a folder, and a
                // conversation is a way of showing several of them at once.
                as_conversations: false,
            },
            0,
            u32::MAX,
        )?;

        let asked = match self.account_of(folder)? {
            Some(account) => as_asked::of_account(&self.profile, account)?,
            None => Vec::new(),
        };
        let folded = as_asked::folded(&asked, |account| self.trash_of(account));

        let mut taken: Vec<MessageId> = held
            .into_iter()
            .filter(|message| {
                !folded
                    .copy(*message, folder)
                    .is_some_and(|shown| shown.leaving)
            })
            .collect();
        taken.extend(
            self.arriving_into(folder)?
                .into_iter()
                .map(|row| row.message_id),
        );

        Ok(taken)
    }

    /// The identifiers occupying positions `from..=to`, for range selection over rows the list has
    /// not loaded, and for what emptying a folder starts from.
    pub fn get_view_ids_in_range(
        &self,
        query: &ViewQuery,
        from: u32,
        to: u32,
    ) -> Result<Vec<MessageId>, ReadModelError> {
        if to < from {
            return Ok(Vec::new());
        }

        let connection = self
            .the_file_holding(&query.scope)?
            .get()
            .map_err(|_| ReadModelError::Unavailable)?;
        let shape = Shape::of(query);

        if shape.asks_nothing() {
            return Ok(Vec::new());
        }

        let mut statement = connection
            .prepare(&shape.identifiers())
            .map_err(|_| ReadModelError::Unavailable)?;

        // Saturating, because `empty_folder` asks for `0..=u32::MAX`: `to - from + 1` wraps to
        // zero there, and a `LIMIT 0` empties nothing while reporting that it did.
        let arguments =
            shape.arguments(&connection, from, to.saturating_sub(from).saturating_add(1))?;

        let ids = statement
            .query_map(rusqlite::params_from_iter(&arguments), |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|_| ReadModelError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ReadModelError::Unavailable)?;

        ids.into_iter().map(message_id).collect()
    }
}

/// Where the rows of one file sit in the order, how many there are, and what notices a change.
type InOrder = (Vec<(Place, i64)>, i64, i64);

mod statements;

use statements::{message_id, read_row, the_inbox_copy, Shape};

/// The statement the list actually runs, for a plan or a measurement to look at.
///
/// Reading a query written beside this one reported a window as taking 0.22 ms where the read
/// model took 222. Anything measuring this view asks it here.
#[cfg(test)]
pub(crate) fn the_statement_behind(query: &ViewQuery) -> String {
    let shape = Shape { query };

    // A view that spans files never runs `rows`. It reads places out of every file and fetches the
    // fifty the merge chose, and the places are what its cost is: one row read for everything above
    // the window, against fifty fetched. Handing back `rows` here printed a plan for a statement the
    // product does not issue — and hid that the places behind the real view were joining `messages`
    // once per conversation.
    if shape.spans_accounts() {
        shape.places()
    } else {
        shape.rows()
    }
}

#[cfg(test)]
mod tests;
