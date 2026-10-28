//! The pictures a message draws with a style, and the road they take.
//!
//! A background fetched from a host tells its owner the message was opened, the hour, and roughly
//! where from — the same thing a picture in an `<img>` tells them, so it takes the same road: the
//! address leaves here, this application fetches it if the reader asked, and what stays in the
//! stylesheet is a name that means nothing anywhere else.
//!
//! Its own file because two doors lead here — the `style=` attribute, which the sanitizer hands
//! over one declaration at a time, and the `<style>` block, which it keeps whole — and one answer
//! has to serve both. Two copies of it would answer differently the day one was edited.

use std::sync::atomic::Ordering;
use std::sync::Mutex;

use super::styling;
use super::{as_an_address, picture_written_in, serving, Counting, Link, Pictures};
use crate::infrastructure::journal::journal;
use crate::ports::body::InlinePart;
use crate::ports::journal::{Journal, Note, Value, Word};

/// Every `<style>` block of a cleaned document, with its pictures served from here.
///
/// The blocks are found by their tags, and by the tokenizer's own rule for where one ends rather
/// than by the first `</style` in the text. A raw-text element ends on `</style` followed by
/// whitespace, `/` or `>` and on nothing else, so `</stylex` is CSS and not a tag — and a scan that
/// stopped there came out of step with the document and went on to rewrite bytes belonging to no
/// stylesheet at all: a paragraph's own text, and in one shape an address written only as prose
/// turned into something this application went and fetched.
///
/// The corpus was the argument for the simpler scan and it could not have been: it holds what
/// people have sent, and the shape that breaks this is one somebody writes on purpose.
pub(super) fn stylesheets_pointing_at_pictures(
    html: &str,
    counting: &Counting,
    pointed_at: &Mutex<Vec<Link>>,
    remote: Pictures,
    inside: &std::collections::HashMap<String, String>,
) -> String {
    let mut written = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(at) = a_tag_at(rest, "<style") {
        let after = &rest[at..];
        let Some(opened) = after.find('>') else {
            break;
        };
        let Some(closed) = a_tag_at(&after[opened..], "</style") else {
            break;
        };

        let css = &after[opened + 1..opened + closed];
        let (served, blocked, never) = styling::pictures_pointed_at(css, |address| {
            a_picture_a_stylesheet_points_at(address, counting, pointed_at, remote, inside)
        });

        counting.blocked.fetch_add(blocked, Ordering::SeqCst);
        counting.not_shown.fetch_add(never, Ordering::SeqCst);

        written.push_str(&rest[..at]);
        written.push_str(&after[..=opened]);
        written.push_str(&served);
        rest = &after[opened + closed..];
    }

    written.push_str(rest);
    written
}

/// What became of one picture a style pointed at, written down.
///
/// Where it was meant to come from, and if it is not coming, why. Never the address: that is
/// written by whoever sent the message and says who they are and who is reading. What is here is
/// enough to tell "the message carries this and it was not found" from "nobody has asked yet" from
/// "not an address this build fetches" — which is exactly the question the three counters on screen
/// cannot answer, because they are the same number under different questions.
fn said(from: Word, why: Option<Word>) {
    let journal = journal();
    if !journal.keeps(Note::Detail) {
        return;
    }

    match why {
        Some(why) => journal.write_down(
            Note::Detail,
            "picture.refused",
            &[
                ("where", Value::Word(Word::InAStylesheet)),
                ("from", Value::Word(from)),
                ("why", Value::Word(why)),
            ],
        ),
        None => journal.write_down(
            Note::Detail,
            "picture.taken",
            &[
                ("where", Value::Word(Word::InAStylesheet)),
                ("from", Value::Word(from)),
            ],
        ),
    }
}

/// Where this tag really begins, if it does.
///
/// A tag name ends at whitespace, `/` or `>`, so `<stylex` and `</stylex` are not the tag and
/// html5ever does not treat them as one. Matched without regard to case because that is how a name
/// is compared, and searched byte by byte because the haystack is a whole document.
fn a_tag_at(html: &str, tag: &str) -> Option<usize> {
    // Compared as bytes rather than as text: a slice of a `str` has to end on a character boundary
    // and the byte after a tag name is chosen by whoever sent the message, so `<style` followed by
    // a letter outside ASCII is a slice this would panic on. A match always begins at `<`, which is
    // never the middle of a character, so the position handed back is a boundary.
    let html = html.as_bytes();
    let name = tag.as_bytes();

    (0..html.len().saturating_sub(name.len())).find(|at| {
        html[*at..*at + name.len()].eq_ignore_ascii_case(name)
            && matches!(
                html[*at + name.len()],
                b'\t' | b'\n' | b'\x0C' | b'\r' | b' ' | b'/' | b'>'
            )
    })
}

/// What to do with one address a stylesheet points at.
///
/// The same three answers a picture in an `<img>` gets, and by the same rules: one the message
/// carries is already in hand and is shown; one from a host is fetched only when the reader asked;
/// anything else never arrives. Written once because a background and a picture are the same
/// question, and two copies of it would answer differently the day one of them was edited.
pub(super) fn a_picture_a_stylesheet_points_at(
    address: &str,
    counting: &Counting,
    pointed_at: &Mutex<Vec<Link>>,
    remote: Pictures,
    inside: &std::collections::HashMap<String, String>,
) -> styling::ThePicture {
    // A picture the message carried, named the way a message names one. The `<img>` road has always
    // found these; a background naming the same picture found nothing and was counted as one that
    // could never arrive — a logo the message was carrying all along, drawn as an empty box.
    if let Some(carried) = address.strip_prefix("cid:") {
        let carried = carried.trim_matches(['<', '>']);

        return match inside.get(carried) {
            Some(name) => {
                said(Word::Carried, None);
                styling::ThePicture::ServedAs(name.clone())
            }
            None => {
                said(Word::Carried, Some(Word::NotCarried));
                styling::ThePicture::NeverArriving
            }
        };
    }

    // Carried inside the message. Nothing is fetched and nobody learns it was opened.
    if let Some((kind, bytes)) = picture_written_in(address) {
        let Some(name) = serving::random_name() else {
            said(Word::WrittenIn, Some(Word::NoNameToBeHad));
            return styling::ThePicture::NeverArriving;
        };
        said(Word::WrittenIn, None);

        counting
            .written_in
            .lock()
            .expect("the lock is never poisoned")
            .push(InlinePart {
                name: name.clone(),
                content_type: kind.to_owned(),
                bytes,
            });

        return styling::ThePicture::ServedAs(name);
    }

    let address = as_an_address(address);

    if !crate::infrastructure::http::fetchable(&address) {
        said(Word::Elsewhere, Some(Word::NotFetchable));
        return styling::ThePicture::NeverArriving;
    }

    if remote != Pictures::Shown {
        said(Word::Elsewhere, Some(Word::NobodyAsked));
        return styling::ThePicture::NotAskedFor;
    }

    let Some(name) = serving::random_name() else {
        said(Word::Elsewhere, Some(Word::NoNameToBeHad));
        return styling::ThePicture::NeverArriving;
    };

    said(Word::Elsewhere, None);

    pointed_at
        .lock()
        .expect("the lock is never poisoned")
        .push(Link {
            name: name.clone(),
            url: address,
        });

    styling::ThePicture::ServedAs(name)
}
