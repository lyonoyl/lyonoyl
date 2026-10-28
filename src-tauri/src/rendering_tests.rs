//! What the sanitizer writes and what the frame can fetch, held against each other.
//!
//! The two halves were each tested against a fake of the other: the sanitizer against a body
//! nobody served, and the fetching against a renderer that invented its own names. Between them
//! nothing said the name in `src="/…"` is the name a picture is stored under — which is the whole
//! of whether a picture ever appears.
//!
//! These live here rather than beside the use case because they wire the real adapter to the real
//! store, and a use case may not reach for an adapter even in a test. Beside the use case the
//! fakes stay and prove the orchestration; here the two real halves prove they agree.

use crate::application::open_message::put_the_pictures_in;
use crate::infrastructure::html::body_of;
use crate::infrastructure::html::serving::BodyBeingRead;
use crate::ports::body::{BodyForm, BodyKind, Link, Pictures, RenderedBody, Viewing};
use crate::ports::pictures::{FetchedPicture, PictureOutcome};

/// The corpus, which is where every parsing and rendering bug this product has found is kept.
fn corpus() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("eml")
}

/// Pictures shown, which is the setting under which a name has to reach the store.
fn shown() -> Viewing {
    Viewing {
        form: BodyForm::Sent,
        pictures: Pictures::Shown,
    }
}

#[test]
fn what_the_real_sanitizer_names_is_what_the_frame_is_handed() {
    let raw = std::fs::read(corpus().join("hostile-remote-content.eml"))
        .expect("the corpus is in the repository");

    let mut body = body_of(&raw, shown());
    assert!(!body.remote.is_empty(), "the message points at pictures");

    let all_arrived: Vec<PictureOutcome> = body
        .remote
        .iter()
        .map(|_| {
            PictureOutcome::Shown(FetchedPicture {
                content_type: "image/png".to_owned(),
                bytes: b"a picture".to_vec(),
            })
        })
        .collect();
    put_the_pictures_in(&mut body, all_arrived);

    assert_eq!(body.remote_blocked, 0, "every one of them arrived");

    for pointed_at in &body.remote {
        assert!(
            body.html.contains(&format!("src=\"/{}\"", pointed_at.name)),
            "the body does not refer to {}",
            pointed_at.name
        );
        assert!(
            body.parts.iter().any(|part| part.name == pointed_at.name),
            "nothing is stored under {}, so the frame asks for it and gets nothing",
            pointed_at.name
        );
    }

    // And the store the frame actually asks answers for each of them.
    let held = BodyBeingRead::default();
    let token = held.hold(&body, None).expect("a name");
    assert!(held.page(&token).is_some(), "the page is not served");
    for pointed_at in &body.remote {
        assert!(
            held.part(&pointed_at.name).is_some(),
            "the frame asks for {} and is answered with nothing",
            pointed_at.name
        );
    }
}

#[test]
fn a_picture_that_never_came_is_not_left_in_the_body_as_something_to_ask_for() {
    // The frame fetches what the body points at, and it has nowhere to ask but the store. A name
    // left in the document with nothing behind it is a request that comes back empty — which is a
    // broken picture drawn where the message meant one, and a second telling of a loss the reader
    // has already been counted.
    let mut body = a_body_pointing_at_two();

    put_the_pictures_in(
        &mut body,
        vec![
            PictureOutcome::Shown(FetchedPicture {
                content_type: "image/png".to_owned(),
                bytes: b"the first".to_vec(),
            }),
            PictureOutcome::CouldNotBeHad,
        ],
    );

    // Every name the body still points at is one the store can answer for.
    let held = BodyBeingRead::default();
    let token = held.hold(&body, None).expect("a name");
    assert!(held.page(&token).is_some());

    for pointed_at in &body.remote {
        let asked_for = body.html.contains(&format!("src=\"/{}\"", pointed_at.name));
        let answerable = held.part(&pointed_at.name).is_some();

        assert_eq!(
            asked_for, answerable,
            "the body points at {} and the store has nothing under it",
            pointed_at.name
        );
    }

    assert!(
        body.html.contains("src=\"/one\""),
        "the one that arrived is still shown"
    );
}

#[test]
fn no_message_in_the_corpus_is_left_pointing_at_a_picture_that_never_came() {
    // Every message, and the worst day each of them can have: not one host answers. What must come
    // out is a body that asks the store for nothing it cannot be given — a name with nothing behind
    // it is a request that comes back empty, and the frame draws that.
    let directory = corpus();
    let mut with_pictures = 0;

    for entry in std::fs::read_dir(&directory).expect("the corpus is in the repository") {
        let path = entry.expect("an entry").path();
        if path.extension().is_none_or(|kind| kind != "eml") {
            continue;
        }

        let raw = std::fs::read(&path).expect("the file is readable");
        let mut body = body_of(&raw, shown());

        if body.remote.is_empty() {
            continue;
        }
        with_pictures += 1;

        let nothing_answered: Vec<PictureOutcome> = body
            .remote
            .iter()
            .map(|_| PictureOutcome::CouldNotBeHad)
            .collect();
        put_the_pictures_in(&mut body, nothing_answered);

        let held = BodyBeingRead::default();
        let token = held.hold(&body, None).expect("a name");
        assert!(held.page(&token).is_some());

        for pointed_at in &body.remote {
            assert!(
                !body.html.contains(&format!("src=\"/{}\"", pointed_at.name)),
                "{} still points at a picture nothing answered for",
                path.display()
            );
            assert!(
                held.part(&pointed_at.name).is_none(),
                "{} holds bytes for a picture that never came",
                path.display()
            );
        }
    }

    assert!(
        with_pictures >= 4,
        "the corpus stopped carrying messages that point at pictures: {with_pictures}"
    );
}

/// A body that points at two pictures somewhere else, as a renderer would leave it.
fn a_body_pointing_at_two() -> RenderedBody {
    RenderedBody {
        attachments: Vec::new(),
        html: "<p><img src=\"/one\"><img src=\"/two\"></p>".to_owned(),
        parts: Vec::new(),
        links: Vec::new(),
        remote: vec![
            Link {
                name: "one".to_owned(),
                url: "https://example.net/one.png".to_owned(),
            },
            Link {
                name: "two".to_owned(),
                url: "https://example.net/two.png".to_owned(),
            },
        ],
        kind: BodyKind::Html,
        remote_blocked: 0,
        remote_not_shown: 0,
        remote_not_had: 0,
        truncated: false,
        declares_colours: false,
    }
}
