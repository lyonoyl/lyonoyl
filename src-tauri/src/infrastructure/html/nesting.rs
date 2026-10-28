//! Reading the bytes of a document before a parser is handed them.
//!
//! Everything here works on bytes and never builds a tree: the tree is the expensive thing, and the
//! question asked here is whether building one is worth what it costs. Building it costs the square
//! of how deep it is, so the depth is counted first, by walking the text once — and every walk here
//! costs the length of the document and nothing more, whatever shape it takes, because a count
//! that could itself be made slow would be the thing it guards against.

/// How deeply tags may nest before the body is shown as text instead of drawn.
///
/// 512, because that is as deep as the engines this application draws in build a document:
/// WebKit, which is WKWebView and WebKitGTK, stops at `maximumHTMLParserDOMTreeDepth`, and
/// Chromium, which is WebView2, at `kMaximumHTMLParserDOMTreeDepth`, both 512, and each attaches
/// whatever is deeper beside the element at that depth. A deeper document shows nothing more on
/// screen, so drawing one buys nothing. Mail comes nowhere near it: the corpus's reply quoting
/// thirty-six earlier messages, Apple's and Gmail's quoting in turn, nests 123 deep, and one quoting
/// a hundred nests about 350.
///
/// What it guards against is measured by `how_deep_the_whole_path_goes`, which renders documents
/// through the guard exactly as a message is rendered — its own thread with `STACK`, the time
/// limit, the catch for a panic — through parsing, the cleaning with this build's callbacks,
/// serialising and the drop of every tree, each depth in a process of its own. The stack never runs
/// out: in a release build the deepest document that fits in `LARGEST_BODY`, 419,429 `<div>` never
/// closed, is refused by the time limit and then, run to its end as a production thread is,
/// finishes after 336 seconds with the stack intact, and no depth the time limit lets through
/// overflows it in a debug build. What gives way is time, because building the tree costs the
/// square of its depth. In a release build 50,000 levels of `<div>` take four seconds and 100,000 are refused; in
/// a debug build the first refusal is 12,000 levels of quoted replies. So 512 sits twenty-three
/// times below the smallest failure measured anywhere, and nearly two hundred times below it in the
/// build a person runs.
///
/// Depth is only one of the three shapes in which a small document costs a large tree. The other
/// two are the number of elements, bounded by `MOST_ELEMENTS`, and the number of formatting
/// elements held open at once, bounded by `MOST_FORMATTING` — which the same scan below counts,
/// because depth alone does not see the tree a parser rebuilds out of orphaned `<b>` and `<font>`.
pub(super) const DEEPEST_NESTING: usize = 512;

/// How many formatting elements a message may hold open at once before it is shown as text instead.
///
/// A parser keeps a second list of the formatting elements that are open — `<a>`, `<b>`, `<font>`,
/// `<i>` and the rest — and, once a block above them is closed while they are still open, rebuilds
/// every one of them again before each later run of text. A message that leaves N of them open and
/// then writes M blocks under the one that orphaned them costs N times M elements in the tree out
/// of N plus M in the source: `DEEPEST_NESTING` does not see it, because the depth stays low, and
/// `MOST_ELEMENTS` does not see it, because it counts the source rather than the tree. Bounding how
/// many may be open at once bounds that product — with at most `MOST_ELEMENTS` blocks the tree it
/// rebuilds holds at most this many times that — and the count runs before any tree is built, so
/// the tree the amplification would grow is never allocated.
///
/// 64 is far above real mail, which nests formatting a handful deep at the most and closes what it
/// opens; a message that holds scores of them open is doing it to be counted here rather than read.
/// The names are matched without regard to case, because `<B>` is `<b>` to a parser and would
/// amplify the same way.
const MOST_FORMATTING: usize = 64;

/// How many elements a message may have before it is shown as text instead.
///
/// Not about bytes and not about depth, both of which are already bounded: it is about what the
/// document costs to lay out, which is what the number of elements decides. It bounds the
/// stylesheet and the `style=` attribute alike, because the cost is in the elements rather than in
/// where their declarations were written.
///
/// 8,000 is far above every real message: the ordinary messages in the corpus run from 5 to 552
/// elements, the largest a reply quoting thirty-six others, and the one written to be hostile about
/// depth has 5,002. A message is laid out inside a frame of its own, on a thread the window does not
/// answer on, and the engine probe — which hands the main thread a piece of work every fifty
/// milliseconds and records how long the longest one waited — loses nothing at either 8,000 or
/// 46,000 elements: 115 ms and 113 ms on WKWebView with a stylesheet written to be expensive,
/// 21 ms on WebKitGTK, against 2,960 ms when the main thread is blocked for three seconds on
/// purpose, so the instrument is not blind. The bound stays low all the same, because it is cheap
/// and one engine on one machine is not three.
pub(crate) const MOST_ELEMENTS: usize = 8_000;

const NEVER_NEST: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Elements that a second one of the same closes, the way a parser does it.
///
/// Written out because the scan below keeps its own stack: a message full of `<p>` that never
/// closes one is ordinary mail, and counting each of them as another level deep would show it as
/// text for no reason.
const CLOSES_ITS_TWIN: [&str; 8] = ["p", "li", "td", "th", "tr", "option", "dt", "dd"];

/// The formatting elements — the ones a parser keeps a second list of and rebuilds after they are
/// orphaned. Counted so that `MOST_FORMATTING` can bound the tree that rebuilding grows.
const FORMATTING: [&str; 14] = [
    "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u",
];

/// Whether a tag name is a formatting element, without regard to case: `<B>` is `<b>` to a parser.
fn is_formatting(name: &[u8]) -> bool {
    FORMATTING
        .iter()
        .any(|known| name.eq_ignore_ascii_case(known.as_bytes()))
}

/// Whether tags nest deeper than a document plausibly does.
///
/// A refusal rather than an analysis, and deliberately shaped so that its mistakes are the safe
/// ones: a closing tag that matches nothing changes nothing — a parser drops it, and a scan that
/// counted it down would let `<div></b>` repeated ten thousand times walk the counter between one
/// and zero for ever while the tree really did get deeper. Over-counting only costs a message its
/// styling; under-counting costs the process.
/// Whether a document opens more elements than this build lays out.
///
/// The same scan as the one below, asked a different question, and deliberately not folded into
/// it: one answers "is this a tree" and the other "is this a page somebody wrote", and a message
/// can fail either without failing the other.
pub(super) fn elements_beyond(html: &str, most: usize) -> bool {
    let bytes = html.as_bytes();
    let mut counted = 0;
    let mut at = 0;

    while at < bytes.len() {
        if bytes[at] != b'<' {
            at += 1;
            continue;
        }

        // A name is what follows `<`, and a closing tag or a comment is not one being opened.
        let name = bytes[at + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_alphanumeric())
            .count();

        if name > 0 {
            counted += 1;
            if counted > most {
                return true;
            }
        }

        at += 1 + name.max(1);
    }

    false
}

pub(super) fn nesting_beyond(html: &str, deepest: usize) -> bool {
    let bytes = html.as_bytes();
    let mut open: Vec<&[u8]> = Vec::new();
    // How many of the elements on the stack are formatting ones, kept in step with the stack so it
    // never has to be walked. Over-counted where a message misnests its tags, which only ever costs
    // that message its styling.
    let mut formatting = 0usize;
    let mut names = Names::new(bytes);
    let mut at = 0;

    while at < bytes.len() {
        if bytes[at] != b'<' {
            at += 1;
            continue;
        }

        // Inside one of these, what looks like markup is text: `<!--` in a `<textarea>` is four
        // characters somebody typed, and `<div>` there opens nothing. Skipped whole, so neither is
        // mistaken for the other. A tag nobody closed means the scan cannot say what follows, and
        // the answer to that is the same as everywhere else here — refuse.
        if let Some(name) = raw_text_opening(bytes, at, &mut names) {
            // Everything after `<plaintext>` is text to the end of the document, so nothing more
            // can nest and the count that has been reached is the answer.
            if name == "plaintext" {
                return false;
            }

            let after = at + 1 + name.len();
            let mut closing = Vec::with_capacity(name.len() + 2);
            closing.extend_from_slice(b"</");
            closing.extend_from_slice(name.as_bytes());

            match find_ignoring_case(&bytes[after..], &closing) {
                Some(end) => {
                    at = after + end + closing.len();
                    continue;
                }
                None => return true,
            }
        }

        // What is inside a comment is not markup. Both endings are looked for: a parser ends a
        // comment at `--!>` as well, and a scan that only knew about `-->` would read everything
        // after one as a comment and count nothing at all.
        if bytes[at..].starts_with(b"<!--") {
            let rest = &bytes[at + 4..];

            // `<!-->` and `<!--->`: the parser ends the comment there and then. A scan that went
            // looking for `-->` would read the whole rest of the document as commented out and
            // count nothing — five bytes in front of a body and this guard never runs.
            if rest.first() == Some(&b'>') {
                at += 5;
                continue;
            }
            if rest.starts_with(b"->") {
                at += 6;
                continue;
            }

            match comment_ends(rest) {
                Some(end) => at += 4 + end,
                // A comment nobody closed swallows the rest of the document, for a parser too —
                // which means this scan cannot see what is in there. It refuses rather than
                // guesses: a message counted as too deep is shown as text, and a message waved
                // through on a guess is the process.
                None => return true,
            }
            continue;
        }

        match bytes.get(at + 1) {
            Some(b'/') => {
                let name = names.at(at + 2);
                // Only when it closes what is actually open. Anything else is a tag a parser
                // throws away, and throwing it away here is what keeps the two in step.
                if open.last() == Some(&name) {
                    open.pop();
                    if is_formatting(name) {
                        formatting -= 1;
                    }
                }
            }
            Some(next) if next.is_ascii_alphabetic() => {
                let name = names.at(at + 1);

                if !NEVER_NEST.iter().any(|never| never.as_bytes() == name) {
                    if open.last() == Some(&name)
                        && CLOSES_ITS_TWIN.iter().any(|twin| twin.as_bytes() == name)
                    {
                        open.pop();
                        // A twin is never a formatting element, so this never decrements; kept for
                        // the two counts to move together whatever the lists come to hold.
                        if is_formatting(name) {
                            formatting -= 1;
                        }
                    }

                    open.push(name);
                    if is_formatting(name) {
                        formatting += 1;
                    }
                    if open.len() > deepest || formatting > MOST_FORMATTING {
                        return true;
                    }
                }
            }
            _ => {}
        }
        at += 1;
    }

    false
}

/// Where a comment that has just opened ends: one past the first `-->` or `--!>`.
///
/// One pass for both endings. Looking for each separately and taking the nearer searches the rest
/// of the document for the one that is not there, once per comment: two megabytes of empty comments
/// then cost hours rather than milliseconds, in the scan whose whole job is to be cheap.
pub(super) fn comment_ends(rest: &[u8]) -> Option<usize> {
    (0..rest.len()).find_map(|at| match &rest[at..] {
        [b'-', b'-', b'>', ..] => Some(at + 3),
        [b'-', b'-', b'!', b'>', ..] => Some(at + 4),
        _ => None,
    })
}

/// The names of tags, read from one document front to back.
///
/// A name ends at whitespace, a slash or `>`, and a document can go a very long way without any of
/// them: `<a<a<a…` is one name after another, each running to the end. Reading each to its end
/// afresh costs the length of the document per `<`. So where the last name ended is kept, and a
/// name that starts before that point ends there too — nothing between the two could have ended
/// it. Names are asked for in the order they appear, which is what makes that true.
struct Names<'a> {
    bytes: &'a [u8],
    /// Where the last search for an end began, and where it found one — or the end of the
    /// document when it found none.
    searched: Option<(usize, usize)>,
}

impl<'a> Names<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            searched: None,
        }
    }

    /// The name that starts at `start`, as bytes. Compared rather than read, so it is never
    /// checked for being text — checking would cost its length, which is the cost this avoids.
    fn at(&mut self, start: usize) -> &'a [u8] {
        let start = start.min(self.bytes.len());

        let ends_at = match self.searched {
            Some((from, ends_at)) if from <= start && start <= ends_at => ends_at,
            _ => {
                let ends_at = self.bytes[start..]
                    .iter()
                    .position(|byte| byte.is_ascii_whitespace() || *byte == b'>' || *byte == b'/')
                    .map_or(self.bytes.len(), |end| start + end);
                self.searched = Some((start, ends_at));
                ends_at
            }
        };

        &self.bytes[start..ends_at]
    }
}

/// The elements whose content a parser reads as text rather than as markup.
///
/// RAWTEXT and RCDATA in the HTML specification, plus `plaintext`, which never ends. What matters
/// to a scan counting depth is that nothing inside them opens anything, and that `<!--` inside them
/// starts no comment.
const READS_AS_TEXT: [&str; 10] = [
    "script",
    "style",
    "textarea",
    "title",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "noscript",
    "plaintext",
];

/// The name of a raw-text element opening at `at`, if one is.
///
/// Case-insensitive, because `<TEXTAREA>` is the same element and a scan that only knew the
/// lowercase spelling would read what is inside it as markup — which is the whole failure this
/// exists to prevent.
fn raw_text_opening(bytes: &[u8], at: usize, names: &mut Names<'_>) -> Option<&'static str> {
    if !bytes.get(at + 1).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let name = names.at(at + 1);

    READS_AS_TEXT
        .iter()
        .find(|known| name.eq_ignore_ascii_case(known.as_bytes()))
        .copied()
}

/// `find`, ignoring the case of ASCII letters. A closing tag may be spelled either way.
fn find_ignoring_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

/// The longest prefix of `text` that is at most `bytes` long and ends where a character does.
pub(super) fn cut_at_character(text: &str, bytes: usize) -> &str {
    let mut at = bytes.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }

    &text[..at]
}

#[cfg(test)]
mod the_guard_before_the_parser {
    use super::super::*;

    /// A document deep enough to matter, behind whatever prefix is being tried.
    fn deep(prefix: &str) -> String {
        format!("{prefix}{}", "<div>".repeat(DEEPEST_NESTING + 50))
    }

    #[test]
    fn a_plain_document_is_counted_as_it_reads() {
        assert!(!nesting_beyond("<p>hello</p>", DEEPEST_NESTING));
        assert!(nesting_beyond(&deep(""), DEEPEST_NESTING));
    }

    #[test]
    fn a_comment_that_ends_at_once_does_not_hide_what_follows() {
        // `<!-->` and `<!--->` end the comment there and then. Reading them as the start of a
        // comment nobody closed meant the rest of the document counted as nothing: five bytes in
        // front of a body and the guard never ran.
        assert!(nesting_beyond(&deep("<!-->"), DEEPEST_NESTING));
        assert!(nesting_beyond(&deep("<!--->"), DEEPEST_NESTING));
    }

    #[test]
    fn a_comment_nobody_closed_is_refused_rather_than_waved_through() {
        // The scan cannot say what is inside it, and the answer to that is refusal. It costs a
        // message its styling — it is shown as text — where the other answer costs the process.
        assert!(nesting_beyond("<!-- and nothing after it", DEEPEST_NESTING));
        assert!(nesting_beyond(
            &format!("<!--{}", deep("")),
            DEEPEST_NESTING
        ));
    }

    #[test]
    fn a_comment_that_is_only_text_inside_an_element_is_not_a_comment() {
        // `<!--` in a `<textarea>` is four characters somebody typed. Treated as a comment, it
        // swallowed everything after it and the guard counted nothing.
        assert!(nesting_beyond(
            &format!(
                "<textarea><!--</textarea>{}",
                "<div>".repeat(DEEPEST_NESTING + 50)
            ),
            DEEPEST_NESTING
        ));
        // The same element spelled the other way. A scan that only knew one spelling read what was
        // inside the other as markup.
        assert!(nesting_beyond(
            &format!(
                "<TEXTAREA><!--</TEXTAREA>{}",
                "<div>".repeat(DEEPEST_NESTING + 50)
            ),
            DEEPEST_NESTING
        ));
        assert!(nesting_beyond(
            &format!(
                "<title><!--</title>{}",
                "<div>".repeat(DEEPEST_NESTING + 50)
            ),
            DEEPEST_NESTING
        ));
    }

    #[test]
    fn what_is_inside_a_raw_text_element_opens_nothing() {
        // The other direction: a thousand `<div>` inside a `<textarea>` are a thousand characters,
        // not a thousand elements, and a message that types them is shown as it was written.
        let typed = format!("<textarea>{}</textarea><p>after</p>", "<div>".repeat(1000));
        assert!(!nesting_beyond(&typed, DEEPEST_NESTING));
    }

    #[test]
    fn a_raw_text_element_nobody_closed_is_refused_too() {
        assert!(nesting_beyond(
            &format!("<textarea>{}", deep("")),
            DEEPEST_NESTING
        ));
    }

    #[test]
    fn nothing_nests_after_plaintext_because_nothing_is_markup_after_it() {
        // Everything after it is text to the end of the document, for a parser as well, so the
        // count that has been reached is the answer rather than a refusal.
        assert!(!nesting_beyond(&deep("<plaintext>"), DEEPEST_NESTING));
    }

    #[test]
    fn a_message_the_guard_refuses_is_shown_as_text_rather_than_as_nothing() {
        // The product contract for a body that cannot be rendered: it is still readable.
        let hostile = format!("<!-->{}deep", "<div>".repeat(DEEPEST_NESTING + 50));
        let shown = from_html(
            &hostile,
            None,
            Vec::new(),
            Default::default(),
            Pictures::Blocked,
            Vec::new(),
            Vec::new(),
        );

        assert_eq!(shown.kind, BodyKind::PlainText);
        assert!(shown.html.contains("deep"), "{}", shown.html);
    }
}

#[cfg(test)]
mod head_is_not_body {
    use super::super::*;

    fn shown(html: &str) -> String {
        from_html(
            html,
            None,
            Vec::new(),
            Default::default(),
            Pictures::Blocked,
            Vec::new(),
            Vec::new(),
        )
        .html
    }

    #[test]
    fn a_subject_a_template_forgot_to_fill_in_is_not_the_first_line_of_the_message() {
        // What a real provider sent. Every message template puts the subject in the head, and one
        // that failed to substitute it put `${subject}` where the message starts.
        let shown = shown(
            "<!doctype html><html><head><title>${subject}</title></head>\
             <body><p>Hello</p></body></html>",
        );

        assert!(
            !shown.contains("${subject}"),
            "the head of the message was read out as its first line: {shown}"
        );
        assert!(shown.contains("Hello"));
    }

    #[test]
    fn what_the_head_says_about_the_page_is_not_read_out_either() {
        let shown = shown(
            "<html><head><style>p{color:red}</style><title>Newsletter</title>\
             <meta name=\"description\" content=\"never shown\"></head><body>Body</body></html>",
        );

        assert!(
            !shown.contains("Newsletter"),
            "the title became text: {shown}"
        );
        // The stylesheet is kept, and kept as a stylesheet: what it must never become is words
        // somebody reads. That was the whole worry here, and it is still the worry now that the
        // block itself survives.
        let words = shown.replace(
            &shown[shown.find("<style").unwrap_or(0)
                ..shown.find("</style>").map_or(0, |at| at + "</style>".len())],
            "",
        );
        assert!(
            !words.contains("color:red"),
            "the stylesheet became text: {shown}"
        );
        assert!(
            shown.contains("<style>p{color:red}</style>"),
            "the stylesheet was thrown away: {shown}"
        );
        assert!(shown.contains("Body"));
    }

    #[test]
    fn the_text_of_an_ordinary_wrapper_is_still_kept() {
        // The rule is about elements whose text nobody wrote to be read. A tag that merely is not
        // on the allowlist keeps its words, and losing those would lose the message.
        let shown = shown("<section><article>What it says</article></section>");

        assert!(shown.contains("What it says"), "{shown}");
    }
}
