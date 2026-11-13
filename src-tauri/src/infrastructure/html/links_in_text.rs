//! Where the addresses are in text, so that text can offer them as links.
//!
//! An address counts when it is written whole: beginning `http://`, `https://` or `mailto:`, or
//! `www.` at the start of a word, which is opened over `https://`. It runs to the first space, or
//! the first character an address in a sentence does not hold, and leaves behind what a sentence
//! puts after one — a full stop, a comma, a quote, a closing bracket it did not open. What is left
//! is read by the URL standard's parser, and an address it cannot read, or one this build does not
//! open, stays text.
//!
//! Somebody to write to counts written alone too, `zeta@example.com`, and opens as `mailto:`: a
//! name of letters, digits and `._+-`, an `@`, and a domain of letters, digits, dots and hyphens,
//! read for as long as they last and no further. It starts only where no name could be going on,
//! ends only where nothing could go on with it, and has the shape mail is sent to, which
//! [`is_somebody_to_write_to`] describes. Anything else stays text, rather than a link to whoever a
//! piece of it names.
//!
//! The core links these in plain-text mail, in the markup it sends beside a message's text, and
//! above a message passed on in its own form; the window marks the same addresses as a message is
//! written, and links them in a message sent from here before its copy is read back. Both answer
//! the same examples, `tests/fixtures/links-in-text.json`, which the tests on both sides read.

use std::ops::Range;

use super::openable;

/// One address in a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextLink {
    /// Where it is, in bytes of the text.
    pub at: Range<usize>,
    /// What it opens, as the URL standard writes it out: what a person is asked about, and what the
    /// system is handed.
    pub url: url::Url,
}

/// How an address may begin, as lower-case letters.
const BEGINNINGS: [&str; 4] = ["https://", "http://", "mailto:", "www."];

/// What an address never holds in a sentence, besides spaces and control characters. The ASCII
/// ones are the characters the URL standard asks to be written escaped; the rest are the marks
/// that end a clause in scripts that put no space after an address.
const ENDS: [char; 35] = [
    '<', '>', '"', '`', '{', '}', '|', '\\', '^', '\u{3002}', '\u{3001}', '\u{FF0C}', '\u{FF0E}',
    '\u{FF1A}', '\u{FF1B}', '\u{FF01}', '\u{FF1F}', '\u{FF08}', '\u{FF09}', '\u{300C}', '\u{300D}',
    '\u{300E}', '\u{300F}', '\u{3010}', '\u{3011}', '\u{300A}', '\u{300B}', '\u{3008}', '\u{3009}',
    '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}', '\u{00AB}', '\u{00BB}',
];

/// What a sentence puts after an address, and an address almost never ends with.
const AFTER: [char; 10] = ['.', ',', ';', ':', '!', '?', '\'', '"', '*', '\u{2026}'];

/// The longest somebody to write to can be, as mail carries one: 254 characters of ASCII, which is
/// all such an address holds, counted without the full stop a sentence puts after it.
const LONGEST_TO_WRITE_TO: usize = 254;

/// What a name of somebody to write to holds besides letters and digits.
const IN_A_NAME_HERE: &[u8] = b"._+-";

/// What the standard lets a mail name hold besides letters, digits and dots. A word right after one
/// of them is the rest of a name, `o'brien@`, and never a name of its own.
const IN_ANY_NAME: &[u8] = b"!#$%&'*+-/=?^_`{|}~";

/// The kinds of file a picture or a document is named with, `logo@2x.png`, that no domain people
/// write to ends with.
const FILE_KINDS: [&str; 15] = [
    "png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "ico", "pdf", "txt", "html", "htm", "css",
    "js", "json",
];

/// Every address in `text`, in order.
///
/// One pass: each byte is looked at a bounded number of times, so a text of nothing but almost
/// addresses costs what any text of its length costs.
pub fn links_in(text: &str) -> Vec<TextLink> {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;

    while at < bytes.len() {
        let Some(beginning) = beginning_at(text, bytes, at) else {
            if let Some((end, url)) = written_alone_at(text, bytes, at) {
                found.push(TextLink { at: at..end, url });
                at = end;
                continue;
            }
            at += 1;
            continue;
        };

        let end = trimmed(text, at, beginning.len(), end_of_address(text, at));
        match opens(&text[at..end], beginning) {
            Some(url) => {
                found.push(TextLink { at: at..end, url });
                at = end;
            }
            None => at += beginning.len(),
        }
    }

    found
}

/// The beginning an address has at `at`, if one starts there and starts a word.
///
/// A beginning is ASCII, so wherever one matches is the start of a character of the text.
fn beginning_at(text: &str, lower: &[u8], at: usize) -> Option<&'static str> {
    let beginning = BEGINNINGS
        .into_iter()
        .find(|beginning| lower[at..].starts_with(beginning.as_bytes()))?;

    starts_a_word(text, at).then_some(beginning)
}

/// Whether `at` is where a word starts: the start of the text, or after a character that does not
/// join on to an address. A letter in another script does not, since several scripts put no space
/// between a word and the address after it.
fn starts_a_word(text: &str, at: usize) -> bool {
    match text[..at].chars().next_back() {
        None => true,
        Some(before) => {
            !(before.is_ascii_alphanumeric()
                || matches!(before, '@' | '.' | '/' | '-' | '_' | '+' | '%'))
        }
    }
}

/// Where the address starting at `start` stops: at the first space, control character or character
/// in [`ENDS`].
fn end_of_address(text: &str, start: usize) -> usize {
    text[start..]
        .char_indices()
        .find(|&(_, character)| {
            character.is_whitespace() || character.is_control() || ENDS.contains(&character)
        })
        .map_or(text.len(), |(offset, _)| start + offset)
}

/// The end of the address at `start` once what a sentence put after it is left behind: the marks in
/// [`AFTER`], and a closing bracket the address did not open. Never into its beginning, which is
/// `beginning` bytes long.
fn trimmed(text: &str, start: usize, beginning: usize, mut end: usize) -> usize {
    while end > start + beginning {
        let written = &text[start..end];
        let Some(last) = written.chars().next_back() else {
            break;
        };
        let unopened = match last {
            ')' => written.matches(')').count() > written.matches('(').count(),
            ']' => written.matches(']').count() > written.matches('[').count(),
            _ => false,
        };

        if !AFTER.contains(&last) && !unopened {
            break;
        }
        end -= last.len_utf8();
    }

    end
}

/// Somebody to write to, written alone and starting a name at `at`: where it ends, and the
/// `mailto:` it opens.
///
/// Read for as long as its name and its domain last, and no further: every character read is one a
/// name or a domain holds, so no other start lies inside what was read, and a text costs what its
/// length costs. Letters, digits and the rest are ASCII, so every place it stops starts a character.
fn written_alone_at(text: &str, lower: &[u8], at: usize) -> Option<(usize, url::Url)> {
    if !lower[at].is_ascii_alphanumeric() || !starts_a_name(text, at) {
        return None;
    }

    let bytes = text.as_bytes();
    let mut end = at;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || IN_A_NAME_HERE.contains(byte))
    {
        end += 1;
    }
    if bytes.get(end) != Some(&b'@') {
        return None;
    }
    let named = end;
    end += 1;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        end += 1;
    }

    // What follows ends it only where nothing could go on with it: another `@`, or a character a
    // name holds that a sentence does not put after an address.
    if let Some(&next) = bytes.get(end) {
        if next == b'@' || (IN_ANY_NAME.contains(&next) && !AFTER.contains(&char::from(next))) {
            return None;
        }
    }
    // A full stop after the domain is the sentence's.
    while end > named + 1 && bytes[end - 1] == b'.' {
        end -= 1;
    }

    let written = &text[at..end];
    if written.len() > LONGEST_TO_WRITE_TO || !is_somebody_to_write_to(written) {
        return None;
    }
    openable(&format!("mailto:{written}")).map(|url| (end, url))
}

/// Whether `at` is where a name of somebody to write to can start: the start of the text, or after
/// a character no name holds. Stricter than a word's start, since the characters a name may hold
/// are many, and a name read from inside another is somebody else. Not after a colon either, where
/// an address's own sign-in ends, `ftp://user:pass@host`.
fn starts_a_name(text: &str, at: usize) -> bool {
    match text[..at].chars().next_back() {
        None => true,
        Some(before) => {
            !(before.is_ascii_alphanumeric()
                || matches!(before, '.' | '@' | ':')
                || before.is_ascii() && IN_ANY_NAME.contains(&(before as u8)))
        }
    }
}

/// Whether `written` is an address mail is sent to: a name of letters, digits and `._+-` that
/// neither starts nor ends with a dot, one `@`, and a domain of at least two names of letters,
/// digits and inner hyphens, the last of two or more letters and not a kind of file.
fn is_somebody_to_write_to(written: &str) -> bool {
    let Some((name, domain)) = written.split_once('@') else {
        return false;
    };

    let named = !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || IN_A_NAME_HERE.contains(&byte));

    let names: Vec<&str> = domain.split('.').collect();
    let placed = names.len() >= 2
        && names.iter().all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
        && names.last().is_some_and(|last| {
            last.len() >= 2
                && last.bytes().all(|byte| byte.is_ascii_alphabetic())
                && !FILE_KINDS.contains(&last.to_ascii_lowercase().as_str())
        });

    named && placed
}

/// What a written address opens, if it opens anything.
///
/// Something after the beginning, read by the parser every link in a message is read by: a host,
/// for the web; for `mailto:`, anything with an `@` in it, since a person wrote the scheme; and for
/// `www.`, a name with a dot in it.
fn opens(written: &str, beginning: &str) -> Option<url::Url> {
    let rest = written
        .get(beginning.len()..)
        .filter(|rest| !rest.is_empty())?;

    match beginning {
        "www." => {
            let host = rest.split(['/', '?', '#', ':']).next().unwrap_or_default();
            if !host.contains('.') || host.starts_with('.') || host.ends_with('.') {
                return None;
            }
            openable(&format!("https://{written}"))
        }
        "mailto:" => rest.contains('@').then(|| openable(written)).flatten(),
        _ => openable(written).filter(|url| url.host_str().is_some_and(|host| !host.is_empty())),
    }
}

#[cfg(test)]
#[path = "links_in_text_tests.rs"]
mod tests;
