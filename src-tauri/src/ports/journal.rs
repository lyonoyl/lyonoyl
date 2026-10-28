//! What may be written down about a run, and the shape it is allowed to take.
//!
//! A mail client's log is one keystroke away from being the least private file on the disk: every
//! database page this application writes is encrypted, and a log sits beside them in plain text.
//! So the question this file answers is not "how do we write a line" but "what is a line allowed to
//! be", and the answer is enforced by the compiler rather than by anybody remembering it.
//!
//! **A record carries numbers and words this build chose, and nothing else.** There is no variant
//! holding a `String` and none holding a `&str`, so an address, a subject, a filename, a header, a
//! `Content-ID` or the text of somebody else's error cannot be passed in — not by mistake, not
//! under a flag, and not in a development build. Section 6 of the contract says redaction is
//! enforced by types rather than by discipline; this is that type.

/// How much of what happens is written down.
///
/// Three, named by how often they happen rather than by how bad they are. A level that describes a
/// mood is a level everybody argues about; a level that describes a rate is one anybody can place a
/// record into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Note {
    /// Something that went wrong — a pass that stopped, a queue that could not start — and the line
    /// that opens every run. Every build writes it.
    Trouble,
    /// A milestone of the application rather than of one message: an update offered. Every build
    /// writes it, so nothing about one message is written at it: a line for every message opened
    /// would be a record of when somebody read their mail, in a file the databases' key does not
    /// cover.
    #[default]
    Worth,
    /// One line per item: a message opened, a pass and what it cost. Written only when somebody
    /// asked: for the run `LYONOYL_LOG=detail` is set in, or for as long as a file called `detail`
    /// is in the log folder.
    Detail,
}

impl Note {
    /// The word this level is written as, and read back from.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trouble => "trouble",
            Self::Worth => "worth",
            Self::Detail => "detail",
        }
    }

    /// What a stored or typed answer means. Anything unreadable is the quiet one: every way of
    /// being wrong here has to land on the side that writes less down.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "trouble" => Self::Trouble,
            "detail" => Self::Detail,
            _ => Self::Worth,
        }
    }

    /// Whether a record at this level is written when the journal is set to `set_to`.
    pub fn reaches(self, set_to: Self) -> bool {
        self <= set_to
    }
}

/// One thing a record can say about a number.
///
/// `Copy`, and every variant is a number or a word from the list below. That is the barrier: there
/// is nowhere to put a string, so there is nowhere for a message to end up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    /// How many of something.
    Count(u64),
    /// A size. Rounded to a power of two before it is written: an exact length is a fingerprint of
    /// which message was opened to anybody holding a copy of the mail, and no question this log
    /// answers needs the last digit.
    Bytes(u64),
    /// How long something took.
    Millis(u64),
    /// Whether or not.
    Yes(bool),
    /// One of the words this build knows.
    Word(Word),
}

/// The words a record may carry.
///
/// A fieldless enum rather than `&'static str`, and the difference is the whole point:
/// `String::leak` turns anything at all into a `&'static str`, so a variant taking one would be a
/// door with a sign on it. Adding a word means adding a variant here, which is a line in a diff
/// somebody reads — and the list holds what records actually write and nothing kept for later,
/// because a vocabulary nobody uses is a vocabulary nobody keeps honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Word {
    // What a body turned out to be.
    Html,
    // What the frame asked the store for.
    APage,
    APart,
    ALink,
    // Where a picture was named.
    InAnImage,
    InAStyleAttribute,
    InAStylesheet,
    // What a picture turned out to be.
    WrittenIn,
    Carried,
    Elsewhere,
    // Why one is not on screen.
    NobodyAsked,
    NotFetchable,
    NotCarried,
    NotAKindShownHere,
    NoNameToBeHad,
    NotThePictureClaimed,
    HostRefused,
    HostUnreachable,
    TooLarge,
    TooLongToRead,
    // Why a reading pass over an account stopped. The interface collapses these to four sentences
    // a person can act on; the two databases are kept apart here because a database that will not
    // open and one that opened wrong lead to different afternoons.
    TheNetworkWasNotThere,
    TheSignInWasRefused,
    TheDatabaseWasUnavailable,
    TheDatabaseWasDamaged,
    // The two doors behind that one sentence. Both are "this device's storage cannot be read" on
    // screen and they are not the same afternoon: one is a file, the other is the system's own
    // key store refusing a build it did not write.
    TheIndexWouldNotOpen,
    TheMessageKeyWasRefused,
    SomethingNobodyExpected,
}

impl Word {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::APage => "aPage",
            Self::APart => "aPart",
            Self::ALink => "aLink",
            Self::InAnImage => "inAnImage",
            Self::InAStyleAttribute => "inAStyleAttribute",
            Self::InAStylesheet => "inAStylesheet",
            Self::WrittenIn => "writtenIn",
            Self::Carried => "carried",
            Self::Elsewhere => "elsewhere",
            Self::NobodyAsked => "nobodyAsked",
            Self::NotFetchable => "notFetchable",
            Self::NotCarried => "notCarried",
            Self::NotAKindShownHere => "notAKindShownHere",
            Self::NoNameToBeHad => "noNameToBeHad",
            Self::NotThePictureClaimed => "notThePictureClaimed",
            Self::HostRefused => "hostRefused",
            Self::HostUnreachable => "hostUnreachable",
            Self::TooLarge => "tooLarge",
            Self::TooLongToRead => "tooLongToRead",
            Self::TheNetworkWasNotThere => "theNetworkWasNotThere",
            Self::TheSignInWasRefused => "theSignInWasRefused",
            Self::TheDatabaseWasUnavailable => "theDatabaseWasUnavailable",
            Self::TheDatabaseWasDamaged => "theDatabaseWasDamaged",
            Self::TheIndexWouldNotOpen => "theIndexWouldNotOpen",
            Self::TheMessageKeyWasRefused => "theMessageKeyWasRefused",
            Self::SomethingNobodyExpected => "somethingNobodyExpected",
        }
    }
}

/// One `name=value` of a record. The name is a literal in this repository's own source, which the
/// macro below is what guarantees.
pub type Field = (&'static str, Value);

/// What a journal does. A trait so the application layer can write a record without knowing there
/// is a file anywhere — section 1 forbids `use crate::infrastructure` above the adapter.
pub trait Journal: Send + Sync {
    /// Writes one record. Never blocks, never fails, never returns anything to handle: a caller
    /// that has to think about logging is a caller that stops doing it.
    fn write_down(&self, note: Note, event: &'static str, fields: &[Field]);

    /// Whether a record at this level would be kept. Read before building fields, so a level that
    /// is off costs one comparison.
    fn keeps(&self, note: Note) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_can_hold_nothing_that_borrows() {
        // The barrier, asserted rather than described. `Copy` is what a variant holding a `String`
        // could not be, and this stops compiling the day somebody adds one.
        const fn holds_nothing_borrowed<T: Copy + 'static>() {}
        const _: () = holds_nothing_borrowed::<Value>();
        const _: () = holds_nothing_borrowed::<Word>();
    }

    #[test]
    fn every_word_is_written_as_something_and_no_two_the_same() {
        // A word written as the empty string is a field that reads as absent; two words written the
        // same way are two different things a reader cannot tell apart.
        let all = [
            Word::Html,
            Word::APage,
            Word::APart,
            Word::ALink,
            Word::InAnImage,
            Word::InAStyleAttribute,
            Word::InAStylesheet,
            Word::WrittenIn,
            Word::Carried,
            Word::Elsewhere,
            Word::NobodyAsked,
            Word::NotFetchable,
            Word::NotCarried,
            Word::NotAKindShownHere,
            Word::NoNameToBeHad,
            Word::NotThePictureClaimed,
            Word::HostRefused,
            Word::HostUnreachable,
            Word::TooLarge,
            Word::TooLongToRead,
        ];

        let mut written: Vec<&str> = all.iter().map(|word| word.as_str()).collect();
        let counted = written.len();
        written.sort_unstable();
        written.dedup();

        assert_eq!(written.len(), counted, "two words are written the same way");
        assert!(
            written.iter().all(|word| !word.is_empty()),
            "a word is written as nothing"
        );
        // Nothing that could be mistaken for the frame of a line, or for another field.
        assert!(
            written.iter().all(|word| word
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')),
            "a word carries something a reader would have to unquote"
        );
    }

    #[test]
    fn a_level_that_cannot_be_read_writes_less_rather_than_more() {
        assert_eq!(Note::parse("detail"), Note::Detail);
        assert_eq!(Note::parse(" TROUBLE "), Note::Trouble);
        for unreadable in ["", "verbose", "all", "9", "worth "] {
            assert_eq!(Note::parse(unreadable), Note::Worth, "{unreadable:?}");
        }
    }

    #[test]
    fn a_record_reaches_the_file_only_at_or_above_what_was_asked_for() {
        assert!(Note::Trouble.reaches(Note::Worth));
        assert!(Note::Worth.reaches(Note::Worth));
        assert!(!Note::Detail.reaches(Note::Worth));
        assert!(Note::Detail.reaches(Note::Detail));
        assert!(!Note::Worth.reaches(Note::Trouble));
    }
}
