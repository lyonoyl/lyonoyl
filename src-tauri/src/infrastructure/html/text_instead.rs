//! Text in the place of markup this build will not draw.
//!
//! Everything here runs exactly when a document is past what a parser may be handed, so nothing
//! here is a parser: it reads the bytes once, front to back, and never builds a tree. What it hands
//! back is text, which `from_text` escapes before anybody sees it — a mistake in here costs a word
//! its place on the page and can never turn text into markup.

use mail_parser::{Message, MimeHeaders, PartType};

use super::nesting::{comment_ends, cut_at_character};

/// The text part the sender wrote beside this HTML one.
///
/// A text part directly inside the nearest `multipart/alternative` that holds the HTML part: what
/// the sender wrote for a client that shows no HTML. A text part anywhere else in the message — a
/// list's footer, a note attached as text — is not this body said another way, and showing it in
/// the body's place would lose the body.
pub(super) fn written_beside<'a>(parsed: &'a Message<'a>, html_at: u32) -> Option<&'a str> {
    // Which part holds which, worked out once. Searching down from every multipart instead costs
    // the square of how deeply a message nests them, and that depth is the sender's choice.
    let mut holder: Vec<Option<u32>> = vec![None; parsed.parts.len()];
    for (at, part) in parsed.parts.iter().enumerate() {
        for &inside in part.sub_parts().unwrap_or_default() {
            if let Some(slot) = holder.get_mut(inside as usize) {
                *slot = Some(at as u32);
            }
        }
    }

    // One step up per part at most: what the parser builds is a tree, and the bound means that even
    // a message that was not one could cost no more than a walk over its parts.
    let mut at = html_at;
    for _ in 0..parsed.parts.len() {
        let above = holder.get(at as usize).copied().flatten()?;
        let part = parsed.part(above)?;

        if part.content_type().is_some_and(|kind| {
            kind.ctype().eq_ignore_ascii_case("multipart")
                && kind
                    .subtype()
                    .is_some_and(|subtype| subtype.eq_ignore_ascii_case("alternative"))
        }) {
            return part.sub_parts()?.iter().find_map(|&branch| {
                let branch = parsed.part(branch)?;
                let PartType::Text(text) = &branch.body else {
                    return None;
                };

                // Only the plain-text alternative. `mail-parser` also files `text/x-amp-html`,
                // `text/calendar`, `text/enriched` and `text/watch-html` as text, and showing one of
                // those in the body's place would put its own markup, or a raw calendar, on the page
                // — which is the thing this fallback exists to keep off it. A part with no type of
                // its own is plain text, which is what a bare `text/` defaults to.
                let plain = branch.content_type().is_none_or(|kind| {
                    kind.subtype()
                        .is_none_or(|subtype| subtype.eq_ignore_ascii_case("plain"))
                });

                plain.then(|| text.as_ref())
            });
        }

        at = above;
    }

    None
}

/// Elements whose content a parser reads as text rather than as markup, dropped with it.
///
/// Inside them `<div>` is five characters and `<!--` opens nothing, so each is skipped whole, to
/// its own end tag; one nobody closed takes the rest of the document with it, as it does for a
/// parser. What they hold is a script, a stylesheet, a title, a form field's contents, or what to
/// show where a frame or a plug-in cannot run — nothing written to be read as the message.
/// `plaintext` is not here: everything after it is text, and it is kept.
const NOT_WORDS: [&str; 9] = [
    "script", "style", "title", "textarea", "xmp", "iframe", "noembed", "noframes", "noscript",
];

/// Elements whose content is markup nobody is shown, dropped with it where they are closed.
///
/// A head, a template, a drawing and a formula: the drawn path removes the last two with
/// everything in them, and the first two are never on screen. Where one is not closed its content
/// is read like any other, because a parser that finds body content in an open head closes the head
/// there.
const NOT_SHOWN: [&str; 4] = ["head", "template", "svg", "math"];

/// Elements that begin and end a paragraph: a blank line either side.
const PARAGRAPHS: [&str; 15] = [
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "pre",
    "table",
    "ul",
    "ol",
    "dl",
    "hr",
    "figure",
];

/// Elements that begin and end a line.
const LINES: [&str; 22] = [
    "div",
    "li",
    "tr",
    "dt",
    "dd",
    "address",
    "article",
    "aside",
    "caption",
    "center",
    "details",
    "dialog",
    "fieldset",
    "figcaption",
    "footer",
    "form",
    "header",
    "main",
    "nav",
    "section",
    "summary",
    "legend",
];

/// Elements whose content sits beside the one before it rather than under it.
const CELLS: [&str; 2] = ["td", "th"];

/// The words of a document, in lines, and whether they were cut at `most` bytes.
///
/// Tags and comments are dropped, and so is the content of the elements named above. Character
/// references are decoded. Runs of whitespace fold to one space, as a browser folds them, except
/// inside `<pre>`; a `<br>` is a line break, a block is a line of its own, a paragraph has a blank
/// line either side, and there is never more than one blank line in a row.
///
/// One pass, and every search it makes either consumes what it searched or ends the pass: a
/// comment, a tag or a raw-text element nobody closed takes the rest of the document with it, as
/// it does for a parser. The one search that can fail without ending anything — for the end of one
/// of `NOT_SHOWN` — is remembered once it has failed, so a thousand of them cost one scan.
pub(super) fn words_of_markup(html: &str, most: usize) -> (String, bool) {
    let bytes = html.as_bytes();
    let mut words = Words::new(most);
    // Which of `NOT_SHOWN` have been looked for to the end of the document and not found.
    let mut never_closed = [false; NOT_SHOWN.len()];
    let mut at = 0;

    while at < bytes.len() && !words.full() {
        match bytes[at] {
            b'<' => match tag_at(bytes, at) {
                Tag::Text => {
                    words.word("<");
                    at += 1;
                }
                Tag::Skipped(after) => at = after,
                Tag::Unfinished => break,
                Tag::Closing { name, after } => {
                    words.ends(&name);
                    at = after;
                }
                Tag::Opening { name, after } => {
                    at = after;

                    if name == "plaintext" {
                        words.plain(&html[at..]);
                        break;
                    }

                    if NOT_WORDS.contains(&name.as_str()) {
                        match end_tag(bytes, at, &name) {
                            Some(after) => at = after,
                            None => break,
                        }
                        continue;
                    }

                    if let Some(which) = NOT_SHOWN.iter().position(|known| *known == name) {
                        if !never_closed[which] {
                            match end_tag(bytes, at, &name) {
                                Some(after) => {
                                    at = after;
                                    continue;
                                }
                                None => never_closed[which] = true,
                            }
                        }
                    }

                    words.begins(&name);
                }
            },
            b'&' => at = words.reference(html, at),
            byte if is_space(byte) => {
                words.space(byte);
                at += 1;
            }
            _ => {
                // A run of ordinary characters, ended by the next byte that means something. Each
                // of those is ASCII, so the run ends where a character does.
                let run = bytes[at..]
                    .iter()
                    .position(|&byte| byte == b'<' || byte == b'&' || is_space(byte))
                    .map_or(bytes.len(), |length| at + length);
                words.word(&html[at..run]);
                at = run;
            }
        }
    }

    words.finish()
}

/// What starts at a `<`.
enum Tag {
    /// Not markup: a `<` somebody typed.
    Text,
    /// A comment, a doctype, or anything else that says nothing, ending at this byte.
    Skipped(usize),
    /// Markup that never ends, which takes the rest of the document with it.
    Unfinished,
    Opening {
        name: String,
        after: usize,
    },
    Closing {
        name: String,
        after: usize,
    },
}

fn tag_at(bytes: &[u8], at: usize) -> Tag {
    let rest = &bytes[at..];

    if rest.starts_with(b"<!--") {
        // `<!-->` and `<!--->` end where they begin, for a parser as well.
        if rest[4..].starts_with(b">") {
            return Tag::Skipped(at + 5);
        }
        if rest[4..].starts_with(b"->") {
            return Tag::Skipped(at + 6);
        }

        return match comment_ends(&rest[4..]) {
            Some(end) => Tag::Skipped(at + 4 + end),
            None => Tag::Unfinished,
        };
    }

    match rest.get(1) {
        // A doctype, a `<![CDATA[`, a `<?xml`: nothing a person reads, ending at the first `>`.
        Some(b'!' | b'?') => past(rest, 2).map_or(Tag::Unfinished, |end| Tag::Skipped(at + end)),
        Some(b'/') => match rest.get(2) {
            Some(letter) if letter.is_ascii_alphabetic() => {
                let name = name_at(&rest[2..]);
                match end_of_tag(rest, 2 + name.len()) {
                    Some(end) => Tag::Closing {
                        name,
                        after: at + end,
                    },
                    None => Tag::Unfinished,
                }
            }
            // `</>` is dropped by a parser without a trace.
            Some(b'>') => Tag::Skipped(at + 3),
            Some(_) => past(rest, 2).map_or(Tag::Unfinished, |end| Tag::Skipped(at + end)),
            None => Tag::Text,
        },
        Some(letter) if letter.is_ascii_alphabetic() => {
            let name = name_at(&rest[1..]);
            match end_of_tag(rest, 1 + name.len()) {
                Some(end) => Tag::Opening {
                    name,
                    after: at + end,
                },
                None => Tag::Unfinished,
            }
        }
        _ => Tag::Text,
    }
}

/// A tag's name, lowercased: what follows `<` or `</` up to whitespace, `/` or `>`.
fn name_at(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|&&byte| !is_space(byte) && byte != b'/' && byte != b'>')
        .map(|byte| byte.to_ascii_lowercase() as char)
        .collect()
}

/// Where a tag ends — one past its `>` — reading its attributes the way a parser does.
///
/// A quote opens a value only after `=`, and a `>` inside a quoted value ends nothing: without that,
/// `<a title="1 > 0">` would put the rest of its own attributes on the page.
fn end_of_tag(bytes: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    let mut after_equals = false;

    while at < bytes.len() {
        match bytes[at] {
            b'>' => return Some(at + 1),
            b'=' => after_equals = true,
            quote @ (b'"' | b'\'') if after_equals => {
                let closing = bytes[at + 1..].iter().position(|&byte| byte == quote)?;
                at += 1 + closing;
                after_equals = false;
            }
            byte if is_space(byte) => {}
            _ => after_equals = false,
        }
        at += 1;
    }

    None
}

/// One past the first `>` at or after `from`.
fn past(bytes: &[u8], from: usize) -> Option<usize> {
    bytes[from..]
        .iter()
        .position(|&byte| byte == b'>')
        .map(|end| from + end + 1)
}

/// One past the end tag of `name`, searching from `from`, in any case.
///
/// Only a whole name ends it: `</scripts>` is text inside a script, as it is for a parser.
fn end_tag(bytes: &[u8], from: usize, name: &str) -> Option<usize> {
    let name = name.as_bytes();
    let mut at = from;

    while let Some(found) = find(&bytes[at..], b"</") {
        let start = at + found + 2;
        let named = bytes
            .get(start..start + name.len())
            .is_some_and(|written| written.eq_ignore_ascii_case(name));
        let whole = bytes
            .get(start + name.len())
            .is_none_or(|&byte| is_space(byte) || byte == b'/' || byte == b'>');

        if named && whole {
            return end_of_tag(bytes, start + name.len());
        }
        at = start;
    }

    None
}

/// Where `needle` starts in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Whitespace as HTML counts it.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

/// The text being written, and what it needs to remember to write the next piece.
struct Words {
    written: String,
    most: usize,
    cut: bool,
    /// How many line breaks the text ends in, so a block after a block does not add another.
    breaks: usize,
    /// Whitespace has been read since the last word and not yet written.
    space: bool,
    /// How many `<pre>` are open: inside one, whitespace is kept as it was written.
    pre: usize,
}

impl Words {
    fn new(most: usize) -> Self {
        Self {
            written: String::new(),
            most,
            cut: false,
            breaks: 0,
            space: false,
            pre: 0,
        }
    }

    fn full(&self) -> bool {
        self.cut
    }

    /// Adds text as it is, stopping at the limit and at the end of a character.
    fn push(&mut self, text: &str) {
        let room = self.most.saturating_sub(self.written.len());

        if text.len() > room {
            self.written.push_str(cut_at_character(text, room));
            self.cut = true;
        } else {
            self.written.push_str(text);
        }
    }

    fn word(&mut self, word: &str) {
        if self.space && self.breaks == 0 && !self.written.is_empty() {
            self.push(" ");
        }
        self.space = false;
        self.breaks = 0;
        self.push(word);
    }

    fn space(&mut self, byte: u8) {
        if self.pre == 0 {
            self.space = true;
            return;
        }

        match byte {
            b'\n' => self.line_break(),
            b' ' | b'\t' => {
                self.breaks = 0;
                self.push(if byte == b' ' { " " } else { "\t" });
            }
            // A carriage return belongs to the line feed beside it, and a form feed draws nothing.
            _ => {}
        }
    }

    /// One `<br>`, or one line feed inside `<pre>`: a line ends, and at most one blank line runs.
    fn line_break(&mut self) {
        self.space = false;
        if self.written.is_empty() || self.breaks >= 2 {
            return;
        }
        self.breaks += 1;
        self.push("\n");
    }

    /// At least `wanted` line breaks at the end of the text, and never more than that.
    fn at_least(&mut self, wanted: usize) {
        self.space = false;
        if self.written.is_empty() {
            return;
        }
        while self.breaks < wanted {
            self.breaks += 1;
            self.push("\n");
        }
    }

    fn begins(&mut self, name: &str) {
        match name {
            "br" => self.line_break(),
            "pre" => {
                self.at_least(2);
                self.pre += 1;
            }
            _ => self.boundary(name),
        }
    }

    fn ends(&mut self, name: &str) {
        match name {
            // `</br>` is a line break to a parser too.
            "br" => self.line_break(),
            "pre" => {
                self.pre = self.pre.saturating_sub(1);
                self.at_least(2);
            }
            _ => self.boundary(name),
        }
    }

    fn boundary(&mut self, name: &str) {
        if PARAGRAPHS.contains(&name) {
            self.at_least(2);
        } else if LINES.contains(&name) {
            self.at_least(1);
        } else if CELLS.contains(&name) {
            self.space = true;
        }
    }

    /// Everything after `<plaintext>`: text to the end, with nothing in it read as markup.
    fn plain(&mut self, text: &str) {
        self.at_least(1);
        self.breaks = 0;
        self.push(text);
    }

    /// A character reference at `at`, written as the character it names. Returns where reading
    /// goes on.
    ///
    /// Only the terminated form: `&amp;`, `&#38;`, `&#x26;`. A name no longer than the longest
    /// one HTML defines is looked for, so an `&` in running text costs a few bytes of looking, and
    /// one that names nothing is written as it stands — which is what a browser shows for it.
    fn reference(&mut self, html: &str, at: usize) -> usize {
        const LONGEST: usize = 32;

        let bytes = html.as_bytes();
        let name = bytes[at + 1..]
            .iter()
            .take(LONGEST)
            .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'#')
            .count();
        let end = at + 1 + name;

        if name == 0 || bytes.get(end) != Some(&b';') {
            self.word("&");
            return at + 1;
        }

        let reference = &html[at..=end];
        match numbered(&reference[1..reference.len() - 1]) {
            Numbered::Names(character) => self.word(character.encode_utf8(&mut [0; 4])),
            Numbered::Nothing => self.word("\u{fffd}"),
            // `&#;` and `&#12a;` are not references, and a browser shows them as written.
            Numbered::NotANumber => {
                self.word("&");
                return at + 1;
            }
            Numbered::NotNumeric => {
                let mut named = String::new();
                mail_parser::decoders::html::add_html_token(
                    &mut named,
                    reference.as_bytes(),
                    false,
                );
                self.word(&named);
            }
        }

        end + 1
    }

    /// The text, without the whitespace it ends in, and whether it was cut.
    fn finish(mut self) -> (String, bool) {
        let kept = self.written.trim_end().len();
        self.written.truncate(kept);
        (self.written, self.cut)
    }
}

/// What the inside of a reference, between `&` and `;`, says as a number.
enum Numbered {
    /// A character.
    Names(char),
    /// A number naming nothing a document may hold: zero, a surrogate, or past the last character.
    /// A browser shows the replacement character for it.
    Nothing,
    /// A `#` without the digits that would make it a number.
    NotANumber,
    /// A name rather than a number.
    NotNumeric,
}

fn numbered(inside: &str) -> Numbered {
    let Some(digits) = inside.strip_prefix('#') else {
        return Numbered::NotNumeric;
    };
    let (digits, radix) = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => (hex, 16),
        None => (digits, 10),
    };

    if digits.is_empty() || !digits.chars().all(|digit| digit.is_digit(radix)) {
        return Numbered::NotANumber;
    }

    // Too many digits for any number is past the last character too.
    match u32::from_str_radix(digits, radix)
        .ok()
        .filter(|&code| code != 0)
        .and_then(char::from_u32)
    {
        Some(character) => Numbered::Names(character),
        None => Numbered::Nothing,
    }
}

#[cfg(test)]
mod the_words_of_markup_nobody_draws {
    use super::*;

    fn words(html: &str) -> String {
        words_of_markup(html, usize::MAX).0
    }

    #[test]
    fn tags_go_and_the_words_between_them_stay() {
        assert_eq!(
            words("<p>One <b>bold</b> word and <a href=\"https://example.com\">a link</a>.</p>"),
            "One bold word and a link."
        );
    }

    #[test]
    fn a_line_of_its_own_is_a_line_of_its_own() {
        // What Apple Mail writes: a `<div>` per line and a `<div><br></div>` per blank one.
        assert_eq!(
            words(
                "<div>First line</div><div>Second line</div><div><br></div><div>After a gap</div>"
            ),
            "First line\nSecond line\n\nAfter a gap"
        );
        assert_eq!(words("one<br>two<br/>three"), "one\ntwo\nthree");
    }

    #[test]
    fn a_run_of_blank_lines_is_one_blank_line() {
        let many = words(&format!("above{}below", "<br>".repeat(40)));

        assert_eq!(many, "above\n\nbelow");
        assert!(!words("<p>a</p><p></p><p></p><div></div><p>b</p>").contains("\n\n\n"));
    }

    #[test]
    fn whitespace_folds_as_a_browser_folds_it_except_inside_pre() {
        assert_eq!(words("  a \n\t b  "), "a b");
        assert_eq!(words("<pre>a  b\n  c</pre>"), "a  b\n  c");
    }

    #[test]
    fn what_is_inside_a_comment_or_a_doctype_is_not_a_word() {
        assert_eq!(
            words("<!DOCTYPE html><!-- hidden --><!--> <?xml x?>shown <!-- --!>too<!---->two"),
            "shown tootwo"
        );
    }

    #[test]
    fn a_script_a_stylesheet_and_the_head_leave_nothing_behind() {
        let shown = words(
            "<html><head><title>Subject line</title><style>p { color: red }</style>\
             <meta charset=\"utf-8\"></head><body><script>alert('x')</script>\
             <SCRIPT type=\"text/javascript\">var y = '<p>not a tag</p>';</SCRIPT>\
             <style>.x { background: url(https://example.com/a.png) }</style>\
             <p>What was written</p><template><p>never shown</p></template></body></html>",
        );

        assert_eq!(shown, "What was written");
    }

    #[test]
    fn character_references_are_the_characters_they_name() {
        assert_eq!(
            words("Fish &amp; chips &lt;3 &#8212; &#x2014; caf&eacute;&nbsp;bar"),
            "Fish & chips <3 \u{2014} \u{2014} caf\u{e9}\u{a0}bar"
        );
        // Written as they stand where they name nothing, as a browser shows them.
        assert_eq!(words("a & b &nosuch; &#;"), "a & b &nosuch; &#;");
        assert_eq!(words("&#0;&#xD800;&#x110000;"), "\u{fffd}\u{fffd}\u{fffd}");
    }

    #[test]
    fn a_greater_than_sign_inside_a_quoted_attribute_ends_nothing() {
        assert_eq!(
            words("<a title=\"1 > 0\" href='x>y'>link</a> <img alt=\"a > b\">after"),
            "link after"
        );
    }

    #[test]
    fn a_less_than_sign_somebody_typed_is_kept() {
        assert_eq!(words("if a < b and 3<4 then"), "if a < b and 3<4 then");
    }

    #[test]
    fn markup_nobody_finished_takes_the_rest_with_it() {
        assert_eq!(words("kept<!-- never closed <p>lost</p>"), "kept");
        assert_eq!(words("kept<script>lost"), "kept");
        assert_eq!(words("kept<a title=\"never closed>lost"), "kept");
        assert_eq!(words("kept<div"), "kept");
    }

    #[test]
    fn a_head_nobody_closed_does_not_hide_the_message() {
        assert_eq!(words("<head><body>Hello"), "Hello");
    }

    #[test]
    fn a_limit_cuts_the_words_at_a_character_and_says_so() {
        let (short, cut) = words_of_markup("<p>ab\u{449}cd</p>", 3);

        assert!(cut);
        assert_eq!(short, "ab");

        let (whole, cut) = words_of_markup("<p>abcd</p>", 64);
        assert!(!cut);
        assert_eq!(whole, "abcd");
    }

    /// The one property this exists for: it is asked exactly when a parser may not be, so what it
    /// costs must grow with the length of the document and nothing else.
    ///
    /// Every shape below is one that makes a careless scan search the rest of the document once
    /// per tag: a `head` or `template` nobody closes, attributes that never end, a thousand
    /// nested levels, ampersands that name nothing. A scan that is quadratic in any of them takes
    /// minutes over two megabytes; this takes milliseconds, so the bound is generous and still
    /// separates the two by orders of magnitude.
    #[test]
    fn what_it_costs_grows_with_the_length_and_nothing_else() {
        let size = 2 * 1024 * 1024;
        let shapes = [
            "<head>",
            "<template>",
            "<svg>",
            "<div>",
            "<b><i>",
            "&amp",
            "& ",
            "<a b='1' c=\"2\">",
            "</x>",
            "< ",
            "<!---->",
            "<p>word ",
        ];

        for shape in shapes {
            let document = shape.repeat(size / shape.len());
            let began = std::time::Instant::now();
            let (text, _) = words_of_markup(&document, size);
            let took = began.elapsed();

            assert!(
                took < std::time::Duration::from_secs(2),
                "{shape:?} repeated to {size} bytes took {took:?}"
            );
            assert!(!text.contains('<') || shape.starts_with("< "), "{shape:?}");
        }
    }
}
