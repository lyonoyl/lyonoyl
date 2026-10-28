//! Where the addresses are in text, so that text can offer them as links.
//!
//! An address counts when it is written whole: beginning `http://`, `https://` or `mailto:`, or
//! `www.` at the start of a word, which is opened over `https://`. It runs to the first space, or
//! the first character an address in a sentence does not hold, and leaves behind what a sentence
//! puts after one — a full stop, a comma, a quote, a closing bracket it did not open. What is left
//! is read by the URL standard's parser, and an address it cannot read, or one this build does not
//! open, stays text.
//!
//! The window marks the same addresses as a message is written. The two answer the same examples,
//! `tests/fixtures/links-in-text.json`, which the tests on both sides read.

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

/// What a written address opens, if it opens anything.
///
/// Something after the beginning, read by the parser every link in a message is read by: a host,
/// for the web; somebody to write to, for `mailto:`; and for `www.`, a name with a dot in it.
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
