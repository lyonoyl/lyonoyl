# Data model

The tables below are among those `src-tauri/src/infrastructure/sqlite/migrations/mod.rs` builds, and that file holds every one. Each is marked `derived` (may be dropped and re-synced) or `authoritative` (the only copy; never deleted automatically).

## Measured sizes

`a_hundred_thousand_messages`, a test started by hand, stores 100,000 messages through the synchronization mail arrives by—each with a subject, a sender, and an opening of some 170 characters—and prints the size of the file, every page encrypted, after a checkpoint:

| Object | Size |
| --- | --- |
| **The whole file**: the messages, their copies, the word index, and conversations | **87.3 MB (0.89 KB per message)** |
| Raw bodies and attachments | in the same file, sealed, one row per message |

What the word index costs over whole bodies was recorded once, on ~3 KB of text per message:

| Index | Size |
| --- | --- |
| FTS5 over bodies, `detail=full` | +215 MB |
| FTS5 over bodies, `detail=column` | +121 MB |
| FTS5 over bodies, `detail=none` | +61 MB |

Do **not** enable FTS prefix indexes: measured to triple the index size.

## Three levels of identity

```
MessageId(u64)              stable local surrogate — the only one reaching UI, router, op_log
Placement(folder_id, uid)   protocol address, valid only with the current folders.uid_validity
rfc822 Message-ID           weak hint for dedup and threading; not unique, not trusted
```

`messages` has **no** `uid` column. A `MessageId` carries its account in its top bits, and a file never hands the same one out twice.

## Core (accounts/<number>.sqlite3, derived—one file per account)

```sql
-- derived
CREATE TABLE messages (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,  -- starts at account << 40
  account_id         INTEGER NOT NULL,
  rfc822_message_id  TEXT,
  thread_id          INTEGER,
  subject            TEXT,
  sender             TEXT,
  sender_address     TEXT,
  recipients_to      TEXT,
  recipients_cc      TEXT,
  references_chain   TEXT,               -- the References header, for threading
  received_at        INTEGER NOT NULL,   -- see "Time" below
  arrived_at         INTEGER,            -- when it reached an inbox; NULL in none
  inbox_folder_id    INTEGER,            -- that inbox
  inbox_unread       INTEGER,            -- unread there; NULL otherwise
  rfc822_size        INTEGER,
  preview            TEXT,               -- up to 320 characters
  body_text          TEXT,               -- the whole text, once read, for search
  read_whole_at      INTEGER,
  body_state         INTEGER NOT NULL,   -- HeadersOnly | PreviewCached | TextCached | FullCached
  flags              INTEGER NOT NULL DEFAULT 0   -- 0 unless a provider keeps flags per message
) STRICT;

-- derived
CREATE TABLE message_placements (
  folder_id      INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
  uid            INTEGER NOT NULL,
  message_id     INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  modseq         INTEGER,
  flags          INTEGER NOT NULL DEFAULT 0, -- this copy's own flags
  received_at    INTEGER NOT NULL,       -- this copy's own date; see "The list window" below
  PRIMARY KEY (folder_id, uid)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_placements_folder_date ON message_placements (folder_id, received_at DESC, uid DESC);

-- derived
CREATE TABLE folders (
  id                      INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id              INTEGER NOT NULL,
  name                    TEXT NOT NULL,
  role                    INTEGER NOT NULL, -- Inbox|Sent|Drafts|Trash|Junk|Archive|AllMail|Flagged|Custom
  uid_validity            INTEGER,
  uid_next                INTEGER,
  sync_cursor             TEXT,             -- opaque
  sync_window_floor_uid   INTEGER,          -- history below this point is not downloaded
  full_history_synced     INTEGER NOT NULL DEFAULT 0,
  view_epoch              INTEGER NOT NULL DEFAULT 0, -- bumped when the set changes under the list
  stopped_short           INTEGER NOT NULL DEFAULT 0,
  new_mail_refused_for_now      INTEGER NOT NULL DEFAULT 0,
  further_back_refused_for_now  INTEGER NOT NULL DEFAULT 0,
  UNIQUE (account_id, name)
) STRICT;

-- derived: one row per conversation, holding what the list orders by and shows
CREATE TABLE conversations (
  thread_id      INTEGER PRIMARY KEY,
  messages       INTEGER NOT NULL CHECK (messages >= 0),
  newest_at      INTEGER NOT NULL,    -- when its newest message arrived
  newest_message INTEGER NOT NULL     -- which message that is
) STRICT, WITHOUT ROWID;

-- derived: every name a conversation is known by; merging hands them all to the survivor
CREATE TABLE thread_names (
  account_id INTEGER NOT NULL,
  name       TEXT    NOT NULL,
  thread_id  INTEGER NOT NULL,
  PRIMARY KEY (account_id, name)
) STRICT, WITHOUT ROWID;
```

## Profile (profile.sqlite3, authoritative)

`mail_accounts`, `account_groups`, and `account_index_counter`; `preferences`, the person's settings, and `device_window_state`, `device_pane_width`, and `device_pane_folded`, this machine's; `account_activity`; `op_log`; `outbox` with `outbox_recipients` and `outbox_files`; `drafts` with `draft_files`; and `asked_counter`.

A migration is appended, never edited, with a test that starts at the version before it. Writes go through a `synchronous = FULL` connection.

```sql
-- authoritative
CREATE TABLE op_log (
  id            INTEGER PRIMARY KEY,
  account_id    TEXT NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
  message_id    INTEGER NOT NULL,  -- the MessageId; never a uid
  folder_id     INTEGER NOT NULL,
  kind          TEXT NOT NULL,     -- set_flags | move | delete
  flags_add     INTEGER NOT NULL DEFAULT 0,
  flags_remove  INTEGER NOT NULL DEFAULT 0,
  move_to       INTEGER,           -- a folder id
  delete_intent TEXT,              -- to_trash | permanently
  state         TEXT NOT NULL DEFAULT 'pending',  -- pending | needs_reresolve | conflicted | done
  attempts      INTEGER NOT NULL DEFAULT 0,
  last_error    TEXT,
  asked_at      INTEGER NOT NULL,
  try_after     INTEGER NOT NULL DEFAULT 0,
  sent_at       INTEGER,           -- committed before the server is told
  owes_a_read   INTEGER NOT NULL DEFAULT 0,
  done_at       INTEGER,
  CHECK (kind <> 'move' OR move_to IS NOT NULL),
  CHECK (kind <> 'delete' OR delete_intent IS NOT NULL),
  CHECK (kind <> 'set_flags' OR flags_add <> 0 OR flags_remove <> 0),
  CHECK (flags_add & flags_remove = 0)
) STRICT;

CREATE UNIQUE INDEX idx_op_log_outstanding ON op_log (account_id, message_id, folder_id, kind)
  WHERE state IN ('pending', 'needs_reresolve') AND sent_at IS NULL;
```

The four `CHECK`s refuse a move without a destination, a delete without its intent, a change of flags that changes nothing, and one that adds and removes the same flag; `idx_op_log_outstanding` refuses a second unsent instruction of one kind about one message in one folder.

`sent_at` is committed before the server is told, so a process killed between telling the server and recording its answer leaves a row that says it went out: a message no longer where it was is read as the instruction carried out, and it is not sent again as untried.

## The list window

Measured on 100,000 messages in one folder, release build, on a Mac with Apple Silicon.

Ordering the list by a date kept only in `messages` would cost a visit to every placement of the folder plus a sort: SQLite reports `SEARCH p USING PRIMARY KEY` + `USE TEMP B-TREE FOR ORDER BY`, and the first window of 50 rows takes **26.6 ms**—inside the 150 ms budget for switching folders, but growing linearly with the mailbox.

Keeping `received_at` on each placement and indexing it turns the same window into an index scan. The price is 3 MB per 100,000 placements and about 14% off sync write throughput. Each copy carries the date its own folder received it, and a folder holding a message twice shows the newer copy.

**The ordering has to match the index exactly.** Two messages can share a second, so the list breaks ties on `uid`, and the index carries the tie-break too: `(folder_id, received_at DESC, uid DESC)`. With the tie-break in the `ORDER BY` and not in the index, SQLite abandons the index and sorts the whole folder in a temporary b-tree—34 ms at the top of the list and 84 ms in the middle.

Measured by `list_windows_at_every_depth` on 100,000 messages in one folder, release build, in windows of a hundred rows—what the list asks for at a time:

| Window of 100 rows | Time |
| --- | --- |
| at the top | 0.32 ms |
| at position 25,000 | 14.55 ms |
| at position 50,000 | 31.68 ms |
| at position 99,900 | 5.13 ms |
| unread filter, at the top | 9.76 ms |

The top and the bottom are cheap and the middle is not, and the fixture is why. A position is found through `folder_days`, a count of messages per day that triggers keep level: the window seeks to that day and skips at most that day's messages. This fixture's 100,000 messages arrive within twenty-eight hours, so a window in its middle skips tens of thousands of them; mail spread over years skips one day's worth at any depth. Every depth is far inside the 150 ms budget for switching folders.

## Time

Time gets rules of its own, because "a message from 1970 at the top of the inbox" is a classic bug:

- A message's `Date` may be missing, from the previous century, in the future, or carry a broken timezone.
- `received_at` = the server's `INTERNALDATE` when available; `Date` is only a fallback. A provider that migrates a mailbox rewrites `INTERNALDATE`.
- Each row shows its date in the person's language and local time zone, as a medium date.
- The domain cannot read the clock—it has no I/O—so time reaches its rules as a value, and a test sets it.

## Search

`message_search` is a contentless FTS5 index over three things `messages` holds: `subject`; `people`, the sender's name and address and the To and Cc recipients; and `text`, the whole text of every message a synchronization has read whole—each pass reads more of them, up to 32 MiB a message—and until then the opening it read, `preview`. Contentless means it keeps tokens and nothing else, so nothing is stored in it a second time. Triggers on `messages` keep it level with every insert, every delete, and every change to those columns.

The tokenizer is `unicode61`, and it **knows letters, not languages**:

- the index folds yo (U+0451) to ie (U+0435) on the way in, and Yo to Ie, and folds what a person types the same way, so a word is found however it is spelled—`unicode61` alone keeps the two apart, with `remove_diacritics` and without it;
- one grammatical form of a word does not match another—there is no morphology;
- the `porter` tokenizer stems English alone, and is no help in any other language.

## Unread

Unread belongs to the copy: a folder's unread rows and its count read each placement's own flags, and the views that span folders read `inbox_unread` through the partial indexes `idx_messages_unread_arrived` and `idx_messages_account_unread_arrived`.

## Corruption and unsupported locations

SQLite in WAL mode can corrupt a database on a network or cloud-synced directory, and people move or link their data directories. A database that will not open meets a recovery screen that names the file and the reason, rather than a panic. A file a newer build wrote is refused before anything touches it and left as it is; any other can be set aside—moved, every byte kept—and the next start makes new ones.
