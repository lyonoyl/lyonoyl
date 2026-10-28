//! Messages kept in the index, which is the database that may be thrown away.

use rusqlite::OptionalExtension;

use crate::infrastructure::secure_storage::message_key::MessageKey;
use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::infrastructure::sqlite::threads;
use crate::ports::message_bodies::{MessageBodies, MessageBodyError};
use lyonoyl_domain::message::{MessageId, Rfc822MessageId};

/// The largest message this keeps a copy of.
///
/// A mailbox holds the occasional message with a video in it, and a local copy of one is a local
/// copy of a video. Above this the message is still readable — it is fetched whenever it is
/// opened — and the store stays a store of mail rather than of files somebody was sent once.
const LARGEST_KEPT: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct SqliteMessageBodies {
    pool: SqlitePool,
    /// What the bytes are sealed under. Handed in rather than fetched here: which store this
    /// device has — a system keychain, or a file behind a master password — is a question the
    /// composition root answers once, and asking it per message would raise a dialog per message
    /// on macOS.
    key: MessageKey,
}

impl SqliteMessageBodies {
    pub fn new(pool: SqlitePool, key: MessageKey) -> Self {
        Self { pool, key }
    }

    /// Says the message was read whole, and nothing else.
    ///
    /// For bytes that are not a message, or that broke the parser: there is nothing to fill in from
    /// them, and the reading still happened. What the pass asks before fetching is whether it has read this message, so
    /// a reading that produced nothing has to answer that question too.
    fn mark_read_whole(&self, message: MessageId) -> Result<(), MessageBodyError> {
        self.pool
            .get()
            .map_err(|_| MessageBodyError::Unavailable)?
            .execute(
                "UPDATE messages SET read_whole_at = unixepoch() WHERE id = ?1",
                [message.get() as i64],
            )
            .map(|_| ())
            .map_err(|_| MessageBodyError::Unavailable)
    }

    /// `fill_in_missing`, with what reads the message handed in: the parser, everywhere but in a
    /// test that needs one to fail.
    fn fill_in_from(
        &self,
        message: MessageId,
        raw: &[u8],
        read: impl FnOnce(&[u8]) -> Option<Addressed>,
    ) -> Result<(), MessageBodyError> {
        // The parser runs here, on whoever asked — the round that reads mail whole for the index,
        // or a person opening a message — rather than behind the guards the reading path has, and
        // the bytes are a stranger's. A panic in it ends this reading and nothing else. Uncaught,
        // it ends the pass over the whole account on every round, and the message never opens.
        let addressed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(raw)));

        let Ok(Some(addressed)) = addressed else {
            // Bytes that are not a message, or that broke the parser, were still read, and reading
            // them again will not change what they are. Recorded anyway, or the pass asks for this
            // message every round for ever — which is the fault this column exists to end, coming
            // back through another door.
            return self.mark_read_whole(message);
        };

        // What the message says, so a search finds it by a word from anywhere in it rather than
        // from its opening alone. Taken behind the guards that every whole message goes through:
        // this converts markup, and a message is written by whoever sent it.
        let read = crate::infrastructure::html::text_to_index(raw);

        // A message nobody could look at is not a message with nothing to say, and only one of the
        // two is a thing to write down. Marking the reading done when it did not happen is what
        // took a message out of every search for good: a message is read whole exactly once, so a
        // busy minute was permanent. Left unmarked, the next round reads it.
        let looked_at = read.was_looked_at();
        let said = match read {
            crate::infrastructure::html::Read::Said(text) => Some(text),
            _ => None,
        };

        // Only what is missing. A column another build filled in is not this one's to overwrite,
        // and `COALESCE` says that in the one place it has to be true.
        //
        // The text is the exception and is written every time: it is made from the message in
        // hand, so a second reading of the same message produces the same words — and a message
        // fetched again after the extraction improved should get the better answer.
        let mut connection = self.pool.get().map_err(|_| MessageBodyError::Unavailable)?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| MessageBodyError::Unavailable)?;

        // Text arriving where there was none changes what a search answers, and a search is a
        // view like any other: its window is anchored to an epoch, and a set that moves without
        // the epoch moving is a list still showing the answer to a question nobody is asking.
        // A subject, a sender or an opening arriving where there was none changes what the list
        // shows of the message, which is the same thing said to the window. Every folder holding
        // the message, because a search spans all of them and adds their epochs together.
        //
        // Asked before the write, because what decides is the state the message is in now. Only
        // when there was nothing: a message read a second time produces the same words, and an
        // epoch moved for that is every open list re-anchoring for no change at all.
        transaction
            .execute(
                "UPDATE folders SET view_epoch = view_epoch + 1
                 WHERE id IN (SELECT folder_id FROM message_placements WHERE message_id = ?1)
                   AND EXISTS (SELECT 1 FROM messages WHERE id = ?1
                               AND ((body_text IS NULL AND ?2 IS NOT NULL)
                                    OR (subject IS NULL AND ?3 IS NOT NULL)
                                    OR (sender IS NULL AND ?4 IS NOT NULL)
                                    OR (preview IS NULL AND ?5 IS NOT NULL)))",
                rusqlite::params![
                    message.get() as i64,
                    &said,
                    addressed.subject,
                    addressed.sender,
                    addressed.preview,
                ],
            )
            .map_err(|_| MessageBodyError::Unavailable)?;

        transaction
            .execute(
                "UPDATE messages
                 SET sender_address   = COALESCE(sender_address, ?2),
                     recipients_to    = COALESCE(NULLIF(recipients_to, ''), ?3),
                     recipients_cc    = COALESCE(NULLIF(recipients_cc, ''), ?4),
                     references_chain = COALESCE(references_chain, ?5),
                     -- What the list shows of it, where the pass that stored it had nothing to
                     -- go on: a server that would not hand it over then, or a parser that broke
                     -- on it, leaves a row of the date, the size and the flags alone.
                     subject          = COALESCE(subject, ?9),
                     sender           = COALESCE(sender, ?10),
                     preview          = COALESCE(preview, ?11),
                     body_text        = COALESCE(?6, body_text),
                     -- What the message weighs, when the server never said. The reading path
                     -- refuses to open a message above a size without fetching it, and reads this
                     -- column to decide, so a message that arrived without a size gets one from
                     -- the only place there is: itself.
                     rfc822_size      = COALESCE(rfc822_size, ?7),
                     -- Read whole, and said so. Kept or declined, extracted or not: what this
                     -- records is that the message was fetched and read, which is what the pass
                     -- asks before fetching it again.
                     read_whole_at    = CASE WHEN ?8 THEN unixepoch() ELSE read_whole_at END
                 WHERE id = ?1",
                rusqlite::params![
                    message.get() as i64,
                    addressed.from,
                    addressed.to,
                    addressed.cc,
                    addressed.chain,
                    said,
                    raw.len() as i64,
                    looked_at,
                    addressed.subject,
                    addressed.sender,
                    addressed.preview,
                ],
            )
            .map_err(|_| MessageBodyError::Unavailable)?;

        if let Some(own) = addressed.own.as_ref() {
            named_or_handed_over(&transaction, message, own, &addressed.references)?;
        }

        transaction
            .commit()
            .map(|_| ())
            .map_err(|_| MessageBodyError::Unavailable)
    }
}

/// Gives a row with no name the one its message goes by, or, where another row already goes by
/// it, hands this row's copies to that one.
///
/// A row with no name is what a pass makes of a message its server would not describe: its date,
/// its size and its flags. Nothing finds it by name, so the next copy of the message in another
/// folder is a stranger, an answer to it starts a conversation of its own, and a message moved
/// while it was refused is two rows — the one the person filed, which loses its last copy and is
/// forgotten with its kept body and with whatever was asked about it, and this one. Read whole,
/// the message says its name, and the row converges on it.
///
/// Named, it is put in its conversation the way an arriving message is, with no conversation of its
/// own first: what a reply that arrived before it left noted under its name counts, and so does
/// what it says it answers. Where another row already goes by that name, that row is this message
/// and the one the account finds it by — every later copy of it links to that row. It is given this
/// row's copies, and this one is left with none, for the end of the pass to forget like anything
/// else that has none — unless something still waiting speaks about it.
///
/// And where they went is written down apart from this row, because it can be asked for longer
/// than the row lasts. The row's identifier may have reached the window and the queue by now —
/// somebody may have opened it, marked it read or filed it while it was the only row of the message
/// there was — and the reader may still be open on it after the end of a pass has forgotten it.
/// [`going_by`] says which message that identifier stands for. Without it, an instruction about the
/// row finds no copy and stops as a message somebody else took away, and the reader still open on
/// it can neither answer it nor show it again.
fn named_or_handed_over(
    transaction: &rusqlite::Transaction<'_>,
    message: MessageId,
    own: &Rfc822MessageId,
    references: &[Rfc822MessageId],
) -> Result<(), MessageBodyError> {
    let unavailable = |_| MessageBodyError::Unavailable;
    let row = message.get() as i64;

    let Some(account) = transaction
        .query_row(
            "SELECT account_id FROM messages WHERE id = ?1 AND rfc822_message_id IS NULL",
            [row],
            |found| found.get::<_, i64>(0),
        )
        .optional()
        .map_err(unavailable)?
    else {
        // Named already, as every message a pass read its name from is.
        return Ok(());
    };

    let holder: Option<i64> = transaction
        .query_row(
            "SELECT id FROM messages
             WHERE account_id = ?1 AND rfc822_message_id = ?2 AND id <> ?3",
            (account, own.as_str(), row),
            |found| found.get(0),
        )
        .optional()
        .map_err(unavailable)?;

    // Every folder showing this row shows something else after this: the same row in another
    // conversation, or another row in its place.
    transaction
        .execute(
            "UPDATE folders SET view_epoch = view_epoch + 1
             WHERE id IN (SELECT folder_id FROM message_placements WHERE message_id = ?1)",
            [row],
        )
        .map_err(unavailable)?;

    match holder {
        Some(holder) => {
            transaction
                .execute(
                    "UPDATE message_placements SET message_id = ?2 WHERE message_id = ?1",
                    [row, holder],
                )
                .map_err(unavailable)?;

            transaction
                .execute(
                    "INSERT INTO handed_over (message_id, handed_to) VALUES (?1, ?2)
                     ON CONFLICT(message_id) DO UPDATE SET handed_to = excluded.handed_to",
                    [row, holder],
                )
                .map(|_| ())
                .map_err(unavailable)
        }
        None => {
            transaction
                .execute(
                    "UPDATE messages SET rfc822_message_id = ?2, thread_id = NULL WHERE id = ?1",
                    (row, own.as_str()),
                )
                .map_err(unavailable)?;

            threads::thread_the_message(transaction, account, row, Some(own), references)
                .map_err(|_| MessageBodyError::Unavailable)
        }
    }
}

/// The message an identifier stands for now: its own row, or the message that row handed its
/// copies to.
///
/// The window and the queue may hold the identifier of a row whose message turned out to be one
/// the account already held under its name. Four questions follow it to that message, the four
/// that need its copies: where it sits in a folder (`where_it_sits`), whether a folder holds it
/// (`holds`), where it is read from (`where_to_read`) and what the reader shows of it
/// (`get_message`). They answer for that message for as long as the account holds it, after the
/// row itself is gone as well: left with no copy, the row is forgotten by the sweep at the end of
/// a pass once nothing queued names it, like any other message with no copy.
///
/// Everything else asked by the row's identifier goes by the row itself. A conversation asked for
/// by it is the row's own, and empty once the row is forgotten; what is queued about the row is
/// laid over no row the list shows, because the list shows the message; and the copy this device
/// keeps under the row's identifier goes with the row. A row whose message is forgotten goes by
/// itself, and finds no copy.
pub(crate) fn going_by(connection: &rusqlite::Connection, message: i64) -> rusqlite::Result<i64> {
    connection
        .query_row(
            "SELECT handed_to FROM handed_over WHERE message_id = ?1",
            [message],
            |row| row.get(0),
        )
        .optional()
        .map(|handed| handed.unwrap_or(message))
}

impl MessageBodies for SqliteMessageBodies {
    fn held(&self, message: MessageId) -> Result<Option<Vec<u8>>, MessageBodyError> {
        self.pool
            .get()
            .map_err(|_| MessageBodyError::Unavailable)?
            .query_row(
                "SELECT raw FROM message_bodies WHERE message_id = ?1",
                [message.get() as i64],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(MessageBodyError::Unavailable),
            })
            // A row that will not open is a row this device cannot read, which is the same
            // situation as not having it: the message is fetched from the server again. It happens
            // to a row written under a key that is gone, and to one somebody edited on the disk.
            .map(|held| held.and_then(|sealed| self.key.open(&sealed)))
    }

    fn fill_in_missing(&self, message: MessageId, raw: &[u8]) -> Result<(), MessageBodyError> {
        self.fill_in_from(message, raw, addressed)
    }

    fn keep(&self, message: MessageId, raw: &[u8]) -> Result<(), MessageBodyError> {
        if raw.len() > LARGEST_KEPT {
            return Ok(());
        }

        // Sealed before it is written, never after. There is no moment at which the message is a
        // row in this file in the form it arrived.
        let Some(sealed) = self.key.seal(raw) else {
            return Err(MessageBodyError::Unavailable);
        };

        self.pool
            .get()
            .map_err(|_| MessageBodyError::Unavailable)?
            .execute(
                // Written again rather than refused: a message re-fetched is the same message, and
                // a row that is already there is not a reason to fail an opening.
                "INSERT INTO message_bodies (message_id, raw, stored_at)
                 VALUES (?1, ?2, unixepoch())
                 ON CONFLICT(message_id) DO UPDATE SET raw = excluded.raw,
                                                       stored_at = excluded.stored_at",
                rusqlite::params![message.get() as i64, sealed],
            )
            .map(|_| ())
            .map_err(|_| MessageBodyError::Unavailable)
    }
}

/// What a message says about who wrote it, who it went to, and the conversation it belongs to —
/// and what the list shows of it.
struct Addressed {
    from: Option<String>,
    to: String,
    cc: String,
    chain: String,
    subject: Option<String>,
    /// Who it is from as the list shows it: the name where the message gave one.
    sender: Option<String>,
    preview: Option<String>,
    /// The name it is found by, and the names of the conversation it continues, as a pass reads
    /// them — so a row named here is named exactly as one the pass described.
    own: Option<Rfc822MessageId>,
    references: Vec<Rfc822MessageId>,
}

/// What a message says about who wrote it and to whom, or nothing when the bytes are not a message.
fn addressed(raw: &[u8]) -> Option<Addressed> {
    let parsed = mail_parser::MessageParser::default().parse(raw)?;

    let from = parsed
        .from()
        .and_then(|list| list.first())
        .and_then(|address| address.address())
        .map(str::to_owned);

    let listed = |named: Option<&mail_parser::Address<'_>>| {
        named
            .map(|list| {
                list.iter()
                    .filter_map(|address| address.address())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    };

    // Oldest first, with the one this message answers last: that is the order a thread is read
    // in, and the order the outbox writes a `References` header in.
    let mut chain: Vec<String> = parsed
        .references()
        .as_text_list()
        .unwrap_or_default()
        .iter()
        .map(|name| name.to_string())
        .collect();
    chain.extend(
        parsed
            .in_reply_to()
            .as_text_list()
            .unwrap_or_default()
            .iter()
            .map(|name| name.to_string()),
    );

    // Described the way the pass describes a message it reads, by the code it uses, so a row it
    // could not describe ends up the same as one it could.
    let shown = crate::infrastructure::imap::described_whole(raw);

    Some(Addressed {
        from,
        to: listed(parsed.to()),
        cc: listed(parsed.cc()),
        chain: chain.join("\n"),
        subject: shown.subject,
        sender: shown.sender,
        preview: shown.preview,
        own: shown.rfc822_message_id,
        references: shown.references,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::fake_provider::SeededIndex;
    use lyonoyl_domain::message::FolderId;

    fn store(name: &str) -> (SeededIndex, SqliteMessageBodies) {
        let index = SeededIndex::with(name, FolderId::new(1).expect("non-zero"), 3);
        let store = SqliteMessageBodies::new(
            index.pool.clone(),
            MessageKey::from(&[3u8; 32]).expect("a key of the right size"),
        );
        (index, store)
    }

    fn a_message(index: &SeededIndex) -> MessageId {
        let id: i64 = index
            .pool
            .get()
            .expect("connection")
            .query_row("SELECT id FROM messages LIMIT 1", [], |row| row.get(0))
            .expect("a message");
        MessageId::new(id as u64).expect("non-zero")
    }

    /// One message as it arrived, with everything an answer needs in it.
    const AS_IT_ARRIVED: &[u8] = b"From: Zeta <zeta@example.com>\r\n\
To: kappa@example.net, sigma@example.org\r\n\
Cc: records@example.com\r\n\
In-Reply-To: <first@example.com>\r\n\
References: <kickoff@example.com>\r\n\
Subject: About Tuesday\r\n\
\r\n\
said";

    /// What the index holds about a message, in the columns an answer depends on.
    fn as_the_index_has_it(
        index: &SeededIndex,
        message: MessageId,
    ) -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT sender_address, recipients_to, recipients_cc, references_chain
                 FROM messages WHERE id = ?1",
                [message.get() as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("readable")
    }

    #[test]
    fn a_message_read_before_the_columns_existed_is_filled_in_from_itself() {
        // Everything an answer needs is in the message, and the message is already here. Nothing
        // is asked of a server.
        let (index, store) = store("filled-in");
        let message = a_message(&index);

        index
            .pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE messages
                 SET sender_address = NULL, recipients_to = '', recipients_cc = '',
                     references_chain = NULL
                 WHERE id = ?1",
                [message.get() as i64],
            )
            .expect("the columns are emptied");

        store
            .fill_in_missing(message, AS_IT_ARRIVED)
            .expect("it is filled in");

        let (from, to, cc, chain) = as_the_index_has_it(&index, message);
        assert_eq!(from.as_deref(), Some("zeta@example.com"));
        assert_eq!(to.as_deref(), Some("kappa@example.net\nsigma@example.org"));
        assert_eq!(cc.as_deref(), Some("records@example.com"));
        assert_eq!(
            chain.as_deref(),
            Some("kickoff@example.com\nfirst@example.com"),
            "the conversation it belongs to, oldest first, with the one it answers last"
        );
    }

    #[test]
    fn what_a_pass_already_wrote_down_is_left_as_it_is() {
        // A column another build filled in is not this one's to overwrite. What is there came from
        // the server's own account of the message, and this is a repair rather than a correction.
        let (index, store) = store("left-alone");
        let message = a_message(&index);

        index
            .pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE messages
                 SET sender_address = 'someone@else.example', recipients_to = 'kept@example.net'
                 WHERE id = ?1",
                [message.get() as i64],
            )
            .expect("the columns are filled");

        store
            .fill_in_missing(message, AS_IT_ARRIVED)
            .expect("it is asked");

        let (from, to, _, _) = as_the_index_has_it(&index, message);
        assert_eq!(from.as_deref(), Some("someone@else.example"));
        assert_eq!(to.as_deref(), Some("kept@example.net"));
    }

    #[test]
    fn bytes_that_are_not_a_message_change_nothing() {
        let (index, store) = store("not-a-message");
        let message = a_message(&index);
        let before = as_the_index_has_it(&index, message);

        store.fill_in_missing(message, b"").expect("it is asked");

        assert_eq!(as_the_index_has_it(&index, message), before);
    }

    /// The column itself says the reading happened, whatever came of it.
    ///
    /// Elsewhere this is asserted through its consequence — the message stops being one the pass
    /// asks for. Here it is the column, because that is the thing the consequence rests on.
    #[test]
    fn the_reading_is_recorded_even_when_the_bytes_are_not_a_message() {
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
        use crate::ports::mail_index::MailIndex;

        let (index, store) = store("unparseable-still-read");
        let message = a_message(&index);
        let folder = FolderId::new(1).expect("non-zero");
        let mail_index = SqliteMailIndex::new(index.pool.clone());

        let waiting = |what: &SqliteMailIndex| {
            what.not_read_whole(folder, 50)
                .expect("readable")
                .into_iter()
                .any(|(waiting, _)| waiting == message)
        };
        assert!(waiting(&mail_index), "nothing has read it yet");

        store.fill_in_missing(message, b"").expect("it is asked");

        let stamp: Option<i64> = index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT read_whole_at FROM messages WHERE id = ?1",
                [message.get() as i64],
                |row| row.get(0),
            )
            .expect("readable");
        assert!(
            stamp.is_some(),
            "a reading that produced nothing is still a reading"
        );
        assert!(
            !waiting(&mail_index),
            "unrecorded, it comes back at the head of every round for ever"
        );
    }

    #[test]
    fn a_message_the_parser_panics_on_is_read_once_and_whoever_asked_goes_on() {
        // The parser runs on whoever asked: the round reading mail whole for the index, or a person
        // opening the message. Uncaught, a panic in it ends the pass over the whole account on
        // every round and the message never opens. Caught, it is a reading that produced nothing,
        // recorded as one, so the round does not ask for this message again.
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
        use crate::ports::mail_index::MailIndex;

        let (index, store) = store("parser-panics");
        let message = a_message(&index);
        let folder = FolderId::new(1).expect("non-zero");
        let mail_index = SqliteMailIndex::new(index.pool.clone());
        let before = as_the_index_has_it(&index, message);

        store
            .fill_in_from(message, AS_IT_ARRIVED, |_| {
                panic!("a parser meeting a message it was not written for")
            })
            .expect("a parser that panicked is a reading that produced nothing");

        assert_eq!(
            as_the_index_has_it(&index, message),
            before,
            "nothing is taken from bytes that broke the parser"
        );
        assert!(
            !mail_index
                .not_read_whole(folder, 50)
                .expect("readable")
                .into_iter()
                .any(|(waiting, _)| waiting == message),
            "unrecorded, it is asked for at the head of every round for ever"
        );
    }

    #[test]
    fn a_message_kept_is_a_message_read_again_without_a_server() {
        let (index, store) = store("kept");
        let message = a_message(&index);

        assert_eq!(store.held(message).expect("readable"), None);

        store
            .keep(message, b"From: someone\r\n\r\nHello")
            .expect("kept");

        assert_eq!(
            store.held(message).expect("readable").as_deref(),
            Some(&b"From: someone\r\n\r\nHello"[..])
        );
    }

    #[test]
    fn what_the_row_holds_is_not_the_message() {
        // The assertion the whole of the encryption comes down to. If the sealing is ever dropped
        // out of this path, every other test here still passes — a message kept and read back is
        // the same message either way — and only this one notices.
        let (index, store) = store("bodies-sealed");
        let message = b"From: zeta@example.com\r\nSubject: Divorce papers\r\n\r\nplease sign";

        let kept = a_message(&index);
        store.keep(kept, message).expect("it is kept");

        let row: Vec<u8> = index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT raw FROM message_bodies WHERE message_id = ?1",
                [kept.get() as i64],
                |row| row.get(0),
            )
            .expect("the row is there");

        assert_ne!(row, message, "the message is in the row as it arrived");
        assert!(
            !row.windows(14).any(|window| window == b"Divorce papers"),
            "what somebody wrote about is legible in the file"
        );
        assert!(
            !row.windows(17)
                .any(|window| window == b"zeta@example.com\r"),
            "who wrote is legible in the body row"
        );
    }

    #[test]
    fn a_row_this_key_cannot_open_reads_as_a_message_this_device_does_not_have() {
        // A row written under a key that is gone, or one somebody edited on the disk. Answering
        // with nothing sends the reader to the server, which is where the message still is —
        // answering with the bytes would hand a parser something nobody sealed.
        let (index, store) = store("bodies-foreign");
        index
            .pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO message_bodies (message_id, raw, stored_at)
                 VALUES (?1, ?2, unixepoch())",
                rusqlite::params![
                    a_message(&index).get() as i64,
                    b"Subject: never sealed\r\n\r\nhello".as_slice()
                ],
            )
            .expect("a row written by something else");

        assert_eq!(store.held(a_message(&index)).expect("readable"), None);
    }

    #[test]
    fn what_a_whole_message_says_becomes_findable_including_markup_only_mail() {
        // The opening is what a synchronisation pass reads, and a word further down was findable
        // in no message at all. Markup-only mail is the case that matters: most of what a person
        // is sent carries no text part, and searching none of it is searching almost nothing.
        let (index, store) = store("what-it-says");
        let message = a_message(&index);

        let raw = b"From: a@x.test\r\n\
                    To: b@x.test\r\n\
                    Subject: Numbers\r\n\
                    Content-Type: text/html; charset=utf-8\r\n\
                    \r\n\
                    <html><body><p>The opening line.</p>\
                    <p>And the balance sheet is attached.</p></body></html>";

        store.fill_in_missing(message, raw).expect("filled in");

        let held: Option<String> = index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT body_text FROM messages WHERE id = ?1",
                [message.get() as i64],
                |row| row.get(0),
            )
            .expect("readable");
        let held = held.expect("a message written only in markup said nothing");
        assert!(held.contains("balance sheet"), "{held}");
        assert!(
            !held.contains("<p>"),
            "the markup itself was indexed: {held}"
        );

        let found: Vec<i64> = index
            .pool
            .get()
            .expect("connection")
            .prepare("SELECT rowid FROM message_search WHERE message_search MATCH ?1")
            .expect("prepared")
            .query_map(
                [lyonoyl_domain::search::asked_for("balance").expect("something typed")],
                |row| row.get(0),
            )
            .expect("asked")
            .collect::<Result<_, _>>()
            .expect("read");
        assert_eq!(
            found,
            vec![message.get() as i64],
            "a word from the middle of the message does not find it"
        );
    }

    #[test]
    fn keeping_the_same_message_twice_replaces_it_rather_than_failing() {
        // A message fetched again is the same message. Refusing the second copy would turn a
        // re-read into an error about storage.
        let (index, store) = store("twice");
        let message = a_message(&index);

        store.keep(message, b"first").expect("kept");
        store.keep(message, b"second").expect("kept again");

        assert_eq!(
            store.held(message).expect("readable").as_deref(),
            Some(&b"second"[..])
        );
    }

    #[test]
    fn a_message_says_its_own_size_once_it_has_been_read() {
        // The reading path refuses to open a message above a size, and reads this column to
        // decide without fetching it. A server is not obliged to name a size, and a message that
        // arrived without one would be opened whatever it weighs — so a message once read says
        // its own size from then on.
        let (index, store) = store("its-own-size");
        let message = a_message(&index);
        index
            .pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE messages SET rfc822_size = NULL WHERE id = ?1",
                [message.get() as i64],
            )
            .expect("a size the server never named");

        store
            .fill_in_missing(message, AS_IT_ARRIVED)
            .expect("read for what it says");

        let size: Option<i64> = index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT rfc822_size FROM messages WHERE id = ?1",
                [message.get() as i64],
                |row| row.get(0),
            )
            .expect("readable");

        assert_eq!(size, Some(AS_IT_ARRIVED.len() as i64));
    }

    #[test]
    fn a_size_the_server_named_is_not_overwritten_by_what_was_fetched() {
        // The envelope is what the list is drawn from, and a partial fetch would otherwise shrink
        // a message on the screen.
        let (index, store) = store("named-size-kept");
        let message = a_message(&index);
        index
            .pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE messages SET rfc822_size = 4096 WHERE id = ?1",
                [message.get() as i64],
            )
            .expect("a size the server named");

        store
            .fill_in_missing(message, AS_IT_ARRIVED)
            .expect("read for what it says");

        let size: Option<i64> = index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT rfc822_size FROM messages WHERE id = ?1",
                [message.get() as i64],
                |row| row.get(0),
            )
            .expect("readable");

        assert_eq!(size, Some(4096));
    }

    /// What the folders holding a message say their epoch is.
    fn epoch_of_the_folders(index: &SeededIndex) -> i64 {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT COALESCE(sum(view_epoch), 0) FROM folders",
                [],
                |row| row.get(0),
            )
            .expect("readable")
    }

    #[test]
    fn text_arriving_in_the_index_moves_the_epoch_a_search_is_anchored_to() {
        // A search is a view like any other: its window is held against an epoch, and rows that
        // appear without the epoch moving are matches an open search never learns about.
        let (index, store) = store("text-moves-the-epoch");
        let message = a_message(&index);
        let before = epoch_of_the_folders(&index);

        store
            .fill_in_missing(message, AS_IT_ARRIVED)
            .expect("read for what it says");

        assert!(
            epoch_of_the_folders(&index) > before,
            "the set a search answers with grew and nothing said so"
        );
    }

    /// What the list shows of a message: its subject, who it is from, and its opening.
    fn as_the_list_shows_it(
        index: &SeededIndex,
        message: MessageId,
    ) -> (Option<String>, Option<String>, Option<String>) {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT subject, sender, preview FROM messages WHERE id = ?1",
                [message.get() as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("readable")
    }

    /// The message as a pass leaves a row it could not describe: its date, its size and its flags,
    /// and nothing the sender wrote. Its words are already in the index, so what moves the epoch
    /// below is what the list shows and nothing else.
    fn as_a_row_nothing_was_read_from(index: &SeededIndex, message: MessageId) {
        index
            .pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE messages
                 SET subject = NULL, sender = NULL, sender_address = NULL, preview = NULL,
                     body_text = 'said'
                 WHERE id = ?1",
                [message.get() as i64],
            )
            .expect("the row is emptied");
    }

    #[test]
    fn a_row_the_pass_could_not_describe_is_described_once_its_message_is_read() {
        // A server that would not hand a message over to the pass leaves a row with no subject and
        // no sender, and one the parser broke on the same. Read whole — by the round that fills in
        // bodies, or by somebody opening it — it is listed as what it says, and the list is told.
        let (index, store) = store("described-once-read");
        let message = a_message(&index);
        as_a_row_nothing_was_read_from(&index, message);
        let before = epoch_of_the_folders(&index);

        store
            .fill_in_missing(message, AS_IT_ARRIVED)
            .expect("read for what it says");

        assert_eq!(
            as_the_list_shows_it(&index, message),
            (
                Some("About Tuesday".to_owned()),
                Some("Zeta".to_owned()),
                Some("said".to_owned())
            )
        );
        assert!(
            epoch_of_the_folders(&index) > before,
            "a row the list shows differently, and nothing told the list"
        );
    }

    #[test]
    fn what_the_list_shows_of_a_message_is_not_written_over_by_reading_it() {
        // What the pass stored came from the server's own account of the message, and this is a
        // repair rather than a correction.
        let (index, store) = store("list-left-alone");
        let message = a_message(&index);
        let before = as_the_list_shows_it(&index, message);
        assert_ne!(before.0, None, "the fixture describes its messages");

        store
            .fill_in_missing(message, AS_IT_ARRIVED)
            .expect("read for what it says");

        assert_eq!(as_the_list_shows_it(&index, message), before);
    }

    /// The folder the rows of these tests that a pass could not describe are filed in.
    const ARCHIVE: i64 = 2;

    /// Stores a row the way a pass stores a message its server would not describe — its date, its
    /// size and its flags, no name — or one it described with `named` and `continuing`, and says
    /// which message it became.
    fn arrived(
        index: &SeededIndex,
        uid: u32,
        named: Option<&str>,
        continuing: &[&str],
    ) -> MessageId {
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
        use crate::ports::mail_index::MailIndex;
        use lyonoyl_domain::message::{FlagScope, Flags, Uid, UidValidity};
        use lyonoyl_domain::sync::{
            ArrivedMessage, FolderDelta, MessageEnvelope, SyncCursor, VanishedReport,
        };

        let name = |written: &str| Rfc822MessageId::parse(written).expect("a name");
        let delta = FolderDelta {
            folder_id: FolderId::new(ARCHIVE as u64).expect("non-zero"),
            uid_validity: UidValidity::new(7).expect("non-zero"),
            covered_from: None,
            stopped_short: false,
            arrived: vec![ArrivedMessage {
                uid: Uid::new(uid).expect("non-zero"),
                modseq: None,
                envelope: MessageEnvelope {
                    preview: None,
                    rfc822_message_id: named.map(name),
                    subject: named.map(|_| "Re: About Tuesday".to_owned()),
                    sender: None,
                    sender_address: None,
                    to: Vec::new(),
                    cc: Vec::new(),
                    received_at: 1_700_000_500,
                    rfc822_size: Some(4096),
                    references: continuing.iter().copied().map(name).collect(),
                    flags: Flags::NONE,
                },
            }],
            flag_updates: Vec::new(),
            vanished: VanishedReport::Unknown,
        };
        SqliteMailIndex::new(index.pool.clone())
            .apply(
                &delta,
                &[],
                &SyncCursor::new(uid.to_string()),
                FlagScope::Placement,
            )
            .expect("stored as a pass stores it");

        holding(index, uid)
    }

    /// The message the archive's copy numbered `uid` is.
    fn holding(index: &SeededIndex, uid: u32) -> MessageId {
        let id: i64 = index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT message_id FROM message_placements WHERE folder_id = ?1 AND uid = ?2",
                [ARCHIVE, i64::from(uid)],
                |row| row.get(0),
            )
            .expect("a copy in the archive");
        MessageId::new(id as u64).expect("non-zero")
    }

    /// The message the index holds under `name`.
    fn called(index: &SeededIndex, name: &str) -> MessageId {
        let id: i64 = index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT id FROM messages WHERE rfc822_message_id = ?1",
                [name],
                |row| row.get(0),
            )
            .expect("a message of that name");
        MessageId::new(id as u64).expect("non-zero")
    }

    /// The name a message goes by, the conversation it is in, and how many copies it has.
    fn as_it_stands(index: &SeededIndex, message: MessageId) -> (Option<String>, i64, i64) {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT rfc822_message_id, thread_id,
                        (SELECT count(*) FROM message_placements WHERE message_id = m.id)
                 FROM messages m WHERE id = ?1",
                [message.get() as i64],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("readable")
    }

    /// The message a row the pass could not describe turns out to be, once it is read whole.
    fn named_as(name: &str, continuing: &str) -> Vec<u8> {
        format!(
            "From: Zeta <zeta@example.com>\r\n\
             Message-ID: <{name}>\r\n\
             References: <{continuing}>\r\n\
             Subject: Re: About Tuesday\r\n\
             \r\n\
             said"
        )
        .into_bytes()
    }

    #[test]
    fn a_row_the_pass_could_not_name_goes_by_its_name_once_its_message_is_read() {
        // A server that would not describe a message leaves a row with no name: the next copy of
        // it is a stranger, and it is a conversation of its own. Read whole, it says its name and
        // the conversation it continues — and a reply that arrived first, which named it before
        // anything here went by that name, is in that conversation too.
        let (index, store) = store("named-once-read");
        index.add_folder(
            ARCHIVE,
            "Archive",
            lyonoyl_domain::folder::FolderRole::Archive,
        );
        let answered = called(&index, "1@fake.invalid");
        let reply = arrived(
            &index,
            8,
            Some("reply@example.com"),
            &["tuesday@example.com"],
        );
        let refused = arrived(&index, 9, None, &[]);

        store
            .fill_in_missing(refused, &named_as("tuesday@example.com", "1@fake.invalid"))
            .expect("read for what it says");

        let (name, conversation, copies) = as_it_stands(&index, refused);
        assert_eq!(name.as_deref(), Some("tuesday@example.com"));
        assert_eq!(copies, 1);
        assert_eq!(
            conversation,
            as_it_stands(&index, answered).1,
            "the conversation it continues"
        );
        assert_eq!(
            conversation,
            as_it_stands(&index, reply).1,
            "the reply that named it before it had a name"
        );

        // The next copy of it is this message.
        let copy = arrived(&index, 10, Some("tuesday@example.com"), &[]);
        assert_eq!(copy, refused);
    }

    #[test]
    fn a_row_the_pass_could_not_name_gives_its_copies_to_the_message_that_goes_by_its_name() {
        // Moved into the archive while its server refused to describe it, a message is two rows:
        // the one the person filed, with its body and whatever was asked about it, and this one in
        // its place. Left so, the first has no copy and is forgotten at the end of the pass. Read
        // whole, this one names the message the account already holds, and gives it the copy.
        let (index, store) = store("handed-over-once-read");
        index.add_folder(
            ARCHIVE,
            "Archive",
            lyonoyl_domain::folder::FolderRole::Archive,
        );
        let filed = called(&index, "1@fake.invalid");
        let refused = arrived(&index, 9, None, &[]);
        let before = epoch_of_the_folders(&index);

        store
            .fill_in_missing(refused, &named_as("1@fake.invalid", "kickoff@example.com"))
            .expect("read for what it says");

        assert_eq!(holding(&index, 9), filed, "the copy is the message's own");
        assert_eq!(
            as_it_stands(&index, filed).2,
            2,
            "where it was, and where it went"
        );
        let (name, _, copies) = as_it_stands(&index, refused);
        assert_eq!(
            (name, copies),
            (None, 0),
            "left with nothing, for the end of the pass to forget"
        );
        assert!(
            epoch_of_the_folders(&index) > before,
            "the archive shows another row and nothing told it"
        );
    }

    /// A server that does what it is told, and remembers which copy each flag was set on.
    #[derive(Default)]
    struct Told(std::sync::Mutex<Vec<(String, u32)>>);

    impl crate::ports::mail_actions::MailActions for Told {
        fn reach(
            &self,
        ) -> Result<
            Box<dyn crate::ports::mail_actions::Acting + '_>,
            crate::ports::mail_actions::MailActionError,
        > {
            Ok(Box::new(Telling(&self.0)))
        }
    }

    struct Telling<'a>(&'a std::sync::Mutex<Vec<(String, u32)>>);

    impl crate::ports::mail_actions::Acting for Telling<'_> {
        fn carry_out(
            &mut self,
            action: crate::ports::mail_actions::ServerAction<'_>,
        ) -> Result<(), crate::ports::mail_actions::MailActionError> {
            if let crate::ports::mail_actions::ServerAction::SetFlags { mailbox, uid, .. } = action
            {
                self.0
                    .lock()
                    .expect("test lock")
                    .push((mailbox.to_owned(), uid.get()));
            }
            Ok(())
        }
    }

    /// The archive, as an instruction names it.
    fn the_archive() -> FolderId {
        FolderId::new(ARCHIVE as u64).expect("non-zero")
    }

    /// A row the pass could not name, filed in the archive, read whole and found to be the message
    /// the account already holds under its name; and that message.
    fn handed_over(index: &SeededIndex, store: &SqliteMessageBodies) -> (MessageId, MessageId) {
        index.add_folder(
            ARCHIVE,
            "Archive",
            lyonoyl_domain::folder::FolderRole::Archive,
        );
        let filed = called(index, "1@fake.invalid");
        let refused = arrived(index, 9, None, &[]);

        store
            .fill_in_missing(refused, &named_as("1@fake.invalid", "kickoff@example.com"))
            .expect("read for what it says");
        assert_eq!(holding(index, 9), filed, "the copy is the message's own");

        (refused, filed)
    }

    #[test]
    fn what_was_asked_about_a_row_is_carried_out_on_the_copy_it_handed_over() {
        // The blank row is marked read while a pass holds the account, and the push waits for the
        // pass. Opening the row reads it whole, and it hands its copy to the message the account
        // already holds. The push then looks for the row's own copy, finds none, and stops the
        // instruction as a message somebody else took away: a conflict shown about a message that
        // is there, which the server goes on calling unread.
        use crate::application::act_on_mail::ActOnMail;
        use crate::application::apply_operations::ApplyOperations;
        use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
        use crate::infrastructure::sqlite::placement::SqlitePlacements;
        use lyonoyl_domain::operation::OperationTarget;

        let (index, store) = store("asked-about-a-handed-row");
        index.add_folder(
            ARCHIVE,
            "Archive",
            lyonoyl_domain::folder::FolderRole::Archive,
        );
        let refused = arrived(&index, 9, None, &[]);
        ActOnMail::new(SqliteOperationLog::new(index.profile.clone()))
            .set_read("acc-1", OperationTarget::new(refused, the_archive()), true)
            .expect("written down");

        store
            .fill_in_missing(refused, &named_as("1@fake.invalid", "kickoff@example.com"))
            .expect("read for what it says");
        assert_eq!(
            holding(&index, 9),
            called(&index, "1@fake.invalid"),
            "the copy is the message's own"
        );

        let told = Told::default();
        let applied = ApplyOperations::new(
            SqliteOperationLog::new(index.profile.clone()),
            SqlitePlacements::new(index.pool.clone()),
            &told,
        )
        .execute("acc-1", 1)
        .expect("the queue is told");

        assert_eq!(
            (applied.carried_out, applied.conflicted),
            (1, 0),
            "carried out rather than stopped"
        );
        assert_eq!(
            *told.0.lock().expect("test lock"),
            vec![("Archive".to_owned(), 9)],
            "on the copy the row handed over"
        );
        let flags: i64 = index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT flags FROM message_placements WHERE folder_id = ?1 AND uid = 9",
                [ARCHIVE],
                |row| row.get(0),
            )
            .expect("readable");
        assert_eq!(flags & 1, 1, "and the index says what the server now holds");
    }

    #[test]
    fn a_folder_holds_a_row_that_handed_its_copy_over_while_it_holds_that_copy() {
        // What a pass asks before it stops laying a move of the row over the index: asked by the
        // row's name, a folder that still has the copy would say it does not, and the move would
        // be settled before the index agreed with it.
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
        use crate::ports::mail_index::MailIndex;

        let (index, store) = store("held-after-handing-over");
        let (refused, _) = handed_over(&index, &store);

        assert!(SqliteMailIndex::new(index.pool.clone())
            .holds(the_archive(), refused)
            .expect("readable"));
    }

    #[test]
    fn a_row_that_handed_its_copy_over_is_read_where_the_copy_went() {
        // Open in the reader when it was read whole: shown again in another form, or saved, it is
        // read from somewhere, and its own copy went to the message it turned out to be.
        use crate::read_model::message::MessageReadModel;

        let (index, store) = store("read-after-handing-over");
        let (refused, filed) = handed_over(&index, &store);
        let reading = MessageReadModel::new(index.pool.clone());

        let place = reading
            .where_to_read(refused)
            .expect("readable")
            .expect("it is read from where its copy went");
        assert_eq!(
            Some(place),
            reading.where_to_read(filed).expect("readable"),
            "where the message it turned out to be is read"
        );
    }

    #[test]
    fn a_row_that_handed_its_copy_over_is_shown_as_the_message_it_turned_out_to_be() {
        // An answer written from the reader still open on the row carries the name it answers,
        // and the reader says where the message is filed — under the identifier it asked by.
        use crate::read_model::message::MessageReadModel;

        let (index, store) = store("shown-after-handing-over");
        let (refused, _) = handed_over(&index, &store);

        let details = MessageReadModel::new(index.pool.clone())
            .get_message(refused)
            .expect("readable")
            .expect("still shown");
        assert_eq!(details.message_id, refused);
        assert_eq!(details.named.as_deref(), Some("1@fake.invalid"));
        assert!(
            details
                .placements
                .iter()
                .any(|placement| placement.folder_id == the_archive()),
            "{:?}",
            details.placements
        );
    }

    /// How many rows `statement` counts, for what a test cannot ask the index any other way.
    fn counted(index: &SeededIndex, statement: &str, message: MessageId) -> i64 {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(statement, [message.get() as i64], |row| row.get(0))
            .expect("readable")
    }

    #[test]
    fn a_row_forgotten_after_it_handed_its_copy_over_is_still_answered_for_by_its_message() {
        // The end of the pass forgets a row with no copy once nothing queued names it, and a
        // mark-read carried out within seconds names it no longer. The reader still open on it asks
        // by its identifier: an answer needs the name it answers, archiving needs the copy, and
        // showing it again needs a place to read it from.
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
        use crate::infrastructure::sqlite::placement::SqlitePlacements;
        use crate::ports::mail_index::MailIndex;
        use crate::ports::placement::Placements;
        use crate::read_model::message::MessageReadModel;

        let (index, store) = store("asked-after-the-row-is-forgotten");
        let (refused, filed) = handed_over(&index, &store);
        let mail_index = SqliteMailIndex::new(index.pool.clone());

        mail_index
            .forget_the_unplaced(1, &[])
            .expect("the end of a pass");
        assert_eq!(
            counted(
                &index,
                "SELECT count(*) FROM messages WHERE id = ?1",
                refused
            ),
            0,
            "forgotten like anything else with no copy, its words and its body with it"
        );

        let reading = MessageReadModel::new(index.pool.clone());
        let place = reading
            .where_to_read(refused)
            .expect("readable")
            .expect("read where its copy went");
        assert_eq!(Some(place), reading.where_to_read(filed).expect("readable"));
        let details = reading
            .get_message(refused)
            .expect("readable")
            .expect("still shown");
        assert_eq!(details.named.as_deref(), Some("1@fake.invalid"));
        let position = SqlitePlacements::new(index.pool.clone())
            .where_it_sits(the_archive(), refused)
            .expect("readable")
            .expect("an instruction about it finds the copy");
        assert_eq!(position.uid.get(), 9);
        assert!(mail_index.holds(the_archive(), refused).expect("readable"));
    }

    #[test]
    fn a_row_whose_message_is_forgotten_is_answered_for_by_nothing() {
        // Every copy of the message gone, and the end of the pass forgets it: what still says where
        // the row's copies went goes with it, and what is asked of the row finds nothing.
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
        use crate::ports::mail_index::MailIndex;
        use crate::read_model::message::MessageReadModel;

        let (index, store) = store("forgotten-with-its-message");
        let (refused, filed) = handed_over(&index, &store);
        index
            .pool
            .get()
            .expect("connection")
            .execute(
                "DELETE FROM message_placements WHERE message_id = ?1",
                [filed.get() as i64],
            )
            .expect("every copy gone");

        SqliteMailIndex::new(index.pool.clone())
            .forget_the_unplaced(1, &[])
            .expect("the end of a pass");

        assert_eq!(
            MessageReadModel::new(index.pool.clone())
                .get_message(refused)
                .expect("readable"),
            None
        );
        assert_eq!(
            counted(
                &index,
                "SELECT count(*) FROM handed_over WHERE message_id = ?1",
                refused
            ),
            0,
            "kept for good, every row that ever handed its copies over would stay written down"
        );
    }

    #[test]
    fn a_row_the_queue_still_names_keeps_the_message_it_handed_its_copy_to() {
        // Moved out of the archive from the reader still open on the row: the archive's listing
        // takes the copy away, the folder it went to cannot be read, and the queue names the row
        // and not the message. Forgotten then, the message comes back a stranger when that folder
        // is read, is read whole again, and what was asked about the row points at nothing.
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
        use crate::ports::mail_index::MailIndex;

        let (index, store) = store("a-named-row-keeps-its-message");
        let (refused, filed) = handed_over(&index, &store);
        index
            .pool
            .get()
            .expect("connection")
            .execute(
                "DELETE FROM message_placements WHERE message_id = ?1",
                [filed.get() as i64],
            )
            .expect("every copy gone");

        SqliteMailIndex::new(index.pool.clone())
            .forget_the_unplaced(1, &[refused])
            .expect("the end of a pass");

        assert_eq!(
            counted(&index, "SELECT count(*) FROM messages WHERE id = ?1", filed),
            1,
            "kept while something waiting speaks about it"
        );
        assert_eq!(
            arrived(&index, 12, Some("1@fake.invalid"), &[]),
            filed,
            "the copy in the folder it went to is the message it was"
        );
    }

    #[test]
    fn reading_the_same_message_again_moves_no_epoch() {
        // The words are made from the message in hand, so a second reading produces the same
        // ones. An epoch moved for that is every open list re-anchoring for no change at all.
        let (index, store) = store("second-reading-is-quiet");
        let message = a_message(&index);
        store
            .fill_in_missing(message, AS_IT_ARRIVED)
            .expect("read once");
        let after_the_first = epoch_of_the_folders(&index);

        store
            .fill_in_missing(message, AS_IT_ARRIVED)
            .expect("read again");

        assert_eq!(epoch_of_the_folders(&index), after_the_first);
    }

    #[test]
    fn a_message_the_pass_named_keeps_its_name_conversation_and_copies_however_often_it_is_read() {
        // Every message whose header a pass read already goes by its name, and it is read whole
        // once by the round that fills in bodies and again at every opening. Put back in its
        // conversation each time, it would move the epoch of every folder holding it at every
        // opening, re-anchoring every open list for no change at all.
        let (index, store) = store("named-read-whole");
        index.add_folder(
            ARCHIVE,
            "Archive",
            lyonoyl_domain::folder::FolderRole::Archive,
        );
        arrived(
            &index,
            8,
            Some("reply@example.com"),
            &["tuesday@example.com"],
        );
        let named = arrived(&index, 9, Some("tuesday@example.com"), &["1@fake.invalid"]);
        let before = as_it_stands(&index, named);
        let read = named_as("tuesday@example.com", "1@fake.invalid");

        store
            .fill_in_missing(named, &read)
            .expect("read by the round");
        let after_the_first = epoch_of_the_folders(&index);
        store.fill_in_missing(named, &read).expect("read again");

        assert_eq!(
            as_it_stands(&index, named),
            before,
            "its name, its conversation and its copies"
        );
        assert_eq!(
            epoch_of_the_folders(&index),
            after_the_first,
            "a second reading told every list holding it that it changed"
        );
    }

    #[test]
    fn a_message_too_large_to_keep_is_still_read_once_and_then_left_alone() {
        // The store declines to keep it — a local copy of a video is a local copy of a video —
        // but declining to keep is not declining to read. What it says goes into the index like
        // any other message, and the pass does not come back for it, because what the pass asks
        // is what was read rather than what was stored.
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
        use crate::ports::mail_index::MailIndex;

        let (index, store) = store("large-read-once");
        let message = a_message(&index);
        let folder = FolderId::new(1).expect("non-zero");
        let mail_index = SqliteMailIndex::new(index.pool.clone());

        let waiting = |what: &SqliteMailIndex| {
            what.not_read_whole(folder, 50)
                .expect("readable")
                .into_iter()
                .any(|(waiting, _)| waiting == message)
        };
        assert!(waiting(&mail_index), "nothing has read it yet");

        let mut raw = b"From: a@x.test\r\n\
                        To: b@x.test\r\n\
                        Subject: Quarterly\r\n\
                        \r\n\
                        The reconciliation is attached, invoice 4471.\r\n"
            .to_vec();
        raw.resize(LARGEST_KEPT + 1, b'.');

        store
            .keep(message, &raw)
            .expect("declining is not an error");
        store
            .fill_in_missing(message, &raw)
            .expect("read for what it says");

        assert_eq!(
            index
                .pool
                .get()
                .expect("connection")
                .query_row(
                    "SELECT count(*) FROM message_bodies WHERE message_id = ?1",
                    [message.get() as i64],
                    |row| row.get::<_, i64>(0)
                )
                .expect("readable"),
            0,
            "too large to keep means it is not kept"
        );

        let held: Option<String> = index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT body_text FROM messages WHERE id = ?1",
                [message.get() as i64],
                |row| row.get(0),
            )
            .expect("readable");
        assert!(
            held.as_deref().unwrap_or_default().contains("invoice 4471"),
            "a message too large to keep is still searchable by what it says"
        );

        assert!(
            !waiting(&mail_index),
            "read once means read once, not once every two minutes for ever"
        );
    }

    #[test]
    fn a_message_larger_than_its_server_said_is_read_once_and_then_left_alone() {
        // The round reading mail whole asks for what the server said is no larger than this build
        // reads. One that turns out larger is read all the same — nothing opens it and no text
        // comes out of it — and the reading is recorded, or the round asks for it, whole, on every
        // round for ever.
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
        use crate::ports::mail_index::MailIndex;

        let (index, store) = store("larger-than-said");
        let message = a_message(&index);
        let folder = FolderId::new(1).expect("non-zero");
        let mail_index = SqliteMailIndex::new(index.pool.clone());

        let waiting = |what: &SqliteMailIndex| {
            what.not_read_whole(folder, 50)
                .expect("readable")
                .into_iter()
                .any(|(waiting, _)| waiting == message)
        };
        assert!(
            waiting(&mail_index),
            "its server said it was small, and nothing has read it"
        );

        let mut raw = b"From: zeta@example.com\r\nSubject: Footage\r\n\r\n".to_vec();
        raw.resize(crate::infrastructure::html::LARGEST_MESSAGE + 1, b'.');

        store
            .keep(message, &raw)
            .expect("declining is not an error");
        store
            .fill_in_missing(message, &raw)
            .expect("read for what it says");

        assert!(
            !waiting(&mail_index),
            "read once means read once, not once every two minutes for ever"
        );
    }

    #[test]
    fn a_message_too_large_to_be_worth_keeping_is_simply_not_kept() {
        // And that is not a failure: the message is readable, it is just fetched each time.
        let (index, store) = store("large");
        let message = a_message(&index);

        store
            .keep(message, &vec![0u8; LARGEST_KEPT + 1])
            .expect("not keeping it is not an error");

        assert_eq!(store.held(message).expect("readable"), None);
    }

    #[test]
    fn removing_a_message_takes_its_copy_with_it() {
        let (index, store) = store("removed");
        let message = a_message(&index);
        store.keep(message, b"anything").expect("kept");

        index
            .pool
            .get()
            .expect("connection")
            .execute("DELETE FROM messages WHERE id = ?1", [message.get() as i64])
            .expect("the message is removed");

        let left: i64 = index
            .pool
            .get()
            .expect("connection")
            .query_row("SELECT count(*) FROM message_bodies", [], |row| row.get(0))
            .expect("readable");
        assert_eq!(left, 0, "the copy outlived the message it was a copy of");
    }
}
