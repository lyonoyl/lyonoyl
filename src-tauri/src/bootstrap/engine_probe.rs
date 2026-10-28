//! Asking the engine what it does, and writing down the answer.
//!
//! Three of the four layers message rendering rests on are properties of a webview engine, and
//! each engine is asked rather than reasoned about: `docs/platforms/differences.md` records what each answered.
//!
//! The questions are answerable by looking at a window — does a message appear, is the picture
//! there — but two of them are not, and looking is not repeatable. Whether the policy in the
//! response header is **enforced** cannot be seen at all: a policy that is ignored looks like a
//! policy that is obeyed until something reaches the network. And an answer obtained by looking
//! once says nothing about the engine after its next update.
//!
//! So the frame is asked instead of watched. Everything it fetches comes from the scheme handler
//! in this process, which counts what it was asked for; and one thing it must **not** fetch is
//! pointed at a socket in this process, which records anything that arrives. What comes out is a
//! table of numbers that means the same thing on every platform and can be run again.
//!
//! Debug builds only. It opens a window, renders one message, waits, writes a report and stops.

use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::infrastructure::html::serving;
use crate::ports::body::{BodyKind, InlinePart, RenderedBody};

/// The environment variable that turns an ordinary start into a probe.
pub const ASKED_FOR: &str = "LYONOYL_ENGINE_PROBE";

/// How long the engine is given to fetch everything it means to fetch.
///
/// Generous: this is a cold webview on a machine that may be a virtual one, and the cost of waiting
/// too long is a slow report while the cost of waiting too little is a wrong one.
const LONG_ENOUGH: Duration = Duration::from_secs(10);

/// A picture nothing may fetch: one pixel, referred to by an address of its own.
const ONE_PIXEL: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

/// What the probe found, in the terms the questions were asked in.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Findings {
    pub platform: &'static str,
    /// Whether the frame fetched the message. Asked of the message's own name: the page the probe
    /// puts the message in comes from the same scheme, and is not the frame loading anything.
    pub frame_loaded_the_document: bool,
    /// Whether the frame fetched the picture the message carried inside it.
    pub inline_picture_was_fetched: bool,
    /// **The one nobody can see.** The document asks for a picture from a host on the open network,
    /// and the policy served with it says `img-src lyomsg:`. `true` here means the engine ignored
    /// the policy in the response header, and a message can reach the network from inside the
    /// frame — which is the second containment layer gone.
    pub policy_was_ignored_and_something_left: bool,
    /// What arrived at the socket, when anything did. Kept so a surprising `true` can be read
    /// rather than guessed at.
    pub what_left: Option<String>,
    /// The path of every request that arrived, which is what tells the five ways apart: the picture
    /// from elsewhere, the `@import`, the font, the background and the cursor.
    pub paths_that_left: Vec<String>,
    /// Whether the window went on answering while the message was laid out.
    ///
    /// A message sitting on the bound this build allows, and a small piece of work handed to the
    /// main thread over and over while the engine works on it. What is measured is **how long each
    /// piece waited**, not how many arrived: work handed to a blocked loop is not lost, it queues
    /// and runs late, so counting arrivals says only that the freeze ended. The longest wait is the
    /// freeze — a person with no sidebar, no list and no way to leave the message, for that many
    /// milliseconds.
    pub the_window_kept_answering: bool,
    pub longest_wait_ms: u64,
    pub main_loop_answered: u32,
    /// Whether anything tried to leave the message frame without being asked to.
    ///
    /// The message carries a link, and nothing here clicks it — a document under `sandbox` cannot
    /// even refresh itself to one, which was measured rather than assumed. So this is not the row
    /// about a person following a link; that one still needs a person. What it catches is an engine
    /// going somewhere out of a message frame on its own, which must never happen and which nothing
    /// was watching for.
    pub anything_navigated_out_of_the_frame: bool,
    /// Every request the frame made of the scheme: the document's, and each picture's and link's,
    /// which are looked up as a page first. The page the probe puts the message in is not one.
    pub pages_asked_for: u32,
    pub parts_asked_for: u32,
    pub links_asked_for: u32,
    /// Asked for and not here: a picture the store no longer had when the frame came for it.
    pub parts_missed: u32,
}

/// The path out of a request line, so a report names the vector rather than quoting a browser.
fn the_path_of(said: &str) -> String {
    said.split_whitespace()
        .nth(1)
        .unwrap_or("?")
        .trim_start_matches('/')
        .to_owned()
}

/// A socket nothing in this application will ever connect to.
///
/// Its address goes into the document as a picture the message points at. Under the policy the
/// frame is served with, that fetch must not happen — so anything arriving here is the engine
/// having ignored the policy, and the bytes are kept so the answer can be read rather than assumed.
struct Doorbell {
    address: SocketAddr,
    rang: Arc<AtomicU32>,
    heard: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Doorbell {
    fn listening() -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        let rang = Arc::new(AtomicU32::new(0));
        // Every arrival rather than the first: the message reaches for this socket five ways now,
        // and "something left" is a different answer from "the font left and the background did
        // not". Which vector got out is the whole use of the row it answers.
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));

        let counted = rang.clone();
        let kept = heard.clone();
        std::thread::Builder::new()
            .name("engine-probe-doorbell".to_owned())
            .spawn(move || {
                for arriving in listener.incoming() {
                    let Ok(mut stream) = arriving else { continue };

                    // What was said is kept **before** the count goes up, and the order is the
                    // whole point: the count is what everything else waits on, so a count that
                    // rises first leaves a moment where the answer is "the policy was ignored" and
                    // "nothing is known about what left" at the same time. That is a report nobody
                    // can act on, and it showed up as this file's own test failing under load.
                    let mut said = [0u8; 512];
                    let read = stream.read(&mut said).unwrap_or(0);
                    if let Ok(mut kept) = kept.lock() {
                        // Bounded, because a stylesheet that got out may keep getting out.
                        if kept.len() < 16 {
                            kept.push(String::from_utf8_lossy(&said[..read]).into_owned());
                        }
                    }

                    counted.fetch_add(1, Ordering::Release);
                }
            })?;

        Ok(Self {
            address,
            rang,
            heard,
        })
    }

    /// Whether anything connected — and, when it did, what it said is already recorded.
    fn rang(&self) -> bool {
        self.rang.load(Ordering::Acquire) > 0
    }
}

/// The one message the engine is shown.
///
/// Built here rather than parsed from a fixture, because what matters is exactly what is in it:
/// two pictures that came with the message and must be fetched -- one of them a drawing, which is
/// the kind an engine may treat differently -- and a picture from elsewhere that must not be.
/// None is written through the sanitizer: the point is to ask the engine about the policy it was
/// served, not about what this application would have removed first.
fn the_message(doorbell: SocketAddr) -> RenderedBody {
    let inline = serving::random_name().unwrap_or_else(|| "picture".to_owned());
    let drawing = serving::random_name().unwrap_or_else(|| "drawing".to_owned());
    let link = serving::random_name().unwrap_or_else(|| "link".to_owned());

    RenderedBody {
        attachments: Vec::new(),
        html: format!(
            // Five reaches for the socket and none may arrive: a picture from elsewhere, and a
            // stylesheet of the message's own trying it four ways. The stylesheet is the half that
            // was never asked — a `style=` attribute is filtered declaration by declaration, and a
            // message's own `<style>` block is kept as written, so the only thing between it and
            // the network is the policy in the response header. That is the thing this measures.
            //
// And a link the message offers. Nothing here clicks it, and nothing can: a document under
            // `sandbox` has the sandboxed automatic features flag set, so a `meta refresh` does not
            // fire either — measured, not assumed. What the link is here for is the other half of
            // the question: if an engine ever goes somewhere out of a message frame **without**
            // being clicked, the handler below records it.
            //
            // The font is applied rather than only declared. Declared and unused it is never
            // fetched, so the row it answers would have come back quiet whatever the policy said —
            // which is a fixture that does not create the condition it checks. The cursor cannot be
            // made to fetch without a pointer over the element, and is written down as such.
            // As many elements as the product lets a message have, and a rule that costs something
            // to lay out on each one. The number is asked of the product rather than copied: it is
            // chosen because 46,000 stopped a window answering and 8,000 did not, and a probe
            // carrying its own copy would keep measuring the old answer after somebody moved it.
            "<style>\
             @import url(http://{doorbell}/imported.css);\
             @font-face{{font-family:probe;src:url(http://{doorbell}/a-font.woff2)}}\
             #probe{{background-image:url(http://{doorbell}/a-background.png);font-family:probe,serif}}\
             p{{cursor:url(http://{doorbell}/a-cursor.png),auto}}\
             </style>\
             <p id=\"probe\">The engine probe.</p>\
             <a id=\"away\" href=\"/{links}/{link}\">a link the message offered</a>\
             <img id=\"inline\" src=\"/{inline}\" alt=\"came with the message\">\
             <img id=\"drawing\" src=\"/{drawing}\" alt=\"a drawing that came with it\">\
             <img id=\"remote\" src=\"http://{doorbell}/must-not-be-fetched.png\" alt=\"elsewhere\">\
             <div id=\"cost\">{costly}</div>",
            links = serving::LINKS,
            costly = costly_to_lay_out(),
        ),
        parts: vec![
            InlinePart {
                name: inline,
                content_type: "image/png".to_owned(),
                bytes: ONE_PIXEL.to_vec(),
            },
            InlinePart {
                name: drawing,
                content_type: "image/svg+xml".to_owned(),
                bytes: br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="red"/></svg>"#.to_vec(),
            },
        ],
        links: vec![crate::ports::body::Link {
            name: link,
            url: "https://elsewhere.invalid/where-the-link-goes".to_owned(),
        }],
        remote: Vec::new(),
        kind: BodyKind::Html,
        remote_blocked: 0,
        remote_not_shown: 0,
        remote_not_had: 0,
        truncated: false,
        declares_colours: false,
    }
}

/// A document as expensive as this build lets one be.
///
/// The bound exists because layout cost is what nothing else bounded: measured on one engine,
/// 46,000 elements stopped a window answering at all and 8,000 cost it nothing. That was one
/// engine and one person watching. What this makes is a message sitting exactly on the bound, so
/// the same question can be asked of an engine by counting whether its window kept answering
/// rather than by watching it.
fn costly_to_lay_out() -> String {
    let each = "<span class=\"c\">x</span>";
    let mut written = String::with_capacity(
        crate::infrastructure::html::nesting::MOST_ELEMENTS * each.len() + 256,
    );

    // A rule that has to be resolved per element rather than once.
    written.push_str(
        "<style>#cost .c{border:1px solid;padding:1px;margin:1px;display:inline-block;\
         box-shadow:0 0 1px rgba(0,0,0,.2);transform:rotate(0.01deg)}</style>",
    );
    for _ in 0..crate::infrastructure::html::nesting::MOST_ELEMENTS {
        written.push_str(each);
    }

    written
}

/// A real message, through the real path, for the window to answer about.
///
/// Everything is the real thing -- the sanitizer, the fetcher, the network, the store -- so what
/// the frame asks for and what the store could not answer become numbers about an ordinary
/// message rather than about a message written to be measured.
fn a_real_message(name: &str) -> Result<RenderedBody, String> {
    use crate::ports::pictures::PictureSource;

    let raw = std::fs::read(name)
        .or_else(|_| std::fs::read(format!("tests/fixtures/eml/{name}.eml")))
        .or_else(|_| std::fs::read(format!("../tests/fixtures/eml/{name}.eml")))
        .map_err(|error| format!("no {name}: {error}"))?;

    let mut body = crate::infrastructure::html::body_of(
        &raw,
        crate::ports::body::Viewing {
            form: crate::ports::body::BodyForm::Sent,
            pictures: crate::ports::body::Pictures::Shown,
        },
    );

    let urls: Vec<String> = body.remote.iter().map(|at| at.url.clone()).collect();

    // On a thread of its own, and waited for: the fetcher blocks until every answer is in, and
    // this runs on the thread that goes on to be the event loop. Blocking that means nothing pumps
    // the webview -- the window exists and never loads, which is the failure this probe is built
    // to tell from a real one.
    let fetched = std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().map_err(|_| "no runtime".to_owned())?;
        let road = Arc::new(
            crate::infrastructure::net::TransportPolicy::new()
                .map_err(|_| "no trust policy".to_owned())?,
        );
        let client = Arc::new(crate::infrastructure::http::HttpClient::new(road));
        let source = crate::infrastructure::http::pictures::RemotePictures::new(
            client,
            runtime.handle().clone(),
        );

        Ok::<_, String>(source.fetch(&urls))
    })
    .join()
    .map_err(|_| "the fetching thread died".to_owned())??;

    // The application's own step, called rather than copied. A probe carrying its own version of
    // this measures the version it carries.
    crate::application::open_message::put_the_pictures_in(&mut body, fetched);

    eprintln!(
        "probe: {} pictures pointed at, {} in hand, {} held in all",
        body.remote.len(),
        body.parts.len(),
        body.parts.len()
    );

    Ok(body)
}

/// Which platform this is, in the words `docs/platforms/differences.md` uses.
const fn platform() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macOS (WKWebView)"
    }
    #[cfg(target_os = "windows")]
    {
        "Windows (WebView2)"
    }
    #[cfg(target_os = "linux")]
    {
        "Linux (WebKitGTK)"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        "unknown"
    }
}

/// Opens the frame, and arranges for the answer to be taken once the engine has had its chance.
///
/// **Returns at once, and that is the whole of why this works.** This runs in `setup`, on the
/// thread that goes on to be the event loop, before it is one. Waiting on that thread means nothing
/// pumps the webview: the window exists and never loads, and the report says the frame did not
/// fetch the document on an engine where the reader plainly works. An instrument that reports the
/// failure it was built to detect is worse than no instrument, because it is read as an answer.
///
/// So the waiting happens on a thread of its own, and the event loop is left to do its work.
pub fn ask_the_engine<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    bodies: &serving::BodyBeingRead,
) -> Result<(), String> {
    let doorbell =
        Doorbell::listening().map_err(|error| format!("no socket to listen on: {error}"))?;

    // `1` asks the four questions. A path or a fixture name shows that message instead, fetched
    // for real, so the window answers for itself about a message somebody actually received.
    let asked_for = std::env::var(ASKED_FOR).unwrap_or_default();
    let showing = match asked_for.as_str() {
        "" | "1" | "true" | "yes" => the_message(doorbell.address),
        name => a_real_message(name)?,
    };

    let token = bodies
        .hold(&showing, None)
        .ok_or("this machine has no randomness to spare")?;

    // The frame, opened the way the reader opens one, which is the whole point. A window whose
    // content is the message directly gives the document an ordinary origin; the reader sees it
    // inside `<iframe sandbox="">`, whose origin is opaque. Measured on WKWebView, the difference
    // decides whether a picture is fetched at all, so a probe that skips the frame reports the
    // engine working while the product shows empty boxes.
    let around = bodies
        .hold_a_frame_around(&token)
        .ok_or("this machine has no randomness to spare")?;

    let before = Counted::of(bodies, &token, &around);

    // Refused here, and recorded. The reader's window refuses a navigation out of the message the
    // same way and then asks the person about it; without the same handler this window would be
    // answering a question nobody asks, because the thing that decides would not be there.
    let refused = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let noticed = Arc::clone(&refused);

    tauri::WebviewWindowBuilder::new(
        app,
        "engine-probe",
        tauri::WebviewUrl::External(
            serving::url_for(&around)
                .parse()
                .map_err(|error| format!("the address is not one: {error}"))?,
        ),
    )
    .title("Lyonoyl engine probe")
    .inner_size(640.0, 480.0)
    .on_navigation(move |url| {
        // The two the probe opens itself are the wrapper and the message. Anything else is the
        // message trying to go somewhere, which is what is being watched for.
        let ours = url.path().trim_start_matches('/');
        let going_away = url.scheme() == serving::SCHEME && ours.starts_with(serving::LINKS);

        if going_away {
            noticed.store(true, Ordering::Release);
        }

        !going_away
    })
    .build()
    .map_err(|error| format!("no window to ask in: {error}"))?;

    // A small piece of work handed to the main thread over and over while the engine lays the
    // message out, and a count of how many came back. A window that has stopped answering is a
    // person with no sidebar, no list and no way to leave the message -- and from outside it looks
    // exactly like a message that is merely slow, which is why nothing but somebody watching could
    // tell them apart until now.
    let longest = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let came_back = Arc::new(AtomicU32::new(0));
    let waiting = Arc::clone(&longest);
    let returning = Arc::clone(&came_back);
    let beating = app.clone();

    std::thread::Builder::new()
        .name("engine-probe-pulse".to_owned())
        .spawn(move || {
            let until = std::time::Instant::now() + LONG_ENOUGH;
            while std::time::Instant::now() < until {
                let held = Arc::clone(&waiting);
                let counted = Arc::clone(&returning);
                let posted = std::time::Instant::now();

                let _ = beating.run_on_main_thread(move || {
                    let waited = posted.elapsed().as_millis() as u64;
                    held.fetch_max(waited, Ordering::AcqRel);
                    counted.fetch_add(1, Ordering::Release);
                });

                std::thread::sleep(Duration::from_millis(50));
            }
        })
        .map_err(|error| format!("no thread to beat on: {error}"))?;

    let asked = bodies.clone();
    let (message, wrapper) = (token.clone(), around.clone());
    std::thread::Builder::new()
        .name("engine-probe".to_owned())
        .spawn(move || {
            // Waited out rather than polled to a condition: what is being measured includes things
            // that never happen, and there is no event for those.
            std::thread::sleep(LONG_ENOUGH);

            let found = Counted::of(&asked, &message, &wrapper).against(
                before,
                &doorbell,
                refused.load(Ordering::Acquire),
                longest.load(Ordering::Acquire),
                came_back.load(Ordering::Acquire),
            );
            write_down(&Ok(found.clone()));

            // The process ends itself rather than through the framework: on Linux
            // `AppHandle::exit` left the shell a zero while the same run's report said the policy
            // had been ignored. The report is written and flushed before this line.
            let _ = std::io::Write::flush(&mut std::io::stdout());
            std::process::exit(found.exit_code());
        })
        .map_err(|error| format!("no thread to wait on: {error}"))?;

    Ok(())
}

/// What the store had been asked for at one moment.
#[derive(Clone, Copy)]
struct Counted {
    pages: u32,
    parts: u32,
    links: u32,
    missed: u32,
    /// How many times the message's own page was answered, and the page the probe puts it in.
    message: u32,
    wrapper: u32,
}

impl Counted {
    fn of(bodies: &serving::BodyBeingRead, message: &str, wrapper: &str) -> Self {
        let asked = bodies.asked();
        Self {
            pages: asked.pages.load(Ordering::Relaxed),
            parts: asked.parts.load(Ordering::Relaxed),
            links: asked.links.load(Ordering::Relaxed),
            missed: bodies.asked().missed.load(Ordering::Acquire),
            message: bodies.times_answered(message),
            wrapper: bodies.times_answered(wrapper),
        }
    }

    /// What happened between then and now.
    fn against(
        self,
        before: Self,
        doorbell: &Doorbell,
        navigated: bool,
        waited: u64,
        answered: u32,
    ) -> Findings {
        Findings {
            platform: platform(),
            frame_loaded_the_document: self.message > before.message,
            inline_picture_was_fetched: self.parts > before.parts,
            policy_was_ignored_and_something_left: doorbell.rang(),
            what_left: doorbell
                .heard
                .lock()
                .ok()
                .and_then(|held| held.first().cloned()),
            paths_that_left: doorbell
                .heard
                .lock()
                .map(|held| held.iter().map(|said| the_path_of(said)).collect())
                .unwrap_or_default(),
            // A quarter of a second is already a stutter a person feels; anything past it is the
            // window not answering rather than the window being slow.
            the_window_kept_answering: waited < 250,
            longest_wait_ms: waited,
            main_loop_answered: answered,
            anything_navigated_out_of_the_frame: navigated,
            // Two counts read a moment apart, so a request landing between them may put the
            // wrapper's ahead; that is never allowed to take the frame's below nothing.
            pages_asked_for: (self.pages - before.pages)
                .saturating_sub(self.wrapper - before.wrapper),
            parts_asked_for: self.parts - before.parts,
            links_asked_for: self.links - before.links,
            parts_missed: self.missed - before.missed,
        }
    }
}

impl Findings {
    /// What this run says, in the one number a shell can branch on.
    ///
    /// `2` is the answer that matters: the policy served with the frame was not enforced. `3` is a
    /// frame that never fetched the message, which leaves the other answers saying nothing.
    fn exit_code(&self) -> i32 {
        if self.policy_was_ignored_and_something_left {
            2
        } else if !self.frame_loaded_the_document {
            3
        } else {
            0
        }
    }
}

/// Writes the findings where somebody driving this over a shell can read them.
///
/// A file beside the working directory and the same thing on standard output: the machine this runs
/// on is a virtual one somebody set up to answer four questions, reached over SSH, and a report
/// that only exists in a window nobody is looking at is not a report.
pub fn write_down(found: &Result<Findings, String>) {
    let written = match found {
        Ok(found) => serde_json::to_string_pretty(found)
            .unwrap_or_else(|_| "{\"error\":\"the findings could not be written\"}".to_owned()),
        Err(why) => format!("{{\"error\":{}}}", serde_json::Value::String(why.clone())),
    };

    println!("{written}");
    let _ = std::fs::write("engine-probe.json", format!("{written}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn the_message_points_at_the_socket_that_must_stay_quiet() {
        // The whole measurement rests on the document actually asking for it. A probe whose
        // document never mentions the address would report "nothing left" on an engine that
        // ignores the policy entirely.
        let doorbell = Doorbell::listening().expect("a socket");
        let shown = the_message(doorbell.address);

        assert!(
            shown.html.contains(&doorbell.address.to_string()),
            "the document does not point at the socket: {}",
            shown.html
        );
        assert!(!doorbell.rang(), "something connected before anything ran");
    }

    #[test]
    fn the_message_carries_a_picture_of_its_own_that_the_frame_must_fetch() {
        // The other half: a fetch that must happen, so a report of "nothing was fetched" can be
        // told apart from a frame that never loaded at all.
        let doorbell = Doorbell::listening().expect("a socket");
        let shown = the_message(doorbell.address);

        let part = shown.parts.first().expect("a picture travelled with it");
        assert!(
            shown.html.contains(&format!("src=\"/{}\"", part.name)),
            "the document does not refer to the picture it carries"
        );
        assert_eq!(part.content_type, "image/png");
        assert!(part.bytes.starts_with(b"\x89PNG"));
    }

    #[test]
    fn the_number_a_gate_branches_on_says_what_happened() {
        let found = |ignored: bool, loaded: bool| Findings {
            platform: platform(),
            frame_loaded_the_document: loaded,
            inline_picture_was_fetched: loaded,
            policy_was_ignored_and_something_left: ignored,
            what_left: None,
            paths_that_left: Vec::new(),
            the_window_kept_answering: true,
            longest_wait_ms: 0,
            main_loop_answered: 0,
            anything_navigated_out_of_the_frame: false,
            pages_asked_for: 1,
            parts_asked_for: 1,
            links_asked_for: 0,
            parts_missed: 0,
        };

        assert_eq!(found(false, true).exit_code(), 0, "a clean run is a zero");
        assert_eq!(
            found(true, true).exit_code(),
            2,
            "a policy that was not enforced is a two"
        );
        assert_eq!(
            found(false, false).exit_code(),
            3,
            "a frame that never loaded is a three"
        );
        assert_eq!(
            found(true, false).exit_code(),
            2,
            "a policy that was not enforced outranks a frame that did not load"
        );
    }

    #[test]
    fn the_page_around_the_message_is_not_the_frame_loading_it() {
        // The probe's window fetches its own page from the same scheme before the frame inside asks
        // for anything. Counted as the frame's, it is a run that says the message loaded when the
        // frame fetched nothing at all.
        let doorbell = Doorbell::listening().expect("a socket to listen on");
        let bodies = serving::BodyBeingRead::default();
        let message = bodies
            .hold(&the_message(doorbell.address), None)
            .expect("a name");
        let around = bodies.hold_a_frame_around(&message).expect("a name");
        let before = Counted::of(&bodies, &message, &around);
        let now =
            || Counted::of(&bodies, &message, &around).against(before, &doorbell, false, 0, 0);

        assert!(bodies.page(&around).is_some());
        let wrapper_only = now();
        assert!(!wrapper_only.frame_loaded_the_document);
        assert_eq!(
            wrapper_only.pages_asked_for, 0,
            "the window's own page is not the frame's"
        );
        assert_eq!(wrapper_only.exit_code(), 3);

        assert!(bodies.page(&message).is_some());
        let loaded = now();
        assert!(loaded.frame_loaded_the_document);
        assert_eq!(loaded.pages_asked_for, 1);
        assert_eq!(loaded.exit_code(), 0);
    }

    #[test]
    fn the_doorbell_hears_anything_that_connects() {
        // The detector itself, checked — because a silent report from a broken listener reads
        // exactly like a policy that held.
        let doorbell = Doorbell::listening().expect("a socket");
        assert!(!doorbell.rang());

        let mut knocking = std::net::TcpStream::connect(doorbell.address).expect("it is listening");
        use std::io::Write;
        let _ = knocking.write_all(b"GET /must-not-be-fetched.png HTTP/1.1\r\n\r\n");
        drop(knocking);

        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until && !doorbell.rang() {
            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(doorbell.rang(), "the doorbell did not hear a connection");
        let heard = doorbell.heard.lock().expect("nothing poisoned it").clone();
        assert!(
            heard
                .iter()
                .any(|said| said.contains("must-not-be-fetched")),
            "what arrived was not kept: {heard:?}"
        );
        // And the path is pulled out of it, because a report that quotes a browser says which
        // browser rather than which way out.
        assert_eq!(
            heard.first().map(|said| the_path_of(said)).as_deref(),
            Some("must-not-be-fetched.png")
        );
    }
}
