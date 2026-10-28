//! Fetching the pictures a message points at, on the reader's word.
//!
//! One service over the client beside it. What it adds is the part that is about pictures rather
//! than about HTTP: how many to fetch at once, how large one may be, and what a picture is allowed
//! to turn out to be. The client knows none of that, and nothing above this file knows there is an
//! HTTP client at all.

use std::sync::Arc;

use futures::stream::StreamExt;
use tokio::runtime::Handle;

use crate::infrastructure::html::picture_served_as;
use crate::infrastructure::http::HttpClient;
use crate::infrastructure::journal::journal;
use crate::ports::journal::{Journal, Note, Value, Word};
use crate::ports::pictures::{FetchedPicture, PictureOutcome, PictureSource};

/// How many are fetched at once.
///
/// A newsletter points at a dozen and a reader waits for all of them, so one at a time is a
/// visible wait. All at once is a dozen connections opened in the same instant by every account
/// that has such a message open.
const AT_ONCE: usize = 6;

/// The largest one this build will hold.
const LARGEST: usize = 8 * 1024 * 1024;

/// How many pictures of one message are fetched at all.
///
/// A message chooses how many it points at, and two megabytes of HTML has room for tens of
/// thousands of them. Past this the rest are simply not fetched, and the reader is told how many
/// pictures are not on screen.
const MOST: usize = 64;

/// How much all the pictures of one message may take together.
///
/// A hundred pictures of eight megabytes is the same memory as one of eight hundred, and only a
/// budget over the whole message notices that.
const LARGEST_TOGETHER: usize = 24 * 1024 * 1024;

pub struct RemotePictures {
    client: Arc<HttpClient>,
    /// The one runtime, borrowed rather than made: a hundred accounts must not become a hundred
    /// runtimes.
    runtime: Handle,
}

impl RemotePictures {
    pub fn new(client: Arc<HttpClient>, runtime: Handle) -> Self {
        Self { client, runtime }
    }

    /// What became of one address.
    async fn one(client: &HttpClient, url: &str) -> PictureOutcome {
        let fetched = match client.get(url, "image/*", LARGEST).await {
            Ok(fetched) => fetched,
            // Every way of not arriving is the same to a reader: the picture is not there. Which
            // of them it was belongs in a log, not in front of anyone.
            Err(why) => {
                // The reason as this build's own word, never the error's text: what a transport puts
                // in a message includes the address it was dialling.
                let word = match why {
                    crate::infrastructure::http::HttpError::Refused => Word::HostRefused,
                    crate::infrastructure::http::HttpError::TooLarge => Word::TooLarge,
                    crate::infrastructure::http::HttpError::Address => Word::NotFetchable,
                    _ => Word::HostUnreachable,
                };
                said_why(word, None, 0);
                return PictureOutcome::CouldNotBeHad;
            }
        };

        // What the host called it decides nothing: the bytes are served as the type the allowlist
        // names, never as the type the host claimed. This one did arrive, and saying it could not
        // be had would send a reader back to try again at something that cannot succeed.
        let Some(content_type) = picture_served_as(&fetched.content_type) else {
            // A picture of a kind this build does not show, or not a picture at all. The two are
            // different sentences: the first is this build's limit and asking again will not move
            // it, the second is a host that answered with a page, a redirect or a tracking
            // endpoint and may answer with the picture next time.
            said_why(Word::NotAKindShownHere, None, fetched.bytes.len());

            return match fetched.content_type.starts_with("image/") {
                true => PictureOutcome::NotAKindShownHere,
                false => PictureOutcome::CouldNotBeHad,
            };
        };

        // And what it called itself decides nothing either. A tracking pixel host that answers a
        // blocked request with an empty 200, a redirect page served as an image, a body that
        // arrived compressed — each is an answer, and counting an answer as a picture is how the
        // notice disappeared while the message still had no pictures in it. What is looked at is
        // the bytes: every format this build shows says what it is in its first few.
        if !looks_like(&fetched.bytes, content_type) {
            said_why(Word::NotThePictureClaimed, None, fetched.bytes.len());

            return PictureOutcome::CouldNotBeHad;
        }

        said_why(Word::Elsewhere, Some(true), fetched.bytes.len());

        PictureOutcome::Shown(FetchedPicture {
            content_type: content_type.to_owned(),
            bytes: fetched.bytes,
        })
    }
}

/// Why one picture is not on screen, written down.
///
/// Every way of not arriving is the same sentence to a reader, and it has to be: "the host said no"
/// and "the answer was not a picture" are not a person's problem. They are the whole of the problem
/// to somebody asked why a message renders with holes, and this is the one place that knows.
///
/// **No address, no host, and no text from the error.** A picture's address is written by whoever
/// sent the message and says who they are and who is reading; section 6 of the contract forbids it
/// in a log, and a transport error carries the address it was dialling in its own text, so the
/// error is turned into one of this build's own words before it gets anywhere near here. The size
/// is rounded to an order of magnitude by the journal for the same reason.
fn said_why(why: Word, arrived: Option<bool>, bytes: usize) {
    let journal = journal();
    if !journal.keeps(Note::Detail) {
        return;
    }

    let event = match arrived {
        Some(true) => "picture.arrived",
        _ => "picture.lost",
    };

    journal.write_down(
        Note::Detail,
        event,
        &[
            ("where", Value::Word(Word::InAnImage)),
            ("why", Value::Word(why)),
            ("bytes", Value::Bytes(bytes as u64)),
        ],
    );
}

/// Whether these bytes are the picture the header claimed.
///
/// Only the opening: enough to tell a picture from a page, which is what this is for. Nothing here
/// decides whether a picture is safe — that is the content type's job, decided before this.
fn looks_like(bytes: &[u8], content_type: &str) -> bool {
    let starts_with = |marker: &[u8]| bytes.starts_with(marker);

    match content_type {
        "image/jpeg" => starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/png" => starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/gif" => starts_with(b"GIF87a") || starts_with(b"GIF89a"),
        "image/webp" => bytes.len() > 12 && starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        "image/bmp" => starts_with(b"BM"),
        // A type this build serves and has no marker for. Anything at all is better than nothing,
        // and an empty answer is still not a picture.
        _ => !bytes.is_empty(),
    }
}

impl PictureSource for RemotePictures {
    fn fetch(&self, urls: &[String]) -> Vec<PictureOutcome> {
        let client = Arc::clone(&self.client);
        let asked: Vec<String> = urls.iter().take(MOST).cloned().collect();
        let wanted = urls.len();

        self.runtime.block_on(async move {
            let mut stream = futures::stream::iter(asked)
                .map(|url| {
                    let client = Arc::clone(&client);
                    async move { Self::one(&client, &url).await }
                })
                // In order and a few at a time: what comes back lines up with what went in, which
                // is what lets the caller put each picture where the body expects it.
                .buffered(AT_ONCE);

            let mut fetched: Vec<PictureOutcome> = Vec::with_capacity(wanted);
            let mut held = 0usize;

            while let Some(one) = stream.next().await {
                // A budget over the whole message, not only over each picture: a hundred pictures
                // of eight megabytes each is the same memory as one of eight hundred, and a message
                // chooses how many it points at.
                match one {
                    PictureOutcome::Shown(picture)
                        if held + picture.bytes.len() <= LARGEST_TOGETHER =>
                    {
                        held += picture.bytes.len();
                        fetched.push(PictureOutcome::Shown(picture));
                    }
                    PictureOutcome::Shown(_) => fetched.push(PictureOutcome::CouldNotBeHad),
                    other => fetched.push(other),
                }
            }

            // The ones past the cap were never asked for. They line up as not had, so the reader is
            // told how many pictures are not on screen rather than left wondering.
            fetched.resize_with(wanted, || PictureOutcome::CouldNotBeHad);

            fetched
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::http::testing::host;

    /// Stand-in bytes. Nothing here parses a picture — what is being checked is that the bytes
    /// arrive whole and are labelled by this build rather than by the host.
    /// A real picture opening, and one whose first bytes happen to be letters — which is why the
    /// fixture is a GIF. A body of made-up letters is exactly what this build now refuses.
    const PICTURE: &[u8] = b"GIF89a00";

    fn service(runtime: &tokio::runtime::Runtime) -> RemotePictures {
        let road = Arc::new(crate::infrastructure::net::TransportPolicy::new().expect("a policy"));

        RemotePictures::new(
            Arc::new(HttpClient::reaching_anywhere(road)),
            runtime.handle().clone(),
        )
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime")
    }

    #[test]
    fn a_picture_is_fetched_and_served_as_what_this_build_shows() {
        let (url, _asked) =
            host("HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: 8\r\n\r\nGIF89a00");
        let runtime = runtime();

        let fetched = service(&runtime).fetch(&[url]);

        assert_eq!(fetched.len(), 1);
        let PictureOutcome::Shown(picture) = &fetched[0] else {
            panic!("the picture arrived: {:?}", fetched[0]);
        };
        assert_eq!(picture.content_type, "image/gif");
        assert_eq!(picture.bytes, PICTURE);
    }

    #[test]
    fn a_drawing_is_served_as_what_it_is_and_not_as_what_it_claimed() {
        let (url, _asked) = host(
            "HTTP/1.1 200 OK\r\nContent-Type: image/svg+xml\r\nContent-Length: 5\r\n\r\n<svg>",
        );
        let runtime = runtime();

        // Shown, and shown as a drawing: the type it goes out under is the one the allowlist
        // names, never the one the host wrote. What keeps that safe is that a picture is reachable
        // as an image and in no other way.
        let fetched = service(&runtime).fetch(&[url]);
        let PictureOutcome::Shown(picture) = &fetched[0] else {
            panic!("the drawing was refused: {:?}", fetched[0]);
        };
        assert_eq!(picture.content_type, "image/svg+xml");
    }

    #[test]
    fn a_host_that_answers_something_else_leaves_that_picture_missing() {
        // One host that is down, or answering with a page, is not a message that cannot be read.
        // A page is not "a kind this build does not show" either: nothing was refused here, and a
        // host that answers with a redirect now may answer with the picture in a minute.
        let (refused, _a) = host("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        let (page, _b) =
            host("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 6\r\n\r\n<html>");
        let runtime = runtime();

        assert_eq!(
            service(&runtime).fetch(&[refused, page]),
            vec![PictureOutcome::CouldNotBeHad, PictureOutcome::CouldNotBeHad]
        );
    }

    #[test]
    fn a_kind_this_build_does_not_show_is_not_a_host_that_failed() {
        // Both end with nothing on screen and they are not the same thing to say. One is this
        // build's own limit; the other is a host that may answer differently next time.
        let (icon, _b) =
            host("HTTP/1.1 200 OK\r\nContent-Type: image/x-icon\r\nContent-Length: 4\r\n\r\nicon");
        let (page, _c) =
            host("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 6\r\n\r\n<html>");
        let runtime = runtime();

        assert_eq!(
            service(&runtime).fetch(&[icon, page]),
            vec![
                PictureOutcome::NotAKindShownHere,
                PictureOutcome::CouldNotBeHad,
            ]
        );
    }

    #[test]
    fn what_comes_back_lines_up_with_what_went_in() {
        // The caller puts each picture where the body expects it, by position. An answer out of
        // order would put one message's picture under another's name.
        let (first, _a) =
            host("HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: 8\r\n\r\nGIF89a00");
        let (missing, _b) = host("HTTP/1.1 500 Nope\r\nContent-Length: 0\r\n\r\n");
        let (last, _c) =
            host("HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: 6\r\n\r\nGIF87a");
        let runtime = runtime();

        let fetched = service(&runtime).fetch(&[first, missing, last]);

        assert_eq!(fetched.len(), 3);
        let shown = |outcome: &PictureOutcome| match outcome {
            PictureOutcome::Shown(picture) => picture.content_type.clone(),
            other => panic!("expected a picture, got {other:?}"),
        };
        assert_eq!(shown(&fetched[0]), "image/gif");
        assert_eq!(fetched[1], PictureOutcome::CouldNotBeHad);
        assert_eq!(shown(&fetched[2]), "image/gif");
    }

    #[test]
    fn a_message_may_not_point_at_more_pictures_than_this_build_fetches() {
        // The finding this closes: nothing capped how many one message could ask for, and two
        // megabytes of HTML has room for tens of thousands of addresses.
        let (url, _asked) =
            host("HTTP/1.1 200 OK\r\nContent-Type: image/gif\r\nContent-Length: 8\r\n\r\nGIF89a00");
        let many: Vec<String> = std::iter::repeat_n(url, MOST + 40).collect();
        let runtime = runtime();

        let fetched = service(&runtime).fetch(&many);

        assert_eq!(
            fetched.len(),
            many.len(),
            "the answer lines up with the question"
        );
        assert!(
            fetched
                .iter()
                .skip(MOST)
                .all(|outcome| *outcome == PictureOutcome::CouldNotBeHad),
            "something past the cap was fetched"
        );
    }

    /// The budget has to leave room for one picture and cost less than all of them: checked where
    /// the numbers are, when the file is compiled.
    const _: () = assert!(LARGEST_TOGETHER >= LARGEST);
    const _: () = assert!(LARGEST_TOGETHER <= MOST * LARGEST);

    #[test]
    fn nothing_asked_for_is_nothing_fetched() {
        let runtime = runtime();

        assert!(service(&runtime).fetch(&[]).is_empty());
    }
}

#[cfg(test)]
mod what_counts_as_a_picture {
    use super::*;

    #[test]
    fn a_picture_is_recognised_by_how_it_starts() {
        assert!(looks_like(&[0xFF, 0xD8, 0xFF, 0xE0], "image/jpeg"));
        assert!(looks_like(b"\x89PNG\r\n\x1a\nrest", "image/png"));
        assert!(looks_like(b"GIF89a...", "image/gif"));
        assert!(looks_like(b"RIFF\0\0\0\0WEBPVP8 ", "image/webp"));
    }

    #[test]
    fn an_empty_answer_is_not_a_picture() {
        // A tracking host answering a blocked request with an empty 200. Counting it as fetched is
        // how the notice disappeared while the message still had no pictures in it.
        assert!(!looks_like(b"", "image/png"));
        assert!(!looks_like(b"", "image/jpeg"));
    }

    #[test]
    fn a_page_served_as_a_picture_is_not_one() {
        assert!(!looks_like(b"<!doctype html><html>", "image/png"));
        assert!(!looks_like(b"{\"error\":\"blocked\"}", "image/jpeg"));
    }

    #[test]
    fn a_type_with_no_marker_is_taken_at_its_word_unless_it_is_empty() {
        // Being unable to check is not a reason to refuse; being handed nothing is.
        assert!(looks_like(b"anything at all", "image/x-something"));
        assert!(!looks_like(b"", "image/x-something"));
    }
}
