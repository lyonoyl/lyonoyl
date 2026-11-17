//! The newest piece of mail nobody has seen, across every account on the device.
//!
//! One question, asked once when a notification is about to be raised, and never while somebody is
//! looking at the window. It reads the same rows the list reads and counts the same mail the badge
//! counts — an inbox placement nobody has opened — so a notice can never describe mail the list
//! does not show.

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::ports::arrivals::{Arrivals, Arrived};
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{FolderId, MessageId};
use lyonoyl_domain::operation::OperationTarget;

/// Reads it out of every account's file and keeps the newest of the answers.
///
/// One statement per file rather than one across them: the mail is one file per account, and
/// SQLite does not join across two. Each file is asked for its own newest and the winner is picked
/// here, which is the same shape the sidebar and the badge already have.
pub struct SqliteArrivals {
    indexes: Vec<(i64, SqlitePool)>,
}

impl SqliteArrivals {
    pub fn spanning(indexes: Vec<(i64, SqlitePool)>) -> Self {
        Self { indexes }
    }
}

impl Arrivals for SqliteArrivals {
    fn newest_unseen(&self) -> Option<Arrived> {
        let mut newest: Option<(i64, Arrived)> = None;

        for (_, index) in &self.indexes {
            // An account whose file will not open is skipped rather than failing the lot. One
            // unreadable file among nine is a notice that names somebody else's mail, which is
            // better than a notice that names none.
            let Ok(connection) = index.get() else {
                continue;
            };

            // Where the account could put the message away is asked in the same statement, so the
            // places a notice offers are the places this account had when the notice was raised.
            let found = connection.query_row(
                &format!(
                    "SELECT m.sender, m.subject, m.preview, p.received_at, p.message_id, p.folder_id,
                            EXISTS (SELECT 1 FROM folders o
                                    WHERE o.account_id = f.account_id AND o.role = {trash}),
                            EXISTS (SELECT 1 FROM folders o
                                    WHERE o.account_id = f.account_id AND o.role = {junk})
                     FROM message_placements p
                     JOIN folders f ON f.id = p.folder_id
                     JOIN messages m ON m.id = p.message_id
                     WHERE f.role = {inbox} AND (p.flags & 1) = 0
                     ORDER BY p.received_at DESC, p.uid DESC
                     LIMIT 1",
                    inbox = FolderRole::Inbox.stored(),
                    trash = FolderRole::Trash.stored(),
                    junk = FolderRole::Junk.stored(),
                ),
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(3)?,
                        Arrived {
                            sender: row.get::<_, Option<String>>(0)?,
                            subject: row.get::<_, Option<String>>(1)?,
                            preview: row.get::<_, Option<String>>(2)?,
                            at: target(row.get::<_, i64>(4)?, row.get::<_, i64>(5)?),
                            has_trash: row.get::<_, bool>(6)?,
                            has_junk: row.get::<_, bool>(7)?,
                        },
                    ))
                },
            );

            let Ok((received_at, arrived)) = found else {
                // No unseen mail in this account's inbox, or a row that would not read. Neither is
                // worth stopping for: the next file may hold what the notice is about.
                continue;
            };

            if newest.as_ref().is_none_or(|(held, _)| received_at > *held) {
                newest = Some((received_at, arrived));
            }
        }

        newest.map(|(_, arrived)| arrived)
    }
}

/// The message and its inbox as the domain names them, or nothing where either number is not one.
fn target(message: i64, folder: i64) -> Option<OperationTarget> {
    let message = u64::try_from(message)
        .ok()
        .and_then(|id| MessageId::new(id).ok())?;
    let folder = u64::try_from(folder)
        .ok()
        .and_then(|id| FolderId::new(id).ok())?;

    Some(OperationTarget::new(message, folder))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::secure_storage::database_key::DatabaseKey;
    use crate::infrastructure::sqlite::connection::open_index;
    use crate::infrastructure::sqlite::migrations::migrate_index;

    /// An account's mail file with one inbox in it, and a way to put mail there.
    struct AnAccount {
        dir: std::path::PathBuf,
        pool: SqlitePool,
    }

    impl AnAccount {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "lyonoyl-arrivals-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).expect("temp dir");

            let path = dir.join("1.sqlite3");
            let pool = open_index(&path, &DatabaseKey::for_a_test()).expect("pool opens");
            {
                let mut connection = pool.get().expect("connection");
                migrate_index(&mut connection, &path.display().to_string()).expect("schema");
                connection
                    .execute_batch(&format!(
                        "INSERT INTO folders (id, account_id, name, role)
                         VALUES (1, 1, 'INBOX', {}), (2, 1, 'Archive', 6);",
                        FolderRole::Inbox.stored()
                    ))
                    .expect("two folders");
            }

            Self { dir, pool }
        }

        /// A folder kept for one kind of mail, beside the inbox and the archive.
        fn keeps(&self, id: i64, name: &str, role: FolderRole) {
            self.pool
                .get()
                .expect("connection")
                .execute(
                    "INSERT INTO folders (id, account_id, name, role) VALUES (?1, 1, ?2, ?3)",
                    rusqlite::params![id, name, role.stored()],
                )
                .expect("a folder");
        }

        /// One message, in the folder named, read or not.
        fn holds(&self, id: i64, folder: i64, at: i64, seen: bool, sender: &str, subject: &str) {
            let connection = self.pool.get().expect("connection");
            connection
                .execute(
                    "INSERT INTO messages (id, account_id, subject, sender, preview, received_at)
                     VALUES (?1, 1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![id, subject, sender, format!("opening of {id}"), at],
                )
                .expect("a message");
            connection
                .execute(
                    "INSERT INTO message_placements (folder_id, uid, message_id, received_at, flags)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![folder, id, id, at, i64::from(seen)],
                )
                .expect("a placement");
        }
    }

    impl Drop for AnAccount {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    #[test]
    fn the_newest_unread_message_in_an_inbox_is_the_one_a_notice_is_about() {
        let account = AnAccount::new("newest");
        account.holds(1, 1, 100, false, "Zeta Quorin", "older");
        account.holds(2, 1, 200, false, "Kappa Vandel", "newer");

        let found = SqliteArrivals::spanning(vec![(1, account.pool.clone())])
            .newest_unseen()
            .expect("something arrived");

        assert_eq!(found.sender.as_deref(), Some("Kappa Vandel"));
        assert_eq!(found.subject.as_deref(), Some("newer"));
        assert_eq!(found.preview.as_deref(), Some("opening of 2"));
    }

    #[test]
    fn mail_already_read_is_not_what_a_notice_is_about() {
        // The same mail the badge counts and the same mail the sidebar shows unread. A notice
        // about a message somebody has already opened is the application telling them about their
        // own reading.
        let account = AnAccount::new("read");
        account.holds(1, 1, 100, false, "Zeta Quorin", "not read");
        account.holds(2, 1, 200, true, "Kappa Vandel", "read already");

        let found = SqliteArrivals::spanning(vec![(1, account.pool.clone())])
            .newest_unseen()
            .expect("something arrived");

        assert_eq!(found.subject.as_deref(), Some("not read"));
    }

    #[test]
    fn mail_filed_anywhere_but_an_inbox_is_not_announced() {
        // What arrives in a folder of somebody's own is usually their own filing coming back from
        // the server, and being interrupted by one's own tidying is worse than not being told.
        let account = AnAccount::new("archive");
        account.holds(1, 2, 300, false, "Zeta Quorin", "filed away");

        let found = SqliteArrivals::spanning(vec![(1, account.pool.clone())]).newest_unseen();

        assert_eq!(found, None);
    }

    #[test]
    fn the_newest_across_accounts_wins_whichever_file_it_is_in() {
        // One statement per file, because SQLite does not join across two, so which one is newest
        // is decided here. Ordered the other way round from how the files are given, so a test
        // that only compared the first file's answer would pass on nothing.
        let first = AnAccount::new("first");
        let second = AnAccount::new("second");
        first.holds(1, 1, 100, false, "Zeta Quorin", "older account");
        second.holds(1, 1, 900, false, "Sigma Ordrin", "newer account");

        let found =
            SqliteArrivals::spanning(vec![(1, first.pool.clone()), (2, second.pool.clone())])
                .newest_unseen()
                .expect("something arrived");

        assert_eq!(found.sender.as_deref(), Some("Sigma Ordrin"));
    }

    #[test]
    fn the_message_a_notice_is_about_is_named_with_the_inbox_it_arrived_in() {
        // What the notice's buttons act on: the newest of the two, in the inbox, and not the one
        // filed away elsewhere.
        let account = AnAccount::new("named");
        account.holds(5, 1, 100, false, "Zeta Quorin", "older");
        account.holds(6, 1, 200, false, "Kappa Vandel", "newer");
        account.holds(9, 2, 300, false, "Sigma Ordrin", "filed away");

        let found = SqliteArrivals::spanning(vec![(1, account.pool.clone())])
            .newest_unseen()
            .expect("something arrived");

        assert_eq!(
            found.at,
            Some(OperationTarget::new(
                MessageId::new(6).expect("an identifier"),
                FolderId::new(1).expect("a folder"),
            ))
        );
    }

    #[test]
    fn where_an_account_can_put_a_message_away_is_said_only_where_it_can() {
        // An inbox and an archive alone: nowhere to throw anything, and no junk.
        let plain = AnAccount::new("plain");
        plain.holds(1, 1, 100, false, "Zeta Quorin", "arrived");

        let found = SqliteArrivals::spanning(vec![(1, plain.pool.clone())])
            .newest_unseen()
            .expect("something arrived");
        assert_eq!((found.has_trash, found.has_junk), (false, false));

        // Each place counted by itself, so neither can stand in for the other.
        for (role, trash, junk) in [
            (FolderRole::Trash, true, false),
            (FolderRole::Junk, false, true),
        ] {
            let account = AnAccount::new(&format!("{role:?}"));
            account.keeps(3, "Somewhere", role);
            account.holds(1, 1, 100, false, "Zeta Quorin", "arrived");

            let found = SqliteArrivals::spanning(vec![(1, account.pool.clone())])
                .newest_unseen()
                .expect("something arrived");
            assert_eq!((found.has_trash, found.has_junk), (trash, junk), "{role:?}");
        }
    }

    #[test]
    fn numbers_that_are_not_identifiers_name_no_message() {
        // Zero is no message and no folder, and a negative number is not one either: the notice
        // still says that mail arrived, and offers nothing to do with it.
        assert_eq!(target(0, 1), None);
        assert_eq!(target(1, 0), None);
        assert_eq!(target(-1, 1), None);
        assert!(target(1, 1).is_some());
    }

    #[test]
    fn an_empty_workspace_has_nothing_to_announce() {
        assert_eq!(SqliteArrivals::spanning(Vec::new()).newest_unseen(), None);
    }
}
