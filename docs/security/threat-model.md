# Threat model

Every defense named here, with the test or the gate that holds it, is in [the defenses](defenses.md).

## Trust boundaries

| | Boundary | What is untrusted |
| --- | --- | --- |
| TB1 | network ↔ Rust core | IMAP/SMTP/HTTP server responses |
| TB2 | message content ↔ renderer, and ↔ the filesystem | MIME, HTML, attachments—**hostile input**. A part a message carried is written to disk under a name the message chose, so the name is made rather than taken |
| TB3 | webview ↔ Rust over Tauri IPC | Every `#[tauri::command]` is a parser of untrusted JSON |
| TB4 | a file this person chose ↔ the message going out | A file read from disk, held in the core and never handed to the window, then written into a message |
| TB5 | the project's releases ↔ every installed copy | The manifest and the archive an update brings down: code that runs as the person |

## Ranked by cost of failure

### 1. Code execution from a message (catastrophic: RCE on the machine)

The only scenario whose outcome is full compromise. One XSS in the window that holds IPC reaches every command the core offers: the mail, sending as the person, and acting on their servers. It reaches no file, shell, or opener of its own—the window is granted none, no command takes a path, and a link opens only by the name the core held it under.

A sanitizer is **not** a security boundary.

Defense is four independent layers: the sanitizer in Rust, a scheme of its own whose every answer carries its own policy, the sandboxed frame, and the application's own policy. Forbidden permanently: `srcdoc`, any `allow-*` in sandbox, DOMPurify in the main window, `initialization_script_for_all_frames`, `img-src https:`.

**Every render answers to a token of its own—32 random bytes, valid only while its message is open.** Without it a sanitizer bypass would become a dump of the entire mailbox with no IPC call at all—the frame would ask the protocol handler for an arbitrary message id.

**An update is the other way code arrives** (TB5), and two independent locks hold it. The first is the seal: minisign, from a key kept outside the tree, checked over the manifest before it is read; the manifest names each archive's size and SHA-256, and the archive is held to both before it is unpacked, wherever the site redirected the copy for it. The second is the running copy's own designated requirement—its identifier, its team, Apple's certificate authority—which `codesign` checks the one bundle the archive unpacked to against, over every file its signature seals, before anything is replaced; a copy whose requirement names no team is not replaced at all. Somebody holding only the update key can get an update refused, never installed; a requirement compared as text would let them install one, because a signer writes that text. The version is inside the signed manifest, so an archive cannot be passed off as another version and nothing older than the running version is offered.

**One address holds every release, and it is not trusted.** A copy asks the project's site, `lyonoyl.com`, over HTTPS, which answers with each file or redirects the copy to wherever it is kept—at most ten addresses in a chain, each judged again, none off HTTPS or on this machine's network; the archive comes from the folder of its own version there, under the plain name the signed manifest gives it, and from nowhere else. Somebody who takes over the site, or a host it redirects to, can withhold a release, serve an older sealed manifest, and see who asks; they cannot install anything, and a file served that is not the one named is refused.

### 2. Losing the user's mail (catastrophic: mail that cannot be recovered)

| Scenario | Defense |
| --- | --- |
| False deletion on an incomplete server response | `VanishedReport::Unknown` never deletes, and a deletion past `max(50, 20%)` of a folder is put to the person first |
| A deferred operation lands on a different message after UIDVALIDITY changes | `op_log` addresses only `MessageId` |
| "Just recreate the index" wipes the outbox and drafts | Two kinds of database: a file of mail for each account, any of which may be dropped and read again, and the profile, which is never deleted automatically |
| An outbox commit lost on power failure | `synchronous = FULL` for authoritative writes; the composer closes only after a confirmed commit. Test: `a_message_is_committed_before_the_call_that_wrote_it_answers` |
| SMTP disconnect after DATA | `Message-ID` generated before sending; automatic retry forbidden, the operation waits for the user |
| Application version rollback | A database whose `user_version` is past every migration this build has is read for that version and nothing else: the file, and any log a newer build left beside it, stay byte for byte as they were. The window says a newer version of Lyonoyl Mail wrote it, names the file, and offers nothing that moves a newer build's profile. Nothing is migrated downward. The version is on the file's encrypted first page, so this holds where the key can be had: Start fresh, offered where the key may be gone for good, moves whatever is there |
| Developer debugging sync against a personal mailbox | A read-only guard on every connection opened to read, which no build and no setting turns off, and on every connection a refusal to make, rename, or delete a mailbox, to name a message by its position, or to expunge what was not named; a dedicated test mailbox |

### 3. Secret leakage

- **Message bodies kept on this device are sealed** with ChaCha20-Poly1305 under a key of their own, and **every database beside them is encrypted page by page** by SQLCipher under another, both kept in the store this device keeps its secrets in: subject, sender, recipients, dates, the search index over them, the drafts, and the outbox are inside that too. FileVault, BitLocker, or LUKS is the layer under this.
- Passwords live only in the store this device keeps its secrets in: the OS keychain, or on a Linux desktop whose first start found none, the file its master password opens. Never in SQLite, logs, or error text.
- Linux without Secret Service: an **explicit** master password plus Argon2id. No silent downgrade to a file.
- Log redaction enforced by types (`secrecy::SecretString`, the journal's own value types), not by discipline.
- **Account removal and uninstall:** a password left in the store is working access to the user's mail, sitting on the system indefinitely. Removing an account clears its secrets from the store this device keeps them in and deletes the account's mail file. Uninstalling clears neither: on macOS and Windows an uninstaller does not touch the keychain.

### 4. Privacy leakage

- **The frame has no network under any setting**, and that is the containment layer: `default-src 'none'`, with `img-src` naming only the scheme the message is served on—`lyomsg:` and `http://lyomsg.localhost`, never `'self'`—means CSS exfiltration and internal-network probing do not work whatever anybody chooses. A picture a message points at is fetched by this process—user agent stripped to a name, no referrer, no cookies, no address outside the public internet, every redirect hop judged again and none leaving encryption—and served back to the frame from the store, so the message never speaks to a host itself.
- **What is left is who learns that mail was opened**, which is a tracking pixel and nothing else. That is the one thing the reader decides: nothing is fetched until asked, message by message, until somebody goes to the settings and says otherwise. A stored answer this build cannot read is read as "ask me," so the careful side is where every way of being wrong lands.
- Autodiscovery runs only when Continue is pressed, never while an address is typed. An account already on this device at the same domain answers first, then a table of 815 domains the application carries—neither tells anybody anything—and only then the domain itself, at two addresses of its own over HTTPS, which learns that somebody is setting up mail there and never the address.
- Anonymized User-Agent: every request the core makes carries the user agent `LyonoylMail` alone—no operating system, no architecture, and no version.
- Anonymized SMTP greeting: the client names itself `[127.0.0.1]` and never the machine, whose name the submission server would write into the `Received` header every recipient reads.
- No Gravatar requests (which leak an email hash), no sender favicons, no prefetch, no read receipts.
- Zero telemetry. An update check is itself a signal—the address it comes from, once a day; it names the product and the system it runs on, and not its version—so on its first start a copy asks whether to look, with the box clear, and looks only after a yes. What reaches the project is in [the privacy page](../../PRIVACY.md). A build made without the update key never looks, whatever the answer.

### 5. Attachments

**This application saves a file and never opens one.** There is no double click to gate: the one thing a listed file offers is Save.

- Paths **never** come from the frontend: `save_attachment(message_id, at)`, with the dialog in Rust. This eliminates the entire path-traversal class architecturally rather than by validation. The second argument is where the part sits in the message, not a name and not a path.
- Filenames: strip the whole bidi control range (`invoice` + U+202E + `gnp.exe` is drawn as `invoiceexe.png`)—every character carrying the standard's `Bidi_Control` property, U+061C included—and reject Windows reserved names. One rule, in `lyonoyl_domain::attachment`, asked by both the file a message carries and the message saved as a file.
- No automatic archive extraction. Held by absence: nothing in the tree opens an archive a message carries, and no manifest carries a crate that reads ZIP, RAR, 7z, or tar. The one archive this application opens is an update, once its seal and its digest hold, under a ceiling.
- Outgoing: what may be attached is bounded before anything is written down—one file against a ceiling as it is chosen, and the whole message against another before it reaches the outbox.
- A file saved out of a message carries the mark of a download on macOS, `com.apple.quarantine`, written by the dialog adapter as it writes the file—the one place that knows where it went—so what the system and other applications do with a download, they do with it. Windows and Linux, built from source, mark nothing.

The window and the save port never learn where the file went: the port answers with a name and nothing else, because a path traveling back to the window is the thing invariant 12 exists to stop. The dialog adapter alone knows it, which is enough to mark the file.

### 6. Denial of service through a message

A message past 32 MiB is refused, and one past 8,000 elements is shown as text; nesting is measured before parsing; at most four are parsed at once, each on its own thread with a 16 MiB stack, under a five-second timeout and `catch_unwind`, with `panic = "unwind"`. An answer from a mail server holds at most 256 MiB.

A message that will not render can still be read as text or as its source, and saved as a file.

## What is not covered

- A picture, or the server settings looked up for an address being added, can be fetched from a private address behind a NAT64 translator that answers under an IPv6 prefix the network chose: there the address cannot be told from a public one.
- A message the server ends the connection over instead of refusing it—Dovecot does so by default with one it cannot read—stops its folder at that message on every check: met among new mail, nothing new arrives in the folder until the server can read it or it is gone; met further back, nothing older than it is read.
- A message the server will not describe is listed by its date and size alone until it is read whole, which never comes for one over 32 MiB. One it cannot give whole is asked for again on every round, more than ten of those in a row leave the older mail behind them to be fetched only when somebody opens it, and one whose size the server understates and that is larger than its request may hold—256 MiB asked alone—is fetched whole and refused on every round.
- A folder whose answer between two checks passes 256 MiB—some two million messages arriving or changing—or whose new mail's headers pass 256 MiB on a server that does not say what number its next message will get, fails every check from then on: nothing more arrives in it.
- While an account's drafts are at their bound or the profile cannot be written, what is on screen when the application ends is not kept, and nothing says so.
- On an account whose server goes on refusing its folder for sent mail, or breaks off or loses step with the conversation about a copy, every message the account sends stays on this device—everybody it went to, blind copies included, and every file it carried—under the profile's encryption, until the server relents or the account is removed. Nothing on screen says so.
- **A freeze.** The seal does not stop somebody able to make the manifest's address answer with an older sealed manifest—every one ever published can have been kept by whoever fetched it—from keeping copies from hearing about a newer version, or steering a copy that has not looked since to the one in between. Doing it takes the site, a host it redirects to, or the connection to them: the seal's timestamp is checked and not kept, so a copy does not refuse a manifest older than the last one it took.
- Somebody who can read the files without the key learns how many accounts there are and how large each file is, and the log beside them, which is written in the clear, says when the application ran and nothing about one message unless somebody turned it up to `detail`.
- Windows and Linux are built from source and not shipped, and what holds differently there is in [the platform differences](../platforms/differences.md): on Windows nothing refuses a link that got past the sanitizer, and a certificate check can send a request to an address the address policy never sees; logging out of a Linux desktop keeps only what was last saved of a message being written.
- Everything the application holds is visible to any process running as the same user: the key is on that machine and unlocked for that person.
- There is no defense against a compromised OS.

## Rules for contributors

- Anything a caller supplies that becomes a header is parsed into a type that refuses a second header: `Subject` and `Rfc822MessageId` reject control characters, and `InThread` holds them rather than strings. This is not only about a compromised window—a `Message-ID` is read off an arriving message and written back out when somebody answers it, so a hostile one would reach the answer of anybody who replied to it.
- Widening the sanitizer allowlist or weakening CSP requires an entry in this file and a justification. The permanent prohibitions are enforced by `scripts/check-layering.sh`, which `scripts/preflight.sh` runs on the machine making the change, and by `scripts/check-layering.test.sh`, which feeds each prohibition a violation and fails if the gate lets it through. The gate reads the whole tree—the Rust, the TypeScript, the JSX, and both policies in `tauri.conf.json`—and refuses the **capability** rather than the spelling, including through APIs that ship with Tauri under other names.
- Every rendering or parsing defect found on a real message is added to the `tests/fixtures/eml/` corpus.
- A vulnerability is never written into a file here before its fix ships: it stays a private security advisory, reported as [the security policy](../../SECURITY.md) says. A limit the design leaves open is written under What is not covered as soon as it is known.
