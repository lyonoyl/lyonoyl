# Security and privacy review

Applies to changes in message rendering, sanitization, CSP, capabilities, URI scheme handlers, secret and keychain handling, attachment saving, IPC commands, and logging.

Read first: [the threat model](../security/threat-model.md).

The guiding question: **a message is hostile input—can it run code or send anything out?**

## Checklist

1. **Message rendering.** Has `srcdoc` appeared? Any `allow-*` in `sandbox`? DOMPurify or any other parsing of untrusted HTML in the main window? `initialization_script_for_all_frames`? `img-src https:`? Each of these is a blocking finding.
2. **Capability token.** Can the message frame request a resource belonging to a **different** message? Is the token invalidated when the open message changes?
3. **CSP.** Has `csp` become `null`? Has the policy been weakened just to make a message render? Is `style-src` specified alongside `style-src-elem` and `style-src-attr`? An engine without CSP3 reads `style-src` alone and one with it reads the other two, so a policy missing any of the three holds differently on one of the platforms.
4. **Capabilities.** Is the window granted any `opener:` permission, or anything beyond the three it holds?
5. **Secrets.** Can a secret reach SQLite, the journal, or error text? Is there a `#[derive(Debug)]` on a type holding an address, subject, or body?
6. **Paths.** Does a file path come from the frontend? Are attachment filenames sanitized against bidi controls? Does anything open a file a message carried rather than save it?
7. **IPC.** Every command parses untrusted JSON. Is there validation, are there size limits, are newtypes used instead of `String`?
8. **Privacy.** Has a network request appeared before explicit user consent? Does the User-Agent leak the OS version? Is a third-party service contacted while the user types an address? Does an update check run without consent? Is a journal event about one message written at `trouble` or `worth` rather than `detail`?
9. **Dependencies.** Has a TLS stack beside `rustls` appeared, such as `native-tls`? `openssl-sys` is here for SQLCipher. Does a new dependency carry a license section 6 of [the working contract](../../AGENTS.md) refuses?
10. **Panic strategy.** Has `panic = "abort"` appeared?
11. **Bodies.** Does a message body cross IPC as a string? Does anything from a message reach the window as text beyond a list row's preview of at most 320 characters, a clicked link's address, and the text an answer or a forward puts in the composer?
12. **The database key.** Does a database open without the key from the store, does the key leave `secure_storage`, or can SQLite's error quoting it reach error text?
13. **Defenses.** Does a change that adds, weakens, moves, or removes a defense change its row in [the defenses](../security/defenses.md) in the same commit?
14. **Anonymity.** Does anything added name a person, an account, a signing identity, a machine, or a home directory, or use a placeholder outside the cast?

## Reporting

Cite file and line for every finding, most severe first. For each, give a concrete failure scenario—inputs and the resulting behavior—and the minimal fix. If nothing is found, say so; do not pad the list.
