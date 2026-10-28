//! Taking the words out of a message, as opposed to showing it.
//!
//! Two shapes. The index wants tokens, so every run of whitespace collapses; an answer wants a
//! quotation and a message passed on wants its text, so the shape of the lines is left alone.
//! Each goes behind the guards, because each converts markup and a message is written by whoever
//! sent it.

use mail_parser::{Message, MessageParser, PartType};

use super::guards::{self, behind_the_guards};

/// The text of a message, for the index that has to find it again.
///
/// Behind the same guards as showing it, and for the same reason: this converts markup, and the
/// message it converts is whole. The synchronisation path reads two kilobytes for the opening and
/// says in its own comment that anything widening what is fed to a parser belongs here instead —
/// this is that.
///
/// The parser's own conversion is used, not the reader's: what goes in is a message and what comes
/// out is words to be tokenised, so there is nothing to sanitize and nothing to escape. A message
/// that carries only markup is exactly the one this is for — without it, most of what a person is
/// sent could not be searched at all.
pub fn text_to_index(raw: &[u8]) -> Read<String> {
    match behind_the_guards(
        raw,
        "message-text",
        guards::Asking::NobodyIsWaiting,
        |message| {
            let parsed = MessageParser::default().parse(message)?;
            let text = parsed.body_text(0)?.to_string();
            let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
            (!collapsed.is_empty()).then_some(collapsed)
        },
    ) {
        guards::Guarded::Answered(Some(text)) => Read::Said(text),
        guards::Guarded::Answered(None) | guards::Guarded::Refused => Read::SaidNothing,
        guards::Guarded::Busy => Read::NotLookedAt,
    }
}

/// What reading a message for the index came to.
///
/// The last of the three is the one that had nowhere to live. A message nobody could look at was
/// recorded as a message with nothing to say — and because a message is read whole exactly once,
/// that took it out of every search for good, on nothing worse than a busy minute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read<T> {
    Said(T),
    /// Looked at, and there is nothing in it to find later by.
    SaidNothing,
    /// Not looked at. Nothing is known, and the round after this one may know.
    NotLookedAt,
}
impl<T> Read<T> {
    /// Whether this reading is one to write down.
    ///
    /// A message nobody could look at is not a message with nothing to say, and only the second is
    /// a fact about the message. Recording the second when the first happened is what took a
    /// message out of every search for good: a message is read whole exactly once, so a minute
    /// when every place was taken was permanent.
    pub fn was_looked_at(&self) -> bool {
        !matches!(self, Self::NotLookedAt)
    }
}

/// The text a message was written in, for quoting it back into an answer.
///
/// The text part as it was written, and where there is none, the message read as text.
///
/// The markup is converted where reading a hostile message is safe — its own thread, its own
/// stack, a time limit and a catch for a panic — built for the index and used here for the same
/// reason, never in a library whose job is reading mail rather than distrusting it. What comes
/// back is words rather than markup, and it goes into a text box, so there is nothing to sanitize
/// and nothing to escape.
///
/// Most of what a person is sent carries no text part at all, so answering one with nothing would
/// open a blank page with nothing on it to say why — the same silence as an application that has
/// failed.
pub fn text_of(raw: &[u8]) -> Option<String> {
    let parsed = MessageParser::default().parse(raw)?;

    text_part(&parsed).or_else(|| text_kept_in_lines(raw))
}

/// What a message says, as text: its text part as written, and where it has none, its markup read
/// as text with the shape of its lines left alone.
///
/// For a caller already behind the guards, which converting markup needs.
pub(super) fn written_text(parsed: &Message<'_>) -> Option<String> {
    text_part(parsed).or_else(|| markup_in_lines(parsed))
}

/// The text part as it was written, where the message has one.
fn text_part(parsed: &Message<'_>) -> Option<String> {
    parsed.text_bodies().find_map(|part| match &part.body {
        PartType::Text(text) => Some(text.to_string()),
        _ => None,
    })
}

/// The message read as text, with the shape of its lines left alone.
///
/// Not `text_to_index`, which collapses every run of whitespace: that is what a tokeniser wants
/// and the opposite of what a quotation is — twenty lines arriving as one paragraph.
fn text_kept_in_lines(raw: &[u8]) -> Option<String> {
    match behind_the_guards(
        raw,
        "message-quote",
        guards::Asking::SomebodyIsWaiting,
        |message| markup_in_lines(&MessageParser::default().parse(message)?),
    ) {
        guards::Guarded::Answered(text) => text,
        guards::Guarded::Refused | guards::Guarded::Busy => None,
    }
}

/// What the parser reads the message's markup as, unless that is nothing but space.
fn markup_in_lines(parsed: &Message<'_>) -> Option<String> {
    let text = parsed.body_text(0)?.to_string();
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::Read;

    #[test]
    fn a_reading_that_did_not_happen_is_not_written_down() {
        assert!(Read::Said("what it says").was_looked_at());
        assert!(
            Read::<&str>::SaidNothing.was_looked_at(),
            "looked at and empty is a fact about the message, and asking again answers the same"
        );
        assert!(
            !Read::<&str>::NotLookedAt.was_looked_at(),
            "recorded, this message is never read again and is in no search for good"
        );
    }
}
