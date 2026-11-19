//! The people an account has written to, for the composer to offer as an address is typed.
//!
//! Read from the account's own index, and kept between keys: a key is answered from what was
//! gathered, and everything is gathered again once the index holds a message it did not hold
//! then. Gathering reads every message the account has; a key reads only what is kept, which is
//! what lets an answer arrive before the next key does.
//!
//! Who is offered, and in what order, is the domain's ([`lyonoyl_domain::correspondent`]): this
//! reads what it decides from.

use std::sync::{Arc, Mutex, PoisonError};

use rusqlite::Connection;

use crate::infrastructure::sqlite::account_groups::folded;
use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::read_model::list::ReadModelError;
use lyonoyl_domain::correspondent::{offered, Correspondent, Gathering};
use lyonoyl_domain::folder::FolderRole;

/// What one account's index was last gathered into.
///
/// One account at a time: a message is written from one account, and somebody switching between
/// two pays for a gathering at each switch rather than every account's people being kept for as
/// long as the application runs.
#[derive(Debug, Default)]
pub struct Correspondents {
    kept: Mutex<Option<Kept>>,
}

#[derive(Debug)]
struct Kept {
    account_id: String,
    /// The newest message the index held when this was gathered.
    newest: i64,
    people: Arc<Vec<Correspondent>>,
}

impl Correspondents {
    /// Who the composer offers for what is being typed into a line of addresses written from this
    /// account.
    ///
    /// Asked with nothing typed, it offers nobody and gathers all the same: the window asks so as a
    /// line of addresses takes the focus, and the first key is then answered from what is kept
    /// rather than after a gathering. A message deleted since the gathering is still counted until
    /// the next message arrives: what that costs is somebody offered once more who was written to
    /// once, against reading every message again at every key.
    pub fn offered(
        &self,
        account_id: &str,
        index: &SqlitePool,
        typed: &str,
    ) -> Result<Vec<Correspondent>, ReadModelError> {
        let connection = index.get().map_err(|_| ReadModelError::Unavailable)?;
        let newest: i64 = connection
            .query_row("SELECT COALESCE(MAX(id), 0) FROM messages", [], |row| {
                row.get(0)
            })
            .map_err(|_| ReadModelError::Unavailable)?;

        // Held while gathering, so keys typed meanwhile wait for one gathering rather than each
        // starting its own.
        let people = {
            let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
            match kept.as_ref() {
                Some(kept) if kept.account_id == account_id && kept.newest == newest => {
                    Arc::clone(&kept.people)
                }
                _ => {
                    let people = Arc::new(gathered(&connection)?);
                    *kept = Some(Kept {
                        account_id: account_id.to_owned(),
                        newest,
                        people: Arc::clone(&people),
                    });
                    people
                }
            }
        };

        Ok(offered(&people, typed, folded)
            .into_iter()
            .cloned()
            .collect())
    }
}

/// Everybody the account's sent mail names, with the names their own mail gives them.
fn gathered(connection: &Connection) -> Result<Vec<Correspondent>, ReadModelError> {
    let mut gathering = Gathering::default();

    // Every message in a folder for sent mail, once however many of them hold it.
    let mut sent = connection
        .prepare(
            "SELECT recipients_to, recipients_cc, received_at
             FROM messages
             WHERE id IN (
                 SELECT p.message_id
                 FROM message_placements p
                 JOIN folders f ON f.id = p.folder_id
                 WHERE f.role = ?1
             )",
        )
        .map_err(|_| ReadModelError::Unavailable)?;
    let mut rows = sent
        .query([FolderRole::Sent.stored()])
        .map_err(|_| ReadModelError::Unavailable)?;
    while let Some(row) = rows.next().map_err(|_| ReadModelError::Unavailable)? {
        let to: Option<String> = row.get(0).map_err(|_| ReadModelError::Corrupted)?;
        let cc: Option<String> = row.get(1).map_err(|_| ReadModelError::Corrupted)?;
        let at: i64 = row.get(2).map_err(|_| ReadModelError::Corrupted)?;

        gathering.sent(entries(to.as_deref()), entries(cc.as_deref()), at);
    }

    // The names people give themselves, from the messages they wrote. A message with no name of
    // its own carries its address in `sender`, which the domain does not take for a name.
    let mut named = connection
        .prepare(
            "SELECT sender_address, sender, received_at
             FROM messages
             WHERE sender_address IS NOT NULL AND sender IS NOT NULL",
        )
        .map_err(|_| ReadModelError::Unavailable)?;
    let mut rows = named.query([]).map_err(|_| ReadModelError::Unavailable)?;
    while let Some(row) = rows.next().map_err(|_| ReadModelError::Unavailable)? {
        let address: String = row.get(0).map_err(|_| ReadModelError::Corrupted)?;
        let name: String = row.get(1).map_err(|_| ReadModelError::Corrupted)?;
        let at: i64 = row.get(2).map_err(|_| ReadModelError::Corrupted)?;

        gathering.named(&address, &name, at);
    }

    Ok(gathering.people(folded))
}

/// The entries of a column holding a list: one per line, as the index writes them.
fn entries(column: Option<&str>) -> impl Iterator<Item = &str> {
    column.unwrap_or_default().split('\n')
}

#[cfg(test)]
mod tests;
