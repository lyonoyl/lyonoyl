//! What a person typed, as a message carries it: the text as it was written, and the same text as
//! markup, for the clients that show a message only as markup.
//!
//! The markup says what the text says and nothing more. Every character that means something to
//! markup is written as the character itself. Every line is a block of its own, which finds its
//! direction from its own words. A space markup would fold away — at the start of a line, or after
//! another space — is a space that does not fold, and a tab is four of them. Lines a reply quotes,
//! under one `>` for every level, are quoted at that level, without the marks. An address the window
//! marks as a link while the message is written is a link, unless its line holds a character that
//! turns text around. The letters are the system's own, and the text and its ground are given no
//! colour, so a message is drawn in the colours its reader chose.

use mail_builder::headers::content_type::ContentType;
use mail_builder::mime::MimePart;

use super::links_in_text::links_in;
use super::LARGEST_BODY;

/// The system's own letters, wherever the message is read: the family Windows names its interface
/// font by first, since a renderer that reads only the first name stops there, then the others'.
const LETTERS: &str =
    "font-family: 'Segoe UI', -apple-system, system-ui, Roboto, Helvetica, Arial, sans-serif";

/// How the text's box breaks it: at its own line ends, and inside a word too long for the screen.
const BREAKS: &str = "white-space: pre-wrap; overflow-wrap: anywhere; word-break: break-word";

/// A quoted level, drawn as most clients draw one: indented, with a line beside it.
const QUOTED: &str = "margin: 0 0 0 0.8ex; padding-left: 1ex; border-left: 1px solid #cccccc";

/// The text as markup: a document of its own, its lines as they were typed.
pub fn markup(text: &str) -> String {
    let mut lines = String::with_capacity(text.len() * 2 + 64);
    let mut depth = 0;
    for line in text.split('\n') {
        let (level, said) = quoted(line.trim_end_matches('\r'));
        while depth < level {
            lines.push_str("<blockquote type=\"cite\" style=\"");
            lines.push_str(QUOTED);
            lines.push_str("\">");
            depth += 1;
        }
        while depth > level {
            lines.push_str("</blockquote>");
            depth -= 1;
        }

        lines.push_str("<div dir=\"auto\">");
        match said.is_empty() {
            true => lines.push_str("<br>"),
            false => line_into(&mut lines, said),
        }
        lines.push_str("</div>");
    }
    for _ in 0..depth {
        lines.push_str("</blockquote>");
    }

    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>\
         <div style=\"{LETTERS}; {BREAKS}\">{lines}</div></body></html>"
    )
}

/// The text and its markup as one part, which a reader shows whichever of the two it can; the text
/// alone where the markup is longer than `room`, or than this application draws whole.
///
/// Cut by a random boundary, as everything else this application sends is: the library's own is
/// the time in nanoseconds and a hash of something of the machine that built it.
pub fn written(text: &str, room: usize) -> MimePart<'_> {
    let markup = markup(text);
    if markup.len() > room.min(LARGEST_BODY) {
        return MimePart::new("text/plain", text);
    }

    MimePart::new(
        ContentType::new("multipart/alternative")
            .attribute("boundary", uuid::Uuid::new_v4().simple().to_string()),
        vec![
            MimePart::new("text/plain", text),
            MimePart::new("text/html", markup),
        ],
    )
}

/// Whether `markup` is the markup of `text`: a message this application wrote, whose markup says
/// nothing its text does not.
///
/// `text` as a text box holds it, without the line end a message's last line may have had, so the
/// markup is asked for both.
pub(super) fn is_the_markup_of(markup: &str, text: &str) -> bool {
    markup == self::markup(text) || markup == self::markup(&format!("{text}\n"))
}

/// How deep a line is quoted, and what it says under its marks: one `>` for every level, each with
/// the space after it, as a reply writes them.
fn quoted(line: &str) -> (usize, &str) {
    let mut level = 0;
    let mut said = line;
    while let Some(after) = said.strip_prefix('>') {
        level += 1;
        said = after.strip_prefix(' ').unwrap_or(after);
    }
    (level, said)
}

/// One line: escaped, with its spaces kept, and every address in it a link to what it opens.
///
/// No links on a line that holds a character turning text around: drawn as written, such a line
/// can show one address and open another.
fn line_into(document: &mut String, line: &str) {
    let links = match line.chars().any(turns_text_around) {
        true => Vec::new(),
        false => links_in(line),
    };

    let mut at = 0;
    let mut folds = true;
    for link in links {
        spaced_into(document, &line[at..link.at.start], &mut folds);
        document.push_str("<a href=\"");
        super::escape_into(document, link.url.as_str());
        document.push_str("\">");
        super::escape_into(document, &line[link.at.clone()]);
        document.push_str("</a>");
        folds = false;
        at = link.at.end;
    }
    spaced_into(document, &line[at..], &mut folds);
}

/// Text escaped into markup, with every space markup would fold — at the start of a line, or after
/// another — written as one that does not, and a tab as four.
fn spaced_into(document: &mut String, text: &str, folds: &mut bool) {
    for character in text.chars() {
        match character {
            ' ' if *folds => document.push_str("&nbsp;"),
            ' ' => document.push(' '),
            '\t' => document.push_str("&nbsp;&nbsp;&nbsp;&nbsp;"),
            other => super::escape_into(document, other.encode_utf8(&mut [0; 4])),
        }
        *folds = matches!(character, ' ' | '\t');
    }
}

/// The characters that turn the order of text around.
fn turns_text_around(character: char) -> bool {
    matches!(
        character,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
#[path = "typed_tests.rs"]
mod tests;
