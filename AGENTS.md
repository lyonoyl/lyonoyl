# AGENTS.md—the working contract

Lyonoyl Mail: Rust + Tauri 2 + React 19 + TypeScript + SQLite + shadcn/ui. What the product is: [the README](README.md), and nothing else goes there—see the table below. How to run and build it: [the development guide](docs/development/guide.md). How it is put together: [the architecture overview](docs/architecture/overview.md).

**This file is the rules, and the rules are what is true now.** There is no record of who decided what, when, or against which alternative—a rule stands because it is written here, and it changes by being rewritten. Section 3 is not negotiable.

**The rules are the latest commit's.** Work starts from the latest commit of the default branch, and a branch is brought up to it again before its pull request: this file may have changed since the branch began, and a change is judged by the rules as they stand there.

## Policy on AI-assisted contributions

AI-generated or AI-assisted contributions are acceptable **only** if they follow the rules in this file and meet the same standards as human-written ones. The author of a pull request is responsible for its content however it was produced.

An agent must not weaken a rule in section 3 to make code compile, widen the sanitizer allowlist or the CSP to make a message render, or disable a failing gate. Where requirements are ambiguous or conflicting, do not guess: state the assumption, or ask.

This file is the single source of instructions, in the vendor-neutral [AGENTS.md](https://agents.md) format. `CLAUDE.md` at the root is a one-line pointer to it, for tools that read that name instead. No other vendor file is kept: reviews run off the shared [review checklists](docs/review-checklists/README.md), whichever tool runs them.

> This file and [the architecture overview](docs/architecture/overview.md) describe the architecture the code is held to. Where the code falls short of them, the code is what changes.

---

## 1. Repository map: what lives where, and what is forbidden there

The third column is the important one. Violating any row blocks the merge.

| Directory | What lives here | Forbidden here |
| --- | --- | --- |
| `crates/lyonoyl-domain/` | Pure types and rules: invariants, decisions, algorithms (`deletions_for`, `after_uid_validity_changed`, `effective_flags`). Its own crate, so the compiler enforces purity | Any dependency beyond `thiserror`. No I/O, no `async` |
| `src-tauri/src/ports/` | Traits through which domain/application describe the outside world | Adapter types in signatures (`rusqlite::Error`, `async_imap::*`, `reqwest::*`). The error type must be our own |
| `src-tauri/src/application/` | Verb-named use cases (`AddAccount`, `SyncFolder`, `SendMessage`) | `use crate::infrastructure`, `use rusqlite`, `use tauri`. Dependencies arrive through ports, injected via constructor |
| `src-tauri/src/infrastructure/` | Adapters: `sqlite`, `imap`, `smtp`, `secure_storage`, `html`, `http`, `updates`, and the rest | Leaking own types outward. `rusqlite::Row`, IMAP responses, HTTP responses are mapped **inside** the adapter |
| `src-tauri/src/commands/` | Thin Tauri transport: DTO ⇄ use case, error mapping | Any business logic, any SQL, any domain branching. A branch appearing in a command belongs to a use case. Building an adapter: a command hands a use case the ones `bootstrap` picks |
| `src-tauri/src/bootstrap/` | Composition root: picks production adapters, wires the graph | Logic. Construction only |
| `src-tauri/src/read_model/` | Direct SQL for lists/threads/search → narrow DTO | Writes. `SELECT` only |
| `src/app/` | Providers, router, shell, styles | Mail domain logic |
| `src/features/<feature>/` | A screen and its controller hooks | Importing from another `features/*`. Shared code moves to `shared/` or `entities/` |
| `src/shared/ipc/` | The **only** place where `invoke` and `listen` live | — |
| `src/shared/i18n/` | Interface text: one JSON catalog per language, the translator, the `ErrorCode` → key map | **Any user-facing string anywhere else in the repository**—in a component, in a `.ts` helper, in an attribute a reader speaks, or assembled from pieces. Prose arriving from Rust. A translation library |
| `src/shared/ui/primitives/` | Unmodified shadcn components | Hand edits. Only a new variant in the existing `cva`. Customization goes into wrappers one level up |
| `src/entities/` | Stable UI types for Account/Message/Folder | Importing from `features/` |
| `docker/mail-server/` | Dovecot for development and tests, loopback only | Anything a production deployment would need. It is a fixture, not a service |
| `tests/fixtures/eml/` | The message corpus: every parsing or rendering bug found on a real message, kept in that message's shape | Personal data, and anybody's own words. A real message keeps its parts, encodings, and markup; its words, names, addresses, and minted identifiers are replaced before it is added |
| `mocks/screenshots/` | The scenes `npm run app:screenshots` plays, a folder per language: `scene.json`, and what its messages show | A real message, person, or company. An address at a domain that answers, other than an account's own at `lyonoyl.com` |
| `README.md` | **A shop window, and only that.** What the product is, what it protects you from, how to install and build it, where its documents are, how to sponsor it or donate, how to contribute, and the license. Section 7 is the whole of how it is written, and it is not advice | **Anything a developer needs and a user does not.** Plans, tasks, what is coming, and any version. Internals, mechanism, and any paragraph saying *why we built it this way*. Warnings and caveats. Long prose where a sentence does. A promise about what does not run yet, or a number that was not measured |
| `docs/development/guide.md` | Running it, building it, the development mail server, anything else somebody working on the code needs | Product copy. It is not a second shop window |
| `.github/` | The files GitHub reads: funding, issue forms, the vulnerability report form, the pull request template | Anything that runs—a workflow, an action, a dependency bot. `.gitignore` ignores all but the community files |

**Dependencies always point inward:**

```
presentation → application → domain (separate crate)
infrastructure → ports ← application
commands → application
read_model → sqlite (bypasses the domain, deliberately)
bootstrap → everything (the single exception)
```

Enforced mechanically by `scripts/check-layering.sh`. The compiler does not catch this within a single crate.

---

## 2. Commands

```bash
docker compose -f docker/mail-server/compose.yml up -d --wait   # required before cargo test
scripts/reset-mail-server.sh      # empties every mailbox and fills it from the corpus again
npm install                       # frontend dependencies
npm start                         # asks what you want and runs it — the one command worth remembering
npm run app                       # work on the code: Tauri dev, hot reload
npm run app:build                 # an application to keep and use, unsigned
npm run app:release               # macOS: signed, notarized, stapled — the one another machine opens
npm run app:screenshots           # macOS: the application over a made-up scene, for screenshots
npm run typecheck                 # tsc --noEmit
npm run build                     # tsc && vite build — the frontend only, no app
npm test                          # vitest
npm run check:machine             # whether this machine has what a build and the preflight need

cargo fmt --all --check           # workspace-wide, from the repository root
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check                       # fast check (~1 s)
scripts/preflight.sh              # everything that must be true before a commit
```

**`scripts/preflight.sh` must be green before every commit**, and it is the whole answer to "what checks this": [the development guide](docs/development/guide.md#before-a-commit) lists its steps, from what this machine has to the application built the way `npm run app:build` builds it. A tool it needs and cannot find stops the run at its first step, with how to get it, rather than being skipped over. The run ends with every step passed or failed, and every failed test named.

**Nothing in this repository runs anything on a machine other than the one making the change.** No pipeline, no job, no schedule, no hosted runner: a person runs `scripts/preflight.sh`. `.github/` holds the community files GitHub reads—funding, issue forms, the vulnerability report form, the pull request template—and `.gitignore` ignores everything else there, so a tool cannot add a workflow without somebody meaning it.

**Development accounts hold what their server holds.** They are added on the account screen like any other—an address at one of the fixture's domains, `127.0.0.1` port `1143` for reading and port `1587` for sending, no encryption, the password `lyonoyl`; [the development guide](docs/development/guide.md#a-mail-server-to-develop-against) has the rest—and their mail arrives by an ordinary sync. Nothing writes messages into the index directly: that gives every folder two versions of the same message, with two dates and two sets of flags, and every question about what the list shows two answers. `scripts/reset-mail-server.sh` is what puts mail on the server, and it is also the way back to a known state.

**`cargo test` needs the development mail server running.** The tests that ask a real mail server what it did—including the two that prove this application removes the message it named and nothing else, and the four that send through it—each **skip themselves and pass** when nothing answers on `127.0.0.1:1143`, or for the four on `127.0.0.1:1587`. A suite that reports green about mail it never touched is worse than one that says nothing, so bring the server up first.

**A test reads only mail it put there itself.** Each one that needs mail signs in as an address of its own—the server takes any—and fills that mailbox first: the corpus over IMAP, or the messages it is about. The development accounts hold whatever somebody last clicked, and nothing on a server started fresh. A test may sign in as one of them only where it asserts nothing about what they hold.

`scripts/preflight.sh` starts the server right after asking what this machine has, before anything is built or tested, and stops the whole run when it cannot. It sets `LYONOYL_MAIL_SERVER_REQUIRED=1`, which turns a test that would skip itself into a failure, so a server that dies halfway cannot leave a green tick behind.

---

## 3. Invariants that must not be broken

Breaking one loses the user's mail, lets a message run code on the machine, or forces the schema to be reworked.

**Data and synchronization**

1. A `UID` is **never** a primary key. Only the stable `MessageId` reaches the UI, the router, caches, and `op_log`. `(folder_id, uid)` lives exactly until `UIDVALIDITY` changes.
2. Deleting local data during sync is allowed **only** on `VanishedReport::Exact`, or on `AuthoritativeSet` within its known UID window. `Unknown` **never** produces deletions.
3. `op_log` addresses a message only by `MessageId` and folder id—`message_id`, `folder_id`, `move_to`. Putting a `uid` there is forbidden.
4. The sync cursor is never written before the delta commits. Applying a delta is one transaction.
5. Two kinds of database: the mail, `accounts/<number>.sqlite3`, one file per account (derived—any of them may be dropped and re-synced), and `profile.sqlite3` (authoritative—`op_log`, outbox, drafts, settings; **the only copy**, never deleted automatically). Authoritative writes go through a connection with `synchronous = FULL`. One file per account is what makes narrowing free: a question about one account opens one file, and dropping an account's mail is deleting a file rather than a sweep of rows.
6. Any transaction that may write opens as `BEGIN IMMEDIATE`. No transaction survives an `.await` or crosses the IPC boundary.

**Security**

7. A message is hostile input. Message bodies **never** cross IPC as a string and are **never** rendered via `srcdoc`. The only path: sanitize in Rust → custom URI scheme with CSP in the response header → `<iframe sandbox="" referrerpolicy="no-referrer">`. Not a single `allow-*` in `sandbox`. Three things from a message do reach the window as text, and nothing else does: a list row's preview, at most 320 characters of plain text; the address of a link somebody clicked, shown for them to confirm; and the text of a message somebody answers or forwards, which goes into the composer's field to be edited and is never drawn as markup.
8. No DOMPurify, and no parsing of untrusted HTML in the main window. Sanitization happens in Rust only.
9. Secrets (passwords, and the key the databases are encrypted under) live only in the store this device keeps its secrets in: the OS keychain, or on a Linux desktop whose first start found none, the file its master password opens. Never in SQLite, logs, or error text. The build made for screenshots, the `screenshots` feature that no other build compiles in, holds nobody's secrets: the made-up password of each account in its scene and the key to databases it makes again at every start live in memory while it runs, and it never asks the system's store.

   **Every page of every database is encrypted** under a key from that same store, and message bodies keep their own sealing inside that. The key is read only inside `secure_storage`, where it leaves for its destination, and no error text may carry it—what SQLite says about a key it refuses is a message quoting that key.
10. `initialization_script_for_all_frames` is forbidden project-wide (grep gate in `scripts/check-layering.sh`).
11. `panic = "unwind"`—permanently. `abort` turns "this message failed to render" into "one message crashed the client."
12. File paths never come from the frontend. Instead of `save_attachment(path)` use `save_attachment(message_id, at)`, `at` being where the part sits in the message; the native dialog in Rust picks the path.

**Transport**

13. `csp` in `tauri.conf.json` is never `null` (`scripts/check-layering.sh`, which reads `devCsp` too).
14. Errors crossing IPC are a typed DTO with a code, never a `String`. The frontend must be able to tell "offline" from "wrong password" from "database corrupted."
15. TS DTO types are generated from Rust (`ts-rs`), committed, and verified by `scripts/preflight.sh`, which regenerates them and runs `git diff --exit-code` over `src/shared/ipc/generated`. Never hand-written.

**Interface text**

16. **Every word a person reads is a catalog key, and there are no exceptions.** Labels, buttons, headings, placeholders, titles, `alt`, every `aria-` attribute a reader speaks, empty states, status lines, tooltips, error text, notifications, menu items, window and dialog titles, the words inside a number—all of it lives in `src/shared/i18n/locales/*.json` and reaches the screen through `t()`. A string in a component, a string in a `.ts` module the component imports, and a string built by concatenation are the same violation: the rule is about what appears, not about which file it was typed in. Three things are outside it, and only three: the product name *Lyonoyl Mail*, which is not translated; text a mail server or a message itself supplies—a folder's own name, a subject line—which belongs to the mail and not to us; and the licenses of the code the product ships, shown word for word as their authors wrote them, with the names of the packages they cover, because a license translated is no longer the license. The notes an update carries are not an exception: the release writes them in every language a catalog exists for, and the window shows the one the person chose.
17. **The core never writes a sentence for a person.** Rust carries a code—`ErrorCode`, or a catalog key as a plain string where it must choose between two sentences—and the window turns it into words. `CommandError::message` is diagnostic English for a report and is never displayed; a component that reads `.message` off anything is showing one hard-coded language to a person who chose another.
18. **Every catalog in `locales/` ships complete, and every change reaches all of them at once.** A language is supported by being there, and the count is whatever the directory holds—the rule is not written for any number. `en` is the source and every other catalog is kept level with it **in the same commit**: a string added, reworded, or removed in English is added, reworded, or removed in every catalog before the change is finished, and never left for later. A key without a translation, a translation left blank, and a key nothing displays each fail `catalog.test.ts`—a translator kept in step with a sentence the product does not show is work spent on nothing. Dropping in a half-finished catalog is therefore not a way to start a language: it ships or it is not in the directory.

    **Nothing reaches a catalog unchecked.** Every string added or changed is read in every language by arbiters who read that language as natives and did not write it—one for what it means against the English and the place it is shown, one for how it reads, each reading for what section 8 says—and what they find is settled before the change lands. What a test can hold, `catalog.test.ts` holds: the placeholders of the source and no others; a plural form that stands for more than one number saying `{count}`, since French `one` is also 0 and Arabic `few` is every number from 3 to 10; the plural categories a language selects and no others; no period at the end in any script and no exclamation mark anywhere; a language not written in Latin letters written in its own, with nothing left in English but the names everybody writes in Latin; a line in Latin letters identical to English only where an arbiter wrote down that the word is the same, or where it is a name nobody translates; a count written in forms wherever English writes it so; every language naming itself as it does, in the direction its script runs; and the list of languages in the order of their own names. A new rule about interface text arrives with the test that holds it.

**What the interface does**

19. **No hand cursor, anywhere.** A control says it can be pressed by lighting up under the pointer, which tells everybody, the keyboard's user included; the hand tells only the mouse, and the window is an application, not a page. `cursor: pointer` and `cursor-pointer` are refused by `scripts/check-layering.sh` wherever they appear. `cursor: default` is the body's own, and a resize handle keeps `cursor-col-resize`—neither of those claims something is pressable.

    **The message frame is the one place where the hand belongs, and only on a link.** What is in there is a document somebody sent rather than this application's interface, a link in a document is a link, and the hand is what a person expects of one. The frame's own stylesheet says it last and says it `!important`, so a message can neither put the hand on its ordinary text nor take it off its links—measured on the engine with a message trying both.
20. **Leaving a screen is the screen's own business.** A screen that stands over the list and the message—writing, the settings, adding an account—is left by the cross the pane puts in its corner, and that cross calls what the screen itself does on the way out. Where leaving costs something, it costs the same by the cross as by any other route: the composer writes down what was typed in the last second and refuses while a message is going out, and a field that stores what it holds when it is left is left first—clicking a button does not take the focus on macOS, so the pane takes it away by hand. A second control that repeats the action without repeating the work is how the text goes missing.

**Anonymity**

21. **Nothing in this repository identifies anybody, and there are no exceptions.** No credential, no account, no signing identity, no certificate or fingerprint, no team or organization, no developer, no personal or company name, no email address, no handle, no machine or host name, no license or serial, no path holding a home directory—in code, in comments, in tests and fixtures, in documentation, in configuration, in a commit message, in a file name, or anywhere else at all. It does not matter that a thing is public elsewhere, or that it is only a name, or that it is merely who did the work: the repository is anonymous, and that is the whole of the rule.

    **Where a real one is needed, it lives outside the tree and is read from there**—an environment variable, a file a developer keeps on their own machine, the system's own store. A build that needs an identity gets it from the environment; a build from a plain checkout goes without and says so. **Signing is the standing example**: nothing here signs anything and nothing here names what would.

    Four things are outside this, and only four: the product name *Lyonoyl Mail*; obviously synthetic placeholders that stand for nobody; the scenes in `mocks/screenshots/`, whose accounts are at the product's own domain, `lyonoyl.com`, as invariant 22 says; and what supports the project: the README's Sponsors section, with the product's own mailbox for sponsors at `lyonoyl.com` and each sponsor's name, logo, and link, and the donation page in `docs/donations/`, with the wallets made for its donations alone. A placeholder must be recognizable as one; a real name is not made anonymous by being someone else's.

    **A commit is authored by the product.** Git will not take a commit without a name and an address, so the name is *Lyonoyl Mail* and the address one that reaches no mailbox. A pull request lands as one such commit, whatever its own commits say. `scripts/check-anonymity.py` holds a clone that commits under the product's name to the product's address, with no signature; any other name is a contributor's own, and is not asked about.

22. **A placeholder is built, never borrowed.** A name taken from a real language—English, German, Vietnamese, Spanish, or any other—is a real name whoever it is pinned on, so the cast in this repository is made up on a scheme instead: a Greek letter for the given name and an invented surname, with the address taken from the given name. Three of them, and they are the only ones:

    | Name | Address |
    | --- | --- |
    | Zeta Quorin | `zeta@example.com` |
    | Kappa Vandel | `kappa@example.net` |
    | Sigma Ordrin | `sigma@example.org` |

    Where a test needs a name outside ASCII, it is one of these written in another script—the scheme survives the encoding, and no nation's name is used to prove that a decoder works.

    **The scenes in `mocks/screenshots/` are the one place the cast is not the whole cast.** A screenshot is a picture of mail as a person has it: the people writing to a scene's accounts are made up for it, with names that read as names, at a reserved domain, and its accounts are at `lyonoyl.com`, the product's own. `scripts/check-layering.sh` lets an address at that domain stand in a scene's own `scene.json`, and the sponsors' mailbox in the README, and nowhere else, and not at a domain under it.

    **What is refused is a shape, not a list of names.** `scripts/check-layering.sh` refuses an address that could reach somebody—a recipient this repository did not invent, at a domain that answers—and a path holding a home directory, across every file git tracks and every one about to be tracked. A list would be a list of the names somebody already found, and it could never hold the ones that matter: writing a real name into a gate to refuse it is putting it in the repository, which is the thing being refused. Sweep by hand with `grep -rEn` or `rg` and never with `git grep -E`, whose engine does not honor `\b` and answers a search for `\bZeta\b` with nothing at all.

    The rest follows the same rule and is not a matter of taste: addresses are at `example.com`, `example.net`, `example.org`, `.test`, `.invalid`, `.example`, or `lyonoyl.local`, the development mail server's own, and only the cast stands at any other domain, where a test recognizing a provider needs it; **network addresses come from the ranges reserved for documentation** (RFC 5737—`192.0.2.0/24`, `198.51.100.0/24`, `203.0.113.0/24`) and autonomous-system numbers from RFC 5398 (`64496`–`64511`); an identifier a real system minted—a tenant, a portal, a campaign token, a message id—is replaced by one of the same shape that counts from zero. A real message kept as a fixture is scrubbed **through its encodings**: a `grep` that finds nothing proves nothing, because base64 and quoted-printable hide an address from it.

**What is written down**

23. **Everything here states what is true now, and nothing states how it got that way.** No calendar date anywhere—not in a rule, a measurement, a heading, a status, or a note. No record of who decided a thing, when, or what was decided before it. No "this used to be," no "superseded by," no "amended by," no justification for why a rule was chosen over an alternative. A rule stands because it is written; it changes by being rewritten, and what it replaced is gone rather than crossed out. Git holds the history.

    **A rule that is dangerous to undo carries its consequence, and that is not history.** `panic = "unwind"` is followed by what happens under `abort`—one crafted message kills the application together with unsaved drafts—because a reader who does not know that will "optimize" it. The consequence is part of the rule. The reason it was adopted is not.

    Three things are outside this, and all three are data rather than prose: a date inside a test fixture or an assertion, a date in a generated file, and the license texts, which are their authors' words. None of them is somebody writing down what happened. `scripts/check-layering.sh` refuses a calendar date anywhere else, the phrases quoted above, and a sentence saying who decided.

---

## 4. Recipe for adding a vertical slice

The order is mandatory. Skipping a step puts technical debt into the foundation.

1. **Use case first.** A verb: `AddAccount`, `SyncFolder`. File in `application/`.
2. **A port, if the outside world is involved.** Trait in `ports/`, signatures in domain types. Own error type.
3. **Test the use case against an in-memory fake port.** Before the adapter. Fakes are hand-written, not `mockall`.
4. **Adapter in `infrastructure/`.** All foreign types are mapped inside it.
5. **Schema migration** if needed: a new `M::up(…).down(…)` appended to its list in `src-tauri/src/infrastructure/sqlite/migrations/mod.rs`, never an edit to one already there, plus an upgrade test that starts at the version before it.
6. **Command in `commands/`**: unpack `State`, map DTO, map error. Nothing else. `async fn` + `spawn_blocking` for SQLite.
7. **Regenerate TS types**, commit them.
8. **Frontend**: call only through `src/shared/ipc/`, screen in `src/features/`. Every string a person reads goes into **every catalog in `src/shared/i18n/locales/`, in the same commit**, and is read through `t()`, never inlined. Invariants 16–18 and section 8 say what that covers and how it is read; `scripts/check-layering.sh` and `catalog.test.ts` refuse what skipped them.
9. **Check the [response budgets](docs/performance/response-budgets.md)** if the slice touches the list, reading, or search.
10. **Leave [the README](README.md) alone** unless a person would now describe the product differently. It is a shop window: reasoning, developer instructions, and what is coming never belong in it—that is the row in section 1.

    **A version is named only in the notes of a release that shipped**, saying what changed in it—never what a version is to hold, or when. The number lives in `[workspace.package] version` and is mirrored in `package.json` and `tauri.conf.json`; `src/app/release.test.ts` fails when they disagree, and when a document names the version being built or carries a table of steps toward one.

11. **A defense is listed.** A change that adds a defense—a check that refuses something hostile, a bound, a seal—adds its row to [the defenses](docs/security/defenses.md), beside the test or the file that holds it, and a change that weakens, moves, or removes one changes its row in the same commit. `src/app/defenses.test.ts` fails when a row names a test or a file the tree no longer has.

---

## 5. Tests: which one for which job

| What is being verified | With what | Where |
| --- | --- | --- |
| Domain invariant, algorithm | Unit test; `proptest` for sync convergence and thread merging | next to the code, `#[cfg(test)]` |
| Use case | Unit test with a hand-written in-memory fake port | next to the code |
| SQL, migrations | Integration test on `Connection::open_in_memory()` / tempfile | next to the code |
| Malformed message parsing, sanitization | Golden tests over an `.eml` corpus | `tests/fixtures/eml/` |
| IMAP behavior under failure | Transcript tests (`tokio_test::io::Builder`) | `src-tauri/src/transcript_tests.rs` |
| Component, interaction | Vitest + Testing Library against mocked IPC | next to the component |

There are no E2E tests.

Before proposing a change in a sensitive area, work through the matching [review checklist](docs/review-checklists/README.md)—the same ones human reviewers use.

**Corpus rule:** every rendering or parsing bug found on a real message is added to `tests/fixtures/eml/` as a regression, in that message's shape, as the row in section 1 says.

---

## 6. Style

- All project documentation, code comments, identifiers, and error messages are in **English**—the repository is public and English is its working language.
- **Markdown is written for the screen that wraps it, in American English.** A paragraph, a list item, and a quote are each one line, however long, and whatever shows the file wraps it. The dash in a sentence is an em dash with no space on either side (`word—word`), a range takes an en dash (`1–5`), and a hyphen joins only the parts of one word. Spelling and punctuation are American: `license`, `catalog`, `color`, `defense`; a comma before the last item of a series of three or more; and a period or a comma inside the closing quotation mark, unless what is quoted is code. `src/app/markdown.test.ts` refuses a paragraph broken over lines and a dash with spaces around it.
- **A commit's first line, and a pull request's title, follow [Conventional Commits](https://www.conventionalcommits.org/)**: `type(scope): summary`. The type is `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`, `chore`, `style`, or `revert`, with `!` after it for a breaking change; the scope is a part of the product where one fits; the summary is in the imperative, lowercase, without a period, within 72 characters, and says what changed in the code's own words. A pull request's title becomes the message of the commit that lands it.
- Rust: `thiserror` in domain and application.
- **No `unsafe` Rust anywhere but `src-tauri/src/infrastructure/app_icon/dock.rs`**, which holds the one call AppKit offers for changing the picture in the Dock. The workspace denies `unsafe_code`, that file allows it for that call, and `scripts/check-layering.sh` refuses `unsafe` in every other file.
- Never log message contents, addresses, or tokens. Redaction is enforced by types (`secrecy::SecretString`, and the journal's own value types), not by discipline.
- **The journal is written in the clear**, outside the encryption in invariant 9: encrypted, it would have nothing to say on the run where that key cannot be had. Its values are numbers and words this build chose, which the types in `src-tauri/src/ports/journal.rs` enforce, and its names are literals, so no word from a message reaches it. Everything about one message is written at `detail` and never at `trouble` or `worth`, the levels every build writes: a line for every message opened is a record of when somebody read their mail. `nothing_a_message_wrote_reaches_the_journal` holds both.
- Do not add a dependency without a stated reason: several hundred transitive crates already ship in an application that holds mail passwords.
- **A dependency must not restrict how the product is used, changed, or passed on, commercially included.** Permissive licenses (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode-3.0) and file-level copyleft that nothing here modifies (MPL-2.0) are fine. **GPL, LGPL, AGPL, SSPL, BUSL, and "non-commercial" licenses are not**—nor is any dependency, font, icon set, or service whose terms require approval, revenue sharing, or a separate agreement. Where an integration does carry such a condition—a provider's API terms, a store's rules, a signing program—it is **named in the pull request before the work starts**, never discovered afterwards.
- **No logic anywhere is particular to one provider.** What ships is generic IMAP and SMTP, which reaches Proton Bridge, and Gmail, iCloud, Yahoo, and Fastmail with an app password. The interface may offer presets that fill in server settings and nothing else, and `AccountProvider` is the one seam, with `ImapProviderAccount` its only implementation.
- **Every outside source is a provider behind one interface.** A service that reaches something outside this machine depends on a trait of its own, each place it reaches is an implementation of that trait in a file of its own, and the places are chosen where the service is built. Adding, replacing, or removing a place is that file and that line: a format, a check, or a script that has to change with it is the seam in the wrong place. The updater is built so—`UpdateProvider` in `src-tauri/src/infrastructure/updates/providers/`, with the site its one implementation.
- **A file holds at most 1171 lines outside its test modules.** The count is physical lines with every `#[cfg(test)] mod … { … }` block removed, comments included—the comments here are the design record, and a measure that discounted them would reward deleting them. A test module longer than a screen moves to a sibling file, where it stops counting either way. It does not apply to a file that is a test module in its own right, nor to a file whose length is a list a reader scans one entry at a time rather than holds at once; each of those is named in `scripts/check-layering.sh` with its reason, and an exemption that has fallen under the limit is itself a violation. The same limit holds in `src/`, where a test lives in a file of its own and the whole file is counted.
- **In `src/`, every component has a file of its own, named as it is.** One component, one responsibility, one file: a component is found by its name, and changing it changes its file and no other. A helper—a function that draws nothing—gets a file when two files have to agree on it: `rows.ts` and `places.ts` exist because the list, the virtualizer, and the keyboard must give the same answer, and two copies would be a list whose scrollbar and whose rows disagree.
- **A `useThing.ts` is earned by an invariant, and by nothing else.** State read and written by one handful of effects is one idea, however many lines it takes; splitting it produces a hook whose parameter list is its documentation and a change that touches three files. Where a piece of it is pure—arithmetic over values, no state, no ref, no effect—that piece comes out as plain functions in a module rather than as a hook.
- **macOS ships, and Windows and Linux are built from source.** A release is a signed, notarized macOS build and the source it was made from; on Windows and Linux, whoever runs Lyonoyl Mail builds it there. The code keeps all three: where they differ—data directories, path separators, keyboard modifiers, window behavior, file dialogs, the keychain—handle the difference at the point where it exists, with a `cfg` and a sentence saying what differs, and never assume the platform you happen to be typing on. Nothing here compiles or runs the other two, so a change to a branch behind a `cfg` for another system is unverified until somebody builds it there. What is known to differ, and what has been run where, is in [the platform differences](docs/platforms/differences.md); [the virtual machines guide](docs/platforms/virtual-machines.md) builds a machine for either.

---

## 7. The README, rule by rule

The README is held to these rules. They are checked by `scripts/check-layering.sh` where a script can check them, and the ones marked *by eye* are checked by whoever reviews the change.

**Size.** At most 200 lines and 1,600 words. No section and no subsection, at any level, runs past 25 lines from its heading to the next one. A paragraph is written on one line, however long, and a line break inside one is refused.

**A bullet is one sentence.** An optional bold lead of at most five words, then at most twenty-five more. No second sentence, no semicolon, no parentheses, and no dash clause carrying a second thought. A table cell is the same sentence with the same bound.

**The head is fixed.** Centered: the icon, the name, the slogan in bold with no period, and the badges—Rust, macOS, Windows, Linux, and the license. Under it, in this order: one sentence saying what Lyonoyl Mail is, the screenshots, dark above light, each at the full width, a line counting the themes, and a line of the languages—a flag for each catalog, in the order of the languages' English names. A test reads the number of themes off the code, and the checker reads the flags off the catalogs.

**The sections are these, in this order, and there are no others**: What it protects you from · Sponsors · Install and build · Documentation · Contributing · Security · Donations · License. Install and build has two subsections, Install and Build, and no other section has any. No section lists features. No hand-written table of contents—GitHub draws its own.

**Documentation is one table**, a row for each main document, in the alphabetical order of their names, each saying in one sentence what is in it.

**Sponsors** shows each sponsor's logo, linking to its product, the mailbox to write to for becoming one, and an orange Be the cat's patron button that writes to it. **Donations** is a line and an orange Feed the cat button, the color of the icon, both leading to the donation page, `docs/donations/README.md`, which holds a table for each way to give, a row for each network in the alphabetical order of their names: the network, its address in a code block, which GitHub gives a copy button, and its QR code beside it, which carries that address—*by eye*, by scanning it.

**Every number appears verbatim in a file under `docs/`, and the README says where**, except the number of themes, which a test reads off the code. A number that moves with an ordinary commit—lines of code, dependency counts, a size from a build nobody recorded—never appears at all. A speed or smoothness adjective may not stand without a measured figure beside it; `instant`, `snappy`, `blazing`, `seamless`, `effortless`, and `stutter` are refused outright.

**A claim is true as it stands, and carries no caveat.** The README sells the product: where something holds only in part, the sentence claims the part that holds, and no exception follows it in the sentence, a neighboring cell, or a note. A gap is not described either—the claim waits until the gap is closed. So no `yet`, `for now`, `currently`, `soon`, `planned`, `will `, and no sentence opening `Note`, `However`, or `Unfortunately`. The limits belong to the threat model, which the README links to.

**No heading and no bold lead is named after something the product does not do**, and the file mentions no absence: Install and build says where to download it and how to build it, in the present tense.

**No first person.** No `we`, `us`, `our`. There are two subjects—**you** and **Lyonoyl Mail**, or `it` after the first mention—and the sentence is active: *your password goes to the keychain*, never *the password is stored*. The file never praises itself: no `honest`, `proven`, `battle-tested`, `robust`, `powerful`, `simply`, `just`.

**No mechanism and no reason.** Connection counts, batch sizes, thread models, what a background round does, why a boundary sits where it does—none of it, in any section. *By eye*, with one question that settles it: could somebody act on this sentence before cloning the repository?

**One fenced code block, of at most five lines**, and it is clone-install-run. Commands, flags, and file names appear nowhere else.

**Every link inside the repository leads somewhere**: to a file the repository holds, and a link to a heading to a heading that exists.

**No placeholder and no HTML comment.** A section with nothing in it does not exist. A heading over a note-to-self renders on GitHub as an empty section, which is what an abandoned project looks like.

**A fact is stated once**, in one section. *By eye*, and it is the rule that most needs a reader.

---

## 8. Interface text, how it is written and read

Invariant 18 has two arbiters read every string in every language, one for what it means and one for how it reads. This section is what they read for. A test holds what a test can; everything here is read by people, and a string either of them finds wrong does not land.

**The English is settled before it is translated.** It is the source of every language, so a mistake in it is copied into all of them. Every claim the window makes about the product—what it protects, keeps, sends, or blocks—is traced to the code and to the threat model's limits first, and a claim true only under a condition says the condition.

**One reading.** A sentence a person could take two ways is wrong, whichever way was meant. Where two clauses seem to contradict each other, the order of events is made plain: not *No link opens by itself, and each shows where it really goes*, but *Before a link opens, you see the real address behind it*.

**Somebody acts.** The reader can tell who does what. A passive or an impersonal verb that leaves the question *who?* names the actor instead: Lyonoyl Mail, the person—*you*—or *we*, the people who make it, where they are the ones acting: the welcome, the promise about security flaws, what an anonymous report is for.

**The reader's words.** A string says what the person sees and does, not how it works inside: *pictures from the web load only when you ask*, not *remote resources are not fetched*. A term somebody who has never written code would stop at appears only where the text beside it explains it.

**Each string does its one job.** A welcome greets and thanks. A heading names what follows and is not a plea or a slogan. An error says what happened and what can be done now, in words that fit the situation: a choice that was never saved is made later, not changed later.

**Native, not translated.** A translation says what the English means the way the best software written in that language would say it: its own word order, idiom, and length, the register its catalog uses, and the words its catalog already has for mail, messages, and settings. A line correct word for word that no native writer would produce is wrong. It is written by a native reader from the meaning of the English, and the two arbiters are two others.

**Never clipped to fit.** A string is as short as it can be while saying everything it must, in whole sentences of its language. It is not shortened, abbreviated, or stripped of its subject to fit a box: the box is sized to the text. Every language is measured in the real window at the smallest size it allows, 960 by 640, before the change lands, because a test reads words and not widths.

**How the arbiters read.** The arbiter for meaning has the English, the place the string is shown, and what the string rests on in the code. The arbiter for how it reads takes the string alone, as a person meeting it for the first time, and asks whether a native would say it so. What either finds goes back to the writer, and the corrected string is read again.
