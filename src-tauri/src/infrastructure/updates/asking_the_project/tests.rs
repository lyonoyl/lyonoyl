use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::infrastructure::updates::providers::Sealed;
use crate::infrastructure::updates::the_seal::made_elsewhere::{PUBLIC, SEALED, SEALED_FILE};
use crate::ports::updates::Token;

/// A build with no key does not look, and says so as *nothing is out* rather than as a failure.
///
/// Which is what every checkout is: the key is put in by whoever builds a release, and a person
/// who cloned this should see an application that quietly never mentions updates rather than one
/// reporting a fault about a key they were never going to have.
#[test]
fn a_build_with_no_key_never_offers_anything() {
    // Read rather than asserted outright: a release build has the key, and a test that demanded
    // its absence would fail exactly where it matters least and pass where it matters most.
    if KEY.is_none() {
        assert!(!this_build_can_be_updated());
    } else {
        assert!(this_build_can_be_updated());
    }
}

/// A place that answers from memory, and writes down what it was asked.
#[derive(Default)]
struct Place {
    /// What it hands over for the manifest, or nothing where it does not answer.
    manifest: Option<Sealed>,
    /// The files of the release of 1.1.0 it holds, by name.
    files: BTreeMap<String, Vec<u8>>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Place {
    /// A place holding the manifest in the fixture and the seal that holds over it.
    fn with_a_sealed_manifest() -> Self {
        Self {
            manifest: Some(Sealed {
                manifest: std::str::from_utf8(SEALED_FILE).expect("text").to_owned(),
                seal: SEALED.to_owned(),
            }),
            ..Self::default()
        }
    }

    /// A place holding a manifest the seal in the fixture does not hold over.
    fn with_another_manifest() -> Self {
        Self {
            manifest: Some(Sealed {
                manifest: "another manifest\n".to_owned(),
                seal: SEALED.to_owned(),
            }),
            ..Self::default()
        }
    }

    /// A place holding the archive of 1.1.0 as `bytes`.
    fn holding(bytes: &[u8]) -> Self {
        Self {
            files: BTreeMap::from([("a.app.tar.gz".to_owned(), bytes.to_vec())]),
            ..Self::default()
        }
    }

    /// What it was asked, kept after the place itself is handed to the service.
    fn asked(&self) -> Arc<Mutex<Vec<String>>> {
        Arc::clone(&self.asked)
    }
}

impl UpdateProvider for Place {
    fn name(&self) -> &str {
        "a place"
    }

    fn the_manifest(&self) -> Result<Sealed, UpdateError> {
        self.asked
            .lock()
            .expect("test lock")
            .push("the manifest".to_owned());

        self.manifest.clone().ok_or(UpdateError::CouldNotAsk)
    }

    fn a_file(
        &self,
        version: Version,
        name: &str,
        at_most: usize,
        watching: Watching<'_>,
    ) -> Result<Vec<u8>, UpdateError> {
        self.asked
            .lock()
            .expect("test lock")
            .push(format!("{version}/{name}"));

        let file = match version.to_string().as_str() {
            "1.1.0" => self.files.get(name).cloned(),
            _ => None,
        }
        .ok_or(UpdateError::CouldNotAsk)?;
        if file.len() > at_most {
            return Err(UpdateError::CouldNotAsk);
        }
        match watching(file.len() as u64, Some(file.len() as u64)) {
            true => Ok(file),
            false => Err(UpdateError::Stopped),
        }
    }
}

/// The service over `places`, in that order.
fn asking(places: Vec<Place>) -> AskingTheProject {
    AskingTheProject::new(
        places
            .into_iter()
            .map(|place| Box::new(place) as Box<dyn UpdateProvider>)
            .collect(),
        std::env::temp_dir(),
    )
}

/// The key the seal in the fixture was made with.
fn the_key() -> TheKey {
    TheKey::written_as(PUBLIC).expect("a key")
}

fn asked_of(asked: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    asked.lock().expect("test lock").clone()
}

#[test]
fn the_first_place_handing_over_a_sealed_manifest_is_the_answer() {
    let first = Place::with_a_sealed_manifest();
    let second = Place::with_a_sealed_manifest();
    let second_asked = second.asked();

    let manifest = asking(vec![first, second])
        .a_sealed_manifest(&the_key())
        .expect("the first place answered");

    assert_eq!(manifest.as_bytes(), SEALED_FILE);
    assert!(
        asked_of(&second_asked).is_empty(),
        "the second place was asked although the first answered, and learned who runs this"
    );
}

#[test]
fn a_place_that_does_not_answer_is_passed_over_for_the_next() {
    let manifest = asking(vec![Place::default(), Place::with_a_sealed_manifest()])
        .a_sealed_manifest(&the_key())
        .expect("the second place answered");

    assert_eq!(manifest.as_bytes(), SEALED_FILE);
}

/// A seal that does not hold is a place that did not answer, not a reason to stop asking: the two
/// files are cached apart for minutes after a release, and the next place may already hold both.
#[test]
fn a_place_whose_seal_does_not_hold_is_passed_over_for_the_next() {
    let manifest = asking(vec![
        Place::with_another_manifest(),
        Place::with_a_sealed_manifest(),
    ])
    .a_sealed_manifest(&the_key())
    .expect("the second place answered");

    assert_eq!(manifest.as_bytes(), SEALED_FILE);
}

#[test]
fn no_place_handing_over_a_sealed_manifest_is_a_question_that_could_not_be_asked() {
    assert_eq!(
        asking(vec![Place::default(), Place::with_another_manifest()])
            .a_sealed_manifest(&the_key()),
        Err(UpdateError::CouldNotAsk)
    );
    assert_eq!(
        asking(vec![]).a_sealed_manifest(&the_key()),
        Err(UpdateError::CouldNotAsk),
        "no place at all was taken for an answer"
    );
}

/// Bytes `abc` as the archive of 1.1.0, and what they hash to.
fn offered(file: &str) -> Offered {
    Offered {
        version: Version::parse("1.1.0").expect("a version"),
        bytes: 3,
        notes: Notes::new(),
        file: Token::of(file.to_owned()),
        digest: Token::of(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_owned(),
        ),
    }
}

#[test]
fn the_archive_is_taken_from_the_first_place_that_hands_it_over() {
    let first = Place::holding(b"abc");
    let first_asked = first.asked();
    let second = Place::holding(b"abc");
    let second_asked = second.asked();

    let archive = asking(vec![first, second])
        .the_archive(&offered("a.app.tar.gz"), &mut |_, _| true)
        .expect("the first place handed it over");

    assert_eq!(archive, b"abc");
    assert_eq!(
        asked_of(&first_asked),
        ["1.1.0/a.app.tar.gz"],
        "not the file the manifest named, in the release of the version offered"
    );
    assert!(
        asked_of(&second_asked).is_empty(),
        "the second place was asked for an archive already in hand"
    );
}

/// Every place holds the same file and the digest says which file that is, so a place without it,
/// with only part of it, or with another one is passed over rather than believed or stopped at.
#[test]
fn a_place_that_does_not_hand_over_the_file_named_is_passed_over_for_the_next() {
    for first in [
        Place::default(),
        Place::holding(b"ab"),
        Place::holding(b"abd"),
    ] {
        let archive = asking(vec![first, Place::holding(b"abc")])
            .the_archive(&offered("a.app.tar.gz"), &mut |_, _| true)
            .expect("the second place handed it over");

        assert_eq!(archive, b"abc");
    }
}

/// Somebody serving this application a file that is not its own is said to be that, wherever in
/// the order it was met, and never reported as a connection that failed.
#[test]
fn a_different_file_and_nothing_else_is_said_to_be_not_ours() {
    assert_eq!(
        asking(vec![Place::holding(b"abd"), Place::default()])
            .the_archive(&offered("a.app.tar.gz"), &mut |_, _| true),
        Err(UpdateError::NotOurs)
    );
    assert_eq!(
        asking(vec![Place::default(), Place::holding(b"abd")])
            .the_archive(&offered("a.app.tar.gz"), &mut |_, _| true),
        Err(UpdateError::NotOurs)
    );
}

#[test]
fn no_place_handing_anything_over_is_a_question_that_could_not_be_asked() {
    assert_eq!(
        asking(vec![Place::default(), Place::holding(b"ab")])
            .the_archive(&offered("a.app.tar.gz"), &mut |_, _| true),
        Err(UpdateError::CouldNotAsk)
    );
    assert_eq!(
        asking(vec![]).the_archive(&offered("a.app.tar.gz"), &mut |_, _| true),
        Err(UpdateError::CouldNotAsk)
    );
}

/// Stop is the person's own doing, and it stops the download rather than moving it elsewhere.
#[test]
fn a_person_saying_stop_stops_it_and_no_other_place_is_asked() {
    let second = Place::holding(b"abc");
    let second_asked = second.asked();

    assert_eq!(
        asking(vec![Place::holding(b"abc"), second])
            .the_archive(&offered("a.app.tar.gz"), &mut |_, _| false),
        Err(UpdateError::Stopped)
    );
    assert!(
        asked_of(&second_asked).is_empty(),
        "a download the person stopped went on at the next place"
    );
}

/// What was offered comes back from above the port, so the name in it is held to the manifest's
/// rule again, and a name that would leave the release's folder is asked of no place at all.
#[test]
fn a_file_whose_name_is_not_plain_is_asked_of_no_place() {
    for name in [
        "",
        "../1.0.9/a.app.tar.gz",
        "inner/a.app.tar.gz",
        "https://example.com/a",
    ] {
        let place = Place::holding(b"abc");
        let asked = place.asked();

        assert_eq!(
            asking(vec![place]).the_archive(&offered(name), &mut |_, _| true),
            Err(UpdateError::CouldNotAsk),
            "{name:?}"
        );
        assert!(asked_of(&asked).is_empty(), "{name:?} was asked for");
    }
}
