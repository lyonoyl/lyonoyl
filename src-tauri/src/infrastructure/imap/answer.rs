//! The whole answer to one command, read to the line that ends it and no further: a fetch, or the
//! listing of an account's mailboxes.
//!
//! Read here rather than through the client library's own readers, because those stop at the
//! command's tagged line whatever the line says, stop without a word when the connection closes,
//! and hand a `BYE` to a queue that nothing reads while a command runs. An answer cut short by a
//! refusal, by a server leaving or by a dropped connection reads there exactly like a whole one —
//! and a whole listing is what a pass is allowed to remove mail by.
//!
//! Each line is turned into what this adapter keeps and then let go. The client library parses a
//! line inside a buffer of four kilobytes or more and the line keeps that buffer for as long as it
//! is held, so a listing kept as the library's own values is four kilobytes a message whatever the
//! message says.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::RangeInclusive;

use async_imap::imap_proto::types::{AttributeValue, MessageSection, SectionPath};
use async_imap::imap_proto::{MailboxDatum, Response, Status};
use async_imap::types::{Flag, NameAttribute};

use super::protocol_failed;
use crate::ports::mail_provider::MailProviderError;
use lyonoyl_domain::message::Flags;

/// What one message's line in an answer said, as far as this adapter reads it.
#[derive(Debug, Default)]
pub(crate) struct Fetched {
    pub uid: Option<u32>,
    /// Nothing where the line said nothing about them, which is not a message with none: a line
    /// about a message's sequence alone says nothing about whether it was read.
    pub flags: Option<Flags>,
    pub modseq: Option<u64>,
    /// `RFC822.SIZE`.
    pub size: Option<u32>,
    /// `INTERNALDATE`, in Unix seconds.
    pub internal_date: Option<i64>,
    /// The header block.
    pub header: Option<Vec<u8>>,
    /// The text after the header, or the part of it that was asked for.
    pub text: Option<Vec<u8>>,
    /// The whole message, header and text together.
    pub whole: Option<Vec<u8>>,
}

/// Everything one fetch was answered with.
#[derive(Debug, Default)]
pub(crate) struct Answer {
    pub fetched: Vec<Fetched>,
    /// What `VANISHED` lines named while the command was being answered.
    ///
    /// Numbers in the mailbox open on the session, which is the mailbox the fetch was sent to: a
    /// line about the one open before it is said while this one is being opened, and never reaches
    /// here.
    pub vanished: Vec<RangeInclusive<u32>>,
    /// How the server refused the command, where it ended the answer with `NO` — which only an
    /// answer read with [`IfRefused::WhatArrived`] can have. What arrived is then all there is, and
    /// a message asked for and missing from it may be one the server would not hand over.
    pub refused: Option<Refusal>,
}

/// What a `NO` that ended an answer said about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// That it is a moment's matter, by one of the codes in [`FOR_NOW`]: a part of the server down,
    /// the mailbox in use, a message being removed by another session, a limit on reading reached.
    /// The same command asked again later may well be answered whole, so nothing that arrived
    /// missing from it says anything about a message — though a server may say it of the same
    /// message for ever, which is the caller's to bound.
    ForNow,
    /// Nothing about until when: what a server refusing one message it cannot read says, and says
    /// again every time it is asked.
    Standing,
}

/// The codes a refusal says it is a moment's matter by.
///
/// From RFC 5530. Three of them it gives a refusal that may pass: a part of the server down, a
/// lock someone else holds, an expunge another session issued. The fourth it gives a command that
/// would take the account over a quota, which a read can do only to a limit on reading — one a
/// server lifts after a while. The client library knows none of them, so each arrives as the first
/// word of what the server said.
const FOR_NOW: [&str; 4] = ["[UNAVAILABLE]", "[INUSE]", "[EXPUNGEISSUED]", "[OVERQUOTA]"];

/// What a refusal says about itself, read from the words the server ended the answer with.
fn refusal_in(said: Option<&str>) -> Refusal {
    match opens_with(said, &FOR_NOW) {
        true => Refusal::ForNow,
        false => Refusal::Standing,
    }
}

/// Whether the words a server ended an answer with open with one of `codes`, in any case.
///
/// A code the client library does not know arrives this way, as the first word of what the server
/// said rather than as a code, bracketed the way RFC 5530 writes it.
pub(crate) fn opens_with(said: Option<&str>, codes: &[&str]) -> bool {
    let said = said.unwrap_or_default().trim_start();

    codes.iter().any(|code| {
        said.get(..code.len())
            .is_some_and(|opening| opening.eq_ignore_ascii_case(code))
    })
}

/// One line for each message an answer spoke about, in the order each was first named.
///
/// A server may say more than one thing about a message in one answer: the line it was asked for,
/// and a flag another device changed while the answer was being sent, which a server with QRESYNC
/// on names by its UID. Two lines are one message. Taken for two, the second is a row with nothing
/// in it, put in the first one's place.
///
/// The sections are taken from whichever line carries them, the first where two do, and so are the
/// size and the date. The flags and the modification sequence are taken from the last line that
/// carries them: a server says what it holds at the moment it writes a line, and a later line is a
/// later moment. A line that names no message by its number cannot be put with any other, and is
/// left out.
pub(crate) fn by_message(lines: Vec<Fetched>) -> Vec<Fetched> {
    let mut merged: Vec<Fetched> = Vec::with_capacity(lines.len());
    let mut at: HashMap<u32, usize> = HashMap::with_capacity(lines.len());

    for line in lines {
        let Some(uid) = line.uid else {
            continue;
        };

        let Some(&index) = at.get(&uid) else {
            at.insert(uid, merged.len());
            merged.push(line);
            continue;
        };

        let held = &mut merged[index];
        held.header = held.header.take().or(line.header);
        held.text = held.text.take().or(line.text);
        held.whole = held.whole.take().or(line.whole);
        held.size = held.size.or(line.size);
        held.internal_date = held.internal_date.or(line.internal_date);
        held.flags = line.flags.or(held.flags);
        held.modseq = line.modseq.or(held.modseq);
    }

    merged
}

/// A mailbox as a listing names it: its name, and what the server says about it.
#[derive(Debug)]
pub(crate) struct Listed {
    pub name: String,
    pub attributes: Vec<NameAttribute<'static>>,
}

/// Lists every mailbox of the account, reading the answer to the line that ends it.
///
/// Read here for the reason the module gives. Through the client library, a listing refused
/// halfway, one the server left in the middle of, and one whose connection closed between two
/// lines each read as an account with fewer mailboxes — and what is not listed is treated as not
/// there: a move into it is taken as read back, what waits to be found in it goes back in the
/// queue as if it had nowhere left to wait, and sent mail with its folder unlisted is never filed.
///
/// `OK` is the whole listing. Everything else is no listing at all: `NO`, `BAD`, the server leaving,
/// the connection ending, and an answer holding more than `at_most` bytes of what is kept — a
/// server that keeps naming mailboxes is refused rather than held, whoever asked.
pub(crate) async fn mailboxes_listed<S>(
    session: &mut async_imap::Session<S>,
    at_most: usize,
) -> Result<Vec<Listed>, MailProviderError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let asked = session
        .run_command("LIST \"\" *")
        .await
        .map_err(protocol_failed)?;

    let mut listed = Vec::new();
    let mut held = 0_usize;

    loop {
        let Some(line) = session
            .read_response()
            .await
            .map_err(|_| MailProviderError::Offline)?
        else {
            // The connection ended before the listing did.
            return Err(MailProviderError::Offline);
        };

        match line.parsed() {
            Response::Done { tag, status, .. } if tag == &asked => {
                return match status {
                    Status::Ok => Ok(listed),
                    _ => Err(MailProviderError::Protocol),
                };
            }
            Response::Data {
                status: Status::Bye,
                ..
            } => return Err(MailProviderError::Offline),
            Response::MailboxData(MailboxDatum::List {
                name_attributes,
                name,
                ..
            }) => {
                // Charged for everything kept of it, before any of it is copied: its name, and each
                // thing the server says about the mailbox. A server may say one of those as often
                // as a line holds, and each is kept — charged for its name alone, a listing of a
                // few hundred megabytes holds gigabytes.
                let kept = name_attributes
                    .iter()
                    .map(|attribute| match attribute {
                        NameAttribute::Extension(written) => {
                            AN_ATTRIBUTE.saturating_add(written.len())
                        }
                        _ => AN_ATTRIBUTE,
                    })
                    .fold(A_LINE.saturating_add(name.len()), usize::saturating_add);
                held = within(held, kept, at_most)?;
                listed.push(Listed {
                    name: name.to_string(),
                    attributes: name_attributes
                        .iter()
                        .cloned()
                        .map(NameAttribute::into_owned)
                        .collect(),
                });
            }
            _ => held = within(held, A_LINE, at_most)?,
        }
    }
}

/// The most one answer to a pass may hold: new mail, what changed, or a listing.
///
/// Far past an honest answer and finite, which is the point: a server that keeps answering is
/// refused rather than held. A line is charged the room a message takes here, a little over a
/// hundred bytes, so a listing of a million messages comes to about a hundred and thirty megabytes;
/// new mail is charged its header and two kilobytes of its text besides, and a pass asks for a
/// window of it rather than for all of it. An honest answer past this — a listing of some two
/// million messages, or new mail from a server that names no next number to bound a window by —
/// fails that folder's pass each time it is asked.
pub(crate) const MOST_A_PASS_ANSWER_HOLDS: usize = 256 * 1024 * 1024;

/// The most an answer may hold when one message is read whole for a person opening or saving it.
///
/// Eight times the largest message this build renders, because a message past that can still be
/// saved as the file it arrived as, and half what the client library holds of one line before it
/// refuses the line itself.
pub(crate) const MOST_A_WHOLE_MESSAGE_HOLDS: usize =
    8 * crate::infrastructure::html::LARGEST_MESSAGE;

/// The most an answer may hold when `asked` messages are read whole for the index.
///
/// The largest message this build reads, and the room its line takes, for each one asked for, so
/// a request of ten at that size fits to the byte and holds no more than it. And never less than
/// the allowance one message has when a person opens it: the round chooses what to ask for by the
/// size a server stated, and a size can be missing or lower than the message — a message with
/// none is asked for to learn it. One past the largest this build reads, refused for it while it
/// is alone in a request, is refused again on every round, whole each time; read, it is recorded
/// as read and not asked for again.
pub(crate) fn most_whole_messages_hold(asked: usize) -> usize {
    crate::infrastructure::html::LARGEST_MESSAGE
        .saturating_add(A_LINE)
        .saturating_mul(asked)
        .max(MOST_A_WHOLE_MESSAGE_HOLDS)
}

/// What a command the server refused leaves of its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IfRefused {
    /// Nothing. What a pass concludes from an answer rests on what is missing from it as much as on
    /// what is there: a message missing from an answer refused halfway is one a cursor would pass,
    /// a range recorded as read, or a listing's reason to remove it.
    Nothing,
    /// What arrived before the refusal, marked as refused. For an answer whose every line stands
    /// alone — a message read whole, or a message described for the index, each paired with the
    /// number it was asked for by — a line that arrived is that message whatever became of the
    /// others. What is missing is the caller's to account for, and a session that has said `NO` is
    /// fit for the next command.
    WhatArrived,
}

/// What one line of an answer costs before anything it carries.
///
/// The room a message takes here once it is kept, charged to a line that is not kept as well, so a
/// server answering with nothing but noise runs out the same way as one answering with mail.
const A_LINE: usize = std::mem::size_of::<Fetched>();

/// What one thing a listing says about a mailbox costs once it is kept, before the words of one
/// this adapter has no name for.
const AN_ATTRIBUTE: usize = std::mem::size_of::<NameAttribute<'static>>();

/// Sends `command` and reads its answer to the end: the line tagged as its own.
///
/// `OK` is the whole answer. `NO` is the command refused, partly or wholly, and what that leaves
/// is `if_refused`. Everything else that can end it is a failure, because each is an answer cut
/// short with nothing to vouch for what arrived: `BAD`, the server leaving, the connection ending,
/// and an answer holding more than `at_most` bytes of what is kept.
pub(crate) async fn answer_to<S>(
    session: &mut async_imap::Session<S>,
    command: &str,
    at_most: usize,
    if_refused: IfRefused,
) -> Result<Answer, MailProviderError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let asked = session
        .run_command(command)
        .await
        .map_err(protocol_failed)?;

    let mut answer = Answer::default();
    let mut held = 0_usize;

    loop {
        let Some(line) = session
            .read_response()
            .await
            .map_err(|_| MailProviderError::Offline)?
        else {
            // The connection ended before the command did.
            return Err(MailProviderError::Offline);
        };

        match line.parsed() {
            Response::Done {
                tag,
                status,
                information,
                ..
            } if tag == &asked => {
                return match (status, if_refused) {
                    (Status::Ok, _) => Ok(answer),
                    (Status::No, IfRefused::WhatArrived) => Ok(Answer {
                        refused: Some(refusal_in(information.as_deref())),
                        ..answer
                    }),
                    _ => Err(MailProviderError::Protocol),
                };
            }
            // The server is leaving, and nothing after this is coming.
            Response::Data {
                status: Status::Bye,
                ..
            } => return Err(MailProviderError::Offline),
            Response::Fetch(_, attributes) => {
                let [header, text, whole] = sections_of(attributes);
                let carried = [header, text, whole]
                    .iter()
                    .flatten()
                    .map(|section| section.len())
                    .fold(A_LINE, usize::saturating_add);

                // Counted before anything is copied out of the line: a line past what is left is
                // refused while it is the only copy there is.
                held = within(held, carried, at_most)?;
                answer.fetched.push(Fetched {
                    header: header.map(<[u8]>::to_vec),
                    text: text.map(<[u8]>::to_vec),
                    whole: whole.map(<[u8]>::to_vec),
                    ..facts_of(attributes)
                });
            }
            Response::Vanished { uids, .. } => {
                let kept = std::mem::size_of::<RangeInclusive<u32>>().saturating_mul(uids.len());
                held = within(held, A_LINE.saturating_add(kept), at_most)?;
                answer.vanished.extend(uids.iter().cloned());
            }
            // About the mailbox rather than an answer to this command, and not kept. Counted all
            // the same: a server that never stops saying it would otherwise be read for as long as
            // it keeps talking.
            _ => held = within(held, A_LINE, at_most)?,
        }
    }
}

/// `held` with `more` added, or a refusal once that is past `at_most`.
fn within(held: usize, more: usize, at_most: usize) -> Result<usize, MailProviderError> {
    held.checked_add(more)
        .filter(|total| *total <= at_most)
        .ok_or(MailProviderError::Protocol)
}

/// The sections a line carries that this adapter keeps: the header, the text, the whole message.
///
/// The first of each where a server sends two, and either spelling of each: a server answers
/// `BODY.PEEK` with the bracketed form, and the older `RFC822` names mean the same sections.
fn sections_of<'a>(attributes: &'a [AttributeValue<'_>]) -> [Option<&'a [u8]>; 3] {
    let (mut header, mut text, mut whole) = (None, None, None);

    for attribute in attributes {
        match attribute {
            AttributeValue::BodySection {
                section: Some(SectionPath::Full(MessageSection::Header)),
                data: Some(data),
                ..
            }
            | AttributeValue::Rfc822Header(Some(data)) => {
                header.get_or_insert(data.as_ref());
            }
            AttributeValue::BodySection {
                section: Some(SectionPath::Full(MessageSection::Text)),
                data: Some(data),
                ..
            }
            | AttributeValue::Rfc822Text(Some(data)) => {
                text.get_or_insert(data.as_ref());
            }
            AttributeValue::BodySection {
                section: None,
                data: Some(data),
                ..
            }
            | AttributeValue::Rfc822(Some(data)) => {
                whole.get_or_insert(data.as_ref());
            }
            _ => {}
        }
    }

    [header, text, whole]
}

/// Everything else a line says about its message: numbers, flags and a date.
fn facts_of(attributes: &[AttributeValue<'_>]) -> Fetched {
    let mut fetched = Fetched::default();
    let mut date = None;

    for attribute in attributes {
        match attribute {
            AttributeValue::Uid(uid) => fetched.uid = Some(*uid),
            AttributeValue::ModSeq(modseq) => fetched.modseq = Some(*modseq),
            AttributeValue::Rfc822Size(size) => fetched.size = Some(*size),
            AttributeValue::Flags(names) => {
                fetched.flags = Some(
                    fetched
                        .flags
                        .unwrap_or(Flags::NONE)
                        .with(flags_named(names)),
                );
            }
            AttributeValue::InternalDate(written) => {
                date.get_or_insert(written.as_ref());
            }
            _ => {}
        }
    }

    fetched.internal_date = date.and_then(seconds_of);
    fetched
}

/// The flags a server names, reduced to the ones that mean the same thing everywhere.
///
/// `\Recent` is deliberately not among them: it describes this session rather than the message.
fn flags_named(names: &[Cow<'_, str>]) -> Flags {
    names.iter().fold(Flags::NONE, |flags, name| {
        flags.with(match Flag::from(name.as_ref()) {
            Flag::Seen => Flags::SEEN,
            Flag::Answered => Flags::ANSWERED,
            Flag::Flagged => Flags::FLAGGED,
            Flag::Draft => Flags::DRAFT,
            Flag::Deleted => Flags::DELETED,
            _ => Flags::NONE,
        })
    })
}

/// The month names `INTERNALDATE` is written with, in order.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `INTERNALDATE` in Unix seconds, or nothing when it is not written the way the protocol writes
/// it: `dd-Mon-yyyy hh:mm:ss +hhmm`, where a day of one digit may stand after a space.
///
/// Nothing rather than a guess: the message's own `Date` header is what stands in, as it does for a
/// server that sends no date at all.
pub(super) fn seconds_of(written: &str) -> Option<i64> {
    let mut words = written.split_whitespace();
    let (date, time, zone) = (words.next()?, words.next()?, words.next()?);
    if words.next().is_some() {
        return None;
    }

    let mut date = date.split('-');
    let (day, month, year) = (date.next()?, date.next()?, date.next()?);
    let mut time = time.split(':');
    let (hour, minute, second) = (time.next()?, time.next()?, time.next()?);
    if date.next().is_some() || time.next().is_some() {
        return None;
    }

    let (sign, offset) = zone.split_at_checked(1)?;
    let (offset_hours, offset_minutes) = offset.split_at_checked(2)?;

    let read = mail_parser::DateTime {
        year: digits(year, 4..=4)?,
        month: u8::try_from(
            MONTHS
                .iter()
                .position(|name| name.eq_ignore_ascii_case(month))?
                + 1,
        )
        .ok()?,
        day: digits(day, 1..=2)?,
        hour: digits(hour, 2..=2)?,
        minute: digits(minute, 2..=2)?,
        second: digits(second, 2..=2)?,
        tz_before_gmt: match sign {
            "-" => true,
            "+" => false,
            _ => return None,
        },
        tz_hour: digits(offset_hours, 2..=2)?,
        tz_minute: digits(offset_minutes, 2..=2)?,
    };

    read.is_valid().then(|| read.to_timestamp())
}

/// A number written in as many digits as `length` allows and nothing else.
fn digits<T: std::str::FromStr>(written: &str, length: RangeInclusive<usize>) -> Option<T> {
    (length.contains(&written.len()) && written.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| written.parse().ok())
        .flatten()
}

#[cfg(test)]
mod one_line_a_message {
    use super::{by_message, Fetched};
    use lyonoyl_domain::message::Flags;

    #[test]
    fn lines_about_one_number_are_one_message_with_the_newest_flags() {
        // The line asked for, and then a flag changed on another device while it was sent: one
        // message, described by the first, read as the second says.
        let lines = vec![
            Fetched {
                uid: Some(4),
                flags: Some(Flags::NONE),
                modseq: Some(40),
                size: Some(120),
                internal_date: Some(837_596_665),
                header: Some(b"Subject: Tuesday\r\n\r\n".to_vec()),
                text: Some(b"See you then.".to_vec()),
                ..Fetched::default()
            },
            Fetched {
                uid: Some(5),
                flags: Some(Flags::FLAGGED),
                ..Fetched::default()
            },
            Fetched {
                uid: Some(4),
                flags: Some(Flags::SEEN),
                modseq: Some(50),
                ..Fetched::default()
            },
            // Its sequence moved again and nothing was said about its flags, which is not a
            // message with none.
            Fetched {
                uid: Some(4),
                modseq: Some(51),
                ..Fetched::default()
            },
            // A line that names no message by its number belongs to none of them.
            Fetched {
                flags: Some(Flags::DELETED),
                ..Fetched::default()
            },
        ];

        let merged = by_message(lines);

        assert_eq!(
            merged.iter().map(|line| line.uid).collect::<Vec<_>>(),
            vec![Some(4), Some(5)],
            "in the order each was first named"
        );
        let four = &merged[0];
        assert_eq!(
            four.header.as_deref(),
            Some(&b"Subject: Tuesday\r\n\r\n"[..])
        );
        assert_eq!(four.text.as_deref(), Some(&b"See you then."[..]));
        assert_eq!(
            (four.size, four.internal_date),
            (Some(120), Some(837_596_665))
        );
        assert_eq!(four.flags, Some(Flags::SEEN));
        assert_eq!(four.modseq, Some(51));
        assert_eq!(merged[1].flags, Some(Flags::FLAGGED));
    }

    #[test]
    fn what_a_later_line_carries_fills_in_what_an_earlier_one_did_not() {
        // The flag first and the message after it: the sections come from the line that carries
        // them, wherever it is, and the first of two copies stands.
        let lines = vec![
            Fetched {
                uid: Some(42),
                flags: Some(Flags::SEEN),
                ..Fetched::default()
            },
            Fetched {
                uid: Some(42),
                whole: Some(b"the message".to_vec()),
                ..Fetched::default()
            },
            Fetched {
                uid: Some(42),
                whole: Some(b"another copy".to_vec()),
                ..Fetched::default()
            },
        ];

        let merged = by_message(lines);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].whole.as_deref(), Some(&b"the message"[..]));
        assert_eq!(merged[0].flags, Some(Flags::SEEN));
    }
}

#[cfg(test)]
mod reading_a_date {
    use super::seconds_of;

    /// The same moment, read by the parser that reads a `Date` header.
    fn as_a_header_says(written: &str) -> i64 {
        mail_parser::DateTime::parse_rfc822(written)
            .expect("a date the message parser reads")
            .to_timestamp()
    }

    #[test]
    fn a_date_written_the_way_the_protocol_writes_one_is_read() {
        assert_eq!(
            seconds_of("17-Jul-1996 02:44:25 -0700"),
            Some(as_a_header_says("Wed, 17 Jul 1996 02:44:25 -0700"))
        );
        assert_eq!(
            seconds_of("01-Jan-2024 00:00:00 +0530"),
            Some(as_a_header_says("Mon, 1 Jan 2024 00:00:00 +0530"))
        );
    }

    #[test]
    fn a_day_with_one_digit_is_padded_with_a_space_or_a_nought() {
        let expected = Some(as_a_header_says("Sun, 7 Jul 1996 02:44:25 +0000"));

        assert_eq!(seconds_of(" 7-Jul-1996 02:44:25 +0000"), expected);
        assert_eq!(seconds_of("07-Jul-1996 02:44:25 +0000"), expected);
    }

    #[test]
    fn anything_else_is_no_date_rather_than_a_guess() {
        // The message's own `Date` header stands in for a date that cannot be read, as it does for
        // a server that sends none.
        for written in [
            "",
            "17-Jul-1996",
            "17-Jul-1996 02:44:25",
            "1996-07-17 02:44:25 -0700",
            "17-Jux-1996 02:44:25 -0700",
            "17-Jul-96 02:44:25 -0700",
            "32-Jul-1996 02:44:25 -0700",
            "17-Jul-1996 24:00:00 -0700",
            "17-Jul-1996 02:44:25 0700",
            "17-Jul-1996 02:44:25 -07000",
            "17-Jul-1996 02:44:25 -0700 extra",
            "\u{0661}7-Jul-1996 02:44:25 -0700",
        ] {
            assert_eq!(seconds_of(written), None, "{written:?}");
        }
    }
}
