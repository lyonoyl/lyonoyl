# How Lyonoyl Mail is put together

This page explains the shape of the code and the reasoning behind it. The rules a change has to follow are in [the working contract](../../AGENTS.md); this page is the tour beside them.

## Two halves and a bridge

An email client is two very different programs sharing a window.

One half draws the interface: lists, panes, keyboard handling, animation. React and TypeScript are good at that. The other half talks to mail servers, watches up to a hundred accounts for new mail at once, keeps a hundred thousand messages and more in each, parses hostile input, and must never lose a letter. Rust is good at that.

They meet at exactly one place—a typed bridge:

```text
React interface
      │  typed calls, no free-form strings
      ▼
Commands            transport only: unpack the request, call a use case, map the answer
      ▼
Use cases           the verbs of the product: AddAccount, SyncFolder, SendMessage
      ▼
Domain              the rules: what a message is, when it may be deleted, how flags work
      ▲
Ports               traits describing the outside world in our own words
      ▲
Adapters            SQLite · IMAP · SMTP · secret store · HTML · HTTP · updates · …
```

Arrows point inward. The domain knows nothing about SQLite, Tauri, IMAP, or JSON, and the interface knows nothing about any of them either—it only knows the bridge.

The interface never opens a socket and never touches a database. Everything it needs comes through one place, `src/shared/ipc/`, where every call into the core is wrapped and typed. The types themselves are generated from Rust and committed, so a rename on one side breaks the build on the other instead of breaking at runtime.

## Where the code lives

```text
crates/lyonoyl-domain/       the rules, as a separate crate
src-tauri/src/
├─ application/              use cases; depend on ports, never on adapters
├─ ports/                    traits: what the outside world must provide
├─ infrastructure/           adapters that implement those traits
│  └─ sqlite/, imap/, smtp/, secure_storage/, html/, http/, …
├─ read_model/               list, thread and search queries straight to SQLite
├─ commands/                 the bridge: DTO in, DTO out, no logic
└─ bootstrap/                the only place that picks real adapters and wires everything
src/
├─ app/                      the window's shell: sidebar, panes, keys, theme, recovery screen
├─ features/                 a screen and the hooks that drive it
├─ entities/                 the folder, mailbox, and message types the screens share
├─ shared/ipc/               the single door to the core, with generated types
├─ shared/i18n/              every string a person reads
└─ shared/ui/                interface primitives
```


## The domain is its own crate

Everything else lives in one crate. The domain has a crate of its own for a reason that is not tidiness: a crate can be *made* unable to reach the outside world. Its manifest allows one dependency for shipped code, so importing SQLite or Tauri there is not a code review discussion—it does not compile.

That matters because the domain holds the rules that silently ruin a mailbox when they are wrong: which identifier addresses a message, when a local message may be deleted, how flags combine. Those rules are tested without a database, a network, or a display, and they run in milliseconds.

## Ports and adapters, concretely

A port is a Rust trait owned by the inside, written in our own types:

```rust
pub trait PreferencesRepository: Send + Sync {
    fn load(&self) -> Result<Preferences, SettingsError>;
    fn save(&self, preferences: &Preferences) -> Result<(), SettingsError>;
}
```

`SqlitePreferencesRepository` implements it. No `rusqlite` type appears in the signature, so the use case above it cannot accidentally depend on the database, and a test hands it a fake written by hand.

Dependencies arrive through the constructor. There is no runtime container that resolves them: `bootstrap` picks the real implementations, and everything else receives what it needs: a command asks it for the adapters a use case takes and never names the type that does the work. Two cases open the databases after the start, and `bootstrap` builds what depends on them then: a device whose secrets are in a master-password file, once that file is unlocked, and a system store still asking the person when the start stopped waiting, once they answer. When you read a use case, its dependencies are the fields of its struct—the whole list.

Talking to a mail server is split into seven ports rather than one, because the conversations vary independently: listing mailboxes, syncing, fetching, watching, acting on mail, filing what was sent, and sending are different conversations with a server—`MailboxDiscovery`, `MailSync`, `MailFetch`, `MailWatch`, `MailActions`, `SentMail`, and `MailSubmission`.

The updater is one service over providers. The `Updates` port has one implementation, which composes parts that know nothing of each other: the places a release is to be had, each an `UpdateProvider` in the one list a copy asks, which holds the project's own site; the manifest, which names files and never addresses; the seal; putting a version in place; and what the updater keeps between runs. A place is added or taken away in `infrastructure/updates/providers/` and nowhere else, and the release check asks the same list through the same interface.

## Two kinds of database, on purpose

`accounts/<number>.sqlite3` holds the local copy of your mail, one file per account. Every byte of it exists on a server too, so any of them can be deleted and read again; dropping one drops that account's changes still waiting for the server, since a file made anew hands out its numbers again. One file per account also makes narrowing free: a question about one account opens one file, and forgetting an account is deleting a file rather than sweeping rows out of a shared one.

`profile.sqlite3` holds what exists nowhere else: accounts, settings, drafts, unsent mail, and the log of changes waiting to reach the server. It is written with `synchronous = FULL`, and it is never deleted automatically.

Keeping them apart means "clear the cache" can never mean "lose the letter you were writing."

Both live in a `databases` folder of their own inside the application's data folder, with the file of secrets on a Linux desktop that has no keyring. Every page of every database is encrypted under a key kept with the passwords, every message body is sealed again inside that, and the folder opens for this person alone. They move together or not at all: a profile and its write-ahead log in two places is a profile opened without the transactions the log still holds.

## Where each kind of data lives

| Data | Who owns the truth | Where it is |
| --- | --- | --- |
| Messages, folders, flags | your mail provider | read over IMAP, sent over SMTP |
| Local index and search | this device | SQLite, encrypted, and it never leaves |
| Accounts and settings | this device | `profile.sqlite3`, encrypted, and they never leave |
| Passwords | this device | system keychain—or, on a Linux desktop whose first start found none, a file only your master password opens |

## A message is hostile input

Mail arrives from strangers and is designed to be rendered. That makes rendering the highest-risk operation the app performs, so it is the one with the most rules: message bodies are sanitized in Rust, never handed to the interface as markup, served on a scheme of their own whose every answer carries its own policy, and displayed inside a sandboxed frame, under the application's own policy. No HTML parsing happens in the main window.

## How it is tested

Each kind of question gets the cheapest test that can answer it: domain rules as plain unit tests and property tests, use cases against hand-written fakes, SQL against a real in-memory database, parsing against a corpus of real messages, and interface behavior against a mocked bridge.

The full matrix is in [the working contract](../../AGENTS.md), section 5. The short version: if a bug was found on a real message, that message joins the corpus, so the same bug cannot come back quietly.
