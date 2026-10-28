//! Handing a sanitized body to the frame that shows it, and to nothing else.
//!
//! The second and third of the four layers between a message and the window. A body never crosses
//! IPC as a string: what crosses is a URL on a scheme of its own, and what that URL answers with
//! carries its own policy in its own response header — a policy the application's own cannot weaken
//! and does not share.
//!
//! What is held is up to four bodies under names of 32 random bytes, each answering until its body
//! is closed — when the reader leaves the message, or a redraw replaces it. A frame that got out of its sandbox would still have
//! nothing to ask for: there is no message identifier in the URL to change, and the only names that
//! exist are the ones this reader has just been given.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use ring::rand::{SecureRandom, SystemRandom};

use crate::infrastructure::journal::journal;
use crate::ports::body::RenderedBody;
use crate::ports::journal::{Journal, Note, Value, Word};

/// What the store was asked for and whether it had it.
///
/// The one place that watches the frame from inside. Everything before this is what this build did;
/// this is what the engine then did with it, and the two have been assumed to agree because nothing
/// was looking.
fn asked_for(what: Word, found: bool, held: usize) {
    let journal = journal();
    if journal.keeps(Note::Detail) {
        journal.write_down(
            Note::Detail,
            "store.asked",
            &[
                ("for", Value::Word(what)),
                ("found", Value::Yes(found)),
                ("bodiesHeld", Value::Count(held as u64)),
            ],
        );
    }
}

/// The scheme a body is served on. Its own scheme means its own origin, and its own origin means a
/// policy that has nothing to do with the application's.
pub const SCHEME: &str = "lyomsg";

/// The policy the body is served under, in the response header rather than in a meta tag: a header
/// applies before the first byte of the document is parsed.
///
/// `default-src 'none'` is the whole of it — nothing loads, nothing connects, nothing runs. The
/// three `style-src` directives are spelled out because an engine without CSP3 falls back to
/// `style-src`, and an engine that has it ignores `style-src` in favour of the other two: leaving
/// any of them out leaves a hole on one of the three platforms. `sandbox` with nothing after it is
/// the strictest form, and it does not depend on the frame that embeds the document.
///
/// **`img-src` names the scheme and must not be turned back into `'self'`.** This document is shown
/// inside `<iframe sandbox="">`, which gives it a **unique opaque origin** — and `'self'` means
/// "this document's origin", which for an opaque origin matches no URL at all. Written as `'self'`
/// this directive does not say "only from us": it says "from nowhere", and every picture in every
/// message is refused before a request is made. Measured on WKWebView: the same document at the
/// top of a window asks for its picture, and inside `sandbox=""` it does not — which is why the
/// engine probe opens the message inside the same frame.
///
/// Naming the scheme grants a message nothing. `lyomsg` is not in the sanitizer's list of schemes
/// (`http`, `https`, `mailto`, `cid`, `data`), so a message cannot write such an address at all;
/// the only ones in the document are the names this build minted for this render, which answer
/// until their body is closed and cannot be spelled by anybody else.
///
/// **Both spellings, and neither may be dropped.** A custom scheme is `lyomsg://` on macOS and
/// Linux and `http://lyomsg.localhost` on Windows, and a policy naming one of them names nothing on
/// the systems that use the other — measured on WebView2, where the frame loaded and every picture
/// in it was refused. `tauri.conf.json` already writes `frame-src` both ways for the same reason;
/// this is the same fact one directive along.
pub const POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; style-src-elem 'unsafe-inline'; style-src-attr 'unsafe-inline'; img-src lyomsg: http://lyomsg.localhost; media-src 'none'; font-src 'none'; connect-src 'none'; script-src 'none'; object-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'; sandbox";

/// The policy the one page that is not a message is served under.
///
/// The engine probe puts a message inside the frame the reader's window puts it in, and that frame
/// needs a page to sit in. Served under [`POLICY`] it could not: `frame-src 'none'` forbids the very
/// thing it exists for, so the message never loaded and the probe reported an engine that fetched
/// nothing — which is exactly the false answer this whole arrangement is here to stop giving.
///
/// It permits one frame, on the scheme this module serves, and nothing else at all.
pub const AROUND_A_MESSAGE: &str = "default-src 'none'; frame-src lyomsg: http://lyomsg.localhost";

/// What a link is asked for under. A word rather than a name, so a link and a picture can never be
/// mistaken for one another.
pub const LINKS: &str = "link";

/// The policy anything that is not a document is served under: it may be looked at and it may do
/// nothing. A picture that arrives here asked for as a page, or a link's own empty answer.
pub const NOTHING_AT_ALL: &str = "default-src 'none'; sandbox";

/// One answer of the scheme a message is served on, headers and all.
///
/// A named function rather than a closure inside the registration, because this is the second of
/// the four layers and it had no test of any kind: `POLICY` was asserted as a string while nothing
/// said the string reached a response. Deleting the header left `cargo test`, `npm test` and the
/// gate all green and served the frame holding a message with no policy at all — the one layer of
/// the four that could be removed by accident and pass everything this project has.
pub fn answer(
    status: u16,
    content_type: &str,
    policy: &str,
    bytes: Vec<u8>,
) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Content-Security-Policy", policy)
        // What this is, is what this header says — never what the message called it.
        .header("X-Content-Type-Options", "nosniff")
        .header("Cache-Control", "no-store")
        .body(bytes)
        .expect("a response with headers this build wrote")
}

/// One answer that is a page: the document policy, and the type a document has.
pub fn document(status: u16, page: Vec<u8>) -> tauri::http::Response<Vec<u8>> {
    answer(status, "text/html; charset=utf-8", POLICY, page)
}

/// How long a name is, in bytes before it is written out as hex.
const NAME_BYTES: usize = 32;

/// One body, waiting for the frame that will show it, and what it refers to.
struct Waiting {
    /// When this body was last asked for anything, on a counter that only goes up.
    ///
    /// What decides which one goes when the ceiling is reached: the one asked for least recently,
    /// never simply the oldest held. The oldest is the body the reader is most likely to be
    /// reading: a message is opened once and then its pictures are fetched over the seconds that
    /// follow, so the one being read is the one whose requests are still arriving. Dropping it
    /// leaves the text on screen -- the page was delivered already -- and answers every picture
    /// with nothing, which is a message that renders with holes where its pictures are and says
    /// nothing about why.
    last_asked: u64,
    token: String,
    page: Option<String>,
    /// How many times that page has been answered. The engine probe reads it to tell the message it
    /// shows from the page it puts the message in, which [`Asked::pages`] counts alike.
    answered: u32,
    /// What that page is served under. A message gets [`POLICY`]; the one page that is not a
    /// message — the frame the probe puts around one — gets its own, because a policy forbidding
    /// frames cannot be the policy of a page whose whole content is a frame.
    policy: &'static str,
    /// By the name the body refers to each of them by.
    parts: std::collections::HashMap<String, Part>,
    /// The addresses behind the links the body offers, by name.
    links: std::collections::HashMap<String, String>,
}

/// What is being held.
#[derive(Default)]
struct Held {
    /// What `last_asked` is stamped from. Only ever goes up, and needs no clock.
    asks: u64,
    /// Newest last. More than one for two reasons: a reader who opened a message twice in the same
    /// instant — which is what a development build does on every mount — must not have the first
    /// answer destroy the second; and two opens for *different* messages race each other too, so
    /// nothing here may throw away a body because a slower answer for something else arrived after
    /// it. What bounds this instead is the count below and the reader closing what it opened.
    bodies: Vec<Waiting>,
    /// The one address a person asked about in a message they sent from here, by the name it was
    /// given, until they ask about another. Such a message is shown before it is read back and is
    /// not served as a body, so its addresses are in none of the bodies above.
    written: Option<(String, String)>,
}

/// How many bodies are kept at once.
///
/// A body is closed by name the moment the reader moves on, so this is the ceiling for the ones
/// whose closing never arrived -- an answer that landed after the reader had already left.
const HELD_AT_ONCE: usize = 4;

/// A picture that travelled inside the message.
#[derive(Debug, Clone)]
pub struct Part {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// A name of `NAME_BYTES` random bytes, written as hex.
///
/// `None` when the machine has no randomness to spare: a name that is not random is a name that can
/// be guessed, and there is nothing here to fall back to.
pub fn random_name() -> Option<String> {
    let mut bytes = [0u8; NAME_BYTES];
    SystemRandom::new().fill(&mut bytes).ok()?;

    Some(
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    )
}

/// The bodies currently being read.
///
/// Up to four renders are held at once, and they may belong to different messages and different
/// accounts: two answers for two different messages race each other, and throwing away everything
/// belonging to another message would let the slower one wipe what the reader is looking at.
///
/// **What makes that safe is that a name is never shared between renders.** Every page, picture and
/// link name is `NAME_BYTES` of fresh randomness, minted for one render, so a name from one of them
/// finds nothing in another whatever the lookup searches. `part` and `link` do search all four —
/// they have only a name to go on, because the frame asks for a picture with the name the body
/// refers to it by and nothing else — and that is sound exactly as long as the names stay random
/// and are never derived, reused or made predictable. `names_are_never_shared_between_renders`
/// below is what fails if that ever stops being true.
#[derive(Clone, Default)]
pub struct BodyBeingRead {
    held: Arc<Mutex<Held>>,
    asked: Arc<Asked>,
}

/// What has been asked of this store.
///
/// Counted because the frame is the one part of this application that runs on an engine nobody
/// here wrote, and the only thing that observes it from inside is the store it fetches from.
/// `pages` counts every request that arrives -- a picture's and a link's are looked up as a page
/// first, and the page the engine probe puts a message in comes from here too -- so it says that
/// something was asked for, not that a message loaded. That is asked of the message's own name,
/// through [`BodyBeingRead::times_answered`]. Nothing reads these but the engine probe and the line
/// a development build prints about a missing picture.
#[derive(Default)]
pub struct Asked {
    pub pages: AtomicU32,
    pub parts: AtomicU32,
    pub links: AtomicU32,
    /// Asked for and not here. The number that tells a picture that was never fetched from one
    /// that was fetched, held, and then let go while the frame was still coming for it.
    pub missed: AtomicU32,
}

impl BodyBeingRead {
    /// What has been asked of this store since it was made.
    pub fn asked(&self) -> &Asked {
        &self.asked
    }

    /// Puts a body and what it refers to in, and gives back the name that fetches the body.
    ///
    /// Nothing is cleared here. Two answers for two different messages race each other — the reader
    /// moves on while the first is still being rendered — and a version of this that threw away
    /// everything belonging to another message would let the slower answer wipe the body the reader
    /// is looking at. What bounds the store is the count and the reader closing what it opened.
    ///
    /// `None` when this machine has no randomness to spare, which is not a state to paper over with
    /// a predictable name.
    pub fn hold(&self, rendered: &RenderedBody, surface: Option<&Surface>) -> Option<String> {
        let token = random_name()?;
        let page = page_of(&rendered.html, surface, rendered.declares_colours);
        let parts = rendered
            .parts
            .iter()
            .map(|part| {
                (
                    part.name.clone(),
                    Part {
                        content_type: part.content_type.clone(),
                        bytes: part.bytes.clone(),
                    },
                )
            })
            .collect();
        let links = rendered
            .links
            .iter()
            .map(|link| (link.name.clone(), link.url.clone()))
            .collect();

        let mut held = self.held.lock().expect("the lock is never poisoned");

        held.asks += 1;
        let last_asked = held.asks;

        held.bodies.push(Waiting {
            last_asked,
            token: token.clone(),
            page: Some(page),
            answered: 0,
            policy: POLICY,
            parts,
            links,
        });

        let journal = journal();
        if journal.keeps(Note::Detail) {
            journal.write_down(
                Note::Detail,
                "store.held",
                &[
                    ("parts", Value::Count(rendered.parts.len() as u64)),
                    ("links", Value::Count(rendered.links.len() as u64)),
                    ("bodiesHeld", Value::Count(held.bodies.len() as u64)),
                ],
            );
        }

        // The one nobody has asked anything of for longest, never simply the oldest held: a body
        // is held once and then asked for over the seconds that follow, so held-longest and
        // wanted-least are close to opposites.
        while held.bodies.len() > HELD_AT_ONCE {
            let quietest = held
                .bodies
                .iter()
                .enumerate()
                .min_by_key(|(_, waiting)| waiting.last_asked)
                .map(|(at, _)| at)
                .unwrap_or(0);

            held.bodies.remove(quietest);
        }

        Some(token)
    }

    /// A page whose whole content is one message, in the frame the reader's window puts it in.
    ///
    /// Only the engine probe holds one. A message opened as the whole of a window has an ordinary
    /// origin; the reader shows it inside `<iframe sandbox="">`, where the origin is opaque and a
    /// policy written against `'self'` matches nothing. A probe that opened the message bare would
    /// report a picture fetched on an engine whose reader fetches none.
    pub fn hold_a_frame_around(&self, token: &str) -> Option<String> {
        let around = random_name()?;
        let page = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"></head><body style=\"margin:0\">\
             <iframe sandbox=\"\" referrerpolicy=\"no-referrer\" src=\"{}\" \
             style=\"border:0;width:100%;height:100vh\"></iframe></body></html>",
            url_for(token)
        );

        let mut held = self.held.lock().expect("the lock is never poisoned");
        held.asks += 1;
        let last_asked = held.asks;
        held.bodies.push(Waiting {
            last_asked,
            token: around.clone(),
            page: Some(page),
            answered: 0,
            policy: AROUND_A_MESSAGE,
            parts: std::collections::HashMap::new(),
            links: std::collections::HashMap::new(),
        });

        Some(around)
    }

    /// The page held under this name, for as long as the body is open.
    ///
    /// Answered on every ask until `forget`: a second ask answered with an empty document would be a
    /// message that renders blank, and pictures whose loading is cancelled halfway.
    ///
    /// Nothing is weakened by answering again. The name is 32 random bytes, it belongs to one
    /// render and to no other, and it stops working the moment its body is closed -- the reader
    /// leaving the message, or a redraw replacing it. `forget` is what ends it, and four bodies is
    /// all that are ever held.
    pub fn page(&self, token: &str) -> Option<(String, &'static str)> {
        let mut held = self.held.lock().expect("the lock is never poisoned");
        self.asked.pages.fetch_add(1, Ordering::Relaxed);

        held.asks += 1;
        let now = held.asks;
        let waiting = held
            .bodies
            .iter_mut()
            .find(|waiting| same(&waiting.token, token))?;

        waiting.last_asked = now;
        let page = waiting.page.clone().map(|html| (html, waiting.policy));
        if page.is_some() {
            waiting.answered = waiting.answered.saturating_add(1);
        }
        let bodies = held.bodies.len();
        drop(held);
        asked_for(Word::APage, page.is_some(), bodies);
        page
    }

    /// How many times the page held under this name has been answered, while its body is held.
    ///
    /// The engine probe's question. It holds two pages, the message and the page it puts the
    /// message in, and [`Asked::pages`] counts every request to either alike.
    pub fn times_answered(&self, token: &str) -> u32 {
        let held = self.held.lock().expect("the lock is never poisoned");

        held.bodies
            .iter()
            .find(|waiting| same(&waiting.token, token))
            .map_or(0, |waiting| waiting.answered)
    }

    /// A picture one of the held bodies refers to.
    ///
    /// Like the page, a picture may be asked for more than once — a webview decides for itself how
    /// often it fetches one — and it answers until its body is closed: when the reader leaves the
    /// message, or a redraw replaces it.
    pub fn part(&self, name: &str) -> Option<Part> {
        let mut held = self.held.lock().expect("the lock is never poisoned");

        self.asked.parts.fetch_add(1, Ordering::Relaxed);

        held.asks += 1;
        let now = held.asks;
        let Some(waiting) = held
            .bodies
            .iter_mut()
            .find(|waiting| waiting.parts.contains_key(name))
        else {
            self.asked.missed.fetch_add(1, Ordering::Relaxed);
            let bodies = held.bodies.len();
            drop(held);
            asked_for(Word::APart, false, bodies);
            return None;
        };

        // A picture asked for is the body it belongs to being read.
        waiting.last_asked = now;
        let part = waiting.parts.get(name).cloned();
        let bodies = held.bodies.len();
        drop(held);
        asked_for(Word::APart, part.is_some(), bodies);
        part
    }

    /// The address behind one of the links the open message offers.
    ///
    /// Looked up rather than trusted: what the frame asked with is a name, and only a name that
    /// belongs to the message being read now finds anything.
    pub fn link(&self, name: &str) -> Option<String> {
        let held = self.held.lock().expect("the lock is never poisoned");

        self.asked.links.fetch_add(1, Ordering::Relaxed);

        let address = held
            .bodies
            .iter()
            .find_map(|waiting| waiting.links.get(name))
            .or_else(|| {
                held.written
                    .as_ref()
                    .filter(|(held_as, _)| held_as == name)
                    .map(|(_, address)| address)
            })
            .cloned();
        let bodies = held.bodies.len();
        drop(held);
        asked_for(Word::ALink, address.is_some(), bodies);
        address
    }

    /// Holds an address a person wrote, under a name of its own, for them to be asked about and for
    /// [`Self::link`] to find when they say yes. The one held before is let go.
    pub fn hold_written_link(&self, address: String) -> Option<String> {
        let name = random_name()?;
        self.held
            .lock()
            .expect("the lock is never poisoned")
            .written = Some((name.clone(), address));

        Some(name)
    }

    /// Forgets one body, by the name it was given.
    ///
    /// Addressed on purpose. A reader who left a message behind is closing *that* body, and an
    /// unaddressed close is one that arrives late and takes whatever replaced it — which is a
    /// message that renders as nothing for no reason a person can see.
    pub fn forget(&self, token: &str) {
        let mut held = self.held.lock().expect("the lock is never poisoned");

        held.bodies.retain(|waiting| !same(&waiting.token, token));
    }
}

/// Whether two names are the same, in a time that does not depend on where they first differ.
///
/// The length is allowed to leak: every name this build issues is the same length, and that length
/// is written down a few lines above.
fn same(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }

    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |differences, (left, right)| {
            differences | (left ^ right)
        })
        == 0
}

/// The address the frame is pointed at.
///
/// The two forms are the same scheme: Windows and Android put a custom scheme behind
/// `http://<scheme>.localhost`, everything else keeps it as a scheme of its own. Built here rather
/// than in the interface, so the difference lives at the one place it exists and the frontend is
/// handed an address it does not have to know anything about.
pub fn url_for(token: &str) -> String {
    #[cfg(any(target_os = "windows", target_os = "android"))]
    {
        format!("http://{SCHEME}.localhost/{token}")
    }
    #[cfg(not(any(target_os = "windows", target_os = "android")))]
    {
        format!("{SCHEME}://localhost/{token}")
    }
}

/// The colours the window is wearing, as the interface computed them.
///
/// Two strings, and each is accepted only if it is a plain colour — see [`a_colour`]. Nothing else
/// crosses into the frame: the interface says what it is wearing, the frame paints itself with it,
/// and the worst a wrong value can do is fail to be a colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surface {
    pub background: String,
    pub foreground: String,
}

impl Surface {
    /// The pair, if both halves are colours and nothing else.
    pub fn of(background: &str, foreground: &str) -> Option<Self> {
        Some(Self {
            background: a_colour(background)?,
            foreground: a_colour(foreground)?,
        })
    }
}

/// A colour, or nothing.
///
/// Strict on purpose: this value is written into a stylesheet inside the frame, so the only shapes
/// accepted are the ones a colour has. A hex triple, or a function of numbers — `oklch`, `rgb`,
/// `hsl` and their alpha forms — with nothing in it but digits, dots, percent signs, slashes and
/// spaces. No `url(`, no `var(`, no semicolon, no brace, and nothing that could close the rule and
/// start another.
fn a_colour(value: &str) -> Option<String> {
    let value = value.trim();

    if value.is_empty() || value.len() > 64 {
        return None;
    }

    if let Some(digits) = value.strip_prefix('#') {
        let ok = matches!(digits.len(), 3 | 4 | 6 | 8)
            && digits.bytes().all(|byte| byte.is_ascii_hexdigit());

        return ok.then(|| value.to_owned());
    }

    let (name, arguments) = value.split_once('(')?;
    let arguments = arguments.strip_suffix(')')?;

    let known = matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "oklch" | "oklab" | "rgb" | "rgba" | "hsl" | "hsla" | "lab" | "lch" | "color"
    );
    let plain = arguments
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b" .,%/-+".contains(&byte));

    (known && plain).then(|| value.to_owned())
}

/// The document a body is wrapped in.
///
/// Deliberately small: a stylesheet that says how to read text, and nothing that fetches.
///
/// Which surface it sits on depends on the message, and the rule is about staying readable. A
/// message that says nothing about colour wears what the application is wearing, and looks like
/// part of it rather than a bright or black rectangle inside it. A message that names its own
/// colours was written against a light background — a newsletter that sets grey text and no
/// background assumes white — so it is shown on one, and stays legible instead of becoming dark on
/// dark.
///
/// On the window's surface a link is the colour of the text, underlined, as a link in the window
/// is: the engine's own blue is a colour for white, and on a dark surface it is a link nobody can
/// find in the text. Every address in a message written as text is one of those.
///
/// Either way the message's own colours win: what is set here is on `html`, and on links before
/// the message, which anything the message says about its own elements overrides.
fn page_of(body: &str, surface: Option<&Surface>, declares_colours: bool) -> String {
    let painted = match (surface, declares_colours) {
        (Some(surface), false) => format!(
            "html{{background:{};color:{}}}a:link,a:visited{{color:inherit}}",
            surface.background, surface.foreground
        ),
        // Written against a light background, or nothing to go on: white, and the system is not
        // asked, because the system's answer is what makes an unstyled message a black rectangle
        // in a window that is not black.
        _ => "html{background:#ffffff;color:#111111}".to_owned(),
    };

    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\
         <meta name=\"referrer\" content=\"no-referrer\">\
         <style>{STYLE}{painted}</style></head><body>{body}<style>{LAST_WORD}</style></body></html>"
    )
}

const STYLE: &str =
    "body{margin:0;padding:16px;font:15px/1.55 system-ui,-apple-system,Segoe UI,Roboto,sans-serif;\
overflow-wrap:anywhere;word-break:break-word}\
img{max-width:100%;height:auto}\
table{max-width:100%;border-collapse:collapse}\
pre{white-space:pre-wrap;font:13px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace}\
blockquote{margin:0 0 0 12px;padding-left:12px;border-left:2px solid rgba(128,128,128,.4)}";

/// What the message does not get to decide, written after everything it said.
///
/// A message keeps its own stylesheet, and a stylesheet can say `cursor`, which a `style=`
/// attribute never could -- the attribute filter has no such property in it. So a message could
/// put the hand under the pointer over its own text, or take it off its links, and neither is the
/// message's to choose.
///
/// **A link in a message gets the hand, and nothing else does.** The window has no hand anywhere
/// (AGENTS.md, invariant 19) because a control there lights up instead, and the hand is the answer
/// that stops at the mouse. Inside this frame the rule is the other way round: what is here is a
/// document somebody sent, a link in a document is a link, and the hand is what a person expects
/// of one. The frame is the boundary between the two.
///
/// Last in the document and `!important` for the same reason: among declarations of one origin the
/// later one wins, so this has to come after everything the message wrote.
const LAST_WORD: &str = "*,*::before,*::after{cursor:default!important}\
a[href],a[href] *{cursor:pointer!important}";

#[cfg(test)]
mod tests {

    #[test]
    fn names_are_never_shared_between_renders() {
        // Up to four renders are held at once and they may belong to different messages, so `part`
        // and `link` search all of them: the frame asks for a picture by the name the body refers
        // to it by, and carries nothing else to narrow it with. That is sound for exactly one
        // reason — a name belongs to one render and to no other — and this is the test that fails
        // if a name ever becomes derived, reused or predictable, which is when the same search
        // would start resolving one message's picture for another message's frame.
        let mut seen = std::collections::HashSet::new();

        for _ in 0..200 {
            let name = random_name().expect("this machine has randomness");
            assert_eq!(name.len(), NAME_BYTES * 2, "a name is not the full length");
            assert!(
                name.chars().all(|character| character.is_ascii_hexdigit()),
                "a name is not what it should be made of: {name}"
            );
            assert!(seen.insert(name), "the same name was minted twice");
        }
    }

    #[test]
    fn a_name_from_one_render_finds_nothing_in_another() {
        // The property above, seen from the store. Two renders held at the same time, and neither
        // one's names reach into the other.
        let held = BodyBeingRead::default();

        let first = held
            .hold(&rendered("<p>first</p>"), None)
            .expect("a name is minted");
        let second = held
            .hold(&rendered("<p>second</p>"), None)
            .expect("a name is minted");

        assert_ne!(first, second);
        // Each page is fetched by its own name and by no other.
        assert!(held.page(&first).is_some());
        assert!(held.page(&second).is_some());
        held.forget(&first);
        assert!(held.page(&first).is_none(), "forgotten and still answering");
    }

    #[test]
    fn every_answer_carries_the_headers_that_make_it_safe_to_look_at() {
        // Layer 2 of the four: the policy has to reach a response, not only exist as a string.
        // Without this, deleting `.header("Content-Security-Policy", policy)` would leave every gate
        // green while the frame holding a message is served with none.
        let served = document(200, b"<p>hello</p>".to_vec());
        let headers = served.headers();

        assert_eq!(
            headers
                .get("Content-Security-Policy")
                .and_then(|value| value.to_str().ok()),
            Some(POLICY),
            "a message is served with no policy of its own"
        );
        // What this is, is what this header says, never what the message called it.
        assert_eq!(
            headers
                .get("X-Content-Type-Options")
                .and_then(|value| value.to_str().ok()),
            Some("nosniff")
        );
        // A message body is not a thing to keep: the name it was served under ends when its body is
        // closed, and a cached copy would outlive it.
        assert_eq!(
            headers
                .get("Cache-Control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
        assert_eq!(
            headers
                .get("Content-Type")
                .and_then(|value| value.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(served.status(), 200);
        assert_eq!(served.body(), b"<p>hello</p>");
    }

    #[test]
    fn what_is_not_a_document_is_served_under_a_policy_that_permits_nothing() {
        // A picture that travelled inside the message, and the empty answer a followed link gets.
        // Either could be asked for as a page by something, and then it must be able to do nothing.
        let picture = answer(
            200,
            "image/png",
            NOTHING_AT_ALL,
            vec![0x89, b'P', b'N', b'G'],
        );

        assert_eq!(
            picture
                .headers()
                .get("Content-Security-Policy")
                .and_then(|value| value.to_str().ok()),
            Some(NOTHING_AT_ALL)
        );
        assert!(NOTHING_AT_ALL.contains("default-src 'none'"));
        assert!(NOTHING_AT_ALL.contains("sandbox"));
        assert_eq!(
            picture
                .headers()
                .get("X-Content-Type-Options")
                .and_then(|value| value.to_str().ok()),
            Some("nosniff"),
            "a picture served without this is a picture the engine may decide is something else"
        );
    }

    #[test]
    fn a_name_that_is_not_alive_is_answered_with_a_page_carrying_the_same_policy() {
        // The refusals go out the same way the pages do. A 404 or a 405 served with no policy is a
        // response an engine still renders.
        for status in [404, 405] {
            let refused = document(status, Vec::new());
            assert_eq!(
                refused
                    .headers()
                    .get("Content-Security-Policy")
                    .and_then(|value| value.to_str().ok()),
                Some(POLICY),
                "a {status} is served with no policy"
            );
        }
    }
    use super::*;
    use crate::ports::body::BodyKind;

    fn rendered(html: &str) -> RenderedBody {
        RenderedBody {
            attachments: Vec::new(),
            html: html.to_owned(),
            parts: Vec::new(),
            links: Vec::new(),
            remote: Vec::new(),
            kind: BodyKind::Html,
            remote_blocked: 0,
            remote_not_shown: 0,
            remote_not_had: 0,
            truncated: false,
            declares_colours: false,
        }
    }

    #[test]
    fn a_body_is_fetched_by_the_name_it_was_given() {
        let held = BodyBeingRead::default();

        let token = held.hold(&rendered("<p>Hello.</p>"), None).expect("a name");
        let page = held.page(&token).expect("the body is there");

        assert!(page.0.contains("<p>Hello.</p>"));
        assert!(page.0.starts_with("<!doctype html>"));
    }

    #[test]
    fn a_name_answers_while_the_message_is_open_and_not_after() {
        // A second ask answered with an empty document would be a message that renders blank, with
        // the pictures it had begun to load cancelled halfway.
        //
        // What ends the name is closing its body -- leaving the message, or a redraw replacing it --
        // which is what `forget` is.
        let held = BodyBeingRead::default();
        let token = held.hold(&rendered("<p>Hello.</p>"), None).expect("a name");

        assert!(held.page(&token).is_some());
        assert!(held.page(&token).is_some(), "the engine's second ask");

        held.forget(&token);
        assert!(held.page(&token).is_none());
    }

    #[test]
    fn a_page_counts_its_own_answers_and_no_other() {
        // What the engine probe tells the message from its own wrapper by. A request for a name not
        // held, or for a picture, is a request the scheme received and not an answer of this page's.
        let held = BodyBeingRead::default();
        let token = held.hold(&rendered("<p>Hello.</p>"), None).expect("a name");
        let around = held.hold_a_frame_around(&token).expect("a name");

        assert!(held.page(&around).is_some());
        assert_eq!(
            held.times_answered(&token),
            0,
            "the wrapper is not the message"
        );
        assert_eq!(held.times_answered(&around), 1);

        assert!(held.page(&"0".repeat(token.len())).is_none());
        assert!(held.part("nothing-of-the-kind").is_none());
        assert_eq!(held.times_answered(&token), 0);

        assert!(held.page(&token).is_some());
        assert!(held.page(&token).is_some(), "the engine's second ask");
        assert_eq!(held.times_answered(&token), 2);

        held.forget(&token);
        assert_eq!(
            held.times_answered(&token),
            0,
            "a closed body has answered nothing"
        );
    }

    #[test]
    fn another_name_fetches_nothing() {
        let held = BodyBeingRead::default();
        let token = held.hold(&rendered("<p>Hello.</p>"), None).expect("a name");
        let guessed = "0".repeat(token.len());

        assert!(held.page(&guessed).is_none());
        assert!(held.page("").is_none());
        assert!(
            held.page(&token).is_some(),
            "a wrong guess must not spend the name that is right"
        );
    }

    #[test]
    fn an_answer_for_another_message_does_not_take_the_one_on_screen_with_it() {
        // The failure this is about: the reader moves on while the first message is still being
        // rendered, its answer lands last, and the body the reader is actually looking at is gone.
        let held = BodyBeingRead::default();
        let on_screen = held
            .hold(&rendered("<p>The one being read.</p>"), None)
            .expect("a name");

        // The abandoned message finishes rendering afterwards.
        let late = held
            .hold(&rendered("<p>The one left behind.</p>"), None)
            .expect("a name");

        assert!(
            held.page(&on_screen)
                .expect("still there")
                .0
                .contains("being read"),
            "a late answer wiped the body on screen"
        );
        assert!(held.page(&late).is_some());
    }

    #[test]
    fn nothing_older_than_the_last_few_is_kept() {
        let held = BodyBeingRead::default();
        let names: Vec<String> = (0..HELD_AT_ONCE + 2)
            .map(|number| {
                held.hold(&rendered(&format!("<p>Body {number}.</p>")), None)
                    .expect("a name")
            })
            .collect();

        for gone in &names[..2] {
            assert!(held.page(gone).is_none(), "an old body is still held");
        }
        for kept in &names[2..] {
            assert!(held.page(kept).is_some(), "a recent body was dropped");
        }
    }

    #[test]
    fn one_message_may_be_opened_twice_without_the_first_answer_killing_the_second() {
        // Both are the message being read, and the calls that make them race each other: a
        // development build mounts the reader twice on every open. The answer that arrives second
        // must not find its body gone.
        let held = BodyBeingRead::default();
        let first = held.hold(&rendered("<p>Once.</p>"), None).expect("a name");
        let second = held.hold(&rendered("<p>Again.</p>"), None).expect("a name");

        assert!(held
            .page(&second)
            .expect("the second is there")
            .0
            .contains("Again."));
        assert!(held
            .page(&first)
            .expect("the first is there too")
            .0
            .contains("Once."));
    }

    #[test]
    fn closing_one_body_leaves_the_other_alone() {
        // The failure this is about: an unaddressed close arrives late and takes the body that
        // replaced the one it meant, and the message renders as nothing for no visible reason.
        let held = BodyBeingRead::default();
        let abandoned = held.hold(&rendered("<p>Once.</p>"), None).expect("a name");
        let shown = held.hold(&rendered("<p>Again.</p>"), None).expect("a name");

        held.forget(&abandoned);

        assert!(held.page(&abandoned).is_none());
        assert!(
            held.page(&shown).is_some(),
            "the live body was taken with it"
        );
    }

    #[test]
    fn a_body_that_was_closed_leaves_nothing_to_fetch() {
        let held = BodyBeingRead::default();
        let token = held.hold(&rendered("<p>Hello.</p>"), None).expect("a name");

        held.forget(&token);

        assert!(held.page(&token).is_none());
    }

    #[test]
    fn a_name_is_thirty_two_bytes_of_randomness_and_never_the_same_twice() {
        let held = BodyBeingRead::default();
        let names: std::collections::HashSet<String> = (0..64)
            .map(|_| held.hold(&rendered("<p>x</p>"), None).expect("a name"))
            .collect();

        assert_eq!(names.len(), 64, "a name repeated");
        for name in &names {
            assert_eq!(name.len(), NAME_BYTES * 2);
            assert!(name.chars().all(|character| character.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn the_address_names_the_scheme_and_carries_nothing_else() {
        let address = url_for("abc123");

        assert!(
            address.contains(SCHEME),
            "{address} does not name the scheme"
        );
        assert!(address.ends_with("/abc123"));
        // No message identifier, no folder, no account: there is nothing in the address for a
        // frame to change into something else.
        assert_eq!(address.matches('/').count(), 3, "{address}");
    }

    #[test]
    fn the_policy_says_no_to_everything_and_spells_out_the_three_style_directives() {
        // An engine without CSP3 falls back to `style-src`; one with it ignores `style-src` in
        // favour of the other two. Leaving any of them out leaves a hole on one platform.
        assert!(POLICY.starts_with("default-src 'none'"));
        for directive in [
            "style-src ",
            "style-src-elem ",
            "style-src-attr ",
            "sandbox",
        ] {
            assert!(
                POLICY.contains(directive),
                "the policy is missing {directive}"
            );
        }
        // Nothing that opens the network, and the check is made against the policy with the one
        // host this build serves on taken out first. Written as a bare `!contains("http:")` it would
        // refuse that host too, which is the only address a picture can come from on Windows.
        let without_ours = POLICY.replace("http://lyomsg.localhost", "");
        for forbidden in ["http:", "https:", "data:", "*"] {
            assert!(
                !without_ours.contains(forbidden),
                "the policy lets {forbidden} through: {POLICY}"
            );
        }
        // `'self'` appears nowhere at all, and that is the correction rather than an oversight.
        // This document is shown inside `<iframe sandbox="">`, which gives it a unique opaque
        // origin, and `'self'` in an opaque origin matches no URL — so written that way the
        // directive said "from nowhere" and refused every picture in every message.
        assert_eq!(POLICY.matches("'self'").count(), 0);
        // The scheme this module serves on, named outright and in both of its spellings. It grants
        // a message nothing: `lyomsg` is not one of the schemes the sanitizer passes, so no message
        // can write such an address, and the only ones in a document are the names minted for that
        // render. Dropping either spelling refuses every picture on the systems that use it, which
        // is what these two lines are here to stop.
        assert!(POLICY.contains("img-src lyomsg: http://lyomsg.localhost"));
        assert!(AROUND_A_MESSAGE.contains("frame-src lyomsg: http://lyomsg.localhost"));
        // Never `'self'`: the document is shown inside `sandbox=""`, whose origin is opaque, and
        // `'self'` in an opaque origin matches nothing. This is the assertion that stops it being
        // "tidied" back into the shorter form that refuses every picture.
        assert!(!POLICY.contains("img-src 'self'"));
    }

    #[test]
    fn the_document_around_a_body_fetches_nothing_of_its_own() {
        let page = page_of("<p>Hello.</p>", None, false);

        for forbidden in ["http://", "https://", "<script", "src="] {
            assert!(!page.contains(forbidden), "the wrapper carries {forbidden}");
        }
        assert!(page.contains("no-referrer"));
    }

    fn dark() -> Surface {
        Surface::of("#282a36", "#f8f8f2").expect("plainly colours")
    }

    #[test]
    fn a_message_that_says_nothing_about_colour_wears_what_the_window_wears() {
        // Otherwise it is painted by whatever the system thinks — a black rectangle inside a
        // window that is not black, or a white one inside a window that is.
        let page = page_of("<p>Hello.</p>", Some(&dark()), false);

        assert!(page.contains("background:#282a36"), "{page}");
        assert!(page.contains("color:#f8f8f2"), "{page}");
    }

    #[test]
    fn a_link_on_the_window_s_surface_is_the_colour_of_the_text_around_it() {
        // The engine's own blue is a colour for white. On the window's dark surface it is a link
        // nobody can find, and every address in a message written as text is a link now.
        let page = page_of("<pre><a href=\"/link/x\">a</a></pre>", Some(&dark()), false);
        assert!(page.contains("a:link,a:visited{color:inherit}"), "{page}");

        // On the white a message with colours of its own assumed, its links are its own or blue.
        let page = page_of("<p style=\"color:#333\">Hello.</p>", Some(&dark()), true);
        assert!(!page.contains("a:link"), "{page}");
    }

    #[test]
    fn a_message_that_names_its_own_colours_is_shown_on_the_light_one_it_assumed() {
        // A newsletter that sets grey text and no background was written against white. Painting
        // the window's dark surface under it makes dark text on dark, which is worse than a
        // mismatched rectangle.
        let page = page_of("<p style=\"color:#333\">Hello.</p>", Some(&dark()), true);

        assert!(page.contains("background:#ffffff"), "{page}");
        assert!(!page.contains("#282a36"), "{page}");
    }

    #[test]
    fn with_nothing_to_go_on_the_surface_is_white_rather_than_whatever_the_system_says() {
        let page = page_of("<p>Hello.</p>", None, false);

        assert!(page.contains("background:#ffffff"), "{page}");
        assert!(!page.contains("color-scheme"), "{page}");
    }

    #[test]
    fn the_window_only_gets_to_say_a_colour() {
        // The value is written into a stylesheet inside the frame. Everything that is not plainly
        // a colour is refused, so the worst a wrong one can do is fail to be a colour.
        for accepted in [
            "#fff",
            "#ffffff",
            "#FFFFFFFF",
            "oklch(0.145 0 0)",
            "rgb(20, 20, 20)",
            "hsl(210 40% 12%)",
            "oklch(1 0 0 / 50%)",
        ] {
            assert_eq!(
                Surface::of(accepted, "#000"),
                Some(Surface {
                    background: accepted.to_owned(),
                    foreground: "#000".to_owned()
                }),
                "{accepted} is a colour"
            );
        }

        for refused in [
            "red;}html{background:url(https://tracker.example.net/p.png)",
            "url(https://tracker.example.net/p.png)",
            "var(--background)",
            "#12",
            "#gggggg",
            "oklch(0.1 0 0);}body{display:none",
            "image-set('a.png')",
            "",
            "   ",
            "expression(alert(1))",
        ] {
            assert_eq!(
                Surface::of(refused, "#000"),
                None,
                "{refused:?} was accepted"
            );
            assert_eq!(
                Surface::of("#000", refused),
                None,
                "{refused:?} was accepted"
            );
        }
    }

    #[test]
    fn a_colour_the_window_sent_cannot_close_the_rule_it_is_written_into() {
        // The whole of the risk in letting the interface name a colour, in one assertion.
        let page = page_of(
            "<p>Hello.</p>",
            Surface::of("#111", "}html{display:none").as_ref(),
            false,
        );

        assert!(!page.contains("display:none"), "{page}");
    }

    #[test]
    fn the_body_being_read_is_not_the_one_thrown_away() {
        // The ceiling is for bodies whose closing never arrived. The oldest held is the one being
        // read: a message is opened once and its pictures are fetched over the seconds after, while
        // every other hold in that time belongs to nothing anybody is looking at -- a development
        // build mounting twice, the colours settling, a press of "show pictures". Losing it leaves
        // the text on screen, because that page has already been delivered, and answers every
        // picture with nothing.
        let held = BodyBeingRead::default();
        let name = "e1e2e3e4".repeat(8);

        let mut reading = rendered("<p>Reading this one.</p>");
        reading.parts.push(crate::ports::body::InlinePart {
            name: name.clone(),
            content_type: "image/png".to_owned(),
            bytes: b"\x89PNG\r\n\x1a\n".to_vec(),
        });

        let token = held.hold(&reading, None).expect("a name");
        assert!(held.page(&token).is_some(), "the frame fetched it");

        // The ones held around that open, none of them looked at.
        for _ in 0..HELD_AT_ONCE {
            held.hold(&rendered("<p>Held and never read.</p>"), None)
                .expect("a name");
            // The frame is still fetching what the message on screen refers to.
            assert!(
                held.part(&name).is_some(),
                "the picture of the message on screen stopped being served"
            );
        }

        assert!(
            held.page(&token).is_some(),
            "the message on screen was thrown away while it was being read"
        );
    }

    #[test]
    fn a_message_does_not_get_to_put_the_hand_under_the_pointer() {
        // A stylesheet can say `cursor`; a `style=` attribute never could, because the attribute
        // filter has no such property. Keeping the stylesheet let a message put the hand over its
        // whole body -- the one thing this window says nowhere.
        let page = page_of(
            "<style>*{cursor:pointer!important}</style><p>Press me, surely.</p>",
            None,
            false,
        );

        let (_, after) = page
            .rsplit_once("</p>")
            .expect("the message is in the page");

        assert!(
            after.contains("cursor:default!important"),
            "the window does not have the last word on the cursor: {page}"
        );
        // And the one place the hand belongs: a link in a message is a link in a document.
        assert!(
            after.contains("a[href]") && after.contains("cursor:pointer!important"),
            "a link in a message lost its hand: {page}"
        );
    }
}
