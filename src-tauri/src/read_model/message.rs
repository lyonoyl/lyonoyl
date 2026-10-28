use rusqlite::OptionalExtension;

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::infrastructure::sqlite::message_bodies::going_by;
use crate::read_model::list::ReadModelError;
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{BodyState, Flags, FolderId, MessageId, Uid, UidValidity};

/// Where a message sits, as the reading pane shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub folder_id: FolderId,
    pub folder_name: String,
    pub folder_role: FolderRole,
}

/// Everything the index holds about one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDetails {
    pub message_id: MessageId,
    pub subject: Option<String>,
    /// Who it is from, as the pane shows it.
    pub sender: Option<String>,
    /// Who it is from, as an answer has to address it. Never the display name beside it.
    pub sender_address: Option<String>,
    /// Who else it was addressed to, which is what an answer to everybody is built from.
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub received_at: i64,
    pub size: Option<u32>,
    pub unread: bool,
    pub body_state: BodyState,
    /// The name the message carries, and the conversation it said it belongs to.
    ///
    /// What an answer needs and nothing else uses: `In-Reply-To` names this message, `References`
    /// carries this chain with this name added. Absent for a message that arrived without a name of
    /// its own, and then it cannot be answered into a thread at all.
    pub named: Option<String>,
    pub chain: Vec<String>,
    /// Every folder the message appears in. One message, many folders.
    pub placements: Vec<Placement>,
}

/// A column holding a list back into the list it holds.
///
/// The other half of what the index does when it stores one. Nothing and an empty column are the
/// same answer — a message that named nobody — and neither may become a list of one blank entry.
fn split(column: Option<String>) -> Vec<String> {
    column
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Where one message sits on the server it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhereToRead {
    /// The account, by the number the index knows it under.
    pub account: i64,
    /// The mailbox, by the name its server uses.
    pub mailbox: String,
    /// The number the message has in that mailbox, which is meaningful there and nowhere else.
    pub uid: Uid,
    /// The generation that number was given in, which is the only one it means anything in.
    ///
    /// Nothing where the folder has none stored that this build can read, and then the number
    /// cannot be checked against the mailbox and is not used to read anything.
    pub uid_validity: Option<UidValidity>,
    /// How large the server said it is, when a pass recorded that. What lets a message too large
    /// to read be refused before it is read rather than after.
    pub size: Option<u64>,
}

/// The message carrying `rfc822_message_id` in the mailbox named `mailbox`, with that folder.
///
/// For the build made for screenshots, which opens on a message its scene names, and knows it by
/// the name the scene gave it. Nothing else asks a message by its name.
#[cfg(any(test, feature = "screenshots"))]
pub fn named_in(
    index: &SqlitePool,
    rfc822_message_id: &str,
    mailbox: &str,
) -> Result<Option<(MessageId, FolderId)>, ReadModelError> {
    let found: Option<(i64, i64)> = index
        .get()
        .map_err(|_| ReadModelError::Unavailable)?
        .query_row(
            "SELECT messages.id, folders.id FROM messages
                 JOIN message_placements ON message_placements.message_id = messages.id
                 JOIN folders ON folders.id = message_placements.folder_id
                 WHERE messages.rfc822_message_id = ?1 AND folders.name = ?2",
            [rfc822_message_id, mailbox],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| ReadModelError::Unavailable)?;

    let Some((message, folder)) = found else {
        return Ok(None);
    };
    let message = u64::try_from(message)
        .ok()
        .and_then(|value| MessageId::new(value).ok())
        .ok_or(ReadModelError::Corrupted)?;
    let folder = u64::try_from(folder)
        .ok()
        .and_then(|value| FolderId::new(value).ok())
        .ok_or(ReadModelError::Corrupted)?;

    Ok(Some((message, folder)))
}

pub struct MessageReadModel {
    pool: SqlitePool,
}

impl MessageReadModel {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Where a message can be read from: which account, which mailbox, and the number it has there.
    ///
    /// Any placement will do — the same message in two folders is the same bytes — so the lowest
    /// folder identifier is taken, which makes the answer the same every time it is asked. One in a
    /// folder with a generation stored comes before one without, because only that one can be
    /// read.
    ///
    /// A row that handed its copies to the message it turned out to be is read where that message
    /// is: the window may still hold the row, open in the reader.
    pub fn where_to_read(
        &self,
        message_id: MessageId,
    ) -> Result<Option<WhereToRead>, ReadModelError> {
        let connection = self.pool.get().map_err(|_| ReadModelError::Unavailable)?;
        let going_by = going_by(&connection, message_id.get() as i64)
            .map_err(|_| ReadModelError::Unavailable)?;

        connection
            .query_row(
                "SELECT f.account_id, f.name, p.uid, m.rfc822_size, f.uid_validity
                 FROM message_placements p
                 JOIN folders f ON f.id = p.folder_id
                 JOIN messages m ON m.id = p.message_id
                 WHERE p.message_id = ?1
                 ORDER BY f.uid_validity IS NULL, f.id
                 LIMIT 1",
                [going_by],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| ReadModelError::Unavailable)?
            .map(|(account, mailbox, uid, size, uid_validity)| {
                let uid = u32::try_from(uid)
                    .ok()
                    .and_then(|value| Uid::new(value).ok())
                    .ok_or(ReadModelError::Corrupted)?;

                Ok(WhereToRead {
                    account,
                    mailbox,
                    uid,
                    uid_validity: uid_validity
                        .and_then(|value| u32::try_from(value).ok())
                        .and_then(|value| UidValidity::new(value).ok()),
                    size: size.and_then(|size| u64::try_from(size).ok()),
                })
            })
            .transpose()
    }

    /// Reads one message, or `None` when it is not in the index.
    ///
    /// Under the identifier it was asked by, and as the message it goes by now: a row that handed
    /// its copies to the message it turned out to be is that message — its name, which an answer
    /// to it carries, and the folders it is in among it.
    pub fn get_message(
        &self,
        message_id: MessageId,
    ) -> Result<Option<MessageDetails>, ReadModelError> {
        let connection = self.pool.get().map_err(|_| ReadModelError::Unavailable)?;
        let id = going_by(&connection, message_id.get() as i64)
            .map_err(|_| ReadModelError::Unavailable)?;

        let row = connection
            .query_row(
                "SELECT subject, sender, sender_address, received_at, rfc822_size, flags,
                        body_state, rfc822_message_id, references_chain, recipients_to,
                        recipients_cc
                 FROM messages WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| ReadModelError::Unavailable)?;

        let Some((
            subject,
            sender,
            sender_address,
            received_at,
            size,
            flags,
            body_state,
            named,
            chain,
            to,
            cc,
        )) = row
        else {
            return Ok(None);
        };

        let mut statement = connection
            .prepare(
                "SELECT f.id, f.name, f.role
                 FROM message_placements p
                 JOIN folders f ON f.id = p.folder_id
                 WHERE p.message_id = ?1
                 ORDER BY f.role, f.name",
            )
            .map_err(|_| ReadModelError::Unavailable)?;

        let rows = statement
            .query_map([id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|_| ReadModelError::Unavailable)?;

        let mut placements = Vec::new();
        for row in rows {
            let (folder_id, folder_name, role) = row.map_err(|_| ReadModelError::Unavailable)?;
            placements.push(Placement {
                folder_id: u64::try_from(folder_id)
                    .ok()
                    .and_then(|value| FolderId::new(value).ok())
                    .ok_or(ReadModelError::Corrupted)?,
                folder_name,
                folder_role: FolderRole::from_stored(role),
            });
        }

        Ok(Some(MessageDetails {
            named,
            chain: split(chain),
            to: split(to),
            cc: split(cc),
            message_id,
            subject,
            sender,
            sender_address,
            received_at,
            size: size.and_then(|value| u32::try_from(value).ok()),
            unread: Flags::from_bits(u32::try_from(flags).unwrap_or(0)).is_unread(),
            body_state: BodyState::from_stored(body_state),
            placements,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::fake_provider::SeededIndex;
    use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
    use crate::ports::mail_index::MailIndex;
    use lyonoyl_domain::message::{FlagScope, ModSeq, Rfc822MessageId, UidValidity};
    use lyonoyl_domain::sync::{
        ArrivedMessage, FolderDelta, MessageEnvelope, SyncCursor, VanishedReport,
    };

    fn folder(id: u64) -> FolderId {
        FolderId::new(id).expect("non-zero")
    }

    fn message(id: u64) -> MessageId {
        MessageId::new(id).expect("non-zero")
    }

    #[test]
    fn reads_what_the_index_holds_about_a_message() {
        let db = SeededIndex::with("message-details", folder(1), 5);
        let details = MessageReadModel::new(db.pool.clone())
            .get_message(message(1))
            .expect("readable")
            .expect("the message is in the index");

        assert_eq!(details.subject, Some("Message 1".to_owned()));
        assert_eq!(details.sender, Some("sender1@fake.invalid".to_owned()));
        assert_eq!(details.size, Some(2049));
        assert_eq!(details.placements.len(), 1);
        assert_eq!(details.placements[0].folder_role, FolderRole::Inbox);
    }

    /// Puts one message in the index the way a pass does, with what it said about its conversation.
    ///
    /// Through the adapter rather than by hand: what is being checked is that the chain survives
    /// the trip — stored by the pass, read back by the pane — and a fixture that wrote the column
    /// itself would prove only that this test can spell it.
    fn arrives(db: &SeededIndex, named: Option<&str>, references: &[&str]) {
        arrives_addressed(
            db,
            named,
            references,
            &["you@example.com", "kappa@example.net"],
            &["sigma@example.org"],
        );
    }

    /// The same, with a say in who the message was addressed to.
    fn arrives_addressed(
        db: &SeededIndex,
        named: Option<&str>,
        references: &[&str],
        to: &[&str],
        cc: &[&str],
    ) {
        let delta = FolderDelta {
            folder_id: folder(1),
            uid_validity: UidValidity::new(7).expect("non-zero"),
            covered_from: None,
            stopped_short: false,
            arrived: vec![ArrivedMessage {
                uid: Uid::new(10).expect("non-zero"),
                modseq: Some(ModSeq::new(10)),
                envelope: MessageEnvelope {
                    rfc822_message_id: named
                        .map(|value| Rfc822MessageId::parse(value).expect("well formed")),
                    subject: Some("About Tuesday".to_owned()),
                    // A display name and the address behind it, which is the ordinary shape of a
                    // `From` header and the one a fixture must not flatten: only one of the two
                    // can be answered at.
                    sender: Some("Zeta Quorin".to_owned()),
                    sender_address: Some("zeta@example.com".to_owned()),
                    to: to.iter().map(|value| (*value).to_owned()).collect(),
                    cc: cc.iter().map(|value| (*value).to_owned()).collect(),
                    received_at: 1_700_000_000,
                    rfc822_size: Some(1024),
                    preview: None,
                    references: references
                        .iter()
                        .map(|value| Rfc822MessageId::parse(value).expect("well formed"))
                        .collect(),
                    flags: Flags::NONE,
                },
            }],
            flag_updates: Vec::new(),
            vanished: VanishedReport::Unknown,
        };

        SqliteMailIndex::new(db.pool.clone())
            .apply(&delta, &[], &SyncCursor::new("1"), FlagScope::Message)
            .expect("the delta applies");
    }

    #[test]
    fn a_message_comes_back_with_the_conversation_it_said_it_belongs_to() {
        // What an answer needs, and the one thing a pass used to parse and throw away. Oldest
        // first, in the order the message wrote them: `References` is a path back through the
        // thread, and a reordered one points somewhere else.
        let db = SeededIndex::with("message-chain", folder(1), 0);
        arrives(
            &db,
            Some("<third@example.com>"),
            &["<first@example.com>", "<second@example.com>"],
        );

        let details = MessageReadModel::new(db.pool.clone())
            .get_message(message(1))
            .expect("readable")
            .expect("the message is in the index");

        // The name is for the pane to show, the address is what an answer is sent to. A pane that
        // had only the first of them could offer an answer nobody can send.
        assert_eq!(details.sender, Some("Zeta Quorin".to_owned()));
        assert_eq!(details.sender_address, Some("zeta@example.com".to_owned()));
        // Who else it was for, which is what an answer to everybody is built from. Both lists
        // survive the trip and keep the order the message wrote them in.
        assert_eq!(
            details.to,
            vec!["you@example.com".to_owned(), "kappa@example.net".to_owned()]
        );
        assert_eq!(details.cc, vec!["sigma@example.org".to_owned()]);
        assert_eq!(details.named, Some("third@example.com".to_owned()));
        assert_eq!(
            details.chain,
            vec![
                "first@example.com".to_owned(),
                "second@example.com".to_owned()
            ]
        );
    }

    #[test]
    fn a_message_that_starts_a_conversation_has_a_name_and_no_chain() {
        let db = SeededIndex::with("message-chain-empty", folder(1), 0);
        arrives_addressed(&db, Some("<only@example.com>"), &[], &[], &[]);

        let details = MessageReadModel::new(db.pool.clone())
            .get_message(message(1))
            .expect("readable")
            .expect("the message is in the index");

        assert_eq!(details.named, Some("only@example.com".to_owned()));
        // A message that named nobody, and a message read before the column existed, are the same
        // answer here: an empty list, never a list of one blank entry.
        assert!(details.to.is_empty());
        assert!(details.cc.is_empty());
        // Empty rather than a chain of one blank name: nothing was stored, and splitting nothing
        // on newlines must not invent an entry.
        assert!(details.chain.is_empty());
    }

    #[test]
    fn a_message_with_no_name_of_its_own_cannot_be_answered_into_a_thread() {
        // It is still a message, and still readable. `In-Reply-To` has nothing to point at, and
        // the pane says so by carrying no name rather than by carrying an empty one.
        let db = SeededIndex::with("message-chain-unnamed", folder(1), 0);
        arrives(&db, None, &["<first@example.com>"]);

        let details = MessageReadModel::new(db.pool.clone())
            .get_message(message(1))
            .expect("readable")
            .expect("the message is in the index");

        assert_eq!(details.named, None);
        assert_eq!(details.chain, vec!["first@example.com".to_owned()]);
    }

    #[test]
    fn says_where_a_message_is_and_how_large_the_server_said_it_is() {
        let db = SeededIndex::with("message-where-to-read", folder(1), 5);
        let place = MessageReadModel::new(db.pool.clone())
            .where_to_read(message(1))
            .expect("readable")
            .expect("the message is in the index");

        assert_eq!(place.mailbox, "INBOX");
        // Carried so that a message too large to read is refused before it is fetched, rather
        // than after the whole of it is in memory.
        assert_eq!(place.size, Some(2049));
        // And the generation its number was given in, which is the only one the number means
        // anything in: a mailbox renumbered since has given it to somebody else's message.
        assert_eq!(place.uid_validity, UidValidity::new(7).ok());
    }

    #[test]
    fn a_copy_that_can_be_read_is_preferred_to_one_whose_numbering_is_unknown() {
        // The seeded folder was read by a pass, so it has a generation; the second one was never
        // read and has none. Lower in number, it would be the one chosen — and a number that cannot
        // be checked against the mailbox is one nothing reads the message by.
        let db = SeededIndex::with("message-where-generation", folder(2), 1);
        db.add_folder(1, "Archive", FolderRole::Archive);
        db.place_existing_message(1, 500, 1);

        let place = MessageReadModel::new(db.pool.clone())
            .where_to_read(message(1))
            .expect("readable")
            .expect("the message is in the index");

        assert_eq!(place.mailbox, "INBOX");
        assert_eq!(place.uid_validity, UidValidity::new(7).ok());
    }

    #[test]
    fn says_that_only_the_envelope_is_stored() {
        let db = SeededIndex::with("message-body-state", folder(1), 1);
        let details = MessageReadModel::new(db.pool.clone())
            .get_message(message(1))
            .expect("readable")
            .expect("the message is in the index");

        assert_eq!(details.body_state, BodyState::HeadersOnly);
        assert!(!details.body_state.has_text());
    }

    #[test]
    fn lists_every_folder_the_message_sits_in() {
        let db = SeededIndex::with("message-placements", folder(1), 1);
        db.add_folder(2, "Archive", FolderRole::Archive);
        db.place_existing_message(2, 500, 1);

        let details = MessageReadModel::new(db.pool.clone())
            .get_message(message(1))
            .expect("readable")
            .expect("the message is in the index");

        assert_eq!(details.placements.len(), 2);
        assert_eq!(details.placements[0].folder_role, FolderRole::Inbox);
        assert_eq!(details.placements[1].folder_role, FolderRole::Archive);
    }

    #[test]
    fn a_message_that_is_not_there_reads_as_nothing() {
        let db = SeededIndex::with("message-missing", folder(1), 1);
        assert_eq!(
            MessageReadModel::new(db.pool.clone())
                .get_message(message(9_999))
                .expect("readable"),
            None
        );
    }

    #[test]
    fn unread_is_read_from_the_stored_flags() {
        // The generator marks every fifth message unread.
        let db = SeededIndex::with("message-unread", folder(1), 10);
        let model = MessageReadModel::new(db.pool.clone());

        let unread = model
            .get_message(message(5))
            .expect("readable")
            .expect("present");
        let read = model
            .get_message(message(1))
            .expect("readable")
            .expect("present");

        assert!(unread.unread);
        assert!(!read.unread);
    }
}
