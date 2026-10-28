//! What the folder for sent mail shows from this device: messages sent here whose copy the
//! account's index does not hold yet.
//!
//! Read from both databases because the answer lives in both: a message is in the profile from the
//! moment somebody presses send, and its copy is in the index once the account has been read back.
//! Nothing here writes to either.

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::read_model::list::ReadModelError;
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::Rfc822MessageId;

/// How much of the text a row shows: as much as the list is given for a message read from a server.
const PREVIEW_CHARS: usize = 320;

/// One message sent from this device, as the list of sent mail shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentHere {
    /// Its row in the outbox.
    pub id: i64,
    /// The address it is from. It goes out with no name beside the address, so this is also what
    /// the list shows for its copy once the account has been read back.
    pub sender: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    /// The opening of the text, cut the way the list cuts the opening of a message it read.
    pub preview: Option<String>,
    /// The names of the files it carries, in the order they were attached.
    pub files: Vec<String>,
    /// When it was written: the date it goes out with, and the date its copy carries.
    pub written_at: i64,
    /// Whether no server has taken it yet.
    pub on_its_way: bool,
}

/// What one account sent from this device that its folder for sent mail does not hold yet, newest
/// first.
///
/// A message given back to the drafts is a draft, and one held is not sent: neither is here. A
/// message whose copy the index holds in the account's folder for sent mail is shown from there
/// and not twice; a copy of it anywhere else, the inbox of somebody who wrote to themselves among
/// them, is a different place and hides nothing.
pub fn of(
    profile: &SqlitePool,
    index: &SqlitePool,
    account_id: &str,
    number: i64,
) -> Result<Vec<SentHere>, ReadModelError> {
    let profile = profile.get().map_err(|_| ReadModelError::Unavailable)?;
    let rows = {
        let mut statement = profile
            .prepare(
                "SELECT id, message_id, sender, subject, body, written_at, state
                 FROM outbox
                 WHERE account_id = ?1
                   AND came_back_as IS NULL
                   AND state IN ('waiting', 'sending', 'sent')
                 ORDER BY written_at DESC, id DESC",
            )
            .map_err(|_| ReadModelError::Unavailable)?;
        let rows = statement
            .query_map([account_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|_| ReadModelError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ReadModelError::Unavailable)?;
        rows
    };
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let index = index.get().map_err(|_| ReadModelError::Unavailable)?;
    let mut sent = Vec::new();
    for (id, message_id, sender, subject, body, written_at, state) in rows {
        if in_sent(&index, number, &message_id)? {
            continue;
        }

        sent.push(SentHere {
            id,
            sender,
            to: recipients(&profile, id, "to")?,
            cc: recipients(&profile, id, "cc")?,
            bcc: recipients(&profile, id, "bcc")?,
            subject,
            preview: preview_of(&body),
            body,
            files: file_names(&profile, id)?,
            written_at,
            on_its_way: state != "sent",
        });
    }

    Ok(sent)
}

/// Whether the index holds a copy of the message in one of the account's folders for sent mail.
///
/// The outbox keeps the header's value in its angle brackets and the index keeps it without them,
/// so it is compared as the domain reads it. A value that does not read as one names nothing the
/// index could hold.
fn in_sent(
    index: &rusqlite::Connection,
    number: i64,
    message_id: &str,
) -> Result<bool, ReadModelError> {
    let Ok(message_id) = Rfc822MessageId::parse(message_id) else {
        return Ok(false);
    };

    index
        .query_row(
            "SELECT EXISTS (
                 SELECT 1
                 FROM messages m
                 JOIN message_placements p ON p.message_id = m.id
                 JOIN folders f ON f.id = p.folder_id
                 WHERE m.account_id = ?1 AND m.rfc822_message_id = ?2 AND f.role = ?3
             )",
            rusqlite::params![number, message_id.as_str(), FolderRole::Sent.stored()],
            |row| row.get(0),
        )
        .map_err(|_| ReadModelError::Unavailable)
}

fn recipients(
    profile: &rusqlite::Connection,
    id: i64,
    kind: &str,
) -> Result<Vec<String>, ReadModelError> {
    let mut statement = profile
        .prepare(
            "SELECT address FROM outbox_recipients
             WHERE outbox_id = ?1 AND kind = ?2
             ORDER BY address",
        )
        .map_err(|_| ReadModelError::Unavailable)?;

    let addresses = statement
        .query_map(rusqlite::params![id, kind], |row| row.get(0))
        .map_err(|_| ReadModelError::Unavailable)?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|_| ReadModelError::Unavailable)?;

    Ok(addresses)
}

fn file_names(profile: &rusqlite::Connection, id: i64) -> Result<Vec<String>, ReadModelError> {
    let mut statement = profile
        .prepare(
            "SELECT name FROM outbox_files
             WHERE outbox_id = ?1
             ORDER BY position, id",
        )
        .map_err(|_| ReadModelError::Unavailable)?;

    let names = statement
        .query_map([id], |row| row.get(0))
        .map_err(|_| ReadModelError::Unavailable)?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|_| ReadModelError::Unavailable)?;

    Ok(names)
}

/// One line, however the text was wrapped, and no more of it than the list is given.
fn preview_of(body: &str) -> Option<String> {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = collapsed.chars().take(PREVIEW_CHARS).collect();
    (!cut.is_empty()).then_some(cut)
}

#[cfg(test)]
mod tests;
