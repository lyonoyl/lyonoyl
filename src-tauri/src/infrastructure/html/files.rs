//! What a message carries beside its body.
//!
//! Its own file because the one it came out of had grown past what a person reads at once, and
//! because this is a separate question from rendering: a body is made safe to look at, and a file
//! is made safe to *save*, or to go on with the message passed on to somebody else. The two share
//! a parser and nothing else.

use std::collections::HashSet;

use mail_parser::{Encoding, Message, MessageParser, MessagePart, MimeHeaders, PartType};

use super::guards::{behind_the_guards, Asking, Guarded};
use super::{picture_served_as, serving, LARGEST_PICTURE};
use crate::ports::body::{Attached, InlinePart};
use crate::ports::message_parts::PassedOn;

/// Takes what a message carries out of it: one file by where it sits, or all of it to pass on.
pub struct MimeParts;

impl crate::ports::message_parts::MessageParts for MimeParts {
    fn file_at(&self, raw: &[u8], at: u32) -> Option<(String, Vec<u8>)> {
        let parsed = MessageParser::default().parse(raw)?;
        let part = parsed.parts.get(at as usize)?;
        let bytes = bytes_of(&parsed, part)?;

        // The same rule the listing followed, applied again rather than trusted from there: what
        // arrives here is a number the window sent, and the message may have been fetched again
        // since the listing was made.
        let name =
            lyonoyl_domain::attachment::safe_name(part.attachment_name().unwrap_or_default());

        Some((name, bytes))
    }

    fn passed_on(&self, raw: &[u8]) -> Option<PassedOn> {
        // Behind the guards, as showing it is: what is taken out converts markup, and the message
        // is whole and written by whoever sent it.
        passed_as(behind_the_guards(
            raw,
            "message-passed-on",
            Asking::SomebodyIsWaiting,
            |message| {
                MessageParser::default()
                    .parse(message)
                    .map(|parsed| to_pass_on(&parsed))
                    .unwrap_or_default()
            },
        ))
    }
}

/// What each of the guards' three answers is for a message passed on.
///
/// Apart from `passed_on` so that it can be asked without making the machine busy, as `shown` is
/// for a body: a test that took the places the process shares would take them from every test
/// beside it.
pub(super) fn passed_as(read: Guarded<PassedOn>) -> Option<PassedOn> {
    match read {
        Guarded::Answered(passed) => Some(passed),
        // Looked at, for as long as this build looks at anything, and nothing came of it: the
        // reader shows nothing of this message either, and a message passed on carries what was
        // seen of it.
        Guarded::Refused => Some(PassedOn::default()),
        Guarded::Busy => None,
    }
}

/// What a message parsed whole carries when it is passed on: its text, and its files in the order
/// it carries them.
///
/// The files are the ones the reader lists beside the message and the pictures drawn inside it:
/// passed on as text, the message draws nothing, and a picture somebody saw in it would otherwise
/// be lost on the way. Every one of them can be taken off before it goes.
fn to_pass_on(parsed: &Message<'_>) -> PassedOn {
    let mut carried = files_inside(parsed);
    carried.extend(pictures_as_files(parsed));
    carried.sort_by_key(|one| one.at);

    PassedOn {
        // Its lines as it wrote them, ending as a text box ends them: a line that ends in a return
        // as well is one a window rewrites the moment it is shown.
        text: super::words::written_text(parsed)
            .map(|text| text.lines().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default(),
        files: carried
            .into_iter()
            .filter_map(|one| {
                let bytes = bytes_of(parsed, parsed.parts.get(one.at as usize)?)?;
                Some((one.name, bytes))
            })
            .collect(),
    }
}

/// What one part holds, as the file it is: the bytes as they were decoded, and a part read as text
/// as that text.
///
/// Text parts too: a message can attach a `.txt` or a `.csv`, and refusing to save it because it
/// happens to be readable would be an odd rule to explain.
fn bytes_of(parsed: &Message<'_>, part: &MessagePart<'_>) -> Option<Vec<u8>> {
    match &part.body {
        PartType::Binary(bytes) | PartType::InlineBinary(bytes) => Some(bytes.to_vec()),
        PartType::Text(text) | PartType::Html(text) => Some(text.as_bytes().to_vec()),
        PartType::Message(_) => carried(parsed, part).map(<[u8]>::to_vec),
        _ => None,
    }
}

/// The bytes of a message carried inside this one, whole.
///
/// Carried as it was written, it is read in place: what the parser keeps as its bytes is the whole
/// of the message carrying it, and where it lies in there is the part's to say. Carried encoded,
/// it was decoded first, and the bytes it was read from are its own.
fn carried<'a>(parsed: &'a Message<'_>, part: &'a MessagePart<'_>) -> Option<&'a [u8]> {
    let PartType::Message(nested) = &part.body else {
        return None;
    };

    match part.encoding {
        Encoding::None => parsed
            .raw_message
            .get(part.offset_body as usize..part.offset_end as usize),
        _ => Some(&nested.raw_message),
    }
}

/// The files a message carries that its body does not refer to.
///
/// Listed, never held. What comes back is a position, a name and a size — the bytes stay in the
/// message until somebody asks to save one, because a message with four photographs in it is four
/// photographs nobody has asked for yet.
///
/// A part is a file here when it has a filename of its own, or says it is an attachment. A part
/// the body points at is a picture and belongs to the body; a part with neither a name nor a
/// disposition is the message's own text in one of its forms.
///
/// A message carried inside it is one, whatever it says about itself. A file written in text — a
/// table, a note, an invitation — is one too. The parser reads every text part as text, whatever
/// it is for, so such a file arrives as text rather than as bytes; it is a file where the parser set
/// it aside rather than reading it as the message, and could decode it: a part it could not is not
/// what was sent.
pub(crate) fn files_inside(parsed: &Message<'_>) -> Vec<Attached> {
    let mut found = Vec::new();
    let set_aside: HashSet<u32> = parsed.attachments.iter().copied().collect();

    for (at, part) in parsed.parts.iter().enumerate() {
        // A message carried inside this one is a file whatever it says about itself: it is not
        // this message's text in any of its forms. Named so a system opens it as a message.
        if let PartType::Message(nested) = &part.body {
            found.push(Attached {
                at: at as u32,
                name: message_named(part.attachment_name(), nested.subject()),
                bytes: carried(parsed, part).map_or(0, <[u8]>::len) as u64,
            });
            continue;
        }

        let size = match &part.body {
            PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes.len(),
            PartType::Text(text) | PartType::Html(text)
                if set_aside.contains(&(at as u32)) && !part.is_encoding_problem =>
            {
                text.len()
            }
            _ => continue,
        };

        // A part the body refers to is a picture in it, not a file beside it. Shown twice, it
        // would be a photograph a person saves and then finds already in the message.
        if part.content_id().is_some() {
            continue;
        }

        let named = part.attachment_name();
        let says_so = part
            .content_disposition()
            .is_some_and(|disposition| disposition.ctype().eq_ignore_ascii_case("attachment"));

        if named.is_none() && !says_so {
            continue;
        }

        found.push(Attached {
            at: at as u32,
            // Never the string the message carried: it is written by whoever sent it, and it is
            // about to reach a save panel.
            name: lyonoyl_domain::attachment::safe_name(named.unwrap_or_default()),
            bytes: size as u64,
        });
    }

    found
}

/// The name a message carried inside another goes by: the name it was given, made safe and ending as
/// a message's file does, or where it was given none, its own subject.
fn message_named(given: Option<&str>, subject: Option<&str>) -> String {
    match given {
        Some(given) if given.to_ascii_lowercase().ends_with(".eml") => {
            lyonoyl_domain::attachment::safe_name(given)
        }
        Some(given) => lyonoyl_domain::attachment::message_file_name(Some(given)),
        None => lyonoyl_domain::attachment::message_file_name(subject),
    }
}

/// The pictures a message carries inside it, listed as files to save.
///
/// The complement of `files_inside`, which passes over every part with a `Content-ID` because the
/// body draws it in place. When the body is shown as text instead of drawn, nothing draws them any
/// more, so a picture that travelled inside the message becomes a file beside it — listed and
/// savable rather than lost. Only for that fallback; the drawn path shows them in place and lists
/// nothing here.
///
/// Every binary part with a `Content-ID` is listed, whatever its kind or size: `files_inside`
/// skipped it for being a picture, and one this build would not have drawn even were it drawing is
/// still a file its reader may want.
pub(crate) fn pictures_as_files(parsed: &Message<'_>) -> Vec<Attached> {
    let mut found = Vec::new();

    for (at, part) in parsed.parts.iter().enumerate() {
        let bytes: &[u8] = match &part.body {
            PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes,
            _ => continue,
        };

        if part.content_id().is_none() {
            continue;
        }

        found.push(Attached {
            at: at as u32,
            name: lyonoyl_domain::attachment::safe_name(part.attachment_name().unwrap_or_default()),
            bytes: bytes.len() as u64,
        });
    }

    found
}

/// The pictures a message carries inside it that this build will not put on screen, by the name
/// the body refers to them by.
///
/// Kept apart from the ones it will. Both are "the picture is not on screen", and they are not the
/// same sentence: one is a message pointing at something it does not carry, which is a broken
/// message, and the other is this build passing one over -- one too large to hold, or of a kind
/// nothing here draws.
///
/// Told apart because the wrong one was being said. A message whose pictures are ten inline
/// drawings was reported as pointing at ten pictures that are not where it says they are, and
/// offered a button to fetch them again -- which could never do anything, because there was
/// nothing anywhere to fetch.
pub(crate) fn pictures_declined(
    parsed: &Message<'_>,
    taken: &[(String, InlinePart)],
) -> std::collections::HashSet<String> {
    let held: std::collections::HashSet<&str> =
        taken.iter().map(|(name, _)| name.as_str()).collect();
    let mut declined = std::collections::HashSet::new();

    for part in parsed.parts.iter() {
        if !matches!(&part.body, PartType::Binary(_) | PartType::InlineBinary(_)) {
            continue;
        }

        let Some(identifier) = part.content_id() else {
            continue;
        };
        let identifier = identifier.trim_matches(|c| c == '<' || c == '>').to_owned();

        // Whatever the reason `pictures_inside` passed it over -- a kind this build will not
        // render, one too large to hold -- the answer to a reader is the same and this cannot
        // drift from it: what it did not take is what is declined.
        if !held.contains(identifier.as_str()) {
            declined.insert(identifier);
        }
    }

    declined
}

pub(crate) fn pictures_inside(parsed: &Message<'_>) -> Vec<(String, InlinePart)> {
    let mut found = Vec::new();

    for part in parsed.parts.iter() {
        let bytes: &[u8] = match &part.body {
            PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes,
            _ => continue,
        };
        if bytes.len() > LARGEST_PICTURE {
            continue;
        }

        let Some(identifier) = part.content_id() else {
            continue;
        };
        let Some(content_type) = part
            .content_type()
            .filter(|kind| kind.ctype().eq_ignore_ascii_case("image"))
            .and_then(|kind| kind.subtype())
            .and_then(picture_served_as)
            .map(str::to_owned)
        else {
            continue;
        };

        let Some(name) = serving::random_name() else {
            continue;
        };
        let identifier = identifier.trim_matches(|c| c == '<' || c == '>').to_owned();

        found.push((
            identifier,
            InlinePart {
                name,
                content_type,
                bytes: bytes.to_vec(),
            },
        ));
    }

    found
}
