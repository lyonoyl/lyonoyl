# Response budgets

Each interaction has a budget. The core's answers are measured by the tools below against a 100,000-message seed database, the start by the journal's `start.storage` and `start.window` lines, and `BEFORE_SAYING_ANYTHING` in `src/features/activity/steady.ts` holds the spinner to 250 ms; no tool here measures the shell, a keypress, a body, or memory.

Measure on a Mac with Apple Silicon; apply a ×1.5 multiplier for a mid-range Windows laptop.

| Interaction | Budget |
| --- | --- |
| Window visible after launch | ≤300 ms |
| Interactive shell with a list rendered from cache | ≤500 ms p50 · ≤800 ms p95 |
| Keypress → visual feedback (selection moves) | ≤16 ms, hard ceiling 50 ms |
| Selecting a message → body rendered from local cache | ≤100 ms p95 |
| Marking a message read or unread → the counts in the sidebar agree with its dot | the same frame |
| Any other change a person makes → every count in the sidebar and the icon agree with it | one IPC round trip after the `op_log` write, and no server |
| Switching folders → rendered list | ≤150 ms |
| First local search result | ≤120 ms |
| A hundred accounts synchronizing at once | no interface stall, no unbounded thread or connection growth |
| Spinner must not appear before | 250 ms |
| Connections to the server, per pass over one account | one to list the mailboxes and one to read them, and a third only where there are messages to read whole—never one per mailbox, per batch, or per message |

## Rules

- The network **never** blocks the UI. Every user action is one `op_log` write, and that write **is** the local state: the list, a conversation, every count in the sidebar, and the number on the icon read the `op_log` laid over the index, through one fold. The network catches up in the background, by a push to that account's server a moment later.
- Optimistic UI: the user never waits for the server to see the result of archive/delete/flag/read. The row changes under the finger, and so do the counts around it for a message marked read or unread; for everything else they are read again the moment the write commits.
- Never roll back silently: a conflict appears on the row itself, with a reason and a Retry action.
- **Nothing gates on these numbers.** They are measured by hand against a checklist, on the machine making the change, with the tool described below.

## What a round costs in round trips

The cost of reading a mailbox is the connection, not the mailbox. No tool here repeats this measurement. Measured against a server on this machine, over plain loopback with no TLS at all: opening a connection and signing in took 7.30 ms and the work it carried took about 2 ms, so setup and teardown were 73% of a pass with nothing to do. The same fifty messages read whole cost 521 ms one connection each and 118 ms on one reused connection. Against a server across a network the gap is wider, because a TLS handshake is two more round trips before the first useful command.

So the rule is about conversations, not milliseconds: **a unit of work may not carry its own connection.** One message, one mailbox, and one folder listing are units of work; a pass over an account is not. What this refuses by name is one connect and one sign-in per message: fifty per mailbox per round, and a mailbox of ten thousand messages paying ten thousand sign-ins for mail the person already owns.

A test holds this rule by counting provider calls; no gate times it.

**Measured against real servers, nine accounts of six mailboxes each, over a network.** The application's own journal recorded it; `scripts/where-the-time-went.sh` prints it.

| | one connection per unit of work | one per pass |
| --- | --- | --- |
| A round over nine accounts | 12.5–13.5 s | 5.5–6.5 s |
| One pass over one account | 12.5 s | 5.7 s |
| Connections opened in it | 7 | 2, and a third where mail was waiting to be read whole |
| Spent opening them | 7.0 s—57% of the pass | 2.05 s—36% |

One connection to a server across a network costs a second—7.0 s for the seven in the table: a name to resolve, a handshake, a sign-in. That is the number every other figure here follows from.

What is left in a pass is 3.65 seconds of commands over six mailboxes—608 ms each, which is two or three round trips for an `EXAMINE` and a fetch. Below that there is no room without either fewer mailboxes read per round or more than one connection reading them at once, and both are decisions rather than tuning.

## Memory budget

The target is **≤400 MB RSS** with three accounts and 200,000 messages. It is the ceiling the body cache size, the materialization of selection id lists, sync batch sizes, and the number of concurrent webviews are decided against.

No tool here measures it. It is the one bound on memory the design states, and with a hundred accounts the per-account pools and page caches are what would reach it.

## Many accounts

A person may add a hundred mailboxes, and all of them synchronize and watch for new mail at the same time. The rules that keep that from turning into a stall:

- **One runtime, not one per account.** Network work shares a single asynchronous runtime; an adapter borrows a handle to it and never builds its own.
- **A cap on what runs at once.** Each account holds one watching connection, up to a hundred, and has one pass at a time; at most sixteen passes talk to servers at once, a round holds at most eight of them, and a pass is given at most ten minutes, so one slow server holds its own place and nobody else's.
- **Nothing waits on the interface thread.** Every network and database call happens off it.

Measured on the development server: a hundred accounts opening a session and listing their mailboxes finish in 0.35 s, with no failure and no wait. The test is `a_hundred_accounts_work_at_the_same_time`, started with `cargo test -p lyonoyl --lib a_hundred_accounts_work_at_the_same_time -- --ignored --nocapture`.

**What that measurement is, exactly.** A hundred threads of its own, each building a provider and listing mailboxes against loopback with no encryption. It touches none of the caps above—not the sixteen passes at once, not the share a round may hold, not the pools—so it says a hundred sessions can be opened and not that the caps hold under a hundred passes. The caps are held by the scheduler's own tests, which drive a round of many accounts and watch what it takes and leaves: `a_hundred_accounts_do_not_become_a_hundred_connections`, `a_round_leaves_places_for_a_person_who_is_waiting`, and `a_watcher_reading_takes_a_place_like_everything_else`.

What the reading side of a hundred accounts costs is the table further down, in [A window that spans accounts](#a-window-that-spans-accounts).

## Measuring this build at a size somebody chooses

The size it is built for is fifty to a hundred accounts of a hundred thousand messages or more—five to ten million messages in one application—and a number that large cannot be seeded in every test run. So the size is a knob:

```bash
cargo test --release -p lyonoyl --lib what_this_build_costs -- --ignored --nocapture
LYONOYL_SCALE_MESSAGES=400000 cargo test --release -p lyonoyl --lib what_this_build_costs -- --ignored --nocapture
```

`--release` is part of the measurement: without it SQLite itself is compiled unoptimized, and every figure comes out about three times what the application gives.

It prints what each interaction cost with the budget beside it, and **fails on none of them**. A machine measuring itself under a full test run is not the machine somebody reads their mail on, and a gate that goes red on a busy laptop teaches people to ignore it, so nothing here fails on a time. What it is for is deciding: a design chosen against these numbers is chosen, and one chosen without them is guessed.

**Measured**, release build, one folder, Apple Silicon. Every row goes through `ListReadModel::get_view_window`—the call the application makes, with its join, its folding of a message a folder holds twice, and the overlay of what a person has asked for since.

**Cold and warm, because they are two questions.** Cold is the first time this is asked; warm is the middle of five asks after it. One sample of each disagrees with itself across two runs minutes apart by up to thirty-nine times: it is a figure about whether a page happened to be in memory. Neither column is the very first search after a machine starts: the file has just been written, so the disk is not in any of these numbers, and nothing here measures it.

| 100,000 messages | Cold | Warm | Budget |
| --- | --- | --- | --- |
| List window at the top | 2.7 ms | **0.30 ms** | ≤150 ms |
| List window at the bottom | 5.4 ms | **4.5 ms** | ≤150 ms |
| Unread list window at the top | 10.1 ms | **5.5 ms** | ≤150 ms |
| Unread list window at the bottom | 46.0 ms | **45.9 ms** | ≤150 ms |
| Unread count for one folder | 9.5 ms | **5.4 ms** | counted in the window above |

### Searching what a message says

100,000 messages, each with the opening its synchronization read, indexed beside the subject and the addresses:

| | Cold | Warm | Budget |
| --- | --- | --- | --- |
| A search matching a phrase in the text—one message | 0.22 ms | **0.09 ms** | ≤120 ms |
| A search matching one sender—about 200 messages | 11.9 ms | **6.2 ms** | ≤120 ms |
| A search matching every message | 87.3 ms | **86.8 ms** | ≤120 ms |

Cost follows the match set and not the column: finding a phrase one message contains is the cheapest question in this table, because one message is what it has to produce. The text in the index costs the other two nothing measurable.

### A window that spans accounts

The view a person with more than one account opens on: every list that spans folders shows conversations rather than messages, and a folder and a search show messages. It cannot know which account holds the row at a given position, so it asks every one of them for everything above the window and merges—and what it asks for is the key it orders by, the row it shows, and the key it fetches by. The subject, the sender, and the preview are read afterwards, for the hundred rows that won, what the list asks for at a time.

| 100,000 messages | Cold | Warm | Budget |
| --- | --- | --- | --- |
| Everything on the device, at the top | 6.5 ms | **2.0 ms** | ≤150 ms |
| Everything on the device, at the bottom | 30.3 ms | **29.4 ms** | ≤150 ms |
| The sidebar, folders, and counts | 24.6 ms | **19.9 ms** | ≤150 ms |

**The sidebar with a person's changes laid over it.** Fifty messages marked read and twenty moves carried out and not read back, in the queue beside the same hundred thousand messages, in the same run as the tables above.

| 100,000 messages | Cold | Warm | Budget |
| --- | --- | --- | --- |
| The sidebar, nothing waiting | 24.6 ms | **19.9 ms** | ≤150 ms |
| The sidebar, seventy changes waiting | 20.3 ms | **20.0 ms** | one IPC round trip |
| The icon, seventy changes waiting | 5.4 ms | **5.9 ms** | one IPC round trip |

What a change costs before every count agrees is one read of the sidebar, and the fold over the queue adds nothing this tool can see to it.

**And the same questions over many accounts**, from `what_many_accounts_cost`. The mail is one file per account, so spanning a workspace is a loop over files: it runs once at one account and a hundred times at a hundred. The same hundred thousand messages each time, split more ways, so what changes between the rows is the number of files rather than the amount of mail.

| 100,000 messages altogether | Everything, at the top | Everything, deep | The sidebar |
| --- | --- | --- | --- |
| 1 account | 5.5 ms | 6.6 ms | 27.5 ms |
| 10 accounts | 2.1 ms | 10.5 ms | 19.0 ms |
| 100 accounts | 5.1 ms | **20.6 ms** | **17.8 ms** |

A hundred files cost three times one file at position 4,900 and nothing like a hundred times: what the merge compares is three integers a row, and the rows above the window are never built into a message. The sidebar, read over every file as the window reads it, goes the other way and gets cheaper the more accounts there are, because its cost follows the messages in a file rather than the number of files. Everything here is far inside the 150 ms budget.

**This is the reading half.** What a hundred accounts cost while they are being *read from their servers* is bounded by the limits rather than measured here: sixteen passes at once, of which a round may hold eight, two connections per pass and a third where mail waits to be read whole, and one watching connection per account. Those are covered by the scheduler's own tests.

Two things decide these figures, and both are about what a row above the window costs. The merge compares three integers per row rather than a message, so nothing above the window is ever built into a subject and a sender. And the keys come out of the index over `conversations` without touching `messages`: joining it to name an account, which only an account-scoped view needs, costs one lookup by row number for every conversation above the window—the bottom of the list takes 387 ms with the join and 145 ms without, so the two views that do not need it do not make it.

The figures are taken on the view the command builds. Taken on a list of messages instead, the bottom reads 57 ms—a number that describes a window nobody opens.

### What encrypting the whole database costs

Both columns are the same run of the same tool on the same machine within a minute of each other, 100,000 messages, one folder—the second with `PRAGMA key` removed and nothing else changed. So the difference is the encryption. It covers what a folder costs; the views above span accounts and are a different reader.

| | In the clear | Encrypted | |
| --- | --- | --- | --- |
| List window at the top | 1.73 ms | **1.89 ms** | ≤150 ms |
| List window at the bottom | 14.63 ms | **15.17 ms** | ≤150 ms |
| Unread list window at the top | 19.56 ms | **20.73 ms** | ≤150 ms |
| Unread list window at the bottom | 124.21 ms | **129.66 ms** | ≤150 ms |
| The sidebar, folders, and counts | 64.96 ms | **67.12 ms** | ≤150 ms |

Reading a page costs the same whether it holds mail or noise, so the cost tracks **how many pages a question touches** rather than how much it returns: the window at the top of a folder, which seeks to a day and reads a handful, pays 9%; the unread window at the bottom, which reads a great many, pays 4%.

**Every question is inside its budget at 100,000 messages**, cold and warm, and the closest is the worst case a search has: a word every message carries, at 87 ms against 120. At 400,000 the one a person can reach over its budget is that search, at 341 ms; everything on the device at its bottom, at 143 ms, and the sidebar, at 120 ms, are inside theirs, and the unread window at the bottom of a folder, at 211 ms, is answered by the read model and not offered by the window. Cost follows the match set, so that search is the one to watch as a mailbox grows—the two ordinary searches are at 12 ms and under a millisecond.

What is left in it is the ordering. An index of words answers in the order it stores them and the list shows mail by date, so a hundred thousand matches are found, then sorted, so that a hundred can be shown. Asking how long the list is costs nothing like that: the word index is contentless and its rows are the messages, so counting the matches in the index is the same number as counting them through the table and takes 41 ms rather than 261. That equality could drift silently, so it is asserted through a deletion and an edit rather than assumed.

**Which figures a reader can check for themselves.** The list, search, and sidebar tables above come from `what_this_build_costs`, `list_windows_at_every_depth` answers the same questions at every depth, and `what_many_accounts_cost` answers them over one account, ten, and a hundred: all three are `#[ignore]`d tests started by hand, all three print what they measured, and all three warm before they time. `what_a_deep_window_costs` answers `OFFSET` against a cursor. The one-per-pass column is the journal's: kept at `detail`, it writes every pass and round, and `scripts/where-the-time-went.sh` prints them. The loopback round trips, the one-per-unit column, and the comparisons taken with code changed for them—the clear column, the join, the subqueries, the table of names—were recorded once and have no instrument in the tree.

**Every figure in the tables above comes from the read model rather than from a query written beside it**, with one exception named where it is: the unread count for one folder is timed on a statement the tool holds its own copy of. A query with one column, one table, no join, and no folding answers three orders of magnitude faster than the one the product runs, and a tool measuring it measures nothing about the product. The tool calls the read model, and asserts that the deepest window holds the rows a walk reaches—so a fast wrong answer cannot pass for a fast right one.

**How a position is found.** A table of how many messages a folder holds on each day, kept by the database rather than by every writer—`folder_days`, with triggers on insert, delete, and a row changing folder, date, or which message it is a copy of. A position turns into a day by adding up counts, a few thousand small rows for ten years of a folder; the window then seeks to that day and skips at most the messages of that one day. The scrollbar stays proportional to the number of messages, and no jump walks to where it is going. The folder total is read from the same map, in 0.12 ms rather than the 50 a count takes—and it makes the map load-bearing for a number a person reads, so drift does not stay invisible.

**The map counts messages, not copies.** A folder can hold the same message twice; the list shows one row, dated by the newer copy. A map counting both would put every position ahead of the row it named, by however many second copies lie above it—an error growing with depth, largest at the bottom, and silent, because the map stays internally consistent. The window therefore keeps the copy that speaks for a message rather than grouping, which is also what lets the index serve it: grouping costs the folder a sort.

**The unread list keeps the same copy, and at 400,000 messages its top is 39 ms.** Its total reads no column from `messages` and so does not join to it, which would cost one lookup by rowid for every unread placement in the folder.

**Its bottom is 211 ms there, over the 150 ms budget.** The cost is linear in the number of **unread** messages, not in the size of the folder; the fixture leaves one in five unread.

**Both ends of both lists are timed, and the unread total on its own**: the top alone makes this table look like the fastest thing in the build, and which of a window's three reads costs the time is not a thing to work out from the shape of the SQL. At 400,000 messages the total is 37 ms of the 39 the top costs.

**How the views that span folders are served.** What the list orders by—when a message arrived in an inbox—and what it filters on are columns rather than correlated subqueries, and each holds nothing when the answer is no. So one partial index serves the filter and the order together, in one pass: `idx_messages_arrived` for the whole device and `idx_messages_unread_arrived` for the unread. Asked as subqueries instead, neither can be served by any index: SQLite reads every message, asks twice about each, and sorts the lot in a temporary b-tree.

The same holds for a list of conversations, which is what every list spanning folders shows: the order and both keys come out of `idx_conversations_newest` without touching `messages` at all.

What these cost is in **A window that spans accounts** above, measured on the view the command builds.

The columns cost nothing where a window is already cheap: a select list is evaluated only for the rows a window returns—a hundred of them, however deep the window sits—so the window at the top of a folder, everything on the device, and search pay nothing for them, and the unread window inside one folder reads the placements directly.

What the columns are really for is the reading that has to produce every row it compares rather than a hundred. Measured beside this, the same statement stepped over 50,199 rows costs 11.5 s with the subqueries and 143 ms without.

The sidebar does not join to `messages`: it reads no column from it, and a foreign key already forbids a placement without a message, so a join could change no row—it would only cost a lookup by rowid for every placement in every folder, on a panel rebuilt whenever a synchronization reports a change.

What the columns cost: synchronizing takes about a quarter longer, and the database is about a sixth larger—`arrived_at`, two partial indexes, and four triggers; `inbox_folder_id` and `inbox_unread`, with two more partial indexes and four more triggers, are not in that figure. Paid on every message stored; earned back every time the application is opened.

**What searching costs.** The figures are in the search table above. The cost is proportional to how much matches: the index answers with every message that matches and the newest hundred of them are taken by sorting the rest. So the ordinary case is far inside its budget, and **the worst case—a word every message carries—is what to watch as a mailbox grows**. The extreme measured here—one word matching all hundred thousand—is a property of a fixture that gives every message the same shape of subject, but a common short word on a real mailbox reaches a large fraction of it, and this is the shape of that cost rather than a number to dismiss.

**What putting messages into conversations costs**, measured on the same fixture: synchronizing takes about 30% longer, and the database is about 6% larger. Per message that is about sixty microseconds, nearly all of it the write that records which conversation the message is in; on a mailbox already synchronized it is paid once per new message.

Only names this device holds no message for are written down. Writing every message's own `Message-ID` into a second table—a copy of a column already there and already indexed—would cost **36 MB** instead of 11, and that alone would more than double the time of the unread window: a table added for one feature makes an unrelated query slower by making the file bigger.

**`OFFSET` cannot address the list.** It walks every row it skips, so the cost is linear in how far down somebody has scrolled—25.9 ms ninety thousand rows down, and the target is fifty times as many messages. Asked as a cursor on the sort key instead—*what comes after this message* rather than *the ninety-thousandth message*—the same window takes **0.07 ms against 25.9 ms**, nearly four hundred times less, because it walks none of them.

**The unread count for a folder reads one index.** Every copy of a message carries its own flags, so the count is answered from a covering index and the table is never opened—37 ms at four hundred thousand messages:

```
unread count   SEARCH p USING COVERING INDEX idx_placements_folder_flags (folder_id=?)
```

What it costs is on the write side: a provider that keeps flags per message writes to each copy rather than to one row, which is one to three rows, once.

## Reading across accounts

The sidebar can select one folder, one account, a group, or every account on the device. The wider three read `conversations` in the order `idx_conversations_newest` keeps, so none of them degrades into sorting a table; what they cost is in [A window that spans accounts](#a-window-that-spans-accounts).
