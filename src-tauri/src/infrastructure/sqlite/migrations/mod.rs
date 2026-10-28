use rusqlite::Connection;
use rusqlite::Transaction;
use rusqlite_migration::{HookError, HookResult, Migrations, SchemaVersion, M};

use crate::infrastructure::sqlite::connection::StorageError;

/// Migrations for the authoritative profile database.
///
/// This database holds the only copy of data that cannot be re-synced. Every migration carries a
/// `down`, so a downgrade is an explicit operation.
fn profile_migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(
            "CREATE TABLE mail_accounts (
             id           TEXT PRIMARY KEY NOT NULL,
             email        TEXT NOT NULL,
             display_name TEXT NOT NULL,
             provider     TEXT NOT NULL,
             created_at   TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
         ) STRICT;",
        )
        .down("DROP TABLE mail_accounts;"),
        // 002: the settings store, split by lifecycle. `preferences` belongs to the person; it is
        // key-value so a build cannot erase settings it does not know. `device_window_state` belongs
        // to this machine and is safe to lose.
        M::up(
            "CREATE TABLE preferences (
             key   TEXT PRIMARY KEY NOT NULL,
             value TEXT NOT NULL
         ) STRICT;

         CREATE TABLE device_window_state (
             window    TEXT PRIMARY KEY NOT NULL,
             x         INTEGER NOT NULL,
             y         INTEGER NOT NULL,
             width     INTEGER NOT NULL,
             height    INTEGER NOT NULL,
             maximized INTEGER NOT NULL DEFAULT 0
         ) STRICT;",
        )
        .down("DROP TABLE device_window_state; DROP TABLE preferences;"),
        // 003: the window state becomes a mode. Full screen hides the menu bar and the dock,
        // maximized fills the area between them, and the two cannot share one flag.
        M::up(
            "CREATE TABLE device_window_state_v3 (
             window TEXT PRIMARY KEY NOT NULL,
             x      INTEGER NOT NULL,
             y      INTEGER NOT NULL,
             width  INTEGER NOT NULL,
             height INTEGER NOT NULL,
             mode   INTEGER NOT NULL DEFAULT 0
         ) STRICT;

         INSERT INTO device_window_state_v3 (window, x, y, width, height, mode)
         SELECT window, x, y, width, height, maximized FROM device_window_state;

         DROP TABLE device_window_state;
         ALTER TABLE device_window_state_v3 RENAME TO device_window_state;",
        )
        .down(
            "CREATE TABLE device_window_state_v2 (
             window    TEXT PRIMARY KEY NOT NULL,
             x         INTEGER NOT NULL,
             y         INTEGER NOT NULL,
             width     INTEGER NOT NULL,
             height    INTEGER NOT NULL,
             maximized INTEGER NOT NULL DEFAULT 0
         ) STRICT;

         INSERT INTO device_window_state_v2 (window, x, y, width, height, maximized)
         SELECT window, x, y, width, height, min(mode, 1) FROM device_window_state;

         DROP TABLE device_window_state;
         ALTER TABLE device_window_state_v2 RENAME TO device_window_state;",
        ),
        // 004: what an account needs to reach its mail. The password is not among these columns
        // and cannot be: it lives in the store this device keeps its secrets in, under a name
        // derived from the id.
        M::up(
            "ALTER TABLE mail_accounts ADD COLUMN imap_host TEXT NOT NULL DEFAULT '';
             ALTER TABLE mail_accounts ADD COLUMN imap_port INTEGER NOT NULL DEFAULT 993;
             ALTER TABLE mail_accounts ADD COLUMN imap_encryption INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE mail_accounts ADD COLUMN imap_username TEXT NOT NULL DEFAULT '';",
        )
        .down(
            "ALTER TABLE mail_accounts DROP COLUMN imap_username;
             ALTER TABLE mail_accounts DROP COLUMN imap_encryption;
             ALTER TABLE mail_accounts DROP COLUMN imap_port;
             ALTER TABLE mail_accounts DROP COLUMN imap_host;",
        ),
        // 005: the number the mail index knows this account by, and whether its folders are folded
        // away in the sidebar.
        //
        // The index stores an account as a small integer next to every message and every folder;
        // repeating a 36-character identifier there would cost more than the message subject. The
        // profile owns the mapping, and it is handed out by a counter rather than by `max() + 1`:
        // removing the newest account and adding another would otherwise hand the new one the
        // number the old one left behind, together with its mail.
        M::up(
            "ALTER TABLE mail_accounts ADD COLUMN index_id INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE mail_accounts ADD COLUMN sidebar_collapsed INTEGER NOT NULL DEFAULT 0;

             UPDATE mail_accounts SET index_id = rowid;

             CREATE UNIQUE INDEX idx_accounts_index_id ON mail_accounts (index_id);

             CREATE TABLE account_index_counter (
                 id   INTEGER PRIMARY KEY CHECK (id = 1),
                 next INTEGER NOT NULL
             ) STRICT;

             INSERT INTO account_index_counter (id, next)
             SELECT 1, COALESCE(max(index_id), 0) + 1 FROM mail_accounts;",
        )
        .down(
            "DROP TABLE account_index_counter;
             DROP INDEX idx_accounts_index_id;
             ALTER TABLE mail_accounts DROP COLUMN sidebar_collapsed;
             ALTER TABLE mail_accounts DROP COLUMN index_id;",
        ),
        // 006: how wide the user dragged each pane. Machine-local like the window geometry, and
        // safe to lose: without a row the pane opens at its default width.
        M::up(
            "CREATE TABLE device_pane_width (
                 pane  TEXT PRIMARY KEY NOT NULL,
                 width INTEGER NOT NULL
             ) STRICT;",
        )
        .down("DROP TABLE device_pane_width;"),
        // 007: what someone asked to happen to a message, before it has happened on the server.
        //
        // Authoritative, and the only copy: an operation exists nowhere else until the server has
        // been told, so losing this table loses what a person did. Which is why it lives here and
        // not in the index, and why this database writes with `synchronous = FULL`.
        //
        // **There is no `uid` column, and there never will be.** A uid is a position that means a
        // different message after the folder resets, so an operation carrying one would, after a
        // reset, apply to a stranger's message — damage on the server, which nothing local undoes.
        // A message is named by `message_id` and a folder by `folder_id`, and where it actually
        // sits is looked up at the moment the operation is applied. A test and a gate both refuse a
        // column that would hold a position.
        //
        // The numbers are plain numbers. That the account is one of them is a mistake, corrected
        // by 009 — an account is known here by its own identifier, and this said the number the
        // *index* uses. Left as it was written, because a migration is a record of what a database
        // was put through rather than a file to edit: changing it here would have fixed nothing on
        // the machines that had already run it, which is exactly what happened.
        //
        // The partial unique index is what makes coalescing safe rather than merely tidy. The
        // folding itself is a decision, so it is made in the domain; this makes a second
        // outstanding operation on the same target *impossible*, so two of them cannot appear
        // through a race between two writers and be applied twice.
        M::up(
            "CREATE TABLE op_log (
                 id            INTEGER PRIMARY KEY,
                 account_id    INTEGER NOT NULL,
                 message_id    INTEGER NOT NULL,
                 folder_id     INTEGER NOT NULL,
                 kind          TEXT    NOT NULL
                               CHECK (kind IN ('set_flags', 'move', 'delete')),
                 flags_add     INTEGER NOT NULL DEFAULT 0,
                 flags_remove  INTEGER NOT NULL DEFAULT 0,
                 move_to       INTEGER,
                 delete_intent TEXT    CHECK (delete_intent IN ('to_trash', 'permanently')),
                 state         TEXT    NOT NULL DEFAULT 'pending'
                               CHECK (state IN ('pending', 'needs_reresolve', 'conflicted', 'done')),
                 attempts      INTEGER NOT NULL DEFAULT 0,
                 last_error    TEXT,
                 asked_at      INTEGER NOT NULL,
                 try_after     INTEGER NOT NULL DEFAULT 0,

                 -- Each kind carries what it needs and nothing it does not, so a row cannot be a
                 -- move with no destination or a delete with no stated intention.
                 CHECK (kind <> 'move' OR move_to IS NOT NULL),
                 CHECK (kind <> 'delete' OR delete_intent IS NOT NULL),
                 CHECK (kind <> 'set_flags' OR flags_add <> 0 OR flags_remove <> 0),
                 -- A flag asked for and against at once has no answer to give a server.
                 CHECK (flags_add & flags_remove = 0)
             ) STRICT;

             CREATE UNIQUE INDEX idx_op_log_outstanding
                 ON op_log (account_id, message_id, folder_id, kind)
                 WHERE state IN ('pending', 'needs_reresolve');

             CREATE INDEX idx_op_log_due
                 ON op_log (account_id, try_after)
                 WHERE state IN ('pending', 'needs_reresolve');",
        )
        .down("DROP TABLE op_log;"),
        // 008: whether the mail is current, and what stopped it being so.
        //
        // One row per account, and only the last answer — a history of passes is a log, and a log
        // is not what somebody looking at their mailbox wants. What they want is one sentence:
        // this is up to date, or it is not and here is why.
        //
        // Kept beside the operation log rather than in the index, because it survives the index
        // being thrown away and rebuilt: "when did this last work" is not something to lose to a
        // resync.
        M::up(
            "CREATE TABLE account_activity (
                 account_id      TEXT PRIMARY KEY NOT NULL
                                 REFERENCES mail_accounts(id) ON DELETE CASCADE,
                 last_success_at INTEGER,
                 last_error      TEXT
                                 CHECK (last_error IN ('offline', 'signIn', 'storage', 'other'))
             ) STRICT;",
        )
        .down("DROP TABLE account_activity;"),
        // 009: the operation log names an account the way the rest of this database does.
        //
        // 007 wrote `account_id` as a number, which was the number the *index* knows an account
        // by. Everything else in this file names an account by its own identifier, so the log was
        // the one table that had to be joined through a second one to say whose instruction it
        // was — and the code that writes it was corrected to text while databases in the field
        // still had the number. That is the bug this repairs: a rebuild rather than an edit to
        // 007, because a database that has already run 007 never sees an edit to it.
        //
        // Instructions already written are carried over and their account looked up. One whose
        // account is not here refers to an account this device does not have, which the key below
        // would refuse anyway.
        M::up(
            "CREATE TABLE op_log_v9 (
                 id            INTEGER PRIMARY KEY,
                 account_id    TEXT    NOT NULL
                               REFERENCES mail_accounts(id) ON DELETE CASCADE,
                 message_id    INTEGER NOT NULL,
                 folder_id     INTEGER NOT NULL,
                 kind          TEXT    NOT NULL
                               CHECK (kind IN ('set_flags', 'move', 'delete')),
                 flags_add     INTEGER NOT NULL DEFAULT 0,
                 flags_remove  INTEGER NOT NULL DEFAULT 0,
                 move_to       INTEGER,
                 delete_intent TEXT    CHECK (delete_intent IN ('to_trash', 'permanently')),
                 state         TEXT    NOT NULL DEFAULT 'pending'
                               CHECK (state IN ('pending', 'needs_reresolve', 'conflicted', 'done')),
                 attempts      INTEGER NOT NULL DEFAULT 0,
                 last_error    TEXT,
                 asked_at      INTEGER NOT NULL,
                 try_after     INTEGER NOT NULL DEFAULT 0,

                 CHECK (kind <> 'move' OR move_to IS NOT NULL),
                 CHECK (kind <> 'delete' OR delete_intent IS NOT NULL),
                 CHECK (kind <> 'set_flags' OR flags_add <> 0 OR flags_remove <> 0),
                 CHECK (flags_add & flags_remove = 0)
             ) STRICT;

             INSERT INTO op_log_v9
                 (id, account_id, message_id, folder_id, kind, flags_add, flags_remove, move_to,
                  delete_intent, state, attempts, last_error, asked_at, try_after)
             SELECT o.id, a.id, o.message_id, o.folder_id, o.kind, o.flags_add, o.flags_remove,
                    o.move_to, o.delete_intent, o.state, o.attempts, o.last_error, o.asked_at,
                    o.try_after
             FROM op_log o
             JOIN mail_accounts a ON a.index_id = o.account_id;

             DROP TABLE op_log;
             ALTER TABLE op_log_v9 RENAME TO op_log;

             CREATE UNIQUE INDEX idx_op_log_outstanding
                 ON op_log (account_id, message_id, folder_id, kind)
                 WHERE state IN ('pending', 'needs_reresolve');

             CREATE INDEX idx_op_log_due
                 ON op_log (account_id, try_after)
                 WHERE state IN ('pending', 'needs_reresolve');",
        )
        // Going back puts the number back, so 007 still has the table it expects to drop.
        .down(
            "CREATE TABLE op_log_v7 (
                 id            INTEGER PRIMARY KEY,
                 account_id    INTEGER NOT NULL,
                 message_id    INTEGER NOT NULL,
                 folder_id     INTEGER NOT NULL,
                 kind          TEXT    NOT NULL
                               CHECK (kind IN ('set_flags', 'move', 'delete')),
                 flags_add     INTEGER NOT NULL DEFAULT 0,
                 flags_remove  INTEGER NOT NULL DEFAULT 0,
                 move_to       INTEGER,
                 delete_intent TEXT    CHECK (delete_intent IN ('to_trash', 'permanently')),
                 state         TEXT    NOT NULL DEFAULT 'pending'
                               CHECK (state IN ('pending', 'needs_reresolve', 'conflicted', 'done')),
                 attempts      INTEGER NOT NULL DEFAULT 0,
                 last_error    TEXT,
                 asked_at      INTEGER NOT NULL,
                 try_after     INTEGER NOT NULL DEFAULT 0,

                 CHECK (kind <> 'move' OR move_to IS NOT NULL),
                 CHECK (kind <> 'delete' OR delete_intent IS NOT NULL),
                 CHECK (kind <> 'set_flags' OR flags_add <> 0 OR flags_remove <> 0),
                 CHECK (flags_add & flags_remove = 0)
             ) STRICT;

             INSERT INTO op_log_v7
                 (id, account_id, message_id, folder_id, kind, flags_add, flags_remove, move_to,
                  delete_intent, state, attempts, last_error, asked_at, try_after)
             SELECT o.id, a.index_id, o.message_id, o.folder_id, o.kind, o.flags_add, o.flags_remove,
                    o.move_to, o.delete_intent, o.state, o.attempts, o.last_error, o.asked_at,
                    o.try_after
             FROM op_log o
             JOIN mail_accounts a ON a.id = o.account_id;

             DROP TABLE op_log;
             ALTER TABLE op_log_v7 RENAME TO op_log;

             CREATE UNIQUE INDEX idx_op_log_outstanding
                 ON op_log (account_id, message_id, folder_id, kind)
                 WHERE state IN ('pending', 'needs_reresolve');

             CREATE INDEX idx_op_log_due
                 ON op_log (account_id, try_after)
                 WHERE state IN ('pending', 'needs_reresolve');",
        ),
        // 010: the outbox — messages written here and not yet accepted by a server.
        //
        // In the profile, and only here. A message somebody wrote is not a copy of anything: lose
        // the index and it is re-read from the server, lose this and the message is gone with the
        // words in it. That is why this file is the one that commits with `synchronous = FULL`.
        //
        // The `Message-ID` is written before anything is sent, so a message that reached the
        // server on an attempt whose answer never arrived can be recognised rather than sent
        // twice. Recipients are rows rather than a list in a column: an address is a thing the
        // schema can refuse, and one folded into a string is a thing it cannot.
        M::up(
            "CREATE TABLE outbox (
                 id            INTEGER PRIMARY KEY,
                 account_id    TEXT    NOT NULL
                               REFERENCES mail_accounts(id) ON DELETE CASCADE,
                 message_id    TEXT    NOT NULL UNIQUE,
                 sender        TEXT    NOT NULL,
                 subject       TEXT    NOT NULL,
                 body          TEXT    NOT NULL,
                 state         TEXT    NOT NULL DEFAULT 'waiting'
                               CHECK (state IN ('waiting', 'sending', 'sent', 'held')),
                 held_because  TEXT,
                 attempts      INTEGER NOT NULL DEFAULT 0,
                 written_at    INTEGER NOT NULL,
                 try_after     INTEGER NOT NULL DEFAULT 0,

                 CHECK (state <> 'held' OR held_because IS NOT NULL)
             ) STRICT;

             CREATE TABLE outbox_recipients (
                 outbox_id INTEGER NOT NULL REFERENCES outbox(id) ON DELETE CASCADE,
                 kind      TEXT    NOT NULL CHECK (kind IN ('to', 'cc', 'bcc')),
                 address   TEXT    NOT NULL,

                 PRIMARY KEY (outbox_id, kind, address)
             ) STRICT, WITHOUT ROWID;

             CREATE INDEX idx_outbox_due
                 ON outbox (account_id, try_after)
                 WHERE state = 'waiting';",
        )
        .down(
            "DROP INDEX idx_outbox_due;
             DROP TABLE outbox_recipients;
             DROP TABLE outbox;",
        ),
        // 011: where an account's messages are handed over.
        //
        // Nullable, and deliberately: reading and sending are separate servers, and an account
        // that can only be read is an ordinary account rather than a broken one. Nothing is
        // guessed from the reading settings — `imap.example.com` on port 587 is a guess that fails
        // slowly, and a message sitting in the outbox with no explanation is worse than one that
        // says nobody has told this account where to send.
        M::up(
            "ALTER TABLE mail_accounts ADD COLUMN submission_host TEXT;
             ALTER TABLE mail_accounts ADD COLUMN submission_port INTEGER;
             ALTER TABLE mail_accounts ADD COLUMN submission_encryption INTEGER;
             ALTER TABLE mail_accounts ADD COLUMN submission_username TEXT;",
        )
        .down(
            "ALTER TABLE mail_accounts DROP COLUMN submission_username;
             ALTER TABLE mail_accounts DROP COLUMN submission_encryption;
             ALTER TABLE mail_accounts DROP COLUMN submission_port;
             ALTER TABLE mail_accounts DROP COLUMN submission_host;",
        ),
        // 012: where a reply belongs in a conversation.
        //
        // RFC 5322 §3.6.4: `In-Reply-To` names the one message being answered, `References` is the
        // chain it belongs to. Without them a reply is filed as the start of a new conversation by
        // every client that receives it — a thread of twelve becomes twelve threads, for everybody.
        //
        // The chain is one column with a newline between names rather than a table of its own. A
        // `Message-ID` cannot contain a newline — RFC 5322 makes it a dot-atom inside angle
        // brackets — so the separator cannot occur inside a name, and the order that matters is the
        // order they are written in.
        M::up(
            "ALTER TABLE outbox ADD COLUMN answering TEXT;
             ALTER TABLE outbox ADD COLUMN chain TEXT;",
        )
        .down(
            "ALTER TABLE outbox DROP COLUMN chain;
             ALTER TABLE outbox DROP COLUMN answering;",
        ),
        // When the server was last told about an instruction, written and committed before it is
        // told rather than after.
        //
        // Without it, a machine that stops between the server carrying an instruction out and the
        // outcome being written down has an instruction that looks untried. It goes out again, the
        // message is no longer where it was, and the person is shown that their instruction failed
        // — which it did not. With it, the two cases are told apart: an instruction that was never
        // sent and whose message is gone is a message somebody else moved, and one that was sent
        // and whose message is gone is one the server carried out.
        M::up("ALTER TABLE op_log ADD COLUMN sent_at INTEGER;")
            .down("ALTER TABLE op_log DROP COLUMN sent_at;"),
        // When a sent message was put in the folder the account keeps sent mail in, for the
        // servers that do not do it themselves.
        //
        // A message a server has taken is gone from this application's hands and must never be
        // handed over twice, so the row that says so stays until its copy is filed. Filing is the
        // half that may be tried again; sending is not. A filed copy takes the row with it, so a row
        // with this column set is one a build that kept filed messages wrote, and the next delivery
        // round deletes it.
        M::up("ALTER TABLE outbox ADD COLUMN filed_at INTEGER;")
            .down("ALTER TABLE outbox DROP COLUMN filed_at;"),
        // A message being written, kept exactly as it was typed.
        //
        // Nothing here is parsed or checked, and that is the difference between this table and the
        // outbox beside it. A message on its way out has to be a message: every field that becomes
        // a header is parsed before it is written down, because what goes on a wire is somebody
        // else's problem the moment it leaves. A message being written is allowed to be half of
        // one — an address with no `@` yet, no subject, nothing in it at all — and refusing to
        // keep that would lose what a person typed at the moment they most need it kept.
        //
        // Recipients are one line each rather than a table of their own: a draft is read whole and
        // written whole, and there is nothing to ask about them until it stops being a draft.
        M::up(
            "CREATE TABLE drafts (
                 id         INTEGER PRIMARY KEY,
                 account_id TEXT    NOT NULL
                            REFERENCES mail_accounts(id) ON DELETE CASCADE,
                 recipients TEXT    NOT NULL DEFAULT '',
                 copies     TEXT    NOT NULL DEFAULT '',
                 subject    TEXT    NOT NULL DEFAULT '',
                 body       TEXT    NOT NULL DEFAULT '',
                 answering  TEXT,
                 chain      TEXT,
                 written_at INTEGER NOT NULL
             ) STRICT;

             CREATE INDEX idx_drafts_by_account ON drafts (account_id, written_at DESC);",
        )
        .down("DROP TABLE drafts;"),
        // Blind copies, which the table above was written without. A person who typed one and
        // closed the window would have lost it silently, which is the one thing a draft exists to
        // prevent. Its own migration rather than a change to the one above: a database that has
        // already run that one will never run it again.
        M::up("ALTER TABLE drafts ADD COLUMN blind TEXT NOT NULL DEFAULT '';")
            .down("ALTER TABLE drafts DROP COLUMN blind;"),
        // 017: the queue is read by message, so it is indexed by message.
        //
        // Both indexes on it lead with the account, which suits the sync round asking "what is
        // waiting for this account". The list asks the opposite question — "is anything waiting
        // for these fifty messages" — on every window it draws, and neither index answers it: one
        // leads with a column the question does not name, and both are written only for the two
        // states a round retries, while the list also asks about the one that stopped.
        //
        // Measured at fifty thousand waiting instructions: a scan of the queue at 4.35 ms against
        // 0.06 ms once this exists, on a query that runs for every frame of a scroll.
        M::up(
            "CREATE INDEX idx_op_log_by_message
                 ON op_log (message_id, folder_id)
                 WHERE state IN ('pending', 'needs_reresolve', 'conflicted');",
        )
        .down("DROP INDEX idx_op_log_by_message;"),
        // 018: groups of accounts, which is how a person with fifty of them arranges the sidebar.
        //
        // A group is an arrangement and nothing more. Mail belongs to an account, never to a
        // group, so this table can be emptied without a message moving.
        //
        // The membership is a column on the account rather than a table of its own, and that is
        // the invariant made structural: a column holds one value, so an account cannot be in two
        // groups and cannot be drawn twice in a sidebar whose purpose is to say where things are.
        //
        // `ON DELETE SET NULL` rather than a cascade: removing a group must not remove the
        // accounts in it. They return to the top of the sidebar, which is where they were before
        // anybody made a group. Removing an *account* takes its membership with it, because the
        // row carrying the membership is the account's own.
        M::up(
            "CREATE TABLE account_groups (
             id        TEXT PRIMARY KEY NOT NULL,
             name      TEXT NOT NULL,
             position  INTEGER NOT NULL,
             -- Folded away or open. A group of twenty accounts that cannot be folded is a heading
             -- rather than a folder, and folding is the whole of what it was for.
             collapsed INTEGER NOT NULL DEFAULT 0
         ) STRICT;

         ALTER TABLE mail_accounts
             ADD COLUMN group_id TEXT REFERENCES account_groups (id) ON DELETE SET NULL;

         ALTER TABLE mail_accounts
             ADD COLUMN position_in_group INTEGER NOT NULL DEFAULT 0;",
        )
        .down(
            "ALTER TABLE mail_accounts DROP COLUMN position_in_group;
             ALTER TABLE mail_accounts DROP COLUMN group_id;
             DROP TABLE account_groups;",
        ),
        // 019: the files a message carries, held with it.
        //
        // In the profile, beside the message they belong to, because this is the database that is
        // never dropped: a message and the file it was going to carry must survive the same crash
        // or neither is worth keeping. A path here instead of the bytes would be a message that
        // goes out with whatever is at that path when it is finally sent, which may be nothing.
        //
        // `ON DELETE CASCADE`, so a message taken back takes its files with it: a row pointing at
        // an outbox entry that is gone is a file nothing will ever send and nothing will ever
        // clear.
        M::up(
            "CREATE TABLE outbox_files (
             id           INTEGER PRIMARY KEY,
             outbox_id    INTEGER NOT NULL REFERENCES outbox(id) ON DELETE CASCADE,
             name         TEXT    NOT NULL,
             content_type TEXT    NOT NULL,
             bytes        BLOB    NOT NULL,
             position     INTEGER NOT NULL
         ) STRICT;

         CREATE INDEX idx_outbox_files_by_message ON outbox_files (outbox_id, position);",
        )
        .down("DROP TABLE outbox_files;"),
        // 020: an instruction the server has carried out, and the mail index has not read back.
        //
        // A flag the server confirmed is written into the index at once, so it needs nothing more.
        // A move or a delete is not: the index holds only what a pass read, and the number a
        // message has in the folder it went to is the server's to give. Until a pass has read both
        // folders the instruction is the only thing that says where the message went, so it stays
        // laid over the index -- `owes_a_read` -- and is not forgotten with the rest of what is
        // done until then.
        //
        // An instruction already told to the server is not folded into what is asked after it,
        // because what it said may already have happened. The one-at-a-time rule is written
        // for what is still unsent, so the opposite of something on its way can wait behind it.
        //
        // What the list and the sidebar ask for is outstanding work and work owing a read, and
        // the index they are answered by holds exactly those rows.
        //
        // What waits behind something else about the same message is found by the message, so
        // handing out the queue asks one index per row rather than reading the queue per row.
        M::up(
            "ALTER TABLE op_log ADD COLUMN owes_a_read INTEGER NOT NULL DEFAULT 0;

             DROP INDEX idx_op_log_outstanding;
             CREATE UNIQUE INDEX idx_op_log_outstanding
                 ON op_log (account_id, message_id, folder_id, kind)
                 WHERE state IN ('pending', 'needs_reresolve') AND sent_at IS NULL;

             DROP INDEX idx_op_log_by_message;
             CREATE INDEX idx_op_log_by_message
                 ON op_log (message_id, folder_id)
                 WHERE state IN ('pending', 'needs_reresolve', 'conflicted') OR owes_a_read = 1;

             CREATE INDEX idx_op_log_in_line
                 ON op_log (account_id, message_id, id)
                 WHERE state IN ('pending', 'needs_reresolve');",
        )
        // Back to one waiting instruction per target, which a queue at this version may hold two
        // of: one on its way and one written behind it. The later one is kept, because it is the
        // person's last word, and the rule below would refuse to be made over both.
        .down(
            "DROP INDEX idx_op_log_in_line;

             DELETE FROM op_log
             WHERE state IN ('pending', 'needs_reresolve')
               AND EXISTS (SELECT 1 FROM op_log later
                           WHERE later.account_id = op_log.account_id
                             AND later.message_id = op_log.message_id
                             AND later.folder_id = op_log.folder_id
                             AND later.kind = op_log.kind
                             AND later.state IN ('pending', 'needs_reresolve')
                             AND later.id > op_log.id);

             DROP INDEX idx_op_log_by_message;
             CREATE INDEX idx_op_log_by_message
                 ON op_log (message_id, folder_id)
                 WHERE state IN ('pending', 'needs_reresolve', 'conflicted');

             DROP INDEX idx_op_log_outstanding;
             CREATE UNIQUE INDEX idx_op_log_outstanding
                 ON op_log (account_id, message_id, folder_id, kind)
                 WHERE state IN ('pending', 'needs_reresolve');

             ALTER TABLE op_log DROP COLUMN owes_a_read;",
        ),
        // 021: when the server carried an instruction out.
        //
        // A move owed a read is settled by any pass once it has been owed for longer than anything
        // done is kept, whatever the index says. That is measured from here, not from when it was
        // asked for: a move asked for on a weekend offline and carried out on Monday has had no
        // pass at all, and measured from the weekend it is settled by the first one without the
        // index agreeing -- which on a server that does not name what left a folder puts every
        // archived message back in the inbox. Empty for what was carried out before this column
        // existed, which is measured from when it was asked for.
        M::up("ALTER TABLE op_log ADD COLUMN done_at INTEGER;")
            .down("ALTER TABLE op_log DROP COLUMN done_at;"),
        // 022: how many instructions a person has asked for, counted as each is written down.
        //
        // What the window orders a change against a read of the sidebar by: writing an instruction
        // answers with where it stands, and a read of the queue says how far the queue had got
        // when it was read, from the same reading. Ordered by a clock in the window instead, a
        // read already on its way when the change is written holds it, has it laid over again,
        // and the number beside a folder moves twice for one message. A row of its own rather than
        // the largest id in the queue: an instruction that cancels another removes a row and
        // writes none, and the id it removed is handed out again.
        M::up(
            "CREATE TABLE asked_counter (
                 id   INTEGER PRIMARY KEY CHECK (id = 1),
                 last INTEGER NOT NULL
             ) STRICT;

             INSERT INTO asked_counter (id, last) VALUES (1, 0);",
        )
        .down("DROP TABLE asked_counter;"),
        // 023: when a server took a message on its way out.
        //
        // The copy kept in the folder for sent mail is looked for a while after a server took the
        // message, because a server that files its own copy takes a moment about it. Counted from
        // when the message was written instead, one that waited an hour for a network is looked
        // for the instant it goes, finds nothing, and is filed twice. Empty for a message a
        // server took before a build said when, which is counted from when it was written.
        M::up("ALTER TABLE outbox ADD COLUMN accepted_at INTEGER;")
            .down("ALTER TABLE outbox DROP COLUMN accepted_at;"),
        // 024: the files a draft carries, and whether it is a message that came back.
        //
        // Beside the draft and going with it, for the reason the outbox keeps its own: a message
        // being written and the file it is going to carry survive the same crash, or the draft
        // comes back without what made it worth sending. The bytes, not a path, because what is
        // attached is attached at the moment it is chosen.
        //
        // `kept_as` is what the composer names a file by from one keep to the next — the token it
        // was attached under, which a reopened draft's fresh token stands for — so a file the
        // draft already holds is recognised and not written again, and one taken off is the one
        // the next keep does not name.
        //
        // `came_back` marks a draft the outbox gave back: a message somebody pressed send on that
        // will not go without them. It is what the foot of the window counts, for exactly as long
        // as the draft exists, and it goes with the draft when the draft is sent or thrown away.
        // A draft written before this column existed is not one, whatever the outbox still holds.
        M::up(
            "CREATE TABLE draft_files (
                 id           INTEGER PRIMARY KEY,
                 draft_id     INTEGER NOT NULL REFERENCES drafts(id) ON DELETE CASCADE,
                 kept_as      TEXT    NOT NULL,
                 name         TEXT    NOT NULL,
                 content_type TEXT    NOT NULL,
                 bytes        BLOB    NOT NULL,
                 position     INTEGER NOT NULL,

                 UNIQUE (draft_id, kept_as)
             ) STRICT;

             ALTER TABLE drafts ADD COLUMN came_back INTEGER NOT NULL DEFAULT 0;",
        )
        .down(
            "ALTER TABLE drafts DROP COLUMN came_back;
             DROP TABLE draft_files;",
        ),
        // 025: the draft a message the outbox gave back came back as.
        //
        // Nothing writes it: a message given back leaves no row to write it on, because the change
        // that writes its draft deletes the row. A row that has it holds nothing that draft does
        // not, files included, and goes before anything else in the next delivery round, whatever
        // state it was left in; given back again before then, it writes no second draft. Empty for
        // every other row, a held one among them, which stays: what it carried may be nowhere else.
        M::up("ALTER TABLE outbox ADD COLUMN came_back_as INTEGER;")
            .down("ALTER TABLE outbox DROP COLUMN came_back_as;"),
        // 026: how many times a server has refused a sent message's copy for now, and when it is
        // asked about again.
        //
        // A copy refused for now waits before it is asked again, longer each time, and one refused
        // after the last wait is not asked again: its message goes the way a filed one does.
        // Without the count, a copy its server will never take is uploaded on every round for
        // ever, and its row stays with everybody it went to and every file it carried. Nought for
        // every row written before, each of which is asked about at once.
        M::up(
            "ALTER TABLE outbox ADD COLUMN filing_refused INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE outbox ADD COLUMN file_after INTEGER NOT NULL DEFAULT 0;",
        )
        .down(
            "ALTER TABLE outbox DROP COLUMN file_after;
             ALTER TABLE outbox DROP COLUMN filing_refused;",
        ),
        // 027: which panes were left folded. Machine-local like their widths, and safe to lose. A
        // row is a pane folded and no row is a pane open, so a first start, a profile written
        // before this and a row lost all open it.
        M::up(
            "CREATE TABLE device_pane_folded (
                 pane TEXT PRIMARY KEY NOT NULL
             ) STRICT;",
        )
        .down("DROP TABLE device_pane_folded;"),
        // 028: the groups and the accounts in no group share one order at the top of the sidebar.
        //
        // A group's place and the place of an account in none were counted apart, and every group
        // was drawn above every account outside one. Counted together, an account can stand above
        // a group. Every profile keeps the sidebar it had: its groups first, in their order, then
        // the accounts in none, in theirs. Nothing is undone going back, because each count still
        // orders what it ordered before.
        M::up(
            "CREATE TEMP TABLE placed AS
                 SELECT 0 AS kind, id,
                        row_number() OVER (ORDER BY position, name, id) - 1 AS place
                 FROM account_groups;

             INSERT INTO placed
                 SELECT 1, id,
                        (SELECT count(*) FROM account_groups)
                        + row_number() OVER (ORDER BY position_in_group, created_at, id) - 1
                 FROM mail_accounts
                 WHERE group_id IS NULL;

             UPDATE account_groups
                 SET position = (
                     SELECT place FROM placed WHERE kind = 0 AND placed.id = account_groups.id
                 );

             UPDATE mail_accounts
                 SET position_in_group = (
                     SELECT place FROM placed WHERE kind = 1 AND placed.id = mail_accounts.id
                 )
                 WHERE group_id IS NULL;

             DROP TABLE placed;",
        )
        .down(""),
    ])
}

/// Migrations for the derived mail index.
///
/// Everything here exists on a mail server as well, so this database may be deleted and rebuilt.
/// It carries no reference to the profile database: the two are separate files, and SQLite cannot
/// enforce a foreign key across them, so `account_id` is a plain number here.
/// How many migrations the index has.
///
/// Beside them rather than in the test that walks them: a number written into a test goes stale the
/// moment a migration is added, and the versions it quietly stopped covering are the ones nobody
/// notices are untested. Saying so was not enough — it went stale anyway, so
/// `the_number_beside_the_index_migrations_is_how_many_there_are` now checks it against
/// `user_version` and fails until this is bumped.
#[cfg(test)]
const INDEX_MIGRATIONS: usize = 31;

/// The version the index was at before message bodies were sealed on disk.
///
/// Named rather than counted back from the end. As `INDEX_MIGRATIONS - 1` it meant "before the
/// sealing" only while the sealing was the last migration, and the first one added after it turned
/// the test that used it into a test of nothing: it set the database up in a state where the
/// sealing had already run, and then checked that the sealing had done its work.
#[cfg(test)]
const BEFORE_THE_SEALING: usize = 11;

/// The version the index was at while nothing said where a row's copies had gone.
///
/// Named for the same reason as the ones below it: as `INDEX_MIGRATIONS - 1` it would stop meaning
/// this the moment a migration is added after it.
#[cfg(test)]
const WHILE_NOTHING_SAID_WHERE_A_ROWS_COPIES_WENT: usize = 29;

/// The version the index was at while it still carried what nothing writes and nothing reads.
///
/// Named for the same reason as the one above it.
#[cfg(test)]
const WHILE_IT_CARRIED_WHAT_NOTHING_READ: usize = 30;

/// The version the index was at while nothing said a folder's last reading was refused for a
/// moment.
///
/// Named for the same reason as the ones below it: as `INDEX_MIGRATIONS - 1` it would stop meaning
/// this the moment a migration is added after it.
#[cfg(test)]
const WHILE_NOTHING_SAID_A_READING_WAS_REFUSED_FOR_NOW: usize = 28;

/// The version the index was at while nothing said a folder's last pass had stopped short.
///
/// Named for the same reason as the ones below it: as `INDEX_MIGRATIONS - 1` it would stop meaning
/// this the moment a migration is added after it.
#[cfg(test)]
const WHILE_NOTHING_SAID_A_FOLDER_STOPPED_SHORT: usize = 27;

/// The version the index was at while nothing recorded that a message had been read whole.
///
/// Named for the same reason as the ones below it: as `INDEX_MIGRATIONS - 1` it would stop meaning
/// this the moment a migration is added after it, and the upgrade it pins would start from a
/// database that already carried the column.
#[cfg(test)]
const WHILE_NOTHING_SAID_A_MESSAGE_WAS_READ: usize = 26;

/// The version the index was at while a conversation carried no date.
///
/// Named for the same reason as the ones below it: written as `INDEX_MIGRATIONS - 1` it would stop
/// meaning this the moment a migration is added after it, and the upgrade it pins would then start
/// from a database that had already been repaired.
#[cfg(test)]
const WHILE_A_CONVERSATION_HAD_NO_DATE: usize = 24;

/// The version the index was at while its map of days still counted copies rather than messages.
///
/// Named for the same reason as the one above, and caught by the same mistake: written as
/// `INDEX_MIGRATIONS - 1` it stopped meaning this the moment a migration was added after it.
#[cfg(test)]
const WHILE_THE_MAP_COUNTED_COPIES: usize = 15;

/// The version the index was at while a number it had handed out could be handed out again.
///
/// Named for the same reason as the two above. The rebuild that follows it is the first migration
/// to drop and recreate a table, so it is also the first that a foreign key could empty.
#[cfg(test)]
const BEFORE_THE_REBUILD: usize = 20;

fn index_migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(
            "CREATE TABLE folders (
             id                      INTEGER PRIMARY KEY,
             account_id              INTEGER NOT NULL,
             name                    TEXT NOT NULL,
             role                    INTEGER NOT NULL,
             uid_validity            INTEGER,
             uid_next                INTEGER,
             sync_cursor             TEXT,
             sync_window_floor_uid   INTEGER,
             full_history_synced     INTEGER NOT NULL DEFAULT 0,
             last_successful_sync_at INTEGER,
             last_error_kind         TEXT,
             UNIQUE (account_id, name)
         ) STRICT;

         -- No uid column: a message is not addressed by its position in a folder. Protocol
         -- addresses live in message_placements, one per folder the message appears in.
         CREATE TABLE messages (
             id                INTEGER PRIMARY KEY,
             account_id        INTEGER NOT NULL,
             rfc822_message_id TEXT,
             thread_id         INTEGER,
             subject           TEXT,
             subject_norm      TEXT,
             sender            TEXT,
             received_at       INTEGER NOT NULL,
             rfc822_size       INTEGER,
             preview           TEXT,
             bodystructure     BLOB,
             body_state        INTEGER NOT NULL DEFAULT 0,
             flags             INTEGER NOT NULL DEFAULT 0
         ) STRICT;

         -- One row per (folder, uid). The same message may appear in several folders, which is what
         -- makes labels and copies expressible. flags_override is used only by providers that keep
         -- flags per mailbox; for the rest it stays NULL and messages.flags is the value.
         CREATE TABLE message_placements (
             folder_id      INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
             uid            INTEGER NOT NULL,
             message_id     INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
             modseq         INTEGER,
             flags_override INTEGER,
             PRIMARY KEY (folder_id, uid)
         ) STRICT, WITHOUT ROWID;

         -- A table rather than JSON in a column: filtering by keyword must not be a full scan.
         CREATE TABLE message_keywords (
             message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
             keyword    TEXT NOT NULL,
             PRIMARY KEY (message_id, keyword)
         ) STRICT, WITHOUT ROWID;

         -- Finding every folder a message sits in, and the reverse lookup a cascade needs.
         CREATE INDEX idx_placements_message ON message_placements (message_id);

         -- The message list orders by arrival within an account.
         CREATE INDEX idx_messages_account_received ON messages (account_id, received_at DESC);

         CREATE INDEX idx_messages_thread ON messages (thread_id) WHERE thread_id IS NOT NULL;

         -- Deduplication across folders matches on the Message-ID header when there is one.
         CREATE INDEX idx_messages_rfc822 ON messages (account_id, rfc822_message_id)
             WHERE rfc822_message_id IS NOT NULL;

         -- Unread is the most frequent filter and matches a small share of rows, so it gets a
         -- partial index. Bit 1 is the Seen flag.
         CREATE INDEX idx_messages_unread ON messages (account_id, received_at DESC)
             WHERE flags & 1 = 0;",
        )
        .down(
            "DROP TABLE message_keywords;
             DROP TABLE message_placements;
             DROP TABLE messages;
             DROP TABLE folders;",
        ),
        // 002: the list orders by arrival within one folder. Reading that order from `messages`
        // means visiting every placement of the folder and sorting the result, which grows with the
        // mailbox. The date is copied into the placement, where an index can serve the window
        // directly. It never changes after a message is stored, so the two cannot drift.
        M::up(
            "ALTER TABLE message_placements ADD COLUMN received_at INTEGER NOT NULL DEFAULT 0;

             UPDATE message_placements
             SET received_at = (SELECT received_at FROM messages WHERE id = message_id);

             CREATE INDEX idx_placements_folder_date
                 ON message_placements (folder_id, received_at DESC);",
        )
        .down("DROP INDEX idx_placements_folder_date;"),
        // 003: a counter of set changes for one folder. The list uses it to tell "the set moved
        // under me" from "my window is stale", and re-anchors instead of reloading.
        M::up("ALTER TABLE folders ADD COLUMN view_epoch INTEGER NOT NULL DEFAULT 0;")
            .down("ALTER TABLE folders DROP COLUMN view_epoch;"),
        // 004: the list needs a deterministic order, and two messages can share a second. Adding
        // the tie-break to the ORDER BY alone made SQLite abandon the index and sort the whole
        // folder in a temporary b-tree; carrying it in the index keeps the window an index scan.
        M::up(
            "DROP INDEX idx_placements_folder_date;

             CREATE INDEX idx_placements_folder_date
                 ON message_placements (folder_id, received_at DESC, uid DESC);",
        )
        .down(
            "DROP INDEX idx_placements_folder_date;

             CREATE INDEX idx_placements_folder_date
                 ON message_placements (folder_id, received_at DESC);",
        ),
        // 005: the list is read for one folder, for one whole account, or for everything on this
        // device. The wider two read `messages` rather than `message_placements`, so a message
        // filed in several folders is one row and not several, and they need their own order:
        // without the tie-break in the index SQLite sorts the whole set in a temporary b-tree.
        M::up(
            "DROP INDEX idx_messages_account_received;

             CREATE INDEX idx_messages_account_date
                 ON messages (account_id, received_at DESC, id DESC);

             CREATE INDEX idx_messages_date ON messages (received_at DESC, id DESC);",
        )
        .down(
            "DROP INDEX idx_messages_date;
             DROP INDEX idx_messages_account_date;

             CREATE INDEX idx_messages_account_received
                 ON messages (account_id, received_at DESC);",
        ),
        // 006: clears the openings a build stored before it could tell markup from words.
        //
        // A message arrives as a prefix of itself, so its last part is unfinished; a parser that
        // cannot classify an unfinished part hands the bytes back as plain text, and a part of HTML
        // read as plain text is a line of tags under a subject. Those lines are removed rather than
        // repaired: what belongs there is read from the message, and a message is where it will be
        // read from again.
        M::up(
            "UPDATE messages
             SET preview = NULL
             WHERE preview LIKE '%</%'
                OR preview LIKE '%<p>%'
                OR preview LIKE '%<br%'
                OR preview LIKE '%<div%'
                OR preview LIKE '%<span%'
                OR preview LIKE '%<table%'
                OR preview LIKE '%<a href%'
                OR preview LIKE '%<img%';",
        )
        // Nothing to put back: what was cleared was never worth keeping.
        .down("SELECT 1;"),
        // 007: clears the openings that arrived still encoded.
        //
        // A part whose encoding the parser did not get to apply comes back as its own bytes, and
        // `I=E2=80=99ll` under a subject is not something anyone wrote. Only the escapes are
        // matched, never a lone `=`: a person writes `a = b`, and losing that line would cost more
        // than leaving it.
        M::up(
            "UPDATE messages
             SET preview = NULL
             WHERE preview LIKE '%=E2=%'
                OR preview LIKE '%=D0=%'
                OR preview LIKE '%=D1=%'
                OR preview LIKE '%=C3=%'
                OR preview LIKE '%=A0%'
                OR preview LIKE '%=3D%';",
        )
        .down("SELECT 1;"),
        // 012: the message itself, kept where it was read.
        //
        // Everything else in this file is a description of a message; this is the message. It goes
        // here rather than in the profile because it is a copy of what the server holds and can be
        // fetched again — the file may be deleted and re-synced, and losing it loses nothing.
        //
        // What it buys is what a local-first client is for: a message read once is readable again
        // with no server, at once, and showing it as plain text or with its pictures is a redraw
        // rather than another conversation with a provider several hundred milliseconds away.
        M::up(
            "CREATE TABLE message_bodies (
                 message_id INTEGER PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
                 raw        BLOB    NOT NULL,
                 stored_at  INTEGER NOT NULL
             ) STRICT;",
        )
        .down("DROP TABLE message_bodies;"),
        // The conversation a message belongs to, as it said itself.
        //
        // The pass has always parsed `References` and always thrown it away. It is what an answer
        // has to carry — RFC 5322 §3.6.4 — and without it a reply is filed as a new conversation by
        // every client that receives it. One column with a newline between names: a `Message-ID`
        // cannot contain one, so the separator cannot occur inside a name.
        M::up("ALTER TABLE messages ADD COLUMN references_chain TEXT;")
            .down("ALTER TABLE messages DROP COLUMN references_chain;"),
        // Who the message is from, as an answer has to address it.
        //
        // `sender` beside it is the display name whenever the message gave one, which is what a
        // list should show and the one thing a reply must not be addressed to. Kept apart rather
        // than parsed back out later: what a `From` header meant is decided once, where the
        // message is read.
        M::up("ALTER TABLE messages ADD COLUMN sender_address TEXT;")
            .down("ALTER TABLE messages DROP COLUMN sender_address;"),
        // Who else the message was addressed to. What an answer to everybody is built from, and
        // the last thing a pass parsed and threw away. A newline between addresses, which cannot
        // occur inside one; two columns rather than one, because `To` and `Cc` say different
        // things about the people in them.
        M::up(
            "ALTER TABLE messages ADD COLUMN recipients_to TEXT;
             ALTER TABLE messages ADD COLUMN recipients_cc TEXT;",
        )
        .down(
            "ALTER TABLE messages DROP COLUMN recipients_to;
             ALTER TABLE messages DROP COLUMN recipients_cc;",
        ),
        // Every message kept before this build is deleted, because every one of them is on the
        // disk as the server sent it.
        //
        // From here the bytes are sealed before they are written, and the ones already there
        // cannot be sealed in place: the key does not exist until the application asks for it,
        // and a migration is not the place to reach into a keychain. They are a cache of a copy —
        // the message is on the server, and opening it fetches it again — so deleting them costs
        // one round trip per message somebody reads a second time, and keeping them would mean
        // shipping a build that says the mail is encrypted while the mail that is already there
        // is not.
        //
        // `body_state` goes back with them, or the list would say a message is held here when the
        // row is gone.
        M::up(
            "DELETE FROM message_bodies;
             UPDATE messages SET body_state = 0 WHERE body_state <> 0;",
        )
        // Nothing to undo: what it deleted is a copy of what a server holds.
        .down(""),
        // Flags belong to the copy, which is what the code has said all along and what the schema
        // did not.
        //
        // A placement used to carry flags only when a provider kept them per mailbox, and
        // otherwise the message's own were the value. Answering "is this copy unread" therefore
        // meant looking the message up, once per row — measured at four hundred thousand messages
        // it was a hundred thousand random lookups and 505 ms for one folder's unread count,
        // against a budget of 150. See docs/performance/response-budgets.md.
        //
        // Now every placement carries its own, so the count is answered from one index and joins
        // nothing. What it costs instead: a provider that keeps flags per message writes to each
        // copy rather than to one row — a message has one to three copies, and the write happens
        // once where the read happened on every row of every window.
        M::up(
            "ALTER TABLE message_placements ADD COLUMN flags INTEGER NOT NULL DEFAULT 0;

             UPDATE message_placements
                SET flags = COALESCE(
                        flags_override,
                        (SELECT flags FROM messages WHERE id = message_id),
                        0
                    );

             -- Folder first, flags second: a count for one folder reads the entries for that
             -- folder and never the table.
             CREATE INDEX idx_placements_folder_flags
                 ON message_placements (folder_id, flags);",
        )
        .down(
            "DROP INDEX idx_placements_folder_flags;
             ALTER TABLE message_placements DROP COLUMN flags;",
        ),
        // The column that used to mean "this provider gave flags for this copy". Nothing reads it
        // any more: every placement carries its own flags, so a value here would be a second
        // account of the same fact, and two accounts of one fact part company.
        //
        // What it recorded is not lost — which of the two places a provider owns is decided when
        // flags are written, which is where it belongs.
        M::up("ALTER TABLE message_placements DROP COLUMN flags_override;")
            .down("ALTER TABLE message_placements ADD COLUMN flags_override INTEGER;"),
        // How many messages a folder holds on each day, so a position in the list can be found
        // without walking to it.
        //
        // `OFFSET` walks every row it skips. Measured at four hundred thousand messages a window
        // at the bottom took 349 ms against a budget of 150, and the size this is designed for is
        // ten times that. With this table a position is turned into a day by adding up counts —
        // a few thousand small rows for ten years of a folder — and the window is then found by
        // seeking to that day and skipping at most the messages of that one day.
        //
        // Kept by the database rather than by every writer. A count that some code path forgets to
        // update is a list that quietly points at the wrong place, and the writers this build has
        // are not the only ones there can be.
        M::up(
            "CREATE TABLE folder_days (
                 folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                 day       INTEGER NOT NULL,
                 messages  INTEGER NOT NULL,

                 PRIMARY KEY (folder_id, day)
             ) STRICT, WITHOUT ROWID;

             INSERT INTO folder_days (folder_id, day, messages)
             SELECT folder_id, received_at / 86400, count(*)
             FROM message_placements
             GROUP BY folder_id, received_at / 86400;

             CREATE TRIGGER folder_days_after_insert
             AFTER INSERT ON message_placements BEGIN
                 INSERT INTO folder_days (folder_id, day, messages)
                 VALUES (NEW.folder_id, NEW.received_at / 86400, 1)
                 ON CONFLICT(folder_id, day) DO UPDATE SET messages = messages + 1;
             END;

             CREATE TRIGGER folder_days_after_delete
             AFTER DELETE ON message_placements BEGIN
                 UPDATE folder_days SET messages = messages - 1
                 WHERE folder_id = OLD.folder_id AND day = OLD.received_at / 86400;

                 DELETE FROM folder_days WHERE messages <= 0;
             END;

             -- A row that moves folder or changes date is a row leaving one day and joining
             -- another. Written as one trigger so the two halves cannot come apart.
             CREATE TRIGGER folder_days_after_update
             AFTER UPDATE OF folder_id, received_at ON message_placements
             WHEN OLD.folder_id <> NEW.folder_id
               OR OLD.received_at / 86400 <> NEW.received_at / 86400
             BEGIN
                 UPDATE folder_days SET messages = messages - 1
                 WHERE folder_id = OLD.folder_id AND day = OLD.received_at / 86400;

                 INSERT INTO folder_days (folder_id, day, messages)
                 VALUES (NEW.folder_id, NEW.received_at / 86400, 1)
                 ON CONFLICT(folder_id, day) DO UPDATE SET messages = messages + 1;

                 DELETE FROM folder_days WHERE messages <= 0;
             END;",
        )
        .down(
            "DROP TRIGGER folder_days_after_update;
             DROP TRIGGER folder_days_after_delete;
             DROP TRIGGER folder_days_after_insert;
             DROP TABLE folder_days;",
        ),
        // 016: the map counts messages, not copies of them.
        //
        // A folder can hold the same message twice, and the list shows those as one row dated by
        // the newer copy. A map counting both answers a position ahead of the row it names, by
        // however many second copies lie above it — an error that grows with depth and is largest
        // at the bottom, where the map is the only thing standing in for a walk. So a message is
        // one tick, filed under the day of its newest copy in that folder.
        //
        // The day is floor division. `received_at / 86400` truncates toward zero, so a date before
        // 1970 shared a bucket with the day after it, and `day * 86400` was not that bucket\'s
        // first second — which is arithmetic the seek depends on, not only a count.
        //
        // A day that empties keeps its row at zero. Removing it needed a `WHERE messages <= 0`
        // that read the whole map for every row deleted: emptying a 20 000-message folder took
        // 44 seconds, against 0.02 with the row left alone. A zero adds nothing to a sum, and the
        // rows are bounded by the days a folder has ever held mail on.
        //
        // `CHECK (messages >= 0)` because a count below zero is a trigger that is wrong, and a
        // write that fails is findable where a list quietly pointing at the wrong row is not.
        M::up(
            "DROP TRIGGER folder_days_after_update;
             DROP TRIGGER folder_days_after_delete;
             DROP TRIGGER folder_days_after_insert;
             DROP TABLE folder_days;

             CREATE TABLE folder_days (
                 folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                 day       INTEGER NOT NULL,
                 messages  INTEGER NOT NULL CHECK (messages >= 0),

                 PRIMARY KEY (folder_id, day)
             ) STRICT, WITHOUT ROWID;

             INSERT INTO folder_days (folder_id, day, messages)
             SELECT folder_id, (newest - ((newest % 86400) + 86400) % 86400) / 86400 AS day,
                    count(*)
             FROM (SELECT folder_id, message_id, max(received_at) AS newest
                   FROM message_placements
                   GROUP BY folder_id, message_id)
             GROUP BY folder_id, day;

             -- A copy arriving moves its message to the newer day, or leaves it where it is.
             CREATE TRIGGER folder_days_after_insert
             AFTER INSERT ON message_placements
             BEGIN
                 UPDATE folder_days SET messages = messages - 1
                 WHERE folder_id = NEW.folder_id AND day = (
                     SELECT (t - ((t % 86400) + 86400) % 86400) / 86400
                     FROM (SELECT max(received_at) AS t FROM message_placements
                           WHERE folder_id = NEW.folder_id AND message_id = NEW.message_id
                             AND uid <> NEW.uid));

                 INSERT INTO folder_days (folder_id, day, messages)
                 SELECT NEW.folder_id, (t - ((t % 86400) + 86400) % 86400) / 86400, 1
                 FROM (SELECT max(received_at) AS t FROM message_placements
                       WHERE folder_id = NEW.folder_id AND message_id = NEW.message_id)
                 WHERE t IS NOT NULL
                 ON CONFLICT(folder_id, day) DO UPDATE SET messages = messages + 1;
             END;

             -- A copy leaving takes the message with it only if it was the last one. Otherwise the
             -- message falls back to whichever copy is now the newest, which may be an older day.
             CREATE TRIGGER folder_days_after_delete
             AFTER DELETE ON message_placements
             BEGIN
                 UPDATE folder_days SET messages = messages - 1
                 WHERE folder_id = OLD.folder_id AND day = (
                     SELECT (t - ((t % 86400) + 86400) % 86400) / 86400
                     FROM (SELECT max(OLD.received_at, COALESCE(
                               (SELECT max(received_at) FROM message_placements
                                WHERE folder_id = OLD.folder_id
                                  AND message_id = OLD.message_id),
                               OLD.received_at)) AS t));

                 INSERT INTO folder_days (folder_id, day, messages)
                 SELECT OLD.folder_id, (t - ((t % 86400) + 86400) % 86400) / 86400, 1
                 FROM (SELECT max(received_at) AS t FROM message_placements
                       WHERE folder_id = OLD.folder_id AND message_id = OLD.message_id)
                 WHERE t IS NOT NULL
                 ON CONFLICT(folder_id, day) DO UPDATE SET messages = messages + 1;
             END;

             -- A row that changes folder, date, or which message it is a copy of, is a copy
             -- leaving one message and joining another. `message_id` is in the list because the
             -- only writer of this table re-points it on conflict, and a map that ignored that
             -- drifted without anything failing.
             CREATE TRIGGER folder_days_after_update
             AFTER UPDATE OF folder_id, message_id, received_at ON message_placements
             WHEN OLD.folder_id <> NEW.folder_id
               OR OLD.message_id <> NEW.message_id
               OR OLD.received_at <> NEW.received_at
             BEGIN
                 UPDATE folder_days SET messages = messages - 1
                 WHERE folder_id = OLD.folder_id AND day = (
                     SELECT (t - ((t % 86400) + 86400) % 86400) / 86400
                     FROM (SELECT max(OLD.received_at, COALESCE(
                               (SELECT max(received_at) FROM message_placements
                                WHERE folder_id = OLD.folder_id
                                  AND message_id = OLD.message_id
                                  AND NOT (folder_id = NEW.folder_id AND uid = NEW.uid)),
                               OLD.received_at)) AS t));

                 INSERT INTO folder_days (folder_id, day, messages)
                 SELECT OLD.folder_id, (t - ((t % 86400) + 86400) % 86400) / 86400, 1
                 FROM (SELECT max(received_at) AS t FROM message_placements
                       WHERE folder_id = OLD.folder_id AND message_id = OLD.message_id
                         AND NOT (folder_id = NEW.folder_id AND uid = NEW.uid))
                 WHERE t IS NOT NULL
                 ON CONFLICT(folder_id, day) DO UPDATE SET messages = messages + 1;

                 UPDATE folder_days SET messages = messages - 1
                 WHERE folder_id = NEW.folder_id AND day = (
                     SELECT (t - ((t % 86400) + 86400) % 86400) / 86400
                     FROM (SELECT max(received_at) AS t FROM message_placements
                           WHERE folder_id = NEW.folder_id AND message_id = NEW.message_id
                             AND NOT (folder_id = NEW.folder_id AND uid = NEW.uid)));

                 INSERT INTO folder_days (folder_id, day, messages)
                 SELECT NEW.folder_id, (t - ((t % 86400) + 86400) % 86400) / 86400, 1
                 FROM (SELECT max(received_at) AS t FROM message_placements
                       WHERE folder_id = NEW.folder_id AND message_id = NEW.message_id)
                 WHERE t IS NOT NULL
                 ON CONFLICT(folder_id, day) DO UPDATE SET messages = messages + 1;
             END;",
        )
        // Going back means the map counts copies again, not that there is no map: the version
        // before this one has one, and its own rollback expects to find it.
        .down(
            "DROP TRIGGER folder_days_after_update;
             DROP TRIGGER folder_days_after_delete;
             DROP TRIGGER folder_days_after_insert;
             DROP TABLE folder_days;

             CREATE TABLE folder_days (
                 folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                 day       INTEGER NOT NULL,
                 messages  INTEGER NOT NULL,

                 PRIMARY KEY (folder_id, day)
             ) STRICT, WITHOUT ROWID;

             INSERT INTO folder_days (folder_id, day, messages)
             SELECT folder_id, received_at / 86400, count(*)
             FROM message_placements
             GROUP BY folder_id, received_at / 86400;

             CREATE TRIGGER folder_days_after_insert
             AFTER INSERT ON message_placements BEGIN
                 INSERT INTO folder_days (folder_id, day, messages)
                 VALUES (NEW.folder_id, NEW.received_at / 86400, 1)
                 ON CONFLICT(folder_id, day) DO UPDATE SET messages = messages + 1;
             END;

             CREATE TRIGGER folder_days_after_delete
             AFTER DELETE ON message_placements BEGIN
                 UPDATE folder_days SET messages = messages - 1
                 WHERE folder_id = OLD.folder_id AND day = OLD.received_at / 86400;

                 DELETE FROM folder_days WHERE messages <= 0;
             END;

             CREATE TRIGGER folder_days_after_update
             AFTER UPDATE OF folder_id, received_at ON message_placements
             WHEN OLD.folder_id <> NEW.folder_id
               OR OLD.received_at / 86400 <> NEW.received_at / 86400
             BEGIN
                 UPDATE folder_days SET messages = messages - 1
                 WHERE folder_id = OLD.folder_id AND day = OLD.received_at / 86400;

                 INSERT INTO folder_days (folder_id, day, messages)
                 VALUES (NEW.folder_id, NEW.received_at / 86400, 1)
                 ON CONFLICT(folder_id, day) DO UPDATE SET messages = messages + 1;

                 DELETE FROM folder_days WHERE messages <= 0;
             END;",
        ),
        // 017: where a conversation is found from a name.
        //
        // Mail does not arrive in the order it was written: a reply is often stored before the
        // message it answers. So a conversation cannot be found by looking for messages naming
        // this one — that search is over the middle of a text column, and the message doing the
        // naming may not be here yet. Every name a message mentions is written down against the
        // conversation instead, so a message arriving later finds itself already named.
        //
        // No reference to `messages`: the conversation is called after its oldest message, and it
        // outlives that message being removed from every folder. What does happen when the last
        // message of a conversation goes is that its names are forgotten.
        M::up(
            "CREATE TABLE thread_names (
                 account_id INTEGER NOT NULL,
                 name       TEXT    NOT NULL,
                 thread_id  INTEGER NOT NULL,

                 PRIMARY KEY (account_id, name)
             ) STRICT, WITHOUT ROWID;

             -- Merging hands every name of one conversation to another.
             CREATE INDEX idx_thread_names_thread ON thread_names (account_id, thread_id);

             -- Mail stored before this build has no conversation, and is given one a few thousand
             -- at a time. The index is what makes 'is this account caught up' a lookup rather than
             -- a walk, and it is empty once the answer is yes.
             CREATE INDEX idx_messages_unthreaded ON messages (account_id)
                 WHERE thread_id IS NULL;",
        )
        .down(
            "DROP INDEX idx_messages_unthreaded;
             DROP INDEX idx_thread_names_thread;
             DROP TABLE thread_names;",
        ),
        // 018: finding mail by what it says about itself.
        //
        // Subject, sender and recipients only. Message text is encrypted where it is stored, and
        // an index over it here would be that text again, in the clear, beside the thing that was
        // encrypted — so search over what a message *says* is answered by reading and decrypting
        // what this device holds, and never from here.
        //
        // Contentless: the index keeps its tokens and nothing else, so no subject or address is
        // stored a second time. The rows themselves are read from `messages` by id.
        //
        // The Cyrillic yo (U+0451) is folded to ie (U+0435) on the way in and asked for the same way
        // on the way out, written as escapes because code here is English and the letters are only
        // data. It cannot be left to the tokenizer: `remove_diacritics` takes marks off Latin letters
        // only, so `unicode61` keeps yo and ie apart, as it keeps short i (U+0439) and i (U+0438)
        // apart, which are two letters, not one letter and an accent. Everything else about case and
        // folding is `unicode61`'s job and is left to it.
        M::up(
            "CREATE VIRTUAL TABLE message_search USING fts5(
                 subject,
                 people,
                 content='',
                 contentless_delete=1,
                 tokenize='unicode61'
             );

             CREATE TRIGGER message_search_after_insert
             AFTER INSERT ON messages BEGIN
                 INSERT INTO message_search (rowid, subject, people)
                 VALUES (
                     NEW.id,
                     replace(replace(COALESCE(NEW.subject, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(
                         COALESCE(NEW.sender, '') || ' ' ||
                         COALESCE(NEW.sender_address, '') || ' ' ||
                         COALESCE(NEW.recipients_to, '') || ' ' ||
                         COALESCE(NEW.recipients_cc, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'));
             END;

             CREATE TRIGGER message_search_after_delete
             AFTER DELETE ON messages BEGIN
                 DELETE FROM message_search WHERE rowid = OLD.id;
             END;

             -- What a message says about itself is written once, when it is stored, so there is
             -- nothing to follow afterwards. The one exception is a message whose sender or
             -- recipients were filled in later by a build that fetched them separately.
             CREATE TRIGGER message_search_after_update
             AFTER UPDATE OF subject, sender, sender_address, recipients_to, recipients_cc
             ON messages BEGIN
                 DELETE FROM message_search WHERE rowid = OLD.id;

                 INSERT INTO message_search (rowid, subject, people)
                 VALUES (
                     NEW.id,
                     replace(replace(COALESCE(NEW.subject, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(
                         COALESCE(NEW.sender, '') || ' ' ||
                         COALESCE(NEW.sender_address, '') || ' ' ||
                         COALESCE(NEW.recipients_to, '') || ' ' ||
                         COALESCE(NEW.recipients_cc, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'));
             END;

             INSERT INTO message_search (rowid, subject, people)
             SELECT id,
                    replace(replace(COALESCE(subject, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                    replace(replace(
                        COALESCE(sender, '') || ' ' ||
                        COALESCE(sender_address, '') || ' ' ||
                        COALESCE(recipients_to, '') || ' ' ||
                        COALESCE(recipients_cc, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}')
             FROM messages;",
        )
        .down(
            "DROP TRIGGER message_search_after_update;
             DROP TRIGGER message_search_after_delete;
             DROP TRIGGER message_search_after_insert;
             DROP TABLE message_search;",
        ),
        // 019: when a message arrived, as a column rather than as a question asked per row.
        //
        // The views spanning folders — one account, and everything on the device — are ordered by
        // when a message arrived in an inbox, and show only messages that did. Both were correlated
        // subqueries over `message_placements`, so no index could serve either: SQLite read every
        // message, asked twice about each, and sorted the lot in a temporary b-tree. That is the
        // view the application opens on.
        //
        // NULL means the message is in no inbox, so one column answers both questions and one
        // partial index serves both. Role 1 is the inbox.
        M::up(
            "ALTER TABLE messages ADD COLUMN arrived_at INTEGER;

             UPDATE messages SET arrived_at = (
                 SELECT max(p.received_at) FROM message_placements p
                 JOIN folders f ON f.id = p.folder_id
                 WHERE p.message_id = messages.id AND f.role = 1);

             CREATE INDEX idx_messages_arrived ON messages (arrived_at DESC, id DESC)
                 WHERE arrived_at IS NOT NULL;

             CREATE INDEX idx_messages_account_arrived
                 ON messages (account_id, arrived_at DESC, id DESC)
                 WHERE arrived_at IS NOT NULL;

             -- A copy arriving in an inbox can only make the date later, so the hot path is one
             -- conditional write rather than a recount. Everything else recounts, because losing
             -- a copy can make it earlier and nothing row-by-row can tell by how much.
             CREATE TRIGGER messages_arrived_after_insert
             AFTER INSERT ON message_placements
             WHEN (SELECT role FROM folders WHERE id = NEW.folder_id) = 1
             BEGIN
                 UPDATE messages SET arrived_at = NEW.received_at
                 WHERE id = NEW.message_id
                   AND (arrived_at IS NULL OR arrived_at < NEW.received_at);
             END;

             CREATE TRIGGER messages_arrived_after_delete
             AFTER DELETE ON message_placements BEGIN
                 UPDATE messages SET arrived_at = (
                     SELECT max(p.received_at) FROM message_placements p
                     JOIN folders f ON f.id = p.folder_id
                     WHERE p.message_id = OLD.message_id AND f.role = 1)
                 WHERE id = OLD.message_id;
             END;

             CREATE TRIGGER messages_arrived_after_update
             AFTER UPDATE OF folder_id, message_id, received_at ON message_placements BEGIN
                 UPDATE messages SET arrived_at = (
                     SELECT max(p.received_at) FROM message_placements p
                     JOIN folders f ON f.id = p.folder_id
                     WHERE p.message_id = OLD.message_id AND f.role = 1)
                 WHERE id = OLD.message_id;

                 UPDATE messages SET arrived_at = (
                     SELECT max(p.received_at) FROM message_placements p
                     JOIN folders f ON f.id = p.folder_id
                     WHERE p.message_id = NEW.message_id AND f.role = 1)
                 WHERE id = NEW.message_id;
             END;

             -- A folder becoming the inbox, or ceasing to be it, changes this for everything in
             -- it. Rare, and worth the one pass it costs when it happens.
             CREATE TRIGGER messages_arrived_after_role_change
             AFTER UPDATE OF role ON folders
             WHEN OLD.role <> NEW.role
             BEGIN
                 UPDATE messages SET arrived_at = (
                     SELECT max(p.received_at) FROM message_placements p
                     JOIN folders f ON f.id = p.folder_id
                     WHERE p.message_id = messages.id AND f.role = 1)
                 WHERE id IN (SELECT message_id FROM message_placements
                              WHERE folder_id = NEW.id);
             END;",
        )
        .down(
            "DROP TRIGGER messages_arrived_after_role_change;
             DROP TRIGGER messages_arrived_after_update;
             DROP TRIGGER messages_arrived_after_delete;
             DROP TRIGGER messages_arrived_after_insert;
             DROP INDEX idx_messages_account_arrived;
             DROP INDEX idx_messages_arrived;
             ALTER TABLE messages DROP COLUMN arrived_at;",
        ),
        // 020: which copy speaks for a message, and whether it has been read, as columns rather
        // than as questions asked per row.
        //
        // Both were correlated subqueries in the select list. A window hides what they cost:
        // SQLite evaluates a select list only for the rows a LIMIT returns, so fifty rows meant
        // fifty evaluations however deep the window sat. Reading the same view from several files
        // has to produce every row it compares, and then the count is the whole comparison rather
        // than the window.
        //
        // `inbox_unread` is NULL when the message is in no inbox, which is what the subquery it
        // replaces answered: `min()` over no rows is NULL, and NULL is not 0. `inbox_folder_id`
        // falls back to the lowest-numbered copy, so a message filed nowhere near an inbox still
        // names a folder — the search view shows messages that never reached one.
        //
        // The fallback was already ordered; the inbox arm was not, and is now. One account has one
        // inbox, so this settles a choice that had no second case rather than changing one.
        M::up(
            "ALTER TABLE messages ADD COLUMN inbox_folder_id INTEGER;
             ALTER TABLE messages ADD COLUMN inbox_unread INTEGER;

             UPDATE messages SET
                 inbox_folder_id = COALESCE(
                     (SELECT p.folder_id FROM message_placements p
                      JOIN folders f ON f.id = p.folder_id
                      WHERE p.message_id = messages.id AND f.role = 1
                      ORDER BY p.folder_id LIMIT 1),
                     (SELECT p.folder_id FROM message_placements p
                      WHERE p.message_id = messages.id
                      ORDER BY p.folder_id LIMIT 1)),
                 inbox_unread = (
                     SELECT min(p.flags & 1) FROM message_placements p
                     JOIN folders f ON f.id = p.folder_id
                     WHERE p.message_id = messages.id AND f.role = 1);

             -- The unread view was the one filter no index could serve, because the question was
             -- asked of every row before it could be answered.
             CREATE INDEX idx_messages_unread_arrived
                 ON messages (arrived_at DESC, id DESC)
                 WHERE arrived_at IS NOT NULL AND inbox_unread = 0;

             CREATE INDEX idx_messages_account_unread_arrived
                 ON messages (account_id, arrived_at DESC, id DESC)
                 WHERE arrived_at IS NOT NULL AND inbox_unread = 0;

             -- A copy arriving, leaving or being read can move either value in either direction,
             -- and nothing row-by-row can tell by how much. So every one of these recounts, and
             -- there is no conditional shortcut of the kind the arrival date allows.
             CREATE TRIGGER messages_inbox_after_insert
             AFTER INSERT ON message_placements BEGIN
                 UPDATE messages SET
                     inbox_folder_id = COALESCE(
                         (SELECT p.folder_id FROM message_placements p
                          JOIN folders f ON f.id = p.folder_id
                          WHERE p.message_id = NEW.message_id AND f.role = 1
                          ORDER BY p.folder_id LIMIT 1),
                         (SELECT p.folder_id FROM message_placements p
                          WHERE p.message_id = NEW.message_id
                          ORDER BY p.folder_id LIMIT 1)),
                     inbox_unread = (
                         SELECT min(p.flags & 1) FROM message_placements p
                         JOIN folders f ON f.id = p.folder_id
                         WHERE p.message_id = NEW.message_id AND f.role = 1)
                 WHERE id = NEW.message_id;
             END;

             CREATE TRIGGER messages_inbox_after_delete
             AFTER DELETE ON message_placements BEGIN
                 UPDATE messages SET
                     inbox_folder_id = COALESCE(
                         (SELECT p.folder_id FROM message_placements p
                          JOIN folders f ON f.id = p.folder_id
                          WHERE p.message_id = OLD.message_id AND f.role = 1
                          ORDER BY p.folder_id LIMIT 1),
                         (SELECT p.folder_id FROM message_placements p
                          WHERE p.message_id = OLD.message_id
                          ORDER BY p.folder_id LIMIT 1)),
                     inbox_unread = (
                         SELECT min(p.flags & 1) FROM message_placements p
                         JOIN folders f ON f.id = p.folder_id
                         WHERE p.message_id = OLD.message_id AND f.role = 1)
                 WHERE id = OLD.message_id;
             END;

             CREATE TRIGGER messages_inbox_after_update
             AFTER UPDATE OF folder_id, message_id, flags ON message_placements BEGIN
                 UPDATE messages SET
                     inbox_folder_id = COALESCE(
                         (SELECT p.folder_id FROM message_placements p
                          JOIN folders f ON f.id = p.folder_id
                          WHERE p.message_id = OLD.message_id AND f.role = 1
                          ORDER BY p.folder_id LIMIT 1),
                         (SELECT p.folder_id FROM message_placements p
                          WHERE p.message_id = OLD.message_id
                          ORDER BY p.folder_id LIMIT 1)),
                     inbox_unread = (
                         SELECT min(p.flags & 1) FROM message_placements p
                         JOIN folders f ON f.id = p.folder_id
                         WHERE p.message_id = OLD.message_id AND f.role = 1)
                 WHERE id = OLD.message_id;
                 UPDATE messages SET
                     inbox_folder_id = COALESCE(
                         (SELECT p.folder_id FROM message_placements p
                          JOIN folders f ON f.id = p.folder_id
                          WHERE p.message_id = NEW.message_id AND f.role = 1
                          ORDER BY p.folder_id LIMIT 1),
                         (SELECT p.folder_id FROM message_placements p
                          WHERE p.message_id = NEW.message_id
                          ORDER BY p.folder_id LIMIT 1)),
                     inbox_unread = (
                         SELECT min(p.flags & 1) FROM message_placements p
                         JOIN folders f ON f.id = p.folder_id
                         WHERE p.message_id = NEW.message_id AND f.role = 1)
                 WHERE id = NEW.message_id;
             END;

             -- A folder becoming the inbox, or ceasing to be it, changes both answers for
             -- everything filed in it.
             CREATE TRIGGER messages_inbox_after_role_change
             AFTER UPDATE OF role ON folders
             WHEN OLD.role <> NEW.role
             BEGIN
                 UPDATE messages SET
                     inbox_folder_id = COALESCE(
                         (SELECT p.folder_id FROM message_placements p
                          JOIN folders f ON f.id = p.folder_id
                          WHERE p.message_id = messages.id AND f.role = 1
                          ORDER BY p.folder_id LIMIT 1),
                         (SELECT p.folder_id FROM message_placements p
                          WHERE p.message_id = messages.id
                          ORDER BY p.folder_id LIMIT 1)),
                     inbox_unread = (
                         SELECT min(p.flags & 1) FROM message_placements p
                         JOIN folders f ON f.id = p.folder_id
                         WHERE p.message_id = messages.id AND f.role = 1)
                 WHERE id IN (SELECT message_id FROM message_placements
                              WHERE folder_id = NEW.id);
             END;",
        )
        .down(
            "DROP TRIGGER messages_inbox_after_role_change;
             DROP TRIGGER messages_inbox_after_update;
             DROP TRIGGER messages_inbox_after_delete;
             DROP TRIGGER messages_inbox_after_insert;
             DROP INDEX idx_messages_account_unread_arrived;
             DROP INDEX idx_messages_unread_arrived;
             ALTER TABLE messages DROP COLUMN inbox_unread;
             ALTER TABLE messages DROP COLUMN inbox_folder_id;",
        ),
        // 021: a number this file has handed out is never handed out again.
        //
        // `INTEGER PRIMARY KEY` alone reuses: empty the table and the next message takes the
        // number the last one had. An identifier that has already reached the interface, the
        // router, a cache and `op_log` then names a different message in each of them. The
        // sequence `AUTOINCREMENT` keeps survives a delete, and a table cannot be given it by
        // being altered — it has to be built again and everything on it put back.
        M::up_with_hook("", tables_that_hand_out_numbers_get_a_sequence)
            .down_with_hook("", tables_that_hand_out_numbers_lose_their_sequence),
        // 022: the conversations this file holds, kept by the database rather than counted.
        //
        // A list of conversations needs its own length: how many messages there are is not how many
        // rows it shows, and the scrollbar is drawn from it. Counting them is a scan — 60 ms at two
        // hundred thousand messages against 3.9 for counting messages, and seconds at the sizes
        // this is built for.
        //
        // So they are kept. Not as a number somebody adds to and subtracts from — a number kept by
        // hand drifts, and a drifted one draws a scrollbar the wrong length for ever — but as a row
        // per conversation, maintained by the same triggers that see every message arrive and
        // leave. Counting them is then reading the size of a small table: 0.04 ms whatever the
        // mailbox holds.
        //
        // Measured: storing two hundred thousand messages goes from 210 ms to 669. Those messages
        // arrive over a network that takes minutes to deliver them, so half a second is not where
        // the time goes — and the count is exact rather than nearly right.
        //
        // `messages` is the size of the conversation, which the row showing it needs anyway.
        M::up(
            "CREATE TABLE conversations (
                 thread_id      INTEGER PRIMARY KEY,
                 -- Zero for the instant between a message leaving and the row going with it.
                 -- Below zero is a trigger that is wrong, which is what this catches.
                 messages       INTEGER NOT NULL CHECK (messages >= 0),
                 -- When its newest message arrived, and which one that is. The list is ordered by
                 -- the first and shows the second, so both are here rather than asked for per row:
                 -- asked for, they are the scan this table exists to avoid.
                 newest_at      INTEGER NOT NULL,
                 newest_message INTEGER NOT NULL
             ) STRICT, WITHOUT ROWID;

             INSERT INTO conversations (thread_id, messages, newest_at, newest_message)
             SELECT m.thread_id, count(*), COALESCE(max(m.arrived_at), 0),
                    (SELECT n.id FROM messages n WHERE n.thread_id = m.thread_id
                     ORDER BY n.arrived_at DESC, n.id DESC LIMIT 1)
             FROM messages m WHERE m.thread_id IS NOT NULL GROUP BY m.thread_id;

             CREATE INDEX idx_conversations_newest
                 ON conversations (newest_at DESC, thread_id DESC);

             CREATE TRIGGER conversations_after_insert
             AFTER INSERT ON messages WHEN NEW.thread_id IS NOT NULL BEGIN
                 INSERT INTO conversations (thread_id, messages, newest_at, newest_message)
                 VALUES (NEW.thread_id, 1, COALESCE(NEW.arrived_at, 0), NEW.id)
                 ON CONFLICT(thread_id) DO UPDATE SET
                     messages = messages + 1,
                     -- The message before the date, because the date is about to move.
                     -- The pair, not the date: two messages can share a second, and then which
                     -- of them speaks for the conversation is decided by the number — the same
                     -- tie-break the list is ordered by, or the row would name one message and
                     -- sit in the place of another.
                     newest_message = CASE
                         WHEN (COALESCE(NEW.arrived_at, 0), NEW.id) > (newest_at, newest_message)
                         THEN NEW.id ELSE newest_message END,
                     newest_at = max(newest_at, COALESCE(NEW.arrived_at, 0));
             END;

             -- Losing a message can make the conversation older, and nothing row-by-row can say
             -- by how much, so what is left is looked at. Mail leaves far less often than it
             -- arrives, which is why the arriving side is arithmetic and this side is a question.
             CREATE TRIGGER conversations_after_delete
             AFTER DELETE ON messages WHEN OLD.thread_id IS NOT NULL BEGIN
                 UPDATE conversations SET
                     messages = messages - 1,
                     newest_at = COALESCE((SELECT max(n.arrived_at) FROM messages n
                                           WHERE n.thread_id = OLD.thread_id), 0),
                     newest_message = COALESCE((SELECT n.id FROM messages n
                                                WHERE n.thread_id = OLD.thread_id
                                                ORDER BY n.arrived_at DESC, n.id DESC LIMIT 1), 0)
                 WHERE thread_id = OLD.thread_id;

                 DELETE FROM conversations WHERE thread_id = OLD.thread_id AND messages <= 0;
             END;

             -- A message joins a conversation after it is stored, and two conversations tie into
             -- one, so the conversation a message belongs to changes more often than a message is
             -- stored. Both sides move.
             CREATE TRIGGER conversations_after_update
             AFTER UPDATE OF thread_id ON messages
             WHEN OLD.thread_id IS NOT NEW.thread_id
             BEGIN
                 UPDATE conversations SET
                     messages = messages - 1,
                     newest_at = COALESCE((SELECT max(n.arrived_at) FROM messages n
                                           WHERE n.thread_id = OLD.thread_id), 0),
                     newest_message = COALESCE((SELECT n.id FROM messages n
                                                WHERE n.thread_id = OLD.thread_id
                                                ORDER BY n.arrived_at DESC, n.id DESC LIMIT 1), 0)
                 WHERE thread_id = OLD.thread_id;

                 DELETE FROM conversations WHERE thread_id = OLD.thread_id AND messages <= 0;

                 INSERT INTO conversations (thread_id, messages, newest_at, newest_message)
                 SELECT NEW.thread_id, 1, COALESCE(NEW.arrived_at, 0), NEW.id
                 WHERE NEW.thread_id IS NOT NULL
                 ON CONFLICT(thread_id) DO UPDATE SET
                     messages = messages + 1,
                     -- The pair, not the date: two messages can share a second, and then which
                     -- of them speaks for the conversation is decided by the number — the same
                     -- tie-break the list is ordered by, or the row would name one message and
                     -- sit in the place of another.
                     newest_message = CASE
                         WHEN (COALESCE(NEW.arrived_at, 0), NEW.id) > (newest_at, newest_message)
                         THEN NEW.id ELSE newest_message END,
                     newest_at = max(newest_at, COALESCE(NEW.arrived_at, 0));
             END;",
        )
        .down(
            "DROP TRIGGER conversations_after_update;
             DROP TRIGGER conversations_after_delete;
             DROP TRIGGER conversations_after_insert;
             DROP INDEX idx_conversations_newest;
             DROP TABLE conversations;",
        ),
        // What a message says, as far as this device holds it, in the index that already holds who
        // wrote it and what it was called.
        //
        // The whole file is encrypted now, so an index over message text is no longer that text in
        // the clear beside something sealed — which is the reason it was left out. What goes in is
        // `preview`: the opening of the message, parsed out of the two kilobytes every
        // synchronisation already fetches and cleaned of the reply it quotes. So this covers every
        // message rather than the few somebody opened, and costs no byte of network and no row of
        // storage — the index is contentless and keeps tokens alone.
        //
        // The table is rebuilt rather than altered: FTS5 has no `ADD COLUMN`. Rebuilt from
        // `messages`, which holds everything it indexes, so nothing is lost by dropping it.
        M::up(
            "DROP TRIGGER message_search_after_update;
             DROP TRIGGER message_search_after_delete;
             DROP TRIGGER message_search_after_insert;
             DROP TABLE message_search;

             CREATE VIRTUAL TABLE message_search USING fts5(
                 subject,
                 people,
                 text,
                 content='',
                 contentless_delete=1,
                 tokenize='unicode61'
             );

             INSERT INTO message_search (rowid, subject, people, text)
             SELECT
                 id,
                 replace(replace(COALESCE(subject, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                 replace(replace(
                     COALESCE(sender, '') || ' ' ||
                     COALESCE(sender_address, '') || ' ' ||
                     COALESCE(recipients_to, '') || ' ' ||
                     COALESCE(recipients_cc, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                 replace(replace(COALESCE(preview, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}')
             FROM messages;

             CREATE TRIGGER message_search_after_insert
             AFTER INSERT ON messages BEGIN
                 INSERT INTO message_search (rowid, subject, people, text)
                 VALUES (
                     NEW.id,
                     replace(replace(COALESCE(NEW.subject, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(
                         COALESCE(NEW.sender, '') || ' ' ||
                         COALESCE(NEW.sender_address, '') || ' ' ||
                         COALESCE(NEW.recipients_to, '') || ' ' ||
                         COALESCE(NEW.recipients_cc, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(COALESCE(NEW.preview, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'));
             END;

             CREATE TRIGGER message_search_after_delete
             AFTER DELETE ON messages BEGIN
                 DELETE FROM message_search WHERE rowid = OLD.id;
             END;

             -- `preview` joins the list this follows: a message stored by a pass that had not read
             -- its opening yet gets one filled in afterwards, and an index that missed that update
             -- would answer about a message it holds nothing of.
             CREATE TRIGGER message_search_after_update
             AFTER UPDATE OF subject, sender, sender_address, recipients_to, recipients_cc, preview
             ON messages BEGIN
                 DELETE FROM message_search WHERE rowid = OLD.id;

                 INSERT INTO message_search (rowid, subject, people, text)
                 VALUES (
                     NEW.id,
                     replace(replace(COALESCE(NEW.subject, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(
                         COALESCE(NEW.sender, '') || ' ' ||
                         COALESCE(NEW.sender_address, '') || ' ' ||
                         COALESCE(NEW.recipients_to, '') || ' ' ||
                         COALESCE(NEW.recipients_cc, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(COALESCE(NEW.preview, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'));
             END;",
        )
        .down("SELECT 1;"),
        // The whole of what a message says, for the messages this device holds whole.
        //
        // `preview` is the opening a synchronisation pass reads, and it is what the index holds
        // until the message itself is here. This column is the rest of it, written from the message
        // in hand and never by asking a server again — so the index answers about the opening of
        // every message and about the whole of every message that has been downloaded, and the
        // trigger picks whichever of the two is known.
        //
        // Kept as its own column rather than indexed straight from Rust: the index is contentless
        // and cannot be read back, so a row rebuilt for any other reason — a subject filled in
        // late — would silently drop the text if this were not written down somewhere.
        M::up(
            "ALTER TABLE messages ADD COLUMN body_text TEXT;

             DROP TRIGGER message_search_after_update;
             DROP TRIGGER message_search_after_delete;
             DROP TRIGGER message_search_after_insert;
             CREATE TRIGGER message_search_after_insert
             AFTER INSERT ON messages BEGIN
                 INSERT INTO message_search (rowid, subject, people, text)
                 VALUES (
                     NEW.id,
                     replace(replace(COALESCE(NEW.subject, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(
                         COALESCE(NEW.sender, '') || ' ' ||
                         COALESCE(NEW.sender_address, '') || ' ' ||
                         COALESCE(NEW.recipients_to, '') || ' ' ||
                         COALESCE(NEW.recipients_cc, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(COALESCE(NEW.body_text, NEW.preview, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'));
             END;

             CREATE TRIGGER message_search_after_delete
             AFTER DELETE ON messages BEGIN
                 DELETE FROM message_search WHERE rowid = OLD.id;
             END;

             CREATE TRIGGER message_search_after_update
             AFTER UPDATE OF subject, sender, sender_address, recipients_to, recipients_cc,
                             preview, body_text
             ON messages BEGIN
                 DELETE FROM message_search WHERE rowid = OLD.id;

                 INSERT INTO message_search (rowid, subject, people, text)
                 VALUES (
                     NEW.id,
                     replace(replace(COALESCE(NEW.subject, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(
                         COALESCE(NEW.sender, '') || ' ' ||
                         COALESCE(NEW.sender_address, '') || ' ' ||
                         COALESCE(NEW.recipients_to, '') || ' ' ||
                         COALESCE(NEW.recipients_cc, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(COALESCE(NEW.body_text, NEW.preview, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'));
             END;",
        )
        .down(
            // The triggers go first: dropping a column a trigger names is refused while it names
            // it, and what comes back is the pair that indexed the opening alone.
            "DROP TRIGGER message_search_after_update;
             DROP TRIGGER message_search_after_delete;
             DROP TRIGGER message_search_after_insert;

             ALTER TABLE messages DROP COLUMN body_text;

             CREATE TRIGGER message_search_after_insert
             AFTER INSERT ON messages BEGIN
                 INSERT INTO message_search (rowid, subject, people, text)
                 VALUES (
                     NEW.id,
                     replace(replace(COALESCE(NEW.subject, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(
                         COALESCE(NEW.sender, '') || ' ' ||
                         COALESCE(NEW.sender_address, '') || ' ' ||
                         COALESCE(NEW.recipients_to, '') || ' ' ||
                         COALESCE(NEW.recipients_cc, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(COALESCE(NEW.preview, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'));
             END;

             CREATE TRIGGER message_search_after_delete
             AFTER DELETE ON messages BEGIN
                 DELETE FROM message_search WHERE rowid = OLD.id;
             END;

             CREATE TRIGGER message_search_after_update
             AFTER UPDATE OF subject, sender, sender_address, recipients_to, recipients_cc, preview
             ON messages BEGIN
                 DELETE FROM message_search WHERE rowid = OLD.id;

                 INSERT INTO message_search (rowid, subject, people, text)
                 VALUES (
                     NEW.id,
                     replace(replace(COALESCE(NEW.subject, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(
                         COALESCE(NEW.sender, '') || ' ' ||
                         COALESCE(NEW.sender_address, '') || ' ' ||
                         COALESCE(NEW.recipients_to, '') || ' ' ||
                         COALESCE(NEW.recipients_cc, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'),
                     replace(replace(COALESCE(NEW.preview, ''), '\u{0451}', '\u{0435}'), '\u{0401}', '\u{0415}'));
             END;",
        ),
        // 025: a conversation follows the date its newest message arrived, and not only the
        // conversation that message is in.
        //
        // `conversations` caches `newest_at`, and `newest_at` is `messages.arrived_at`. But
        // `arrived_at` is not written when a message is stored: it is written by a trigger on
        // `message_placements`, which is a later statement, and by three more that recount it when
        // a copy is removed, moved, or when a folder becomes the inbox or stops being it. The
        // triggers of 022 fire on the message row and on its conversation changing, and both of
        // those happen before any copy exists — so every conversation was cached with the zero
        // that `COALESCE(arrived_at, 0)` gives for a column still empty, and nothing went back.
        //
        // The list reads `WHERE newest_at > 0`, so the effect was the whole of it: mail arriving
        // by an ordinary synchronisation produced conversation rows that no view could show, and a
        // new installation opened on an empty list. Only the tests disagreed, because the fixture
        // that placed a conversation wrote `arrived_at` in the same statement as `thread_id` — a
        // row no synchronisation can produce.
        //
        // The missing piece is one trigger. All four ways the date moves end in the same
        // `UPDATE messages SET arrived_at`, so following that column follows all of them.
        // Recounted rather than nudged, because a copy going away can make a conversation older
        // and nothing row-by-row can say by how much — the same reason 022 recounts on the way
        // out.
        //
        // Recounting on the way **in** as well was a mistake, and 026 undoes it. Measured through
        // `apply` on one conversation: 1.06 s at a thousand messages, 2.81 at two thousand, 8.48
        // at four — a doubling costing three times as much, because every message stored read the
        // whole conversation and sorted it. The suite could not see it: every message a fixture
        // makes carries its own Message-ID and no references, so every conversation in every test
        // holds exactly one message.
        M::up(
            // Every row already written carries the zero, so they are all built again from what
            // the messages say. Cheap at any size this runs at: one grouped pass over an index.
            "DELETE FROM conversations;

             INSERT INTO conversations (thread_id, messages, newest_at, newest_message)
             SELECT m.thread_id, count(*), COALESCE(max(m.arrived_at), 0),
                    (SELECT n.id FROM messages n WHERE n.thread_id = m.thread_id
                     ORDER BY n.arrived_at DESC, n.id DESC LIMIT 1)
             FROM messages m WHERE m.thread_id IS NOT NULL GROUP BY m.thread_id;

             -- `UPDATE OF` fires on the column being named in the SET list rather than on its
             -- value changing, and the recounting triggers name it whether or not it moves, so
             -- the guard is on the value.
             CREATE TRIGGER conversations_after_arrived
             AFTER UPDATE OF arrived_at ON messages
             WHEN OLD.arrived_at IS NOT NEW.arrived_at AND NEW.thread_id IS NOT NULL
             BEGIN
                 UPDATE conversations SET
                     newest_at = COALESCE((SELECT max(n.arrived_at) FROM messages n
                                           WHERE n.thread_id = NEW.thread_id), 0),
                     newest_message = COALESCE((SELECT n.id FROM messages n
                                                WHERE n.thread_id = NEW.thread_id
                                                ORDER BY n.arrived_at DESC, n.id DESC LIMIT 1), 0)
                 WHERE thread_id = NEW.thread_id;
             END;",
        )
        .down("DROP TRIGGER conversations_after_arrived;"),
        // 026: the arriving side of a conversation is arithmetic again.
        //
        // 022 says it in its own comment — the arriving side is arithmetic and the leaving side is
        // a question — and 025 put a question on the arriving side. It recomputed the whole
        // conversation for every message stored, and the second half of that recount sorts it:
        // `ORDER BY arrived_at DESC, id DESC LIMIT 1` has no index to walk, so SQLite builds a
        // temporary b-tree over the conversation each time.
        //
        // The cost is quadratic in the size of one conversation, measured through `apply`: 1.06 s
        // at a thousand messages, 2.81 at two thousand, 8.48 at four. A thread of a few messages
        // pays nothing anybody can measure; a mailing list of thousands makes a synchronisation
        // hold its write open for seconds.
        //
        // Split by what the date did. Arriving, or moving forward, a conversation can only become
        // newer, and which message speaks for it is decided by the same pair the list is ordered
        // by — one row, found by its key. Going away or moving back, nothing row-by-row can say
        // how much older it became, so what is left is looked at. Only the second reads the
        // conversation, and only a copy being removed or a folder ceasing to be the inbox reaches
        // it.
        M::up(
            "DROP TRIGGER conversations_after_arrived;

             CREATE TRIGGER conversations_after_arrived
             AFTER UPDATE OF arrived_at ON messages
             WHEN NEW.thread_id IS NOT NULL
              AND NEW.arrived_at IS NOT NULL
              AND (OLD.arrived_at IS NULL OR NEW.arrived_at > OLD.arrived_at)
             BEGIN
                 UPDATE conversations SET
                     -- The message before the date, because the date is about to move, and the
                     -- pair rather than the date: two messages can share a second, and then which
                     -- of them speaks for the conversation is decided by the number — the same
                     -- tie-break the list is ordered by.
                     newest_message = CASE
                         WHEN (NEW.arrived_at, NEW.id) > (newest_at, newest_message)
                         THEN NEW.id ELSE newest_message END,
                     newest_at = max(newest_at, NEW.arrived_at)
                 WHERE thread_id = NEW.thread_id;
             END;

             CREATE TRIGGER conversations_after_unarrived
             AFTER UPDATE OF arrived_at ON messages
             WHEN NEW.thread_id IS NOT NULL
              AND OLD.arrived_at IS NOT NULL
              AND (NEW.arrived_at IS NULL OR NEW.arrived_at < OLD.arrived_at)
             BEGIN
                 UPDATE conversations SET
                     newest_at = COALESCE((SELECT max(n.arrived_at) FROM messages n
                                           WHERE n.thread_id = NEW.thread_id), 0),
                     newest_message = COALESCE((SELECT n.id FROM messages n
                                                WHERE n.thread_id = NEW.thread_id
                                                ORDER BY n.arrived_at DESC, n.id DESC LIMIT 1), 0)
                 WHERE thread_id = NEW.thread_id;
             END;",
        )
        .down(
            "DROP TRIGGER conversations_after_unarrived;
             DROP TRIGGER conversations_after_arrived;

             CREATE TRIGGER conversations_after_arrived
             AFTER UPDATE OF arrived_at ON messages
             WHEN OLD.arrived_at IS NOT NEW.arrived_at AND NEW.thread_id IS NOT NULL
             BEGIN
                 UPDATE conversations SET
                     newest_at = COALESCE((SELECT max(n.arrived_at) FROM messages n
                                           WHERE n.thread_id = NEW.thread_id), 0),
                     newest_message = COALESCE((SELECT n.id FROM messages n
                                                WHERE n.thread_id = NEW.thread_id
                                                ORDER BY n.arrived_at DESC, n.id DESC LIMIT 1), 0)
                 WHERE thread_id = NEW.thread_id;
             END;",
        ),
        // 027: when a message was read whole, as the answer to the question the pass actually asks.
        //
        // The pass asks which messages it has not read whole yet, and it was answered with "which
        // have no stored body". Those are different questions, and every fault in this area came
        // out of treating them as one. A message the store declines to keep — over eight megabytes,
        // where a local copy of a video is a local copy of a video — has no stored body however
        // often it is read, so it came back at the head of every round for ever. Excluding it by
        // its size stopped that and cost the other half: never fetched is never read, so nothing
        // put what it says into the index, and mail over that size could not be found by a word
        // from its own text.
        //
        // Recorded once, and the question answers itself: a message not read whole is fetched,
        // read for what it says, and marked — kept or not, extracted or not. Nothing is fetched
        // twice, and nothing is left out of the index for being large.
        //
        // A message whose body is here **and whose text reached the index** was read whole to get
        // both, and says so from when it was stored. Not the body alone: bodies have been kept
        // since 012 and `body_text` only arrives at 024, so a message stored by a build in between
        // has its body here and its words in no index. Marking that one read would leave it out of
        // every search for ever. Left unmarked it is read once more — over the network, which is
        // the price of the words — and then marked whatever comes of it.
        M::up(
            "ALTER TABLE messages ADD COLUMN read_whole_at INTEGER;

             UPDATE messages SET read_whole_at = (
                 SELECT b.stored_at FROM message_bodies b WHERE b.message_id = messages.id)
             WHERE body_text IS NOT NULL
               AND EXISTS (SELECT 1 FROM message_bodies b WHERE b.message_id = messages.id);",
        )
        .down("ALTER TABLE messages DROP COLUMN read_whole_at;"),
        // 028: whether a folder's last pass read new mail only as far as one window.
        //
        // What the pass after it needs if it cannot read the folder at all. A message filed into
        // the folder past where that window ended has lost its last copy elsewhere and has not
        // been seen arriving here; forgotten while the folder fails, it comes back as a stranger,
        // read again from the server. Every folder starts as one that did not stop short.
        M::up("ALTER TABLE folders ADD COLUMN stopped_short INTEGER NOT NULL DEFAULT 0;")
            .down("ALTER TABLE folders DROP COLUMN stopped_short;"),
        // 029: whether a reading of a folder stands refused for a moment, one column for the new
        // mail and one for the range further back.
        //
        // A refusal a server says is a moment's matter is asked again rather than kept as rows
        // with no name, and a server may say it of the same message on every pass: taken at its
        // word each time, the folder stops there for good. What the next reading needs is whether
        // one of its kind was refused so with none of its kind applied since, and refused so again
        // then it is a refusal like any other. Two, because the two readings of a pass are refused
        // apart: one column taken back by the reading of new mail would stall the reading further
        // back for ever. Every folder starts as one nothing was refused for.
        M::up(
            "ALTER TABLE folders ADD COLUMN new_mail_refused_for_now INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE folders ADD COLUMN further_back_refused_for_now
                 INTEGER NOT NULL DEFAULT 0;",
        )
        .down(
            "ALTER TABLE folders DROP COLUMN further_back_refused_for_now;
             ALTER TABLE folders DROP COLUMN new_mail_refused_for_now;",
        ),
        // 030: which message a row handed its copies to.
        //
        // A row a pass made of a message its server would not describe has no name, and once the
        // message is read whole and turns out to be one the account already holds under its name,
        // that one takes the row's copies. The row's own identifier may have reached the window
        // and the queue by then — somebody may have opened it, marked it read or filed it — and
        // what is asked about it has to find the copies where they went.
        //
        // A table of its own, because what is asked about the row can outlast the row: left with
        // no copy, it is forgotten by the sweep at the end of a pass once nothing queued names it,
        // like any other message with no copy, while the reader may still be open on it. Keyed by
        // the row's number, which this file never hands out again, so a pointer kept after the
        // row cannot be taken for another message's; and it goes with the message it points at,
        // so a row whose message is gone goes by itself, and finds no copy.
        M::up(
            "CREATE TABLE handed_over (
                 message_id INTEGER PRIMARY KEY,
                 handed_to  INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE
             );

             CREATE INDEX handed_over_by_holder ON handed_over (handed_to);",
        )
        .down("DROP TABLE handed_over;"),
        // 031: what nothing writes and nothing reads, taken out: a message's parsed structure and
        // folded subject, a folder's last clean read and last error, the keywords a message
        // carried, and the index for unread kept over a message's own flags. Each copy keeps its
        // own flags, so every message stood in that index and no question was asked of it.
        M::up(
            "DROP INDEX idx_messages_unread;
             DROP TABLE message_keywords;
             ALTER TABLE messages DROP COLUMN bodystructure;
             ALTER TABLE messages DROP COLUMN subject_norm;
             ALTER TABLE folders DROP COLUMN last_error_kind;
             ALTER TABLE folders DROP COLUMN last_successful_sync_at;",
        )
        .down(
            "ALTER TABLE folders ADD COLUMN last_successful_sync_at INTEGER;
             ALTER TABLE folders ADD COLUMN last_error_kind TEXT;
             ALTER TABLE messages ADD COLUMN subject_norm TEXT;
             ALTER TABLE messages ADD COLUMN bodystructure BLOB;
             CREATE TABLE message_keywords (
                 message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                 keyword    TEXT NOT NULL,
                 PRIMARY KEY (message_id, keyword)
             ) STRICT, WITHOUT ROWID;
             CREATE INDEX idx_messages_unread ON messages (account_id, received_at DESC)
                 WHERE flags & 1 = 0;",
        ),
    ])
}

/// Runs migrations with foreign keys off, and refuses to finish if that left anything dangling.
///
/// A migration that rebuilds a table drops the old one, and under `PRAGMA foreign_keys = ON` —
/// which every pooled connection sets — dropping a table performs the deletes its children asked
/// for: `message_placements` references `messages` `ON DELETE CASCADE`, so rebuilding `messages`
/// empties it. There is no error and the migration succeeds; the mail is simply gone. The pragma
/// is a no-op inside a transaction, so a migration cannot turn it off for itself and this has to
/// happen out here.
///
/// `foreign_key_check` afterwards is the other half: with the enforcement off, a migration that
/// really did break a reference would otherwise be committed and only fail much later, somewhere
/// that cannot say what did it.
fn migrating<M>(connection: &mut Connection, path: &str, run: M) -> Result<(), StorageError>
where
    M: FnOnce(&mut Connection) -> Result<(), rusqlite_migration::Error>,
{
    let failed = |reason: String| StorageError::Migrate {
        path: path.to_owned(),
        reason,
    };

    connection
        .execute_batch("PRAGMA foreign_keys = OFF;")
        .map_err(|error| failed(error.to_string()))?;

    let outcome = run(connection).map_err(|error| failed(error.to_string()));

    let dangling: Result<Option<String>, _> = connection
        .query_row("PRAGMA foreign_key_check", [], |row| {
            row.get::<_, String>(0)
        })
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        });

    // Back on before anything is handed back, whether the migration worked or not: this connection
    // goes on to serve reads and writes, and it must enforce what the schema says.
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|error| failed(error.to_string()))?;

    outcome?;

    match dangling.map_err(|error| failed(error.to_string()))? {
        Some(table) => Err(failed(format!(
            "the upgrade left rows in {table} pointing at nothing"
        ))),
        None => Ok(()),
    }
}

/// What the schema says about one kind of object, so a rebuild can put back what it takes away.
///
/// Read rather than retyped. A migration holding a copy of a definition an earlier migration wrote
/// would be a second source for it, and the two could only drift: a migration already applied can
/// never be edited, so the copy could not be corrected either.
fn definitions(
    transaction: &Transaction,
    kind: &str,
    on_table: Option<&str>,
) -> Result<Vec<(String, String)>, rusqlite::Error> {
    // A `sql` of nothing is an index SQLite made for a constraint; it comes back with the table.
    let mut statement = transaction.prepare(
        "SELECT name, sql FROM sqlite_schema
         WHERE type = ?1 AND sql IS NOT NULL AND (?2 IS NULL OR tbl_name = ?2)
         ORDER BY name",
    )?;
    let found = statement
        .query_map(rusqlite::params![kind, on_table], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(found)
}

fn columns_of(transaction: &Transaction, table: &str) -> Result<String, rusqlite::Error> {
    let mut statement = transaction.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names.join(", "))
}

/// Builds `messages` and `folders` again, with a sequence that a delete does not rewind.
///
/// Every trigger comes off first, not only the ones on these two tables: `ALTER TABLE ... RENAME`
/// re-reads the whole schema, and a trigger naming a table that is not there at that moment stops
/// the rename. The triggers that keep the copies in step sit on `message_placements` and speak
/// about `messages`.
///
/// Copying with the numbers unchanged leaves the search index correct, and sets the sequence to the
/// largest number already used, so nothing is renumbered here.
fn rebuilt_carrying(transaction: &Transaction, sequence: bool) -> HookResult {
    let triggers = definitions(transaction, "trigger", None)?;
    for (name, _) in &triggers {
        transaction.execute_batch(&format!("DROP TRIGGER {name};"))?;
    }

    for table in ["messages", "folders"] {
        let indexes = definitions(transaction, "index", Some(table))?;
        let (_, declared) = definitions(transaction, "table", Some(table))?
            .pop()
            .ok_or_else(|| rusqlite::Error::InvalidParameterName(format!("no table {table}")))?;

        // From the first bracket onward, because how the name is written is not fixed: a table
        // that has been renamed comes back as `CREATE TABLE "messages" (`, quoted, and a rebuild
        // renames. Matching the name as text worked once and then silently stopped.
        let body = declared.find('(').ok_or_else(|| {
            rusqlite::Error::InvalidParameterName(format!("{table} is declared without columns"))
        })?;
        let mut building = format!("CREATE TABLE {table}_rebuilt {}", &declared[body..]);
        building = if sequence {
            building.replacen(
                "INTEGER PRIMARY KEY",
                "INTEGER PRIMARY KEY AUTOINCREMENT",
                1,
            )
        } else {
            // Going back reads a declaration that already carries it, so this takes it off rather
            // than merely not putting it on.
            building.replacen(
                "INTEGER PRIMARY KEY AUTOINCREMENT",
                "INTEGER PRIMARY KEY",
                1,
            )
        };
        // Both edits are string surgery on what the schema handed back, so both are checked
        // rather than assumed: a rebuild that quietly kept the old shape would look like a
        // migration that worked.
        if building.contains("AUTOINCREMENT") != sequence {
            return Err(HookError::RusqliteError(
                rusqlite::Error::InvalidParameterName(format!(
                    "the declaration of {table} is not shaped as this rebuild expects"
                )),
            ));
        }

        let held = columns_of(transaction, table)?;
        transaction.execute_batch(&format!(
            "{building};
             INSERT INTO {table}_rebuilt ({held}) SELECT {held} FROM {table};
             DROP TABLE {table};
             ALTER TABLE {table}_rebuilt RENAME TO {table};"
        ))?;

        for (_, sql) in indexes {
            transaction.execute_batch(&format!("{sql};"))?;
        }
    }

    for (_, sql) in triggers {
        transaction.execute_batch(&format!("{sql};"))?;
    }

    Ok(())
}

fn tables_that_hand_out_numbers_get_a_sequence(transaction: &Transaction) -> HookResult {
    rebuilt_carrying(transaction, true)
}

fn tables_that_hand_out_numbers_lose_their_sequence(transaction: &Transaction) -> HookResult {
    rebuilt_carrying(transaction, false)
}

pub fn migrate_index(connection: &mut Connection, path: &str) -> Result<(), StorageError> {
    let migrations = index_migrations();
    refusing_a_newer_build(connection, path, &migrations)?;
    migrating(connection, path, |connection| {
        migrations.to_latest(connection)
    })
}

pub fn migrate_profile(connection: &mut Connection, path: &str) -> Result<(), StorageError> {
    let migrations = profile_migrations();
    refusing_a_newer_build(connection, path, &migrations)?;
    migrating(connection, path, |connection| {
        migrations.to_latest(connection)
    })
}

/// The version a newer build left this profile at, when it is past every migration here.
pub fn written_ahead_of_the_profile(connection: &Connection) -> Option<usize> {
    ahead_of(connection, &profile_migrations())
}

/// The version a newer build left this file of mail at, when it is past every migration here.
pub fn written_ahead_of_the_index(connection: &Connection) -> Option<usize> {
    ahead_of(connection, &index_migrations())
}

fn ahead_of(connection: &Connection, migrations: &Migrations<'_>) -> Option<usize> {
    match migrations.current_version(connection) {
        Ok(SchemaVersion::Outside(version)) => Some(version.get()),
        _ => None,
    }
}

/// Refuses a file a newer build wrote, before anything is done to it.
///
/// Nothing here migrates downward: this build does not have the migrations that put the file where
/// it is, so it cannot undo them, and the newer build reads the file as it stands. Asked before
/// `migrating` turns enforcement off, so a refused file is one nothing was run against.
fn refusing_a_newer_build(
    connection: &Connection,
    path: &str,
    migrations: &Migrations<'_>,
) -> Result<(), StorageError> {
    match ahead_of(connection, migrations) {
        Some(found) => Err(StorageError::FromANewerBuild {
            path: path.to_owned(),
            found,
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests;
