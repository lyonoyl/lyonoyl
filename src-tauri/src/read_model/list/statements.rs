//! The statements the list is read by, and the one place a row is read back out of one.
//!
//! Kept together because they are one contract: `Shape::rows` chooses the columns and their order,
//! and `read_row` decodes them by position. Split apart, the pair would be a comment asking to be
//! believed instead of two functions a reader can hold at once.

use rusqlite::types::Value;
use rusqlite::{OptionalExtension, Row};

use lyonoyl_domain::message::{FolderId, MessageId};

use super::{ListRow, ReadModelError, ViewQuery, ViewScope};

/// Mail that arrived, as opposed to mail that was written, thrown away or filed away.
///
/// A message is in an inbox when one of its placements is: the same message may also sit in an
/// archive or carry a label, and it is still the one message that arrived. Held as a column with
/// nothing in it when the answer is no, so one partial index serves both this and the order below;
/// asked as a subquery per row it could be served by no index at all, and the view the application
/// opens on read every message in the database to answer it.
fn arrived_in_an_inbox() -> String {
    "m.arrived_at IS NOT NULL".to_owned()
}

/// When the message arrived in the inbox, which is not always when the message is dated.
///
/// A folder's own rows are ordered by the copy sitting in it. A view spanning folders that ordered
/// by the message instead put the same mail in a different order, so the row in a given place was
/// one message here and another one there — and for a message a folder holds twice, the two dates
/// are hours apart.
fn arrived_at() -> String {
    "m.arrived_at".to_owned()
}

/// Which copy of the message the view shows, the inbox one where there is one.
///
/// Held as a column because a select list is evaluated for the rows a window returns, and reading
/// this view from several files returns every row it compares rather than fifty.
pub(super) fn the_inbox_copy() -> String {
    "m.inbox_folder_id".to_owned()
}

/// Unread when any copy of it is, wherever the message is filed.
///
/// A search spans folders, and outside a folder there is no copy to take the flag from. A message
/// found in an archive is still unread mail if nobody opened it, and a message in no folder at all
/// — which cannot happen, but the read has to say something — counts as unread rather than as
/// nothing. Written once because the row reports it and the filter selects on it: two spellings
/// would be a list showing rows the filter says are not there.
fn unread_in_any_copy() -> String {
    "COALESCE((SELECT min(p.flags & 1) FROM message_placements p
               WHERE p.message_id = m.id), 1) = 0"
        .to_owned()
}

/// What a search asks of `messages` beyond the words: the words themselves, and the filter.
fn what_a_search_matches(unread_only: bool) -> String {
    let mut conditions =
        vec!["m.id IN (SELECT rowid FROM message_search WHERE message_search MATCH ?1)".to_owned()];
    if unread_only {
        conditions.push(unread_in_any_copy());
    }
    format!("WHERE {}", conditions.join(" AND "))
}

/// The least-read copy speaks for the message, the same way a folder's own rows are read.
///
/// Taking whichever copy came first made a message read here and unread beside it. NULL is a
/// message in no inbox, and is neither read nor unread.
fn unread_in_the_inbox() -> String {
    "m.inbox_unread = 0".to_owned()
}

/// Where in the folder a position lies, as a day boundary and what is left to skip inside it.
///
/// `OFFSET` walks every row it skips. The map holds how many messages a folder has on each day, so
/// adding those up from the newest day tells which day the position falls in without reading a
/// single message. A folder of ten years is a few thousand small rows; the messages in it are
/// hundreds of thousands.
///
/// A folder the map knows nothing about — an empty one — answers with a boundary no message can
/// be under, and the window comes back empty, which is what a position past the end means.
fn where_the_window_starts(
    connection: &rusqlite::Connection,
    folder: i64,
    offset: u32,
) -> Result<(i64, i64), ReadModelError> {
    let offset = i64::from(offset);

    // The oldest day whose newer days do not already account for the position: the newer ones are
    // above the window, and this is the day the window opens in. Days holding nothing sum to
    // nothing, so an emptied day is stepped over rather than landed on.
    let found: Option<(i64, i64)> = connection
        .query_row(
            "SELECT day, newer FROM (
                 SELECT day,
                        COALESCE(sum(messages) OVER (ORDER BY day DESC
                                 ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING), 0) AS newer
                 FROM folder_days WHERE folder_id = ?1)
             WHERE newer <= ?2
             ORDER BY day ASC
             LIMIT 1",
            (folder, offset),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| ReadModelError::Unavailable)?;

    match found {
        Some((day, newer)) => Ok((
            day.saturating_add(1).saturating_mul(86_400),
            offset.saturating_sub(newer),
        )),
        None => Ok((i64::MAX, offset)),
    }
}

/// The rows of a folder, one per message, starting from a day the map named.
///
/// `?1` the folder, `?2` how many rows, `?3` how many to skip inside the day the window starts in,
/// `?4` the first second the window is *not* interested in. Because `?4` is a day boundary, `?3`
/// is never larger than the number of messages that day holds, so the walk it costs is bounded by
/// a day rather than by the depth of the window.
///
/// `NOT EXISTS` rather than a copy chosen by a nested `ORDER BY … LIMIT 1`: both name the same
/// rows, and the second puts a sort inside the folder scan, which is the cost this is here to
/// remove.
fn the_copy_that_speaks(unread_only: bool) -> String {
    // The filter goes in twice, and both are needed. On the row, because only unread mail is
    // shown; and inside the search for a newer copy, because otherwise a message whose newest copy
    // has been read would be represented by nothing and disappear from a list it belongs in.
    let only_unread = |of: &str| {
        if unread_only {
            format!(" AND ({of}.flags & 1) = 0")
        } else {
            String::new()
        }
    };

    format!(
        "SELECT p.folder_id, p.message_id, p.received_at, p.uid
         FROM message_placements p
         WHERE p.folder_id = ?1
           AND p.received_at < ?4{}
           AND NOT EXISTS (
               SELECT 1 FROM message_placements q
               WHERE q.folder_id = p.folder_id
                 AND q.message_id = p.message_id{}
                 AND (q.received_at > p.received_at
                  OR (q.received_at = p.received_at AND q.uid > p.uid)))
         ORDER BY p.received_at DESC, p.uid DESC
         LIMIT ?2 OFFSET ?3",
        only_unread("p"),
        only_unread("q")
    )
}

/// The four statements one scope needs, and the value they are keyed by.
///
/// Built in one place so the window, its total and its epoch cannot end up describing different
/// sets of mail.
pub(super) struct Shape<'a> {
    pub(super) query: &'a ViewQuery,
}

impl<'a> Shape<'a> {
    pub(super) fn of(query: &'a ViewQuery) -> Self {
        Self { query }
    }

    /// Whether a position in this view is found through the map of days rather than by walking.
    ///
    /// One folder, unfiltered. The map counts what a folder holds on a day and knows nothing
    /// about flags, so the unread view cannot be answered from it.
    fn seeks_by_day(&self) -> bool {
        matches!(self.query.scope, ViewScope::Folder(_)) && !self.query.unread_only
    }

    /// What the statements above are asked with: the scope's own value, then how many rows, then
    /// where to start.
    ///
    /// A folder is started from a day the map named. Everything else counts rows from the top,
    /// which is what a boundary of `i64::MAX` means here: no day was named, so nothing is above
    /// the window and the whole set is walked.
    pub(super) fn arguments(
        &self,
        connection: &rusqlite::Connection,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Value>, ReadModelError> {
        let mut arguments = self.key();
        arguments.push(Value::Integer(i64::from(limit)));

        match &self.query.scope {
            ViewScope::Folder(folder) => {
                let (boundary, residual) = if self.seeks_by_day() {
                    where_the_window_starts(connection, folder.get() as i64, offset)?
                } else {
                    (i64::MAX, i64::from(offset))
                };
                arguments.push(Value::Integer(residual));
                arguments.push(Value::Integer(boundary));
            }
            _ => arguments.push(Value::Integer(i64::from(offset))),
        }

        Ok(arguments)
    }

    /// The value the scope is keyed by, as the statements below expect it: `?1` where there is
    /// one, and nothing at all for the whole device.
    pub(super) fn key(&self) -> Vec<Value> {
        match &self.query.scope {
            ViewScope::Folder(folder) => vec![Value::Integer(folder.get() as i64)],
            ViewScope::Account(account) => vec![Value::Integer(*account)],
            ViewScope::Everything | ViewScope::Group(_) => Vec::new(),
            ViewScope::Looking(typed) => {
                // What was typed never reaches the query language. A word of it that the index
                // reads as an operator is a word somebody was looking for.
                vec![Value::Text(
                    lyonoyl_domain::search::asked_for(typed).unwrap_or_default(),
                )]
            }
        }
    }

    pub(super) fn rows(&self) -> String {
        match self.query.scope {
            ViewScope::Folder(_) => format!(
                // A folder can hold the same message twice: two copies a server keeps apart by
                // number are one message here, and showing both would be two rows that open the
                // same thing. Rather than group them, the window keeps only the copy that speaks
                // for the message — the newest, and the highest number among any that share a
                // second — and leaves the rest out. Grouping cost the whole folder a sort; this
                // is a walk along the index from wherever the window starts.
                //
                // Read only when every copy is: the flag belongs to the copy, and a copy nobody
                // has opened is unread mail whatever the other one says. In the unread view every
                // row satisfies this by construction, and it is asked anyway so that one rule
                // answers the question everywhere.
                "SELECT w.message_id, m.subject, m.sender, m.preview, w.received_at,
                        (SELECT min(q.flags & 1) FROM message_placements q
                         WHERE q.folder_id = w.folder_id AND q.message_id = w.message_id) = 0,
                        w.folder_id
                 FROM ({}) w
                 JOIN messages m ON m.id = w.message_id
                 ORDER BY w.received_at DESC, w.uid DESC",
                the_copy_that_speaks(self.query.unread_only)
            ),
            // A search answers with messages, and it comes before the conversations arm below so
            // that it keeps doing so however the caller asked. Grouping the hits would show one
            // row where three messages matched, and the person would open the row to find out
            // which — a search is asked to name a message, not the conversation it is in.
            ViewScope::Looking(_) => format!(
                // Ordered by the message's own date rather than by how well it matches. Somebody
                // looking through their mail is looking for a message they remember, and what
                // they remember about it is when it was.
                "SELECT m.id, m.subject, m.sender, m.preview, m.received_at,
                        {},
                        {}
                 FROM messages m
                 {}
                 ORDER BY m.received_at DESC, m.id DESC
                 LIMIT ?2 OFFSET ?3",
                unread_in_any_copy(),
                the_inbox_copy(),
                what_a_search_matches(self.query.unread_only)
            ),
            // One row per conversation, read straight from the table that keeps them: which
            // message speaks for each and when it arrived are held there, so this is the same
            // window it would be over messages — ordered by an index, no grouping, no scan.
            ViewScope::Account(_) | ViewScope::Everything | ViewScope::Group(_)
                if self.query.as_conversations =>
            {
                format!(
                    "SELECT m.id, m.subject, m.sender, m.preview, c.newest_at, {}, {}, c.messages,
                            c.thread_id
                     FROM conversations c
                     JOIN messages m ON m.id = c.newest_message
                     WHERE c.newest_at > 0{}
                     ORDER BY c.newest_at DESC, c.thread_id DESC
                     LIMIT {} OFFSET {}",
                    unread_in_the_inbox(),
                    the_inbox_copy(),
                    match self.query.scope {
                        ViewScope::Account(_) => " AND m.account_id = ?1",
                        _ => "",
                    },
                    self.limit_marker(),
                    self.offset_marker()
                )
            }
            ViewScope::Account(_) | ViewScope::Everything | ViewScope::Group(_) => format!(
                "SELECT m.id, m.subject, m.sender, m.preview, {}, {}, {}
                 FROM messages m
                 {}
                 ORDER BY {} DESC, m.id DESC
                 LIMIT {} OFFSET {}",
                arrived_at(),
                unread_in_the_inbox(),
                the_inbox_copy(),
                self.message_where(),
                arrived_at(),
                self.limit_marker(),
                self.offset_marker()
            ),
        }
    }

    /// Where each row sits in the order, and nothing else.
    ///
    /// A window that spans files cannot know which file holds the row at a position, so it asks
    /// every file for everything above the window and merges. Asking for whole rows meant a
    /// hundred thousand subjects, senders and previews read out of SQLite and built into Rust
    /// strings so that fifty could be kept — measured at 183 ms of reading and 280 ms of building,
    /// for fifty rows. This asks for what the merge actually compares: the key it orders by and
    /// the identifier it ends up with. The rows themselves are fetched afterwards, for the fifty
    /// that won.
    ///
    /// `LIMIT` and no `OFFSET`: the merge always starts from the top of each file, because the row
    /// at a merged position can be anywhere.
    pub(super) fn places(&self) -> String {
        match self.query.scope {
            // A folder lives in one file and is never merged, so nothing asks this. Written to
            // return the same two columns as the others all the same: a branch that is right only
            // because nobody reaches it is a trap for whoever reaches it next.
            ViewScope::Folder(_) => format!(
                "SELECT w.message_id, w.received_at, w.message_id FROM ({}) w
                 ORDER BY w.received_at DESC, w.uid DESC",
                the_copy_that_speaks(self.query.unread_only)
            ),
            ViewScope::Looking(_) => format!(
                "SELECT m.id, m.received_at, m.id
                 FROM messages m
                 {}
                 ORDER BY m.received_at DESC, m.id DESC
                 LIMIT ?2",
                what_a_search_matches(self.query.unread_only)
            ),
            ViewScope::Account(_) | ViewScope::Everything | ViewScope::Group(_)
                if self.query.as_conversations =>
            {
                // The join only when an account has to be named. Everything the order and the keys
                // need is in `conversations` and in the index over it, so joining `messages`
                // anyway cost one lookup by row number for every conversation above the window —
                // a hundred thousand of them to show fifty, which the plan named outright:
                // `SEARCH m USING INTEGER PRIMARY KEY (rowid=?)`.
                let (join, narrowed) = match self.query.scope {
                    ViewScope::Account(_) => (
                        "JOIN messages m ON m.id = c.newest_message",
                        " AND m.account_id = ?1",
                    ),
                    _ => ("", ""),
                };

                format!(
                    "SELECT c.newest_message, c.newest_at, c.thread_id
                     FROM conversations c
                     {join}
                     WHERE c.newest_at > 0{narrowed}
                     ORDER BY c.newest_at DESC, c.thread_id DESC
                     LIMIT {}",
                    self.limit_marker()
                )
            }
            ViewScope::Account(_) | ViewScope::Everything | ViewScope::Group(_) => format!(
                "SELECT m.id, {}, m.id
                 FROM messages m
                 {}
                 ORDER BY {} DESC, m.id DESC
                 LIMIT {}",
                arrived_at(),
                self.message_where(),
                arrived_at(),
                self.limit_marker()
            ),
        }
    }

    /// What the third column of `places` is, and therefore what `rows_for` is keyed by.
    ///
    /// A list of conversations is fetched by `thread_id`, which is that table's primary key. Keyed
    /// by `newest_message` instead — the column the row is shown under — the fetch had no index to
    /// use and scanned the whole table: measured at 5.6 ms against 0.06, on every window, on every
    /// account file, and this is the view the window always shows.
    ///
    /// It is also what a conversation keeps when a reply arrives. The newest message changes and
    /// the thread does not, so a conversation whose newest message moved between the two reads is
    /// still found, and the window comes back the length it was asked for rather than one short.
    pub(super) fn fetched_by_the_thread(&self) -> bool {
        self.query.as_conversations
            && matches!(
                self.query.scope,
                ViewScope::Account(_) | ViewScope::Everything | ViewScope::Group(_)
            )
    }

    /// The rows the merge chose, named rather than counted to.
    ///
    /// The same columns `rows` returns, in the same order, so one reader answers both. The search
    /// join is dropped: these identifiers came out of a search, and asking the index a second time
    /// would be asking it to agree with itself.
    pub(super) fn rows_for(&self, how_many: usize) -> String {
        let places = std::iter::repeat_n("?", how_many)
            .collect::<Vec<_>>()
            .join(", ");

        match self.query.scope {
            ViewScope::Looking(_) => format!(
                "SELECT m.id, m.subject, m.sender, m.preview, m.received_at, {}, {}
                 FROM messages m
                 WHERE m.id IN ({places})",
                unread_in_any_copy(),
                the_inbox_copy()
            ),
            ViewScope::Account(_) | ViewScope::Everything | ViewScope::Group(_)
                if self.query.as_conversations =>
            {
                format!(
                    "SELECT m.id, m.subject, m.sender, m.preview, c.newest_at, {}, {}, c.messages,
                            c.thread_id
                     FROM conversations c
                     JOIN messages m ON m.id = c.newest_message
                     WHERE c.thread_id IN ({places})",
                    unread_in_the_inbox(),
                    the_inbox_copy()
                )
            }
            _ => format!(
                "SELECT m.id, m.subject, m.sender, m.preview, {}, {}, {}
                 FROM messages m
                 WHERE m.id IN ({places})",
                arrived_at(),
                unread_in_the_inbox(),
                the_inbox_copy()
            ),
        }
    }

    pub(super) fn identifiers(&self) -> String {
        match self.query.scope {
            ViewScope::Folder(_) => format!(
                "SELECT w.message_id FROM ({}) w
                 ORDER BY w.received_at DESC, w.uid DESC",
                the_copy_that_speaks(self.query.unread_only)
            ),
            ViewScope::Looking(_) => format!(
                "SELECT m.id FROM messages m
                 {}
                 ORDER BY m.received_at DESC, m.id DESC
                 LIMIT ?2 OFFSET ?3",
                what_a_search_matches(self.query.unread_only)
            ),
            ViewScope::Account(_) | ViewScope::Everything | ViewScope::Group(_) => format!(
                "SELECT m.id
                 FROM messages m
                 {}
                 ORDER BY {} DESC, m.id DESC
                 LIMIT {} OFFSET {}",
                self.message_where(),
                arrived_at(),
                self.limit_marker(),
                self.offset_marker()
            ),
        }
    }

    pub(super) fn total(&self) -> String {
        match self.query.scope {
            // The map already holds this: one entry per message per day. Counting the distinct
            // messages of the folder instead read every placement in it, and did so on every
            // window a person scrolled past.
            ViewScope::Folder(_) if !self.query.unread_only => {
                "SELECT COALESCE(sum(messages), 0) FROM folder_days WHERE folder_id = ?1".to_owned()
            }
            // No join: nothing was ever read from `messages` here, and the join cost one lookup
            // by rowid for every unread placement in the folder.
            ViewScope::Folder(_) => "SELECT count(DISTINCT message_id) FROM message_placements
                 WHERE folder_id = ?1 AND (flags & 1) = 0"
                .to_owned(),
            // The length of a list of conversations is the size of the table holding them, which
            // is what that table is for: counting them from the messages is a scan, 60 ms at two
            // hundred thousand against 0.04 here.
            ViewScope::Account(_) | ViewScope::Everything | ViewScope::Group(_)
                if self.query.as_conversations =>
            {
                match self.query.scope {
                    ViewScope::Account(_) => "SELECT count(*) FROM conversations c
                         JOIN messages m ON m.id = c.newest_message
                         WHERE c.newest_at > 0 AND m.account_id = ?1"
                        .to_owned(),
                    _ => "SELECT count(*) FROM conversations WHERE newest_at > 0".to_owned(),
                }
            }
            // Counted in the word index rather than through the messages. The index is
            // contentless and its rowids are message identifiers, kept in step by triggers on
            // insert, delete and update — so the two counts are the same number, and one of them
            // does not visit a hundred thousand rows to say so. Measured at 41 ms against 261.
            //
            // Only where the whole match set is the answer: "unread" belongs to a copy in a folder
            // and the index holds no copies, so that question is still asked of the messages.
            ViewScope::Looking(_) if !self.query.unread_only => {
                "SELECT count(*) FROM message_search WHERE message_search MATCH ?1".to_owned()
            }
            ViewScope::Looking(_) => format!(
                "SELECT count(*) FROM messages m {}",
                what_a_search_matches(self.query.unread_only)
            ),
            ViewScope::Account(_) | ViewScope::Everything | ViewScope::Group(_) => {
                format!("SELECT count(*) FROM messages m {}", self.message_where())
            }
        }
    }

    /// What the epoch statement is asked with, which is not always what the others are asked with.
    ///
    /// A search is keyed by what somebody typed, and the epoch of a search does not depend on it:
    /// any folder changing can change what a search answers. Handing the statement a value it has
    /// no place for is refused, and it was — every search read failed until this told them apart.
    /// Whether there is nothing here to ask the index.
    ///
    /// Only a search can be in this state: punctuation holds no word, and the index refuses an
    /// empty question with an error rather than an empty answer.
    /// Whether this window has no one file to ask.
    ///
    /// A folder belongs to an account and so does an account; everything on the device, and a
    /// search, belong to all of them.
    pub(super) fn spans_accounts(&self) -> bool {
        matches!(
            self.query.scope,
            ViewScope::Everything | ViewScope::Group(_) | ViewScope::Looking(_)
        )
    }

    pub(super) fn asks_nothing(&self) -> bool {
        match &self.query.scope {
            ViewScope::Looking(typed) => lyonoyl_domain::search::asked_for(typed).is_none(),
            _ => false,
        }
    }

    pub(super) fn epoch_key(&self) -> Vec<Value> {
        match self.query.scope {
            ViewScope::Folder(_) | ViewScope::Account(_) => self.key(),
            ViewScope::Everything | ViewScope::Group(_) | ViewScope::Looking(_) => Vec::new(),
        }
    }

    /// A sum rather than a maximum: every folder counts its own changes, and a maximum would miss
    /// a change in any folder that is not already the busiest one.
    pub(super) fn epoch(&self) -> String {
        match self.query.scope {
            ViewScope::Folder(_) => "SELECT view_epoch FROM folders WHERE id = ?1".to_owned(),
            ViewScope::Account(_) => {
                "SELECT COALESCE(sum(view_epoch), 0) FROM folders WHERE account_id = ?1".to_owned()
            }
            // A search spans every folder, so any of them changing can change what it answers.
            ViewScope::Everything | ViewScope::Group(_) | ViewScope::Looking(_) => {
                "SELECT COALESCE(sum(view_epoch), 0) FROM folders".to_owned()
            }
        }
    }

    fn message_where(&self) -> String {
        let mut conditions = Vec::new();

        if matches!(self.query.scope, ViewScope::Account(_)) {
            conditions.push("m.account_id = ?1".to_owned());
        }

        conditions.push(arrived_in_an_inbox());

        if self.query.unread_only {
            conditions.push(unread_in_the_inbox());
        }

        format!("WHERE {}", conditions.join(" AND "))
    }

    fn limit_marker(&self) -> &'static str {
        match self.query.scope {
            ViewScope::Account(_) => "?2",
            _ => "?1",
        }
    }

    fn offset_marker(&self) -> &'static str {
        match self.query.scope {
            ViewScope::Account(_) => "?3",
            _ => "?2",
        }
    }
}

pub(super) fn read_row(row: &Row<'_>) -> Result<ListRow, ReadModelError> {
    let stored_id: i64 = row.get(0).map_err(|_| ReadModelError::Unavailable)?;

    let folder: Option<i64> = row.get(6).map_err(|_| ReadModelError::Unavailable)?;

    Ok(ListRow {
        // Absent when the list is showing messages: a message stands for itself.
        in_conversation: row.get::<_, Option<i64>>(7).ok().flatten().unwrap_or(1) as u32,
        conversation_id: row
            .get::<_, Option<i64>>(8)
            .ok()
            .flatten()
            .and_then(|stored| u64::try_from(stored).ok())
            .and_then(|value| MessageId::new(value).ok()),
        message_id: message_id(stored_id)?,
        // Filled in from the queue afterwards: the index knows nothing about what a person asked
        // for and the server has not been told.
        stopped: None,
        leaving: false,
        arriving: false,
        folder_id: folder
            .and_then(|stored| u64::try_from(stored).ok())
            .and_then(|value| FolderId::new(value).ok()),
        subject: row.get(1).map_err(|_| ReadModelError::Unavailable)?,
        sender: row.get(2).map_err(|_| ReadModelError::Unavailable)?,
        preview: row.get(3).map_err(|_| ReadModelError::Unavailable)?,
        received_at: row.get(4).map_err(|_| ReadModelError::Unavailable)?,
        unread: row.get(5).map_err(|_| ReadModelError::Unavailable)?,
    })
}

pub(super) fn message_id(stored: i64) -> Result<MessageId, ReadModelError> {
    u64::try_from(stored)
        .ok()
        .and_then(|value| MessageId::new(value).ok())
        .ok_or(ReadModelError::Corrupted)
}
