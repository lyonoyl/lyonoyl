//! Turning a message body into something that can be shown.
//!
//! A message is hostile input, and this is the first of the four layers between it and the person
//! reading it: what leaves here is sanitized HTML, and what makes it safe to show is also the
//! custom scheme it is served on, the response CSP, and the sandboxed frame it is embedded in. A
//! sanitizer is not a boundary — ammonia had three confirmed bypasses in eleven months — so nothing
//! here is allowed to be the only thing standing between a message and the window.
//!
//! What this module never does is hand its result to the interface as a string: the result reaches
//! the frame through its own origin, and the type below stays inside the application.

// Reached from the composition root once a message body can be served. Until then the module is
// exercised by its tests and by the corpus.
#![allow(dead_code)]

mod files;
pub use files::MimeParts;
pub(crate) use files::{files_inside, pictures_as_files, pictures_declined, pictures_inside};
pub(crate) use styling::{filtered_style, plain_value};
pub(crate) use written_in::picture_written_in;

mod guards;
mod links_in_text;
pub use links_in_text::links_in;
pub(crate) mod nesting;
mod styling;
mod text_instead;
mod words;
mod written_in;

// The module is one subject from outside: what a message turns into, whether that is
// something to show or something to search by.
pub use words::{text_of, text_to_index, Read};

use guards::behind_the_guards;
use nesting::{cut_at_character, elements_beyond, nesting_beyond, DEEPEST_NESTING, MOST_ELEMENTS};

pub mod serving;
mod styles_that_draw;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use std::collections::HashMap;
use std::sync::Mutex;

use mail_parser::{MessageParser, PartType};

use crate::infrastructure::html::serving::LINKS;
use crate::ports::body::{
    Attached, BodyForm, BodyKind, BodyRenderer, InlinePart, Link, Pictures, RenderedBody, Viewing,
};

/// The largest message this build will parse.
///
/// A message is a file someone else wrote. Past this, parsing is refused and the reader is told,
/// which is a state the interface can show — unlike the window stopping.
pub const LARGEST_MESSAGE: usize = 32 * 1024 * 1024;

/// Whether a message the index has already measured is past that limit.
///
/// The check inside `body_of` runs once the whole thing is in memory, which is too late to be a
/// defence against size: how large a message is, is chosen by whoever sent it. A pass records what
/// the server said, so the answer is usually known before anything is fetched. Unknown is not
/// refused — a message with no recorded size still meets the limit above, one step later.
pub fn too_large_to_read(size: Option<u64>) -> bool {
    size.is_some_and(|size| size > LARGEST_MESSAGE as u64)
}

/// Whether the sanitized body says anything about colour itself.
///
/// Read from what will actually be shown rather than from the message as it arrived: what was
/// filtered out cannot affect how the result looks, and a message whose only colour was in a
/// property this build refuses is a message with no colour.
///
/// `border-color` is not a colour for this purpose — a border does not decide what surface the
/// text sits on — so `color:` is only counted where a hyphen does not precede it.
fn names_a_colour(html: &str) -> bool {
    if html.contains("background:") || html.contains("background-color:") {
        return true;
    }
    if html.contains("bgcolor=") || html.contains("text=") {
        return true;
    }

    html.match_indices("color:").any(|(at, _)| {
        at == 0
            || !html.as_bytes()[at - 1].is_ascii_alphanumeric() && html.as_bytes()[at - 1] != b'-'
    })
}

/// The largest HTML body this build will sanitize.
///
/// Separate from the message limit: a body is what gets parsed into a tree, and the tree is what
/// costs memory. Past this the text of the message is shown instead.
const LARGEST_BODY: usize = 2 * 1024 * 1024;

/// The sanitizer, as the use cases see it.
pub struct Sanitizer;

impl BodyRenderer for Sanitizer {
    fn render(&self, raw: &[u8], viewing: Viewing) -> RenderedBody {
        body_of(raw, viewing)
    }
}

/// Reads a whole message and gives back what can be shown of its body.
///
/// Runs on a thread of its own, with a stack of its own, a time limit and a catch for a panic: the
/// three ways a parser fed a hostile file takes the process with it. A message that trips any of
/// them shows as nothing, and the reader is told — `panic = "unwind"` is what makes that possible
/// and is why it is kept.
pub fn body_of(raw: &[u8], viewing: Viewing) -> RenderedBody {
    let read = behind_the_guards(
        raw,
        "message-body",
        guards::Asking::SomebodyIsWaiting,
        move |message| rendered_from(message, viewing),
    );

    shown(read)
}

/// What each of the three answers looks like to a reader.
///
/// Apart from `body_of` so that it can be asked without making the machine busy: a test that
/// saturated the places this process shares would make it busy for every test beside it, which is
/// the flakiness this whole change is about.
fn shown(read: guards::Guarded<RenderedBody>) -> RenderedBody {
    match read {
        guards::Guarded::Answered(body) => body,
        guards::Guarded::Refused => RenderedBody::nothing(),
        // Not the same as nothing to show: the machine is busy with other messages, and calling
        // the message empty would be false. It answers for itself, and asking again is worth doing.
        guards::Guarded::Busy => RenderedBody::busy(),
    }
}

fn rendered_from(raw: &[u8], viewing: Viewing) -> RenderedBody {
    // The message itself, read as text. Nothing is parsed to show it — which is the point of
    // offering it at all: it is what to look at when parsing produced nothing worth seeing.
    //
    // On the same thread as everything else even so. It is linear and cannot recurse, so it costs
    // nothing to put here — and a claim that every body goes through one bounded place is either
    // true of all of them or it is not a claim.
    if viewing.form == BodyForm::Source {
        return from_text(&String::from_utf8_lossy(raw), Addresses::AsWritten);
    }

    let Some(parsed) = MessageParser::default().parse(raw) else {
        return RenderedBody::nothing();
    };

    // The part as it was written, never as the parser is willing to convert it. `body_html` turns a
    // text part into HTML and `body_text` turns an HTML part into text — both would put the
    // escaping, or the HTML parsing, in a library whose job is reading mail rather than distrusting
    // it. What is HTML is sanitized here; what is text is escaped here.
    //
    // HTML first: a message that carries both wrote the HTML for a reader and the text for a client
    // that cannot show it.
    // The text alone, when that is what was asked for: a message whose HTML renders as nothing
    // often has a text part that says the same thing.
    if viewing.form == BodyForm::Text {
        return match parsed.text_bodies().find_map(|part| match &part.body {
            PartType::Text(text) => Some(text),
            _ => None,
        }) {
            Some(text) => from_text(text, Addresses::Linked),
            None => RenderedBody::nothing(),
        };
    }

    let pictures = pictures_inside(&parsed);

    for &at in &parsed.html_body {
        if let Some(PartType::Html(html)) = parsed.part(at).map(|part| &part.body) {
            let declined = pictures_declined(&parsed, &pictures);

            return from_html(
                html,
                text_instead::written_beside(&parsed, at),
                pictures,
                declined,
                viewing.pictures,
                files_inside(&parsed),
                pictures_as_files(&parsed),
            );
        }
    }

    // A message that is only text still carries what came with it. Listed here as well as in the
    // markup path, because most messages with an invoice attached say so in one line of text.
    for part in parsed.text_bodies() {
        if let PartType::Text(text) = &part.body {
            return RenderedBody {
                attachments: files_inside(&parsed),
                ..from_text(text, Addresses::Linked)
            };
        }
    }

    // Nothing to read, and possibly something to save: a message whose whole content is a file is
    // a message this application must still hand over.
    RenderedBody {
        attachments: files_inside(&parsed),
        ..RenderedBody::nothing()
    }
}

/// The schemes a link may be offered in.
///
/// What this build is willing to hand to the system when a person says yes. Anything else — a
/// `file:` that would open something on this machine, a scheme some other application registered —
/// is not something a message gets to offer.
const OPENABLE: [&str; 3] = ["http", "https", "mailto"];

/// The address a link is offered and opened as, or nothing when it is not one this build opens.
///
/// Read by the URL standard's parser and written back out by it, and that writing is the address
/// from then on: the host the person is asked about, the address shown under it, and what is
/// handed to the system are one string. As written in a message, an address is two addresses
/// wherever two parsers disagree about it. `https://good.example\@evil.example/` is a path on
/// `good.example` to the standard, which takes a backslash for a slash, and a sign-in at
/// `evil.example` to the parser macOS opens links with — a panel naming the one host, and the
/// other opening. Written out, the backslash is a slash, and there is nothing left for the second
/// reader to read differently. An address the standard cannot read at all is not offered.
pub fn openable(written: &str) -> Option<url::Url> {
    let parsed = url::Url::parse(written).ok()?;

    OPENABLE.contains(&parsed.scheme()).then_some(parsed)
}

/// The largest picture this build will serve from inside a message.
const LARGEST_PICTURE: usize = 8 * 1024 * 1024;

/// The types a picture inside a message may be served as.
///
/// An allowlist. What the message claimed the type is decides nothing else — the bytes are served
/// as the type this list names.
///
/// SVG is in it, and is the one that needs a reason. An SVG *is* a document that can carry script,
/// and this build refused it for that. What that reasoning missed is the only way one can ever be
/// reached here: the sanitizer keeps `src` on `img` and `href` on `a`, and drops `object`,
/// `embed`, `iframe`, `svg` and every other element that could point at one — so a picture is
/// loaded as an image and in no other way. An SVG in an `<img>` is rendered in the restricted mode
/// every engine uses for it: no script runs, no external reference is followed, nothing is
/// interactive. On top of that the part is served under `default-src 'none'; sandbox`, so a
/// request that reached it as a page could still do nothing at all.
///
/// Three things keep that true, and each has a test: the sanitizer's element list, the policy on
/// the part, and the policy on the frame.
const PICTURES: [(&str, &str); 6] = [
    ("png", "image/png"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
    ("svg+xml", "image/svg+xml"),
];

/// The type a picture may be served as, from what it claims to be.
///
/// One list for both places a picture can come from — inside the message and from a host it points
/// at — because the question is the same: what will this build put in front of a reader. What the
/// message or the host claimed decides nothing else.
pub fn picture_served_as(claimed: &str) -> Option<&'static str> {
    let subtype = claimed
        .rsplit('/')
        .next()?
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase();

    PICTURES
        .iter()
        .find(|(named, _)| *named == subtype)
        .map(|(_, served)| *served)
}

/// Sanitized HTML, or text in its place when the markup is past what this build draws.
fn from_html(
    html: &str,
    // The text part the sender wrote beside this one, for a client that shows no HTML.
    beside: Option<&str>,
    pictures: Vec<(String, InlinePart)>,
    // Names the message carries a picture under that this build will not render.
    declined: std::collections::HashSet<String>,
    remote: Pictures,
    attachments: Vec<Attached>,
    // The pictures carried inside the message, as files to save. Shown in place when the body is
    // drawn and listed by nobody; when it is shown as text, nothing draws them, so they are listed
    // here beside whatever else the message carried.
    carried: Vec<Attached>,
) -> RenderedBody {
    let truncated = html.len() > LARGEST_BODY;
    let within = match truncated {
        true => cut_at_character(html, LARGEST_BODY),
        false => html,
    };

    // All three counted before anything is parsed, and each by a scan that builds no tree. Depth is
    // what the parser's work grows with, the number of elements is what the engine's layout grows
    // with, and the number of formatting elements held open is what the tree the parser rebuilds
    // grows with. `DEEPEST_NESTING`, `MOST_ELEMENTS` and `MOST_FORMATTING` say what each bound is
    // and what it was measured against; a document past any of them is shown as text.
    let too_deep = nesting_beyond(within, DEEPEST_NESTING);
    let too_many = elements_beyond(within, MOST_ELEMENTS);

    if too_deep || too_many {
        let mut files = attachments;
        files.extend(carried);
        return RenderedBody {
            attachments: files,
            ..in_place_of_markup(within, truncated, beside, too_deep, too_many)
        };
    }

    drawn(within, truncated, pictures, declined, remote, attachments)
}

/// What a message whose markup is past either bound is shown as.
///
/// Never the markup itself: a reader shown `<div>` and `&lt;` has been shown the source, and the
/// source has a view of its own. First the text part the sender wrote beside the HTML, which is
/// what they wrote for a client that cannot show it; where there is none, the words of the markup,
/// taken out by a scan that builds no tree. Either way it is escaped text, so nothing the message
/// wrote is read as markup here. A message with no words in it at all is said to have nothing to
/// show, which is the sentence that points the reader at its source.
fn in_place_of_markup(
    html: &str,
    cut: bool,
    beside: Option<&str>,
    too_deep: bool,
    too_many: bool,
) -> RenderedBody {
    // A text part with nothing in it is not the message said another way, and showing it would
    // show nothing where the markup has words.
    let beside = beside.filter(|text| !text.trim().is_empty());

    let shown = match beside {
        // The part as it was written. Whether the HTML was cut says nothing about this part, which
        // is whole unless it is past the limit itself.
        Some(text) => from_text(text, Addresses::Linked),
        None => {
            let (words, capped) = text_instead::words_of_markup(html, LARGEST_BODY);

            match words.trim().is_empty() {
                true => RenderedBody::nothing(),
                false => {
                    let shown = from_text(&words, Addresses::Linked);
                    RenderedBody {
                        truncated: cut || capped || shown.truncated,
                        ..shown
                    }
                }
            }
        }
    };

    // Said in the journal because nothing on screen says it: a reader looking at a reply as text
    // cannot tell a message written as text from one this build declined to draw, and whoever reads
    // a report needs to.
    use crate::ports::journal::{Journal, Note, Value};

    let journal = crate::infrastructure::journal::journal();
    if journal.keeps(Note::Detail) {
        journal.write_down(
            Note::Detail,
            "body.shownAsText",
            &[
                ("tooDeep", Value::Yes(too_deep)),
                ("tooMany", Value::Yes(too_many)),
                ("textPart", Value::Yes(beside.is_some())),
                ("truncated", Value::Yes(shown.truncated)),
                ("bytes", Value::Bytes(html.len() as u64)),
            ],
        );
    }

    shown
}

/// The markup cleaned against the allowlist, with every picture and link it names counted and
/// put behind a name. Only for a document that is within both bounds: this is where it becomes a
/// tree.
fn drawn(
    within: &str,
    truncated: bool,
    pictures: Vec<(String, InlinePart)>,
    declined: std::collections::HashSet<String>,
    remote: Pictures,
    attachments: Vec<Attached>,
) -> RenderedBody {
    let began = std::time::Instant::now();

    let blocked = Arc::new(AtomicU32::new(0));
    let not_shown = Arc::new(AtomicU32::new(0));
    let written_in: Arc<Mutex<Vec<InlinePart>>> = Arc::new(Mutex::new(Vec::new()));
    let declined = Arc::new(declined);
    let offered: Arc<Mutex<Vec<Link>>> = Arc::new(Mutex::new(Vec::new()));
    let pointed_at: Arc<Mutex<Vec<Link>>> = Arc::new(Mutex::new(Vec::new()));
    let inside: Arc<HashMap<String, String>> = Arc::new(
        pictures
            .iter()
            .map(|(identifier, part)| (identifier.clone(), part.name.clone()))
            .collect(),
    );

    let cleaned = cleaner(
        Counting {
            blocked: Arc::clone(&blocked),
            not_shown: Arc::clone(&not_shown),
            written_in: Arc::clone(&written_in),
            declined: Arc::clone(&declined),
        },
        Arc::clone(&inside),
        Arc::clone(&offered),
        Arc::clone(&pointed_at),
        remote,
    )
    .clean(within)
    .to_string();

    // A message's own stylesheet, which the sanitizer keeps whole because dropping it showed the
    // message as something other than what was written. Its pictures take the same road as every
    // other: the address leaves and a name served from here takes its place. Done after the
    // cleaning rather than before, on text the sanitizer has already vouched for — and the golden
    // test pins that nothing between those tags is ever markup, so where a block ends is not a
    // question.
    let cleaned = {
        let counting = Counting {
            blocked: Arc::clone(&blocked),
            not_shown: Arc::clone(&not_shown),
            written_in: Arc::clone(&written_in),
            declined: Arc::clone(&declined),
        };

        styles_that_draw::stylesheets_pointing_at_pictures(
            &cleaned,
            &counting,
            &pointed_at,
            remote,
            &inside,
        )
    };

    // Only the ones the body actually refers to: a part nobody points at is an attachment, and an
    // attachment is not something to hold in memory for the sake of a picture nobody asked for.
    let mut referred: Vec<InlinePart> = pictures
        .into_iter()
        .map(|(_, part)| part)
        .filter(|part| cleaned.contains(&part.name))
        .collect();

    // The ones the body carried in itself, held under the names the cleaning gave them.
    referred.extend(
        written_in
            .lock()
            .expect("the lock is never poisoned")
            .drain(..)
            .filter(|part| cleaned.contains(&part.name)),
    );

    // Only the ones that survived the cleaning: a link inside markup the sanitizer dropped was
    // given a name and then thrown away with the element that carried it.
    let offered: Vec<Link> = offered
        .lock()
        .expect("the lock is never poisoned")
        .drain(..)
        .filter(|link| cleaned.contains(&link.name))
        .collect();

    let pointed_at: Vec<Link> = pointed_at
        .lock()
        .expect("the lock is never poisoned")
        .drain(..)
        .filter(|picture| cleaned.contains(&picture.name))
        .collect();

    let body = RenderedBody {
        attachments,
        declares_colours: names_a_colour(&cleaned),
        html: cleaned,
        parts: referred,
        links: offered,
        remote: pointed_at,
        kind: BodyKind::Html,
        remote_blocked: blocked.load(Ordering::SeqCst),
        remote_not_shown: not_shown.load(Ordering::SeqCst),
        remote_not_had: 0,
        truncated,
    };

    // One line saying what the reader is about to be told and what is behind it. The three counters
    // reach the screen as two sentences and a button, and a person reading "nine pictures are not
    // shown here" cannot tell which of four different things happened — the message named a picture
    // it does not carry, or an address this build does not fetch, or a kind it does not draw. Here
    // they are apart, beside how many are actually held, which is the number the sentences never
    // say and the only one that says whether anything will appear.
    use crate::ports::journal::{Journal, Note, Value, Word};

    let journal = crate::infrastructure::journal::journal();
    if journal.keeps(Note::Detail) {
        journal.write_down(
            Note::Detail,
            "body.rendered",
            &[
                ("kind", Value::Word(Word::Html)),
                ("held", Value::Count(body.parts.len() as u64)),
                ("toFetch", Value::Count(body.remote.len() as u64)),
                ("blocked", Value::Count(u64::from(body.remote_blocked))),
                ("notShown", Value::Count(u64::from(body.remote_not_shown))),
                ("links", Value::Count(body.links.len() as u64)),
                ("truncated", Value::Yes(body.truncated)),
                ("bytes", Value::Bytes(body.html.len() as u64)),
                ("took", Value::Millis(began.elapsed().as_millis() as u64)),
            ],
        );
    }

    body
}

/// Whether the addresses in a text are offered as links.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Addresses {
    /// As links, the way a message's own markup offers them: text somebody reads.
    Linked,
    /// As the text they are: a message's source, which is shown as it arrived.
    AsWritten,
}

/// Text made into a document, with every character that means something in HTML escaped, and every
/// address in it a link behind a name, as an address in markup is: following one reaches this
/// application, which asks the person before anything leaves the machine.
fn from_text(text: &str, addresses: Addresses) -> RenderedBody {
    let truncated = text.len() > LARGEST_BODY;
    let within = match truncated {
        true => cut_at_character(text, LARGEST_BODY),
        false => text,
    };

    let found = match addresses {
        Addresses::Linked => links_in(within),
        Addresses::AsWritten => Vec::new(),
    };

    let mut escaped = String::with_capacity(within.len() + 16);
    let mut offered = Vec::with_capacity(found.len());
    let mut at = 0;
    escaped.push_str("<pre>");
    for link in found {
        escape_into(&mut escaped, &within[at..link.at.start]);
        match serving::random_name() {
            Some(name) => {
                escaped.push_str(&format!("<a href=\"/{LINKS}/{name}\">"));
                escape_into(&mut escaped, &within[link.at.clone()]);
                escaped.push_str("</a>");
                offered.push(Link {
                    name,
                    url: link.url.into(),
                });
            }
            // Nothing to name it by is an address left as the text it is.
            None => escape_into(&mut escaped, &within[link.at.clone()]),
        }
        at = link.at.end;
    }
    escape_into(&mut escaped, &within[at..]);
    escaped.push_str("</pre>");

    RenderedBody {
        attachments: Vec::new(),
        // Escaped text and nothing else: whatever the message said about colour before it was
        // turned into text is not in what is shown.
        declares_colours: false,
        html: escaped,
        parts: Vec::new(),
        links: offered,
        remote: Vec::new(),
        kind: BodyKind::PlainText,
        remote_blocked: 0,
        remote_not_shown: 0,
        remote_not_had: 0,
        truncated,
    }
}

/// Text written into a document, with every character that means something in HTML escaped.
fn escape_into(document: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '<' => document.push_str("&lt;"),
            '>' => document.push_str("&gt;"),
            '&' => document.push_str("&amp;"),
            '"' => document.push_str("&quot;"),
            '\'' => document.push_str("&#39;"),
            other => document.push(other),
        }
    }
}

/// The allowlist: what a message is allowed to be made of.
///
/// Named rather than defaulted, so what is permitted is a decision in this file and a change to it
/// is a change to this file. No `style` anywhere — a stylesheet fetches, and what it fetches is a
/// message telling someone that it was opened.
/// What the cleaning keeps a running answer in.
///
/// One value rather than six arguments: what a picture turned out to be is one question with
/// several answers, and the cleaner is handed the place to put them.
struct Counting {
    blocked: Arc<AtomicU32>,
    not_shown: Arc<AtomicU32>,
    written_in: Arc<Mutex<Vec<InlinePart>>>,
    declined: Arc<std::collections::HashSet<String>>,
}

fn cleaner(
    counting: Counting,
    inside: Arc<HashMap<String, String>>,
    offered: Arc<Mutex<Vec<Link>>>,
    pointed_at: Arc<Mutex<Vec<Link>>>,
    remote: Pictures,
) -> ammonia::Builder<'static> {
    let mut builder = ammonia::Builder::default();

    builder
        .tags(std::collections::HashSet::from([
            "a",
            "abbr",
            "b",
            "blockquote",
            "br",
            "caption",
            "code",
            "dd",
            "div",
            "dl",
            "dt",
            "em",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "hr",
            "i",
            "img",
            "li",
            "ol",
            "p",
            "pre",
            "s",
            "small",
            "span",
            "strong",
            "sub",
            "sup",
            "table",
            "tbody",
            "td",
            "tfoot",
            "th",
            "thead",
            "tr",
            "u",
            "ul",
        ]))
        // `style` is here and filtered below, declaration by declaration: a message that cannot
        // say how it looks is a message shown as something other than what was written, and inside
        // a frame with no network and no script the things CSS is usually cut for cannot happen.
        .generic_attributes(std::collections::HashSet::from(["title", "style", "align"]))
        // `src` and `href` are listed here and then always removed by the filter below. An
        // attribute the sanitizer drops on its own never reaches that filter, and what the reader
        // is told — how many pictures a message wanted to fetch — is counted there.
        .tag_attributes(std::collections::HashMap::from([
            (
                "img",
                std::collections::HashSet::from(["alt", "src", "width", "height"]),
            ),
            ("a", std::collections::HashSet::from(["href"])),
            (
                "td",
                std::collections::HashSet::from([
                    "colspan", "rowspan", "width", "height", "valign", "bgcolor",
                ]),
            ),
            (
                "th",
                std::collections::HashSet::from([
                    "colspan", "rowspan", "width", "height", "valign", "bgcolor",
                ]),
            ),
            (
                "table",
                std::collections::HashSet::from([
                    "width",
                    "border",
                    "cellpadding",
                    "cellspacing",
                    "bgcolor",
                ]),
            ),
            ("tr", std::collections::HashSet::from(["valign", "bgcolor"])),
        ]))
        // The schemes a message may plausibly carry, listed so that an attribute holding one
        // reaches the filter below, which is what decides. Anything else -- a `javascript:` link
        // above all -- is dropped before that, by the sanitizer itself.
        //
        // `data` is here so a picture written into the body reaches the filter and is decoded
        // there. Dropped before it, the whole element went with it: no picture, no count, and
        // nothing on screen to say why. It is still only ever a picture: the filter serves what it
        // can decode as one and removes everything else, and `openable` keeps it out of links.
        .url_schemes(std::collections::HashSet::from([
            "http", "https", "mailto", "cid", "data",
        ]))
        // Elements whose text is not something anybody wrote to be read. A tag that is neither
        // allowed nor named here has its tag dropped and its **text kept**, which is right for a
        // `<div>` and wrong for a `<title>`: every message template puts the subject line in the
        // head, and one that failed to fill it in put `${subject}` at the top of the message.
        // Named with `add_` so `script` and `style`, which are cleared by default, stay cleared.
        // A message's own stylesheet, kept as written.
        //
        // It was dropped, and never by a decision: `style` is in the sanitizer's own list of
        // things to remove whole, and `head` -- where a message puts its stylesheet -- was in
        // ours. What that cost is a message shown as something other than what was written: a
        // newsletter is a table whose every rule lives in that block, and without it the columns
        // collapse, the spacing goes, and a rating strip lands under a picture instead of beside
        // it.
        //
        // What holds it is the policy the document is served under, and the frame it is served
        // into. `default-src 'none'` with `img-src lyomsg:` means the frame reaches nothing except
        // this application: `@import`, `@font-face` and a cursor are refused outright, and a
        // background is drawn only where the address has already been taken out and a name served
        // from here put in its place -- measured on the engine rather than reasoned about, and
        // `engine_probe` asks that question every run. There is no script in the frame to read
        // what CSS matched, and a link is followed only through a panel outside the frame that
        // names the address, so a stylesheet cannot turn a person's click into somewhere else.
        //
        // A `style=` attribute keeps its own filter, which passes on a `url(` only where it names
        // a picture this build minted. Two barriers there, one here: that is why the probe
        // measures this one.
        .rm_clean_content_tags(["style"])
        .add_tags(["style"])
        .add_clean_content_tags([
            "title", "meta", "link", "base", "template", "noscript", "textarea", "xmp", "iframe",
            "frame", "frameset", "object", "embed", "applet", "param", "source", "track", "canvas",
            "map", "area", "form", "button", "select", "option", "optgroup", "datalist", "output",
            "svg", "math",
        ])
        .strip_comments(true)
        .attribute_filter(move |element, attribute, value| {
            // What a message says about how it looks, kept declaration by declaration.
            if attribute == "style" {
                // A picture this style draws takes the road a picture in an `<img>` takes: the
                // address leaves, this application fetches it if the reader asked, and what stays
                // is a name served from here. Before this, such a picture was refused outright and
                // counted by nobody — an empty box where a logo belongs, with nothing to press.
                let (kept, blocked, never) = filtered_style(value, |address| {
                    styles_that_draw::a_picture_a_stylesheet_points_at(
                        address,
                        &counting,
                        &pointed_at,
                        remote,
                        &inside,
                    )
                });

                // After the filter and not before it: what is counted is what survived, so a
                // number in front of a reader always has a picture behind it.
                counting.blocked.fetch_add(blocked, Ordering::SeqCst);
                counting.not_shown.fetch_add(never, Ordering::SeqCst);

                return kept.map(std::borrow::Cow::Owned);
            }

            // The old way of saying the same things: a number, a colour or a keyword, and nothing
            // long enough to be anything else.
            if matches!(
                attribute,
                "width"
                    | "height"
                    | "align"
                    | "valign"
                    | "bgcolor"
                    | "border"
                    | "cellpadding"
                    | "cellspacing"
                    | "colspan"
                    | "rowspan"
            ) {
                return plain_value(value).map(std::borrow::Cow::Owned);
            }

            if element == "img" && attribute == "src" {
                // A picture that travelled inside the message is served from beside the body, by a
                // name that means nothing outside this moment. One that lives somewhere else is
                // counted and taken out: fetching it would tell its owner the message was opened.
                // Written into the body itself. Already in hand: nothing is fetched, and nobody
                // learns the message was opened.
                if let Some((kind, bytes)) = picture_written_in(value) {
                    if let Some(name) = serving::random_name() {
                        counting
                            .written_in
                            .lock()
                            .expect("the lock is never poisoned")
                            .push(InlinePart {
                                name: name.clone(),
                                content_type: kind.to_owned(),
                                bytes,
                            });

                        return Some(std::borrow::Cow::Owned(format!("/{name}")));
                    }
                }

                if let Some(carried) = value.strip_prefix("cid:") {
                    let carried = carried.trim_matches(|c| c == '<' || c == '>');

                    if let Some(name) = inside.get(carried) {
                        return Some(std::borrow::Cow::Owned(format!("/{name}")));
                    }

                    // Carried, and of a kind this build will not put on screen. Said as its own
                    // sentence: fetching it again cannot help, because there is nothing anywhere
                    // to fetch -- it is right here, and the refusal is ours.
                    if counting.declined.contains(carried) {
                        counting.not_shown.fetch_add(1, Ordering::SeqCst);
                        return None;
                    }
                }

                // Somewhere else, and the reader asked for it: the body gets a name, and the
                // bytes are fetched by this application and put under that name. The frame still
                // reaches nothing but this origin.
                let address = as_an_address(value);

                if remote == Pictures::Shown && crate::infrastructure::http::fetchable(&address) {
                    if let Some(name) = serving::random_name() {
                        pointed_at
                            .lock()
                            .expect("the lock is never poisoned")
                            .push(Link {
                                name: name.clone(),
                                url: address,
                            });

                        return Some(std::borrow::Cow::Owned(format!("/{name}")));
                    }
                }

                counting.blocked.fetch_add(1, Ordering::SeqCst);
                return None;
            }

            if element == "a" && attribute == "href" {
                // The address stays here; what goes into the body is a name. Following it reaches
                // this application, which asks the person before anything leaves the machine.
                // Folded the same way a picture's address is, and read back the same way: a
                // link a person is shown and asked about must be the one that was written, not
                // the one the wrapping made of it.
                //
                // Kept in the spelling the parser writes out rather than the message's own: the
                // same address, as the one string the person is asked about and the system is
                // handed.
                if let Some(address) = openable(&as_an_address(value)) {
                    if let Some(name) = serving::random_name() {
                        offered
                            .lock()
                            .expect("the lock is never poisoned")
                            .push(Link {
                                name: name.clone(),
                                url: address.into(),
                            });

                        return Some(std::borrow::Cow::Owned(format!("/{LINKS}/{name}")));
                    }
                }

                // A scheme this build will not open, or an address nothing can read, is not a
                // link: it is dropped like any other markup nobody can act on.
                return None;
            }

            // Nothing else is kept: what is not in the allowlist above is gone either way, and this
            // is where what is worth counting is counted.
            None
        });

    builder
}

/// Elements that never contain anything, so they never add depth.
#[cfg(test)]
mod tests;

#[cfg(test)]
mod long_thread_tests;

/// One address as a browser reads it.
///
/// Mail is folded: a long address in a message arrives wrapped across lines, and what the parser
/// hands over then has a newline and the next line's indent inside it. Sent as it stands it is not
/// the address anybody wrote, and the host answers with a refusal — which reached the reader as
/// "the picture could not be fetched", about a picture nobody had ever asked for correctly.
///
/// The rule is the URL standard's own: a tab, a line feed and a carriage return are removed
/// wherever they appear, and what is left is trimmed. Nothing else is touched -- an address with a
/// space in it is an address with a space in it, and inventing one is worse than not fetching one.
fn as_an_address(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '\t' | '\n' | '\r'))
        .collect::<String>()
        .trim()
        .to_owned()
}
