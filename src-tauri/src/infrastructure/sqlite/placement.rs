//! Where a message sits, read from the index at the moment something is about to be done to it,
//! and the flags a server has just confirmed, written back onto the copy they were set on.
//!
//! One database: everything an instruction needs to be turned into an address is in the index,
//! because the index is what the last pass wrote — and the one thing written here is what the
//! server answered about that address.

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::infrastructure::sqlite::message_bodies::going_by;
use crate::ports::placement::{PlacementError, Placements, Position};
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{Flags, FolderId, MessageId, Uid, UidValidity};

pub struct SqlitePlacements {
    index: SqlitePool,
}

impl SqlitePlacements {
    pub fn new(index: SqlitePool) -> Self {
        Self { index }
    }
}

impl Placements for SqlitePlacements {
    fn where_it_sits(
        &self,
        folder: FolderId,
        message: MessageId,
    ) -> Result<Option<Position>, PlacementError> {
        let connection = self.index.get().map_err(|_| PlacementError::Unavailable)?;

        // An instruction asked about a row that has since handed its copies to the message it
        // turned out to be is about those copies: the row was the only one of the message there
        // was when somebody acted on it.
        let message =
            going_by(&connection, message.get() as i64).map_err(|_| PlacementError::Unavailable)?;

        // The folder's generation comes out with the number, and a folder that has none yet is a
        // folder no pass has finished: there is a number stored and nothing that says what it
        // counts, which is not an address.
        let found: Option<(String, i64, Option<i64>)> = connection
            .query_row(
                "SELECT f.name, p.uid, f.uid_validity
                 FROM message_placements p
                 JOIN folders f ON f.id = p.folder_id
                 WHERE p.folder_id = ?1 AND p.message_id = ?2",
                rusqlite::params![folder.get() as i64, message],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(PlacementError::Unavailable),
            })?;

        // A number the index holds but this build cannot read is a message with no address, which
        // is the same answer as one that is not there: it stops and a person is shown it.
        Ok(found.and_then(|(mailbox, uid, generation)| {
            let uid = u32::try_from(uid).ok().and_then(|uid| Uid::new(uid).ok())?;
            let uid_validity = u32::try_from(generation?)
                .ok()
                .and_then(|value| UidValidity::new(value).ok())?;

            Some(Position {
                mailbox,
                uid,
                uid_validity,
            })
        }))
    }

    fn mailbox_of(&self, folder: FolderId) -> Result<Option<String>, PlacementError> {
        self.index
            .get()
            .map_err(|_| PlacementError::Unavailable)?
            .query_row(
                "SELECT name FROM folders WHERE id = ?1",
                [folder.get() as i64],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(PlacementError::Unavailable),
            })
    }

    fn folder_for(
        &self,
        account: i64,
        role: FolderRole,
    ) -> Result<Option<FolderId>, PlacementError> {
        let found: Option<i64> = self
            .index
            .get()
            .map_err(|_| PlacementError::Unavailable)?
            .query_row(
                // The oldest one, so an account that somehow grew two answers the same way every
                // time rather than moving mail into whichever the query happened to see first.
                "SELECT id FROM folders
                 WHERE account_id = ?1 AND role = ?2
                 ORDER BY id ASC
                 LIMIT 1",
                rusqlite::params![account, role.stored()],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(PlacementError::Unavailable),
            })?;

        Ok(found
            .and_then(|id| u64::try_from(id).ok())
            .and_then(|id| FolderId::new(id).ok()))
    }

    fn confirmed_flags(
        &self,
        folder: FolderId,
        at: &Position,
        add: Flags,
        remove: Flags,
    ) -> Result<bool, PlacementError> {
        let mut connection = self.index.get().map_err(|_| PlacementError::Unavailable)?;

        // `IMMEDIATE`, because it writes (invariant 6), and the epoch in the same transaction as the
        // flags, the way a pass moves it: a list filtered to unread mail holds a different set once
        // one of them is read, and the epoch is what tells it so. The cursor is not touched — it
        // says how far a pass has read, and this is not a pass.
        let writing = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| PlacementError::Unavailable)?;

        let changed = writing
            .execute(
                "UPDATE message_placements
                 SET flags = (flags | ?3) & ~?4
                 WHERE folder_id = ?1 AND uid = ?2
                   AND flags <> ((flags | ?3) & ~?4)
                   AND EXISTS (SELECT 1 FROM folders WHERE id = ?1 AND uid_validity = ?5)",
                rusqlite::params![
                    folder.get() as i64,
                    i64::from(at.uid.get()),
                    i64::from(add.bits()),
                    i64::from(remove.bits()),
                    i64::from(at.uid_validity.get()),
                ],
            )
            .map_err(|_| PlacementError::Unavailable)?;

        if changed > 0 {
            writing
                .execute(
                    "UPDATE folders SET view_epoch = view_epoch + 1 WHERE id = ?1",
                    [folder.get() as i64],
                )
                .map_err(|_| PlacementError::Unavailable)?;
        }

        writing.commit().map_err(|_| PlacementError::Unavailable)?;

        Ok(changed > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::fake_provider::SeededIndex;

    fn folder() -> FolderId {
        FolderId::new(1).expect("non-zero")
    }

    #[test]
    fn a_message_in_the_folder_has_a_mailbox_and_a_number() {
        let db = SeededIndex::with("placement-there", folder(), 3);
        let places = SqlitePlacements::new(db.pool.clone());

        let sitting = places
            .where_it_sits(folder(), MessageId::new(1).expect("non-zero"))
            .expect("readable")
            .expect("the seed put it there");

        assert_eq!(sitting.mailbox, "INBOX");
        assert!(sitting.uid.get() > 0);
        assert!(
            sitting.uid_validity.get() > 0,
            "a number without the generation it counts under is not an address"
        );
    }

    #[test]
    fn a_folder_no_pass_has_finished_has_no_address_in_it() {
        // The number is there and what it counts is not. Acting on it would act on whichever
        // message holds that number now, which is a message nobody asked about.
        let db = SeededIndex::with("placement-ungenerated", folder(), 3);
        let places = SqlitePlacements::new(db.pool.clone());

        db.pool
            .get()
            .expect("a connection")
            .execute("UPDATE folders SET uid_validity = NULL WHERE id = ?1", [1])
            .expect("the generation is forgotten");

        assert_eq!(
            places
                .where_it_sits(folder(), MessageId::new(1).expect("non-zero"))
                .expect("readable"),
            None
        );
    }

    #[test]
    fn a_message_that_is_not_in_that_folder_has_no_address() {
        // Which is what stops an instruction rather than resolving it to the nearest message.
        let db = SeededIndex::with("placement-elsewhere", folder(), 3);
        let places = SqlitePlacements::new(db.pool.clone());

        assert_eq!(
            places
                .where_it_sits(
                    FolderId::new(99).expect("non-zero"),
                    MessageId::new(1).expect("non-zero")
                )
                .expect("readable"),
            None
        );
        assert_eq!(
            places
                .where_it_sits(folder(), MessageId::new(9_999).expect("non-zero"))
                .expect("readable"),
            None
        );
    }

    #[test]
    fn a_folder_is_named_the_way_its_server_names_it() {
        let db = SeededIndex::with("placement-name", folder(), 1);
        let places = SqlitePlacements::new(db.pool.clone());

        assert_eq!(
            places.mailbox_of(folder()).expect("readable"),
            Some("INBOX".to_owned())
        );
        assert_eq!(
            places
                .mailbox_of(FolderId::new(99).expect("non-zero"))
                .expect("readable"),
            None
        );
    }

    #[test]
    fn an_account_with_no_trash_says_so_rather_than_offering_another_folder() {
        // The one case with no good answer, and the answer must not be "some other folder".
        let db = SeededIndex::with("placement-trash", folder(), 1);
        let places = SqlitePlacements::new(db.pool.clone());

        assert_eq!(
            places.folder_for(1, FolderRole::Trash).expect("readable"),
            None
        );
        assert_eq!(
            places.folder_for(1, FolderRole::Inbox).expect("readable"),
            Some(folder()),
            "the inbox the seed made is the inbox"
        );
    }

    /// Where message 5 sits in the seeded folder, which is one the generator leaves unread.
    fn the_unread_one(places: &SqlitePlacements) -> Position {
        places
            .where_it_sits(folder(), MessageId::new(5).expect("non-zero"))
            .expect("readable")
            .expect("the seed put it there")
    }

    fn flags_of(db: &SeededIndex, message: i64) -> i64 {
        db.pool
            .get()
            .expect("a connection")
            .query_row(
                "SELECT flags FROM message_placements WHERE folder_id = 1 AND message_id = ?1",
                [message],
                |row| row.get(0),
            )
            .expect("readable")
    }

    fn the_folder(db: &SeededIndex) -> (i64, Option<String>) {
        db.pool
            .get()
            .expect("a connection")
            .query_row(
                "SELECT view_epoch, sync_cursor FROM folders WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("readable")
    }

    #[test]
    fn a_flag_the_server_confirmed_is_written_onto_that_copy_and_no_other() {
        let db = SeededIndex::with("confirmed-flag", folder(), 10);
        let places = SqlitePlacements::new(db.pool.clone());
        let at = the_unread_one(&places);
        let neighbour = flags_of(&db, 10);
        let (epoch, cursor) = the_folder(&db);

        assert!(places
            .confirmed_flags(folder(), &at, Flags::SEEN, Flags::NONE)
            .expect("written"));

        assert_eq!(flags_of(&db, 5) & 1, 1, "read, as the server said");
        assert_eq!(flags_of(&db, 10), neighbour, "and nothing else was touched");
        assert_eq!(
            the_folder(&db),
            (epoch + 1, cursor),
            "the epoch moves as a pass would move it, and the cursor does not"
        );
    }

    #[test]
    fn a_flag_already_there_changes_nothing_and_moves_no_epoch() {
        let db = SeededIndex::with("confirmed-twice", folder(), 10);
        let places = SqlitePlacements::new(db.pool.clone());
        let at = the_unread_one(&places);

        places
            .confirmed_flags(folder(), &at, Flags::SEEN, Flags::NONE)
            .expect("written");
        let settled = the_folder(&db);

        assert!(!places
            .confirmed_flags(folder(), &at, Flags::SEEN, Flags::NONE)
            .expect("asked"));
        assert_eq!(the_folder(&db), settled);
    }

    #[test]
    fn a_folder_that_renumbered_since_is_not_written_to() {
        // The number now belongs to another message, and the flag the server confirmed was about
        // the one that had it before.
        let db = SeededIndex::with("confirmed-renumbered", folder(), 10);
        let places = SqlitePlacements::new(db.pool.clone());
        let at = the_unread_one(&places);
        db.pool
            .get()
            .expect("a connection")
            .execute(
                "UPDATE folders SET uid_validity = uid_validity + 1 WHERE id = 1",
                [],
            )
            .expect("renumbered");

        assert!(!places
            .confirmed_flags(folder(), &at, Flags::SEEN, Flags::NONE)
            .expect("asked"));
        assert_eq!(flags_of(&db, 5) & 1, 0);
    }

    #[test]
    fn a_flag_taken_away_is_taken_away() {
        let db = SeededIndex::with("confirmed-unread", folder(), 10);
        let places = SqlitePlacements::new(db.pool.clone());
        let at = places
            .where_it_sits(folder(), MessageId::new(1).expect("non-zero"))
            .expect("readable")
            .expect("there");

        places
            .confirmed_flags(folder(), &at, Flags::NONE, Flags::SEEN)
            .expect("written");

        assert_eq!(flags_of(&db, 1) & 1, 0);
    }
}
