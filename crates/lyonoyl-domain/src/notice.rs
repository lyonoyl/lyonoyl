//! Making a message's own words safe to put on a surface this application does not own.
//!
//! A notification is drawn by the operating system, often over a locked screen, and it cannot be
//! withdrawn or corrected once it is shown. What goes into it here — a sender's name, a subject,
//! the opening of a message — was written by whoever sent the mail, which makes it the same
//! hostile input as the body: three fields a stranger fills in and this application repeats.
//!
//! So nothing is repeated as it arrived. What comes out of here holds no control character, no
//! character that reverses the text around it, and no more than it was asked for.

use crate::attachment::reorders_text;

/// What is put between two parts of a notice, and the only such character allowed through.
///
/// A line break reads as a line break and a subject running into an opening does not, so the
/// separator is worth having. It is written here rather than passed in because a message may not
/// supply one: `for_a_notice` takes every control character out, which is what keeps a subject of
/// forty blank lines from pushing the rest of the notice off the screen.
pub const BETWEEN_PARTS: char = '\n';

/// One part of a notice, as it may be shown.
///
/// `None` when nothing legible is left. That is not the same as an error: a message with no
/// subject is ordinary, and a subject made entirely of characters that cannot be shown is the same
/// thing to a reader.
///
/// `at_most` is counted in characters rather than bytes, and the cut lands on a character boundary
/// — a name cut mid-way through a letter is a name with a broken character on the end of it.
pub fn for_a_notice(raw: &str, at_most: usize) -> Option<String> {
    // Every run of space becomes one space. A subject is one line here however it was typed, and a
    // message that wrote its own indentation does not get to keep it on somebody's lock screen.
    let mut written = String::with_capacity(raw.len().min(at_most * 4));
    let mut spaced = false;

    for character in raw.chars() {
        if reorders_text(character) {
            continue;
        }

        if character.is_whitespace() || character.is_control() {
            // A control character that is not whitespace leaves nothing behind: it is not a gap
            // between words, and a space in its place would be this application inventing one.
            if character.is_whitespace() && !written.is_empty() {
                spaced = true;
            }
            continue;
        }

        if spaced {
            written.push(' ');
            spaced = false;
        }

        written.push(character);

        if written.chars().count() == at_most {
            break;
        }
    }

    (!written.is_empty()).then_some(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_text_comes_through_as_it_was_written() {
        assert_eq!(
            for_a_notice("Lunch on Friday?", 60).as_deref(),
            Some("Lunch on Friday?")
        );
    }

    #[test]
    fn a_subject_cannot_reverse_what_is_shown_around_it() {
        // The character that turned an attachment called `gpj.exe` into one that reads as a
        // picture. In a notification it reverses the rest of the line, so a message could make the
        // name it is from read as somebody else's.
        let written = for_a_notice("Invoice \u{202E}gpj.pdf", 60).expect("something is left");

        assert!(!written.contains('\u{202E}'), "{written}");
        assert!(written.contains("gpj.pdf"), "{written}");
    }

    #[test]
    fn a_message_cannot_write_its_own_lines() {
        // Without this a subject of a few blank lines pushes everything after it out of sight, and
        // a subject that looks like two fields reads as two fields.
        let written = for_a_notice("Approved\n\n\nFrom: the bank", 60).expect("something is left");

        assert_eq!(written, "Approved From: the bank");
        assert!(!written.contains(BETWEEN_PARTS));
    }

    #[test]
    fn a_tab_and_a_newline_are_one_gap() {
        assert_eq!(
            for_a_notice("two\t \n words", 60).as_deref(),
            Some("two words")
        );
    }

    #[test]
    fn a_control_character_leaves_no_gap_behind_it() {
        // A bell or an escape is not a space between words, and putting one there would join two
        // halves of a word with a gap the sender did not write.
        assert_eq!(for_a_notice("re\u{0007}ply", 60).as_deref(), Some("reply"));
    }

    #[test]
    fn nothing_legible_is_nothing_rather_than_an_empty_line() {
        assert_eq!(for_a_notice("   \n\t  ", 60), None);
        assert_eq!(for_a_notice("\u{202E}\u{200F}", 60), None);
        assert_eq!(for_a_notice("", 60), None);
    }

    #[test]
    fn what_is_too_long_is_cut_on_a_character() {
        let written = for_a_notice("αβγδεζηθικλ", 5).expect("something is left");

        assert_eq!(written, "αβγδε");
        assert_eq!(written.chars().count(), 5);
    }

    #[test]
    fn a_cut_counts_characters_rather_than_bytes() {
        // Eight characters and sixteen bytes. Counted in bytes the cut would land inside one.
        let written = for_a_notice("ζζζζζζζζ", 4).expect("something is left");

        assert_eq!(written.chars().count(), 4);
    }

    #[test]
    fn the_space_a_cut_would_have_left_is_not_kept() {
        // The gap before a word is written only when the word is, so a cut that lands on the gap
        // does not end the line with a space.
        let written = for_a_notice("one two", 3).expect("something is left");

        assert_eq!(written, "one");
    }
}
