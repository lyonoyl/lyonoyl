//! A provider that answers from a script instead of a network.
//!
//! It exists to prove the storage and sync model at full size before any real protocol is
//! involved: a hundred thousand messages, a connection that drops mid-pass, a timeout, and a
//! folder whose UID space is reset between passes.

use std::sync::Mutex;

use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::ports::mail_provider::{MailProviderError, MailSync, SyncRequest};
use lyonoyl_domain::message::{
    FlagScope, Flags, FolderId, ModSeq, Rfc822MessageId, Uid, UidValidity,
};
use lyonoyl_domain::sync::{
    ArrivedMessage, FolderDelta, MessageEnvelope, SyncBatch, SyncCursor, SyncScope, VanishedReport,
};

/// Deterministic message content: the same UID always produces the same message.
pub fn arrival(uid_value: u32) -> ArrivedMessage {
    ArrivedMessage {
        uid: Uid::new(uid_value).expect("generated UIDs are non-zero"),
        modseq: Some(ModSeq::new(u64::from(uid_value))),
        envelope: MessageEnvelope {
            rfc822_message_id: Some(
                Rfc822MessageId::parse(&format!("<{uid_value}@fake.invalid>"))
                    .expect("generated headers are well formed"),
            ),
            subject: Some(format!("Message {uid_value}")),
            sender: Some(format!("sender{}@fake.invalid", uid_value % 500)),
            sender_address: Some(format!("sender{}@fake.invalid", uid_value % 500)),
            to: vec!["you@fake.invalid".to_owned()],
            cc: Vec::new(),
            received_at: 1_700_000_000 + i64::from(uid_value),
            rfc822_size: Some(2048 + uid_value % 8192),
            preview: Some(format!(
                "Generated for looking at the interface. This one is number {uid_value}, and it \
                 runs on for a couple of lines so the list has something to lay out under the \
                 subject the way a real message would."
            )),
            references: Vec::new(),
            flags: if uid_value.is_multiple_of(5) {
                Flags::NONE
            } else {
                Flags::SEEN
            },
        },
    }
}

#[cfg(test)]
/// A batch of consecutive arrivals for one folder.
pub fn batch(
    folder_id: FolderId,
    uid_validity: UidValidity,
    first_uid: u32,
    count: u32,
    cursor_before: Option<&str>,
    cursor_after: &str,
) -> SyncBatch {
    SyncBatch {
        scope: SyncScope::Mailbox,
        cursor_before: cursor_before.map(SyncCursor::new),
        cursor_after: SyncCursor::new(cursor_after),
        per_mailbox: vec![FolderDelta {
            folder_id,
            uid_validity,
            covered_from: Uid::new(first_uid).ok(),
            stopped_short: false,
            arrived: (first_uid..first_uid + count).map(arrival).collect(),
            flag_updates: Vec::new(),
            vanished: VanishedReport::Unknown,
        }],
    }
}

/// Answers passes in the order they were queued.
pub struct ScriptedProvider {
    answers: Mutex<Vec<Result<SyncBatch, MailProviderError>>>,
    flag_scope: FlagScope,
}

impl ScriptedProvider {
    /// The answers are consumed from the front, so they read in the order they will be given.
    pub fn new(mut answers: Vec<Result<SyncBatch, MailProviderError>>) -> Self {
        answers.reverse();
        Self {
            answers: Mutex::new(answers),
            flag_scope: FlagScope::Message,
        }
    }
}

impl MailSync for ScriptedProvider {
    fn flag_scope(&self) -> FlagScope {
        self.flag_scope
    }

    fn fetch(&self, _request: &SyncRequest) -> Result<SyncBatch, MailProviderError> {
        self.answers
            .lock()
            .expect("test lock")
            .pop()
            .unwrap_or(Err(MailProviderError::Protocol))
    }
}

/// A temporary mail index filled through the real sync path.
///
/// Lives here rather than next to the read model: seeding writes, and the read model may not.
#[cfg(test)]
pub struct SeededIndex {
    dir: std::path::PathBuf,
    pub pool: crate::infrastructure::sqlite::connection::SqlitePool,
    /// The authoritative database beside it, holding what a person has asked for and the server
    /// has not been told about yet. A list reads both, so a fixture that has only one of them can
    /// only test half of what a list shows.
    pub profile: crate::infrastructure::sqlite::connection::SqlitePool,
}

#[cfg(test)]
impl SeededIndex {
    pub fn with(name: &str, folder_id: FolderId, count: u32) -> Self {
        use crate::application::sync::SyncFolder;
        use crate::infrastructure::sqlite::connection::open_index;
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
        use crate::infrastructure::sqlite::migrations::migrate_index;

        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-seeded-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("temp dir");

        let path = dir.join("index.sqlite3");
        let pool = open_index(&path, &DatabaseKey::for_a_test()).expect("pool opens");
        let mut connection = pool.get().expect("connection");
        migrate_index(&mut connection, &path.display().to_string()).expect("schema");

        let profile_path = dir.join("profile.sqlite3");
        let profile = crate::infrastructure::sqlite::connection::open_profile(
            &profile_path,
            &DatabaseKey::for_a_test(),
        )
        .expect("pool opens");
        {
            let mut profile_connection = profile.get().expect("connection");
            crate::infrastructure::sqlite::migrations::migrate_profile(
                &mut profile_connection,
                &profile_path.display().to_string(),
            )
            .expect("schema");
            profile_connection
                .execute(
                    "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                     VALUES ('acc-1', 'user@example.com', 'User', 'imap', 1)",
                    [],
                )
                .expect("an account for instructions to belong to");
        }
        connection
            .execute(
                "INSERT INTO folders (id, account_id, name, role) VALUES (?1, 1, 'INBOX', 1)",
                [folder_id.get() as i64],
            )
            .expect("folder");
        drop(connection);

        if count > 0 {
            let provider = ScriptedProvider::new(vec![Ok(batch(
                folder_id,
                UidValidity::new(7).expect("non-zero"),
                1,
                count,
                None,
                "1",
            ))]);
            SyncFolder::new(provider, SqliteMailIndex::new(pool.clone()))
                .execute(folder_id)
                .expect("the pass applies");
        }

        Self { dir, pool, profile }
    }

    /// Adds an empty folder belonging to another account.
    ///
    /// For the tests about what one account may name: a folder of somebody else's account, with a
    /// name that collides with one of this account's, which is the ordinary case rather than a
    /// contrived one — `INBOX`, `Archive` and `Sent` are in every account there is.
    pub fn add_folder_of_account(
        &self,
        account: i64,
        id: i64,
        name: &str,
        role: lyonoyl_domain::folder::FolderRole,
    ) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO folders (id, account_id, name, role) VALUES (?1, ?2, ?3, ?4)",
                (id, account, name, role.stored()),
            )
            .expect("folder");
    }

    /// Adds an empty folder beside the seeded one.
    pub fn add_folder(&self, id: i64, name: &str, role: lyonoyl_domain::folder::FolderRole) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO folders (id, account_id, name, role) VALUES (?1, 1, ?2, ?3)",
                (id, name, role.stored()),
            )
            .expect("folder");
    }

    /// Puts an existing message into another folder, as a label or a copy would.
    pub fn place_existing_message(&self, folder_id: i64, uid: i64, message_id: i64) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO message_placements (folder_id, uid, message_id, received_at)
                 SELECT ?1, ?2, ?3, received_at FROM messages WHERE id = ?3",
                (folder_id, uid, message_id),
            )
            .expect("placement");
    }

    /// Moves every copy of a message into another folder, as archiving it does.
    ///
    /// Answers with the message it moved: a test that picked one by hand would be picking by the
    /// same order the view it checks is built on.
    pub fn move_newest_message(&self, into: i64) -> i64 {
        let connection = self.pool.get().expect("connection");
        let moved: i64 = connection
            .query_row(
                "SELECT message_id FROM message_placements ORDER BY received_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("a message to move");

        connection
            .execute(
                "UPDATE message_placements SET folder_id = ?1 WHERE message_id = ?2",
                (into, moved),
            )
            .expect("moved");

        moved
    }

    /// Fills a folder that already exists, whichever account it belongs to.
    ///
    /// The pass continues from whatever cursor the folder already stores: one that claimed to
    /// start from nothing would be refused as stale, which is the point of that rule.
    /// Puts messages of one conversation into a folder, each with the date it arrived.
    ///
    /// Written rather than synced because what this sets up is the conversation itself: which
    /// messages answer each other, and when each of them arrived. A sync would decide both from
    /// headers, which is a different thing to be testing.
    /// Puts the messages of a conversation in a folder, the way a synchronisation would.
    ///
    /// `arrived_at` is left to the trigger on `message_placements`, and the message is stored
    /// before it is given its conversation — which is the order `apply` writes in. Filling the
    /// column in by hand instead produced a row no synchronisation can produce, and the tests
    /// that stood on it passed while the list they describe was empty.
    pub fn place_conversation(&self, folder: FolderId, arrivals: &[(i64, i64)], conversation: i64) {
        let connection = self.pool.get().expect("connection");
        for (message, arrived) in arrivals {
            connection
                .execute(
                    "INSERT INTO messages (id, account_id, received_at) VALUES (?1, 1, ?2)",
                    [message, arrived],
                )
                .expect("a message of the conversation");
            connection
                .execute(
                    "UPDATE messages SET thread_id = ?2 WHERE id = ?1",
                    [message, &conversation],
                )
                .expect("its conversation");
            connection
                .execute(
                    "INSERT INTO message_placements (folder_id, uid, message_id, received_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    [&(folder.get() as i64), message, message, arrived],
                )
                .expect("its copy in the folder");
        }
    }

    /// Puts a message of a conversation in a folder that is not the inbox, as a sent reply is.
    ///
    /// `arrived_at` is left to the trigger, which sets it only for a copy in the inbox — so this
    /// message has none, and neither has it a read flag from an inbox. That is the state the
    /// column holds for everything a person sent, and the fixture has to produce it rather than
    /// describe it.
    pub fn place_message_outside_the_inbox(
        &self,
        folder: FolderId,
        uid: i64,
        message: i64,
        received_at: i64,
        conversation: i64,
    ) {
        let connection = self.pool.get().expect("connection");
        connection
            .execute(
                "INSERT INTO messages (id, account_id, received_at, thread_id)
                 VALUES (?1, 1, ?2, ?3)",
                [&message, &received_at, &conversation],
            )
            .expect("a message of the conversation");
        connection
            .execute(
                "INSERT INTO message_placements (folder_id, uid, message_id, received_at, flags)
                 VALUES (?1, ?2, ?3, ?4, 1)",
                [&(folder.get() as i64), &uid, &message, &received_at],
            )
            .expect("its copy outside the inbox");

        let arrived: Option<i64> = connection
            .query_row(
                "SELECT arrived_at FROM messages WHERE id = ?1",
                [message],
                |row| row.get(0),
            )
            .expect("the message it just wrote");
        assert!(
            arrived.is_none(),
            "the fixture meant to place a message that reached no inbox and did not"
        );
    }

    /// Gives a stored message a subject and the opening a synchronisation would have read.
    pub fn what_a_message_says(&self, message: i64, subject: &str, opening: &str) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE messages SET subject = ?2, preview = ?3 WHERE id = ?1",
                rusqlite::params![message, subject, opening],
            )
            .expect("what the message says");
    }

    pub fn fill_folder(&self, folder_id: FolderId, first_uid: u32, count: u32) {
        use crate::application::sync::SyncFolder;
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;

        let stored: Option<String> = self
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT sync_cursor FROM folders WHERE id = ?1",
                [folder_id.get() as i64],
                |row| row.get(0),
            )
            .expect("the folder is there");

        let provider = ScriptedProvider::new(vec![Ok(batch(
            folder_id,
            UidValidity::new(7).expect("non-zero"),
            first_uid,
            count,
            stored.as_deref(),
            &format!("{}", first_uid + count - 1),
        ))]);

        SyncFolder::new(provider, SqliteMailIndex::new(self.pool.clone()))
            .execute(folder_id)
            .expect("the pass applies");
    }

    /// A copy of a message in a folder, under the `Message-ID` it went out with, written the way a
    /// sync writes the header's value: without its angle brackets.
    pub fn holds_a_copy_of(&self, folder: i64, uid: i64, message_id: &str) {
        let connection = self.pool.get().expect("connection");
        connection
            .execute(
                "INSERT INTO messages (account_id, rfc822_message_id, received_at)
                 VALUES (1, ?1, 0)",
                [message_id],
            )
            .expect("the message");
        let message = connection.last_insert_rowid();
        connection
            .execute(
                "INSERT INTO message_placements (folder_id, uid, message_id, received_at)
                 VALUES (?1, ?2, ?3, 0)",
                [folder, uid, message],
            )
            .expect("its copy in the folder");
    }

    /// Another account in the profile beside this index, which the index knows by `number`.
    pub fn add_account(&self, id: &str, number: i64) {
        self.profile
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO mail_accounts (id, email, display_name, provider, index_id)
                 VALUES (?1, ?1 || '@example.com', '', 'imap', ?2)",
                rusqlite::params![id, number],
            )
            .expect("the account is stored");
    }

    /// Dates a message in the outbox, as one written at that moment is dated.
    pub fn date_written(&self, outbox_id: i64, at: i64) {
        self.profile
            .get()
            .expect("connection")
            .execute(
                "UPDATE outbox SET written_at = ?2 WHERE id = ?1",
                [outbox_id, at],
            )
            .expect("dated");
    }

    /// Gives a message in the outbox the draft it came back as, the column the outbox lets such a
    /// row go by.
    pub fn names_its_draft(&self, outbox_id: i64) {
        self.profile
            .get()
            .expect("connection")
            .execute(
                "UPDATE outbox SET came_back_as = 1 WHERE id = ?1",
                [outbox_id],
            )
            .expect("it names its draft");
    }

    /// Makes this file stop answering, the way a truncated or corrupt one does.
    ///
    /// Here rather than in the test that wants it: a read model may not write, and the gate that
    /// enforces that greps for the statement rather than asking who meant it. A fixture that can
    /// break itself keeps the break where writing is allowed.
    pub fn stops_answering(&self) {
        // Both tables, because "unreadable" has to be true for whoever reads it: the list reads
        // the messages, the sidebar reads the folders and never touches a message. Breaking one
        // leaves the other answering happily, and a test written against that proves nothing.
        self.pool
            .get()
            .expect("connection")
            .execute_batch("PRAGMA foreign_keys = OFF; DROP TABLE messages; DROP TABLE folders;")
            .expect("the file stops being readable");
    }

    /// Adds a folder belonging to a different account.
    pub fn add_folder_of_another_account(&self, id: i64, account_id: i64) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO folders (id, account_id, name, role) VALUES (?1, ?2, 'INBOX', 1)",
                (id, account_id),
            )
            .expect("folder");
    }
    /// Writes down one instruction, the way asking for it would.
    ///
    /// Lives here rather than beside the tests that use it: `read_model` may only read, and that
    /// is enforced by a gate rather than by care — a fixture that writes there would have to be
    /// an exception somebody remembers.
    #[cfg(test)]
    pub fn asked_to_mark(&self, message: u64, folder: u64, read: bool) {
        let (add, remove) = match read {
            true => (1, 0),
            false => (0, 1),
        };

        self.profile
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO op_log
                     (account_id, message_id, folder_id, kind, flags_add, flags_remove, asked_at)
                 VALUES ('acc-1', ?1, ?2, 'set_flags', ?3, ?4, 0)",
                rusqlite::params![message as i64, folder as i64, add, remove],
            )
            .expect("it is written");
    }

    /// Writes down an instruction that stopped, the way one that could not be carried out would.
    #[cfg(test)]
    pub fn stopped_asking(&self, message: u64, folder: u64, reason: &str) {
        self.profile
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO op_log
                     (account_id, message_id, folder_id, kind, flags_add, state, last_error,
                      asked_at)
                 VALUES ('acc-1', ?1, ?2, 'set_flags', 1, 'conflicted', ?3, 0)",
                rusqlite::params![message as i64, folder as i64, reason],
            )
            .expect("it is written");
    }

    /// Writes down that a message was told to go somewhere else, and nothing has happened yet.
    pub fn asked_to_file(&self, message: u64, folder: u64, into: u64) {
        self.profile
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO op_log
                     (account_id, message_id, folder_id, kind, move_to, asked_at)
                 VALUES ('acc-1', ?1, ?2, 'move', ?3, 0)",
                rusqlite::params![message as i64, folder as i64, into as i64],
            )
            .expect("it is written");
    }

    /// The same for throwing one away.
    pub fn asked_to_throw_away(&self, message: u64, folder: u64) {
        self.profile
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO op_log
                     (account_id, message_id, folder_id, kind, delete_intent, asked_at)
                 VALUES ('acc-1', ?1, ?2, 'delete', 'to_trash', 0)",
                rusqlite::params![message as i64, folder as i64],
            )
            .expect("it is written");
    }

    /// Marks everything waiting as carried out by the server, with the moves and deletes among
    /// it not read back yet — the moment between a push and the pass that reads it back.
    #[cfg(test)]
    pub fn carried_out_and_not_read_back(&self) {
        self.profile
            .get()
            .expect("connection")
            .execute(
                "UPDATE op_log
                 SET state = 'done', sent_at = 1, owes_a_read = (kind IN ('move', 'delete'))",
                [],
            )
            .expect("it reaches the server");
    }

    /// Settles every move and delete, the way a pass does once it has read them back.
    #[cfg(test)]
    pub fn read_back_everything(&self) {
        self.profile
            .get()
            .expect("connection")
            .execute("UPDATE op_log SET owes_a_read = 0", [])
            .expect("it is settled");
    }

    /// Writes down a way back for a message filed away, asked of where it went.
    #[cfg(test)]
    pub fn asked_back(&self, message: u64, from: u64, into: u64) {
        self.asked_to_file(message, from, into);
    }

    /// The same for throwing one away for good.
    #[cfg(test)]
    pub fn asked_to_throw_away_for_good(&self, message: u64, folder: u64) {
        self.profile
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO op_log
                     (account_id, message_id, folder_id, kind, delete_intent, asked_at)
                 VALUES ('acc-1', ?1, ?2, 'delete', 'permanently', 0)",
                rusqlite::params![message as i64, folder as i64],
            )
            .expect("it is written");
    }

    /// Marks everything waiting as having reached the server.
    #[cfg(test)]
    pub fn everything_reached_the_server(&self) {
        self.profile
            .get()
            .expect("connection")
            .execute("UPDATE op_log SET state = 'done'", [])
            .expect("it reaches the server");
    }
    /// Puts a message a folder already holds into it a second time, the way a server does.
    ///
    /// Two copies with different numbers and the same `Message-ID` header are two messages to a
    /// server and one message here — which is what the identity decision means, and what makes "one
    /// row per message" the rule the list and the counts both follow.
    #[cfg(test)]
    pub fn a_second_copy_of(&self, folder: u64, message: u64) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO message_placements (folder_id, uid, message_id, received_at)
                 SELECT folder_id, (SELECT max(uid) + 1 FROM message_placements), message_id,
                        received_at
                 FROM message_placements
                 WHERE folder_id = ?1 AND message_id = ?2
                 LIMIT 1",
                rusqlite::params![folder as i64, message as i64],
            )
            .expect("the second copy is written");
    }

    /// Marks every copy a folder holds of one message as read.
    ///
    /// Here rather than in the test beside the read model: a read model may not write, and a
    /// Names a folder the way a server would, which is not the way a person reads it.
    ///
    /// Here rather than beside the read model: a read model may not write, and a fixture that
    /// writes from there is the gate firing rather than the test running.
    #[cfg(test)]
    pub fn name_folder(&self, id: i64, wire: &str) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE folders SET name = ?2 WHERE id = ?1",
                rusqlite::params![id, wire],
            )
            .expect("the folder is named");
    }

    /// The name as the database holds it, which is the name the server answers to.
    #[cfg(test)]
    pub fn folder_name(&self, id: i64) -> String {
        self.pool
            .get()
            .expect("connection")
            .query_row("SELECT name FROM folders WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .expect("readable")
    }

    /// fixture that writes from there is the gate firing rather than the test running.
    #[cfg(test)]
    pub fn every_copy_read(&self, folder: u64, message: u64) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE message_placements SET flags = 1
                 WHERE folder_id = ?1 AND message_id = ?2",
                rusqlite::params![folder as i64, message as i64],
            )
            .expect("the copies are marked read");
    }

    /// Spreads the folder over a week, two messages to a second.
    ///
    /// The seed gives every message a second of its own, which is a folder no order can be got
    /// wrong in. Real mail arrives in bursts, and two messages sharing a second is what an order
    /// that stops at the date cannot resolve.
    #[cfg(test)]
    pub fn spread_over_a_week(&self) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE message_placements
                 SET received_at = 1700000000 + ((uid / 2) % 7) * 86400 + uid / 2",
                [],
            )
            .expect("the folder is spread");
    }

    /// Empties the folder without emptying the index, which is what a list of nothing reads.
    #[cfg(test)]
    pub fn take_everything_out(&self) {
        self.pool
            .get()
            .expect("connection")
            .execute("DELETE FROM message_placements", [])
            .expect("the placements go");
    }

    /// Makes the map of days say something the folder does not.
    ///
    /// For asking whether a view reads it. A view that must not cannot be shown to ignore the map
    /// by any amount of correct behaviour -- only by behaving the same when the map is wrong.
    #[cfg(test)]
    pub fn make_the_map_of_days_wrong(&self) {
        self.pool
            .get()
            .expect("connection")
            .execute("UPDATE folder_days SET messages = messages + 1000", [])
            .expect("the map is made wrong");
    }

    /// Gives a message a subject somebody could look for.
    #[cfg(test)]
    pub fn say(&self, message: u64, subject: &str) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE messages SET subject = ?2 WHERE id = ?1",
                rusqlite::params![message as i64, subject],
            )
            .expect("the subject is written");
    }

    /// Files a message in one folder and nowhere else.
    ///
    /// For asking what a view shows about mail that did not arrive — what was sent, what was
    /// archived — which is most of what somebody looks for and none of what an inbox holds.
    #[cfg(test)]
    pub fn only_in(&self, message: u64, folder_id: i64) {
        let connection = self.pool.get().expect("connection");
        connection
            .execute(
                "DELETE FROM message_placements WHERE message_id = ?1",
                [message as i64],
            )
            .expect("its placements go");
        connection
            .execute(
                "INSERT INTO message_placements (folder_id, uid, message_id, received_at)
                 SELECT ?2, 9000 + ?1, ?1, received_at FROM messages WHERE id = ?1",
                [message as i64, folder_id],
            )
            .expect("it is filed there");
    }

    /// A second copy that arrived later and that nobody has opened.
    ///
    /// What a real server produces when the same message is delivered twice: two numbers, two sets
    /// of flags, two arrival times, one `Message-ID`. Which of them a view speaks for is the whole
    /// question, and a fixture whose copies are identical cannot ask it.
    #[cfg(test)]
    pub fn another_copy_of(&self, folder: u64, message: u64, later: i64) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO message_placements
                     (folder_id, uid, message_id, received_at, flags)
                 SELECT folder_id, (SELECT max(uid) + 1 FROM message_placements), message_id,
                        received_at + ?3, 0
                 FROM message_placements
                 WHERE folder_id = ?1 AND message_id = ?2
                 LIMIT 1",
                rusqlite::params![folder as i64, message as i64, later],
            )
            .expect("the later copy is written");
    }
}

/// A temporary profile: the accounts a workspace has, without going through the server.
///
/// Here rather than next to the read model for the same reason as [`SeededIndex`]: setting one up
/// writes, and a read model may not.
#[cfg(test)]
pub struct SeededProfile {
    dir: std::path::PathBuf,
    pub pool: crate::infrastructure::sqlite::connection::SqlitePool,
}

#[cfg(test)]
impl SeededProfile {
    pub fn new(name: &str) -> Self {
        use crate::infrastructure::sqlite::connection::open_profile;
        use crate::infrastructure::sqlite::migrations::migrate_profile;

        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-profile-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("temp dir");

        let path = dir.join("profile.sqlite3");
        let pool = open_profile(&path, &DatabaseKey::for_a_test()).expect("pool opens");
        let mut connection = pool.get().expect("connection");
        migrate_profile(&mut connection, &path.display().to_string()).expect("schema");
        drop(connection);

        Self { dir, pool }
    }

    /// Adds an account known by `index_id` in the mail index.
    ///
    /// The number is given rather than taken from the counter, so a fixture can place an account
    /// on the folders it means to test against.
    pub fn add_account(&self, id: &str, email: &str, index_id: i64) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO mail_accounts
                     (id, email, display_name, provider, index_id, created_at)
                 VALUES (?1, ?2, '', 'imap', ?3, ?4)",
                (id, email, index_id, format!("2026-01-{index_id:02}")),
            )
            .expect("the account is stored");
    }

    /// Keeps one message being written from an account, the way the composer keeps it.
    pub fn kept_draft(&self, account_id: &str) {
        use crate::infrastructure::sqlite::drafts::SqliteDrafts;
        use crate::ports::drafts::{Draft, Drafts};

        SqliteDrafts::new(self.pool.clone())
            .keep(
                account_id,
                None,
                &Draft {
                    subject: "Half a thought".to_owned(),
                    ..Draft::default()
                },
                &[],
            )
            .expect("the draft is kept");
    }

    pub fn fold_away(&self, id: &str) {
        use crate::infrastructure::sqlite::account_repository::SqliteAccountRepository;
        use crate::ports::account_repository::AccountRepository;

        SqliteAccountRepository::new(self.pool.clone())
            .set_collapsed(id, true)
            .expect("the account is folded away");
    }
    /// Writes down one instruction in whatever state the test needs it in.
    ///
    /// Here rather than beside the tests that use it: `read_model` may only read, and that is
    /// enforced by a gate rather than by care.
    #[cfg(test)]
    pub fn queued(&self, account_id: &str, state: &str) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO op_log
                     (account_id, message_id, folder_id, kind, flags_add, state, asked_at)
                 VALUES (?1, abs(random()), 1, 'set_flags', 1, ?2, 0)",
                rusqlite::params![account_id, state],
            )
            .expect("it is written");
    }

    /// One instruction asked for this second, which no server has been asked about yet.
    pub fn queued_just_now(&self, account_id: &str) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO op_log
                     (account_id, message_id, folder_id, kind, flags_add, state, asked_at)
                 VALUES (?1, abs(random()), 1, 'set_flags', 1, 'pending', unixepoch())",
                rusqlite::params![account_id],
            )
            .expect("it is written");
    }

    /// A message somebody pressed send on this second, and one that has already been tried.
    pub fn written_just_now(&self, account_id: &str, attempts: i64) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO outbox
                     (account_id, message_id, sender, subject, body, state, attempts, written_at)
                 VALUES (?1, abs(random()), 'zeta@example.com', 'Subject', 'Body', 'waiting', ?2,
                         unixepoch())",
                rusqlite::params![account_id, attempts],
            )
            .expect("it is written");
    }

    /// A message in the outbox, in whichever state the test is about, and the number of its row.
    pub fn written(&self, account_id: &str, state: &str) -> i64 {
        let connection = self.pool.get().expect("connection");
        connection
            .execute(
                "INSERT INTO outbox
                     (account_id, message_id, sender, subject, body, state, held_because,
                      written_at)
                 VALUES (?1, abs(random()), 'zeta@example.com', 'Subject', 'Body', ?2,
                         CASE ?2 WHEN 'held' THEN 'the_server_refused' END, 0)",
                rusqlite::params![account_id, state],
            )
            .expect("it is written");
        connection.last_insert_rowid()
    }

    /// A sent message whose copy is filed, kept until its account is read back.
    pub fn filed_and_kept(&self, account_id: &str) {
        self.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO outbox
                     (account_id, message_id, sender, subject, body, state, written_at, filed_at)
                 VALUES (?1, abs(random()), 'zeta@example.com', 'Subject', 'Body', 'sent', 0, 1)",
                rusqlite::params![account_id],
            )
            .expect("it is written");
    }
}

#[cfg(test)]
impl Drop for SeededProfile {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

#[cfg(test)]
impl Drop for SeededIndex {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}
