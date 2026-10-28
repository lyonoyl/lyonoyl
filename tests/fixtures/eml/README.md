# Message corpus

Messages kept as `.eml`, used to fill the development mail server and to test parsing.

Every rendering or parsing bug found on a real message is added here as a regression, in that message's shape: its parts, encodings, and markup as they were, its words replaced with filler, its names with the made-up ones ([the working contract](../../../AGENTS.md), invariant 22), its addresses with reserved domains, and every identifier a system minted for it with one of the same shape. A message that broke something once is worth more than a message written to look tidy.

## Written in the shape of real mail

| File | What it is |
| --- | --- |
| `long-thread-quoted-in-full.eml` | A reply from Apple Mail quoting thirty-six earlier messages, Apple's and Gmail's quoting in turn, both parts quoted-printable, its HTML nested 123 deep |
| `newsletter-pictures-behind-conditionals.eml` | A newsletter pointing at five pictures, some of them behind Outlook's conditional comments |
| `rating-stars-deep-in-tables.eml` | A notification pointing at ten pictures, its rating stars five tables down |
| `signature-picture-inside-the-message.eml` | A message from Outlook carrying its signature's picture inside itself, beside one it points at elsewhere |

## Hostile messages

Files named `hostile-*` are not messages anybody sent. They are the shapes a message takes when it is trying to do something: run code, report back that it was opened, or take the process down.

| File | What it is |
| --- | --- |
| `hostile-script-and-handlers.eml` | Every ordinary way to run code in one message: a script tag, an event handler, a `javascript:` link, a frame, an object, a form, a stylesheet, a `<base>`, and a conditional comment |
| `hostile-mxss.eml` | Markup that means something else once it is parsed a second time (mutation XSS) |
| `hostile-remote-content.eml` | A tracking pixel, a remote stylesheet, and a link that carries who opened it |
| `hostile-deep-nesting.eml` | Five thousand nested tags: nearly ten times deeper than an engine builds a document, in a tree whose building costs the square of its depth |
| `hostile-broken-boundary.eml` | A multipart message whose boundary never closes |
| `hostile-truncated-base64.eml` | A part whose encoding stops mid-character |
| `hostile-koi8r-body.eml` | A body in a charset that is not UTF-8 |
| `hostile-rtlo-filename.eml` | An attachment named so that it reads backwards on screen |

They are read by the golden test in `src-tauri/src/infrastructure/html`, which asserts that nothing any of them produces can run, fetch, or name a place to go. A message added here is added to that test by being in the directory—there is no list to update.
