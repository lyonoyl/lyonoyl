//! Turning a header block and the opening of a body into what the index stores.
//!
//! The one part of this adapter that never touches a session, a UID or an async_imap type: bytes
//! written by a stranger in, one `MessageEnvelope` out. Somebody asking why a subject shows as
//! mojibake, or why there is markup under one, is looking for this file.

use super::*;

/// How much of the body one pass takes, for the opening line the list shows.
///
/// A prefix, not the message: enough for three lines under a subject, and bounded so a mailbox of
/// large messages does not become a large download. Whatever is cut mid-way is cut — the parser is
/// written for messages that arrive broken.
pub(super) const PREVIEW_BYTES: usize = 2048;
/// How much text the list is given. Longer than three lines at any width, and short enough that a
/// hundred thousand of them are still a small database.
pub(super) const PREVIEW_CHARS: usize = 320;

/// Turns a header block into what the index stores about a message.
///
/// Everything here is untrusted input written by whoever sent the message. Nothing is parsed by
/// hand: a malformed header yields a missing field, never a failure and never a panic.
pub(super) fn envelope_of(
    header: &[u8],
    body: &[u8],
    internal_date: Option<i64>,
    size: Option<u32>,
    flags: Flags,
) -> MessageEnvelope {
    let parsed = MessageParser::default().parse_headers(header);

    let subject = parsed
        .as_ref()
        .and_then(|message| message.subject())
        .map(str::to_owned)
        .filter(|subject| !subject.trim().is_empty());

    let sender = parsed.as_ref().and_then(|message| sender_of(message));
    let sender_address = parsed
        .as_ref()
        .and_then(|message| sender_address_of(message));
    let to = parsed
        .as_ref()
        .map(|message| addresses_of(message.to()))
        .unwrap_or_default();
    let cc = parsed
        .as_ref()
        .map(|message| addresses_of(message.cc()))
        .unwrap_or_default();

    let rfc822_message_id = parsed
        .as_ref()
        .and_then(|message| message.message_id())
        .and_then(|value| Rfc822MessageId::parse(value).ok());

    let references = parsed
        .as_ref()
        .map(|message| references_of(message))
        .unwrap_or_default();

    // The server's own arrival time is preferred: a `Date` header is written by the sender and is
    // wrong often enough that sorting by it puts mail from the future at the top of the list.
    let received_at = internal_date
        .or_else(|| {
            parsed
                .as_ref()
                .and_then(|message| message.date())
                .map(|date| date.to_timestamp())
        })
        .unwrap_or(0);

    MessageEnvelope {
        preview: preview_of(header, body),
        rfc822_message_id,
        subject,
        sender,
        sender_address,
        to,
        cc,
        received_at,
        rfc822_size: size,
        references,
        flags,
    }
}

/// What a message read whole says about itself, described the way a pass describes one: from its
/// header block and the opening of what follows it.
///
/// For a message the pass could not describe — a server that would not hand it over then, or a
/// parser that broke on it — once its bytes are here. The bound a pass rests on holds here too:
/// however long the message is, the parser is given its header and two kilobytes of what follows.
pub(crate) fn described_whole(raw: &[u8]) -> MessageEnvelope {
    // The end of the header block, the blank line included, by either line ending a message may
    // arrive in, and the earlier of the two where both appear.
    let crlf = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|at| at + 4);
    let lf = raw
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|at| at + 2);
    let ends = crlf.into_iter().chain(lf).min().unwrap_or(raw.len());
    let (header, body) = raw.split_at(ends);

    envelope_of(header, body, None, None, Flags::NONE)
}

/// What the index stores about a message whose bytes could not be read: only what the server said
/// about it, with nothing parsed out of what the sender wrote.
///
/// A row with no subject and no sender is still the message. It is listed, counted and opened like
/// any other, and the round that reads messages whole and the opening of one each catch a panic in
/// the parser for themselves — and fill in, from the message they read, what this row is missing.
pub(super) fn nothing_read(
    internal_date: Option<i64>,
    size: Option<u32>,
    flags: Flags,
) -> MessageEnvelope {
    MessageEnvelope {
        preview: None,
        rfc822_message_id: None,
        subject: None,
        sender: None,
        sender_address: None,
        to: Vec::new(),
        cc: Vec::new(),
        received_at: internal_date.unwrap_or(0),
        rfc822_size: size,
        references: Vec::new(),
        flags,
    }
}

/// The opening of a message, as plain text.
///
/// The header block and the piece of body are parsed together, because what the bytes mean —
/// which charset, which transfer encoding, which part of a multipart message — is stated in the
/// headers and nowhere else.
///
/// **This parses HTML, and said for a long time that it did not.** `body_text` hands an HTML part
/// to the parser's own `html_to_text`, so a message written solely in HTML does get a preview — made
/// of its text, not of its markup. The comment that used to stand here said only a text part was
/// used and that HTML was turned into text "in one place, under a sandbox, not here", and a reader
/// would have taken the sync path for one that never touches markup.
///
/// What makes it acceptable is the bound, and the bound is why it is written down: the server is
/// asked for `BODY.PEEK[TEXT]<0.2048>`, so what reaches the parser is two kilobytes and cannot be
/// more. That is the wrong shape for the cost this project guards against — the sanitizer's cost
/// grows faster than its input, and two kilobytes is nowhere near where that begins.
///
/// What it does not have is most of what the reading path has: no thread of its own, no larger
/// stack, no timeout. What it does have is a `catch_unwind` around each message the pass describes,
/// so a panic here leaves that one message described by what the server said about it and the pass
/// goes on — which `panic = "unwind"` is what makes possible. Anything that widens what is fed to
/// this belongs behind the reading path's guards instead.
fn preview_of(header: &[u8], body: &[u8]) -> Option<String> {
    if body.is_empty() {
        return None;
    }

    let mut message = Vec::with_capacity(header.len() + body.len() + 2);
    message.extend_from_slice(header);
    if !header.ends_with(b"\r\n\r\n") {
        message.extend_from_slice(b"\r\n");
    }
    message.extend_from_slice(&body[..body.len().min(PREVIEW_BYTES)]);

    let parsed = MessageParser::default().parse(&message)?;
    let text = match parsed.body_text(0) {
        Some(text) => text.into_owned(),
        // No text part at all: the prefix ends inside the first one, so there is nothing the
        // parser could classify and it offers nothing. The bytes are not used as they stand --
        // what surrounds them is a boundary and a part's own headers, and those under a subject
        // are worse than a blank row. Only a run that could be a message is taken out of them.
        None => a_message_inside_an_unfinished_part(&message)?,
    };

    // A part whose encoding the parser did not get to apply arrives still encoded, and `=E2=80=99`
    // under a subject is not something anyone wrote.
    let decoded = decoded_if_still_encoded(&text);

    // And what was under the encoding may itself be markup, which the parser would have turned into
    // text had it been able to finish the part. Without this the salvage above buys nothing for a
    // message written in HTML and sent as base64: the words are recovered and then thrown away by
    // the refusal at the end, and the row is blank instead of wrong. Bounded by the same prefix --
    // base64 of two kilobytes is fifteen hundred bytes of markup.
    let decoded = match looks_like_markup(&decoded) {
        true => the_text_of_markup(&decoded).unwrap_or(decoded),
        false => decoded,
    };

    // What a person wrote, not what they wrote it under. A reply quotes the message it answers,
    // and a preview of that is a preview of something already read.
    let written = without_quoted_reply(&decoded);

    // One line, however the message was wrapped: the list gives it three lines of its own and
    // decides where they break.
    let collapsed = written.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed: String = collapsed.chars().take(PREVIEW_CHARS).collect();

    // A message arrives as a prefix, so its last part is nearly always unfinished. A parser that
    // cannot tell what an unfinished part is treats it as plain text, and a part of HTML read as
    // plain text is a line of tags under the subject. Nothing is a better preview than that.
    if trimmed.is_empty() || looks_like_markup(&trimmed) {
        return None;
    }

    Some(trimmed)
}

/// Decodes quoted-printable that arrived as text.
///
/// A message comes in as a prefix of itself, so its last part is unfinished; a parser that cannot
/// classify an unfinished part cannot apply its encoding either, and hands back the raw bytes. What
/// that looks like in a list is `I=E2=80=99ll` and a stray `=` where a line was folded.
///
/// The decoder is the parser's own, and it is run over every preview: text nobody encoded comes
/// back unchanged, which is what the tests below check. A result that is not valid text is thrown
/// away and the original kept — a preview is worth less than the certainty that it says something.
fn decoded_if_still_encoded(text: &str) -> String {
    if let Some(written) = base64_that_was_never_decoded(text) {
        return written;
    }

    quoted_printable_decode(text.as_bytes())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| text.to_owned())
}

/// The words behind a part that arrived as base64 and was handed back as characters.
///
/// The same fault as the one above and the more visible half of it: a body reaches here as a prefix
/// of itself, the part it ends inside never closes, and a parser that cannot say what that part is
/// cannot apply its encoding. Quoted-printable leaves `=E2=80=99` under a subject, which is ugly;
/// base64 leaves the whole message as one unbroken word, which is every row of the list unreadable.
///
/// **Recognised by shape, and the shape has to be narrow.** Anything at all decodes as base64 if
/// nothing refuses it, so this asks for what prose never looks like: no spaces at all once the line
/// breaks are gone, long enough not to be a word, only the alphabet, and — the part that does the
/// work — a result that is text a person could have written. A token, a signature or a key comes
/// back as bytes that are not text and is left exactly as it was.
fn base64_that_was_never_decoded(text: &str) -> Option<String> {
    /// Shorter than this is a word, an identifier or a fragment, not a message.
    const ENOUGH_TO_BE_A_MESSAGE: usize = 64;

    let packed: String = text.split_whitespace().collect();

    if packed.len() < ENOUGH_TO_BE_A_MESSAGE
        || !packed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return None;
    }

    // The cut lands mid-group, so the last few characters are not a whole one. They are dropped
    // rather than guessed at: three characters of somebody's sentence are not worth a decoder that
    // invents a fourth.
    let whole = &packed[..packed.len() - packed.len() % 4];
    let written = String::from_utf8(base64_decode(whole.as_bytes())?).ok()?;

    // What came out has to read as writing. Most of what is not text decodes to control characters
    // and replacement marks, and a preview of those is worse than no preview at all.
    let readable = written
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .count();

    (readable * 10 >= written.chars().count() * 9 && !written.trim().is_empty()).then_some(written)
}

/// A message inside a part the parser could not finish, if there is one to be had.
///
/// The last resort, and narrow on purpose. A prefix that ends inside the first part leaves the
/// parser with nothing it can call text, and the raw bytes are not a preview: they open with a
/// boundary and a part's own headers. What is taken instead is the longest stretch of lines that
/// are nothing but the base64 alphabet — which a header is not, and which prose is not — and it is
/// handed to the same decoder as everything else, so it still has to come back reading as writing.
fn a_message_inside_an_unfinished_part(message: &[u8]) -> Option<String> {
    /// Fewer lines than this is a header folded oddly or an identifier, not a message.
    const ENOUGH_LINES: usize = 2;

    let raw = String::from_utf8_lossy(message);
    let mut longest: Vec<&str> = Vec::new();
    let mut run: Vec<&str> = Vec::new();

    for line in raw.lines() {
        let line = line.trim();
        let could_be = line.len() >= 32
            && line
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='));

        match could_be {
            true => run.push(line),
            false => {
                if run.len() > longest.len() {
                    longest = std::mem::take(&mut run);
                }
                run.clear();
            }
        }
    }
    if run.len() > longest.len() {
        longest = run;
    }

    (longest.len() >= ENOUGH_LINES).then(|| longest.join(""))
}

/// The words inside markup, by asking the parser what it would have said.
///
/// A part the parser could not finish is handed back as characters; once its encoding has been
/// undone here, this hands the result back to the parser as the thing it turned out to be. The same
/// `html_to_text` runs on it either way, on the same two-kilobyte bound, so nothing is fed to it
/// that would not have been fed to it had the part arrived whole.
fn the_text_of_markup(markup: &str) -> Option<String> {
    let mut message = Vec::with_capacity(markup.len() + 64);
    message.extend_from_slice(b"Content-Type: text/html; charset=utf-8\r\n\r\n");
    message.extend_from_slice(markup.as_bytes());

    let parsed = MessageParser::default().parse(&message)?;
    let text = parsed.body_text(0)?.into_owned();

    (!text.trim().is_empty()).then_some(text)
}

/// Whether text is really markup that was not recognised as such.
///
/// A rejection, not a parser: the worst it can do is leave a message without a preview, and it is
/// deliberately blunt for that reason. A closing tag or an opening tag with an attribute is not
/// something a person writes into the first lines of a message.
fn looks_like_markup(text: &str) -> bool {
    if text.contains("</") {
        return true;
    }

    text.split('<').skip(1).any(|after| {
        let tag = after.split('>').next().unwrap_or_default();
        // `<p>`, `<div class="x">`, `<br/>` — but not `<name@example.com>`, which is an address.
        !tag.is_empty()
            && tag.len() < 200
            && after.contains('>')
            && !tag.contains('@')
            && tag.starts_with(|first: char| {
                first.is_ascii_alphabetic() || first == '!' || first == '/'
            })
    })
}

/// Drops the part of a reply that is the message being replied to.
///
/// Lines marked with `>` are the quoted original; the line introducing them ("On … wrote:") goes
/// with it. What is left is what this person actually wrote — and when nothing is left, the whole
/// text is used, because a preview of something is better than a preview of nothing.
fn without_quoted_reply(text: &str) -> String {
    let mut written = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('>') {
            // The attribution sits directly above the quotation and reads as part of it.
            if written
                .last()
                .is_some_and(|previous: &&str| previous.trim_end().ends_with("wrote:"))
            {
                written.pop();
            }
            break;
        }
        written.push(line);
    }

    if written.iter().any(|line| !line.trim().is_empty()) {
        written.join("\n")
    } else {
        text.to_owned()
    }
}

/// Who the message is from, as the list shows it: the name when there is one, the address when
/// there is not.
fn sender_of(message: &Message<'_>) -> Option<String> {
    let from = message.from()?;
    let first = from.first()?;

    match (first.name(), first.address()) {
        (Some(name), _) if !name.trim().is_empty() => Some(name.trim().to_owned()),
        (_, Some(address)) if !address.trim().is_empty() => Some(address.trim().to_owned()),
        _ => None,
    }
}

/// The address the message is from, which is where an answer to it goes.
///
/// Never the display name: a reply addressed to `Zeta Quorin` is a reply that cannot be sent, and
/// a display name written to look like an address is a reply sent somewhere the message never came
/// from. Only what the `From` header gave as the address itself.
fn sender_address_of(message: &Message<'_>) -> Option<String> {
    let address = message.from()?.first()?.address()?.trim();
    (!address.is_empty()).then(|| address.to_owned())
}

/// Every address in one of the headers that names people, each once and in the order written.
///
/// Addresses only. A display name cannot be replied to, and a group — `To: friends:;` — is a name
/// for a list whose members the header does not carry, so there is nothing in it to answer.
fn addresses_of(header: Option<&Address<'_>>) -> Vec<String> {
    let Some(header) = header else {
        return Vec::new();
    };

    let mut found: Vec<String> = Vec::new();
    for entry in header.iter() {
        let Some(address) = entry.address().map(str::trim).filter(|it| !it.is_empty()) else {
            continue;
        };
        // The same person named twice in one header is a copy of the answer they did not ask for.
        if !found.iter().any(|held| held == address) {
            found.push(address.to_owned());
        }
    }

    found
}

/// The identifiers this message answers, oldest first. What threading is built from.
fn references_of(message: &Message<'_>) -> Vec<Rfc822MessageId> {
    let mut found: Vec<Rfc822MessageId> = match message.references() {
        HeaderValue::Text(value) => Rfc822MessageId::parse(value).into_iter().collect(),
        HeaderValue::TextList(values) => values
            .iter()
            .filter_map(|value| Rfc822MessageId::parse(value).ok())
            .collect(),
        _ => Vec::new(),
    };

    // `In-Reply-To` names the direct parent, which a broken client sometimes leaves out of
    // `References`.
    if let Some(parent) = message
        .in_reply_to()
        .as_text()
        .and_then(|value| Rfc822MessageId::parse(value).ok())
    {
        if !found.contains(&parent) {
            found.push(parent);
        }
    }

    found
}
