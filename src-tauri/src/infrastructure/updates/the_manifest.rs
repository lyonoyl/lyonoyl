//! What the project says is out, and what has to be true of it before anybody is told.
//!
//! One document, signed as a whole. Everything an update needs is inside it — the version, the file
//! of the archive for each kind of Mac in that version's release, how large it is and what it hashes
//! to, and what changed — so there is one thing to verify and no way for a release to go out
//! half-signed. It names files and never addresses: where a release is to be had is the providers'
//! business, and the same manifest is read whichever place handed it over.
//!
//! **The manifest names one archive and no other.** A seal says the key made a file; the digest says
//! which file. Without it, any archive ever sealed with that key and carrying the offered version
//! would pass — the one built for the other kind of Mac among them, served by whoever can change
//! what a release address answers with. The digest is also what lets any place hand the archive
//! over: what comes down is taken by what it hashes to, never by where it came from.
//!
//! **The version is inside the signed text, and that is the point of putting it there.** A
//! signature says who made a file; it does not say which file you were promised. With the version
//! outside, somebody who can influence what this application reads serves a genuine older release
//! with a hole that was fixed since, and every check still passes.
//!
//! **Its shape is fixed before the first copy reads it.** Every field is refused unless this build
//! knows it, so a field added later is a manifest every earlier copy refuses — which is why each kind
//! of Mac has its own archive from the first release. A manifest of another shape is a file at
//! another address, beside this one.
//!
//! **Nothing here is asked of an interface that counts requests.** The manifest is a file rather
//! than an answer from a service, so a hundred copies asking once a day cost the project nothing and
//! need no account.

use std::collections::BTreeMap;

use serde::Deserialize;

use lyonoyl_domain::version::Version;

use std::fmt::Write;

use crate::ports::updates::{Notes, Offered, Token, UpdateError};

/// The most of a manifest this build will read.
///
/// A manifest is tens of kilobytes: what changed, in every language a catalogue exists for, with
/// every letter past ASCII written as an escape. This is room for that to grow several times over,
/// and a copy holds every manifest it ever reads to it — a longer one is a release that copy never
/// hears of. A host that answers with megabytes is answering with something else.
pub(super) const LONGEST: usize = 256 * 1024;

/// The largest archive this build will bring down.
///
/// The application is tens of megabytes; this is room for it to grow several times over and still
/// refuse a host that has decided to send for ever.
pub(super) const LARGEST: usize = 256 * 1024 * 1024;

/// Whether a file's name is plain: letters, digits, dots, dashes and underscores, not starting
/// with a dot. Nothing in one leaves the folder it is in, asks a question of a host or names a
/// fragment.
pub(super) fn a_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// The manifest as it is written.
///
/// Deserialized with `deny_unknown_fields`: a manifest carrying something this build does not
/// understand is a manifest written for a different build, and guessing at the half it does
/// understand is how a version meant for somebody else gets installed here.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    version: String,
    /// One archive for each kind of Mac the release was built for, under the name its processor goes
    /// by in Rust — `aarch64`, `x86_64`. A copy takes the one for its own kind; a name this build has
    /// never heard of belongs to some other copy and is not a reason to refuse the rest.
    archives: BTreeMap<String, Archive>,
    /// What changed, in each language the release wrote it in, under the name of the window's
    /// catalogue for it. A map rather than one text because a person reads it, and a person reads
    /// in the language they chose — and because the shape is fixed for good once a copy reads it.
    notes: Notes,
}

/// One archive, and what a person is told about it before agreeing.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Archive {
    /// The archive's file, by a plain name, in the release of this version. A name and not an
    /// address: which places hold a release is not the manifest's to say.
    file: String,
    /// What the archive weighs, so a person is told before agreeing rather than after.
    bytes: u64,
    /// What it hashes to under SHA-256, in lowercase hexadecimal.
    sha256: String,
}

/// Reads a manifest whose signature has already been checked, and decides whether it offers
/// anything to this kind of Mac.
///
/// `running` is this build's own version and `machine` the processor it runs on, as
/// `std::env::consts::ARCH` names it. `None` means the manifest names this version or an older one,
/// or carries nothing for this kind of Mac — both ordinary answers, and neither an error. What is
/// offered is a file of the release of its version, by name.
pub(super) fn what_it_offers(
    text: &str,
    running: Version,
    machine: &str,
) -> Result<Option<Offered>, UpdateError> {
    let written: Written = serde_json::from_str(text).map_err(|_| UpdateError::CouldNotAsk)?;

    let version = Version::parse(&written.version).map_err(|_| UpdateError::CouldNotAsk)?;

    if !version.is_newer_than(running) {
        return Ok(None);
    }

    let Some(archive) = written.archives.get(machine) else {
        return Ok(None);
    };

    if archive.bytes == 0 || archive.bytes > LARGEST as u64 {
        return Err(UpdateError::CouldNotAsk);
    }

    // Read here rather than where the connection is made, so a name that leaves the release's
    // folder is refused before anything is opened. What comes down is code, so it comes from the
    // release of the version it is offered as, from the project's own places. A key that seals a
    // manifest does not also choose the host every copy downloads code from.
    if !a_plain_name(&archive.file) {
        return Err(UpdateError::CouldNotAsk);
    }

    let digest_is_written_right = archive.sha256.len() == 64
        && archive
            .sha256
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if !digest_is_written_right {
        return Err(UpdateError::CouldNotAsk);
    }

    Ok(Some(Offered {
        version,
        bytes: archive.bytes,
        notes: written.notes,
        file: Token::of(archive.file.clone()),
        digest: Token::of(archive.sha256.clone()),
    }))
}

/// Whether what came down is the archive the manifest names: as large as it says, and hashing to
/// what it says.
///
/// A download that stopped short is one to try again rather than somebody else's file, and the two
/// are told apart: only the second is said to be not this application's.
pub(super) fn the_archive_must_be_the_one_named(
    offered: &Offered,
    bytes: &[u8],
) -> Result<(), UpdateError> {
    if bytes.len() as u64 != offered.bytes {
        return Err(UpdateError::CouldNotAsk);
    }

    let hashed = ring::digest::digest(&ring::digest::SHA256, bytes);
    let written = hashed
        .as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });

    match written == offered.digest.as_str() {
        true => Ok(()),
        false => Err(UpdateError::NotOurs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running() -> Version {
        Version::parse("1.0.0").expect("a version")
    }

    const DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    /// One archive's entry in a manifest, naming the file given.
    fn archive(file: &str, bytes: u64) -> String {
        format!(r#"{{"file":{file:?},"bytes":{bytes},"sha256":"{DIGEST}"}}"#)
    }

    /// A manifest with one archive, for this kind of Mac.
    fn manifest(version: &str, bytes: u64, file: &str) -> String {
        format!(
            r#"{{"version":"{version}","archives":{{"aarch64":{}}},"notes":{{"en":"what changed","ru":"in the second language"}}}}"#,
            archive(file, bytes)
        )
    }

    #[test]
    fn a_later_version_is_offered_with_what_a_person_needs_to_decide() {
        let offered = what_it_offers(
            &manifest(
                "1.1.0",
                40_000_000,
                "lyonoyl-mail-1.1.0-macos-universal.app.tar.gz",
            ),
            running(),
            "aarch64",
        )
        .expect("the manifest reads")
        .expect("something is offered");

        assert_eq!(offered.version.to_string(), "1.1.0");
        assert_eq!(offered.bytes, 40_000_000);
        assert_eq!(
            offered.notes.get("en").map(String::as_str),
            Some("what changed")
        );
        assert_eq!(
            offered.notes.get("ru").map(String::as_str),
            Some("in the second language")
        );
        assert_eq!(
            offered.file,
            Token::of("lyonoyl-mail-1.1.0-macos-universal.app.tar.gz".to_owned()),
            "not the file the manifest names"
        );
    }

    /// Each kind of Mac takes its own archive, and never the other one's.
    #[test]
    fn each_kind_of_mac_is_offered_its_own_archive() {
        let both = format!(
            r#"{{"version":"1.1.0","archives":{{"aarch64":{},"x86_64":{}}},"notes":{{}}}}"#,
            archive("arm.tar.gz", 10),
            archive("intel.tar.gz", 20)
        );

        for (machine, file, bytes) in [("aarch64", "arm", 10), ("x86_64", "intel", 20)] {
            let offered = what_it_offers(&both, running(), machine)
                .expect("the manifest reads")
                .expect("something is offered");
            assert_eq!(
                offered.file,
                Token::of(format!("{file}.tar.gz")),
                "{machine}"
            );
            assert_eq!(offered.bytes, bytes, "{machine}");
        }
    }

    /// A release that carries nothing for this kind of Mac offers it nothing, rather than another
    /// kind's archive or a failure.
    #[test]
    fn a_mac_the_release_carries_nothing_for_is_offered_nothing() {
        let offered = what_it_offers(&manifest("1.1.0", 1000, "a.tar.gz"), running(), "x86_64")
            .expect("the manifest reads");

        assert!(offered.is_none());
    }

    /// A kind of Mac this build has never heard of is somebody else's archive, not a reason to
    /// refuse the manifest — or a field added later would stop every earlier copy updating.
    #[test]
    fn an_archive_for_a_kind_of_machine_this_build_does_not_know_is_left_alone() {
        let later = format!(
            r#"{{"version":"1.1.0","archives":{{"aarch64":{},"riscv64":{}}},"notes":{{}}}}"#,
            archive("a.tar.gz", 10),
            archive("r.tar.gz", 10)
        );

        assert!(what_it_offers(&later, running(), "aarch64")
            .expect("the manifest reads")
            .is_some());
    }

    /// The version already running, and anything older, offers nothing.
    ///
    /// Not an error: a manifest that names this version is a manifest saying there is nothing to
    /// do, which is what it says on most days.
    #[test]
    fn nothing_is_offered_for_this_version_or_an_older_one() {
        for same_or_older in ["1.0.0", "0.9.9", "0.1.0"] {
            let outcome = what_it_offers(
                &manifest(same_or_older, 1000, "a.tar.gz"),
                running(),
                "aarch64",
            )
            .expect("the manifest reads");

            assert!(outcome.is_none(), "{same_or_older} was offered over 1.0.0");
        }
    }

    /// The archive is a file of the release of the version it is offered as, and nothing else —
    /// whatever key sealed the manifest that names it. A file is a plain name, so nothing written
    /// there leaves the release's folder, reaches another host, asks a question of a place or names
    /// a fragment.
    #[test]
    fn a_file_that_is_not_a_plain_name_in_the_releases_folder_is_refused() {
        assert!(
            what_it_offers(
                &manifest("1.1.0", 1000, "lyonoyl-mail-1.1.0-macos-aarch64.app.tar.gz"),
                running(),
                "aarch64"
            )
            .expect("the manifest reads")
            .is_some(),
            "the name the release step writes was refused, so the refusals below prove nothing"
        );

        for file in [
            "",
            ".",
            "..",
            "../1.0.9/a.tar.gz",
            "%2e%2e/1.0.9/a.tar.gz",
            "inner/a.tar.gz",
            "..\\a.tar.gz",
            "a.tar.gz?from=elsewhere",
            "a.tar.gz#elsewhere",
            ".hidden.tar.gz",
            "a b.tar.gz",
            "a.tar.gz\n",
            "a\u{2215}b.tar.gz",
            "a.tar.gz@example.com",
            "//example.com/a.tar.gz",
            "https://example.com/a.tar.gz",
            "http://lyonoyl.com/releases/1.1.0/a.tar.gz",
            "https://lyonoyl.com/releases/1.1.0/a.tar.gz",
        ] {
            assert!(
                what_it_offers(&manifest("1.1.0", 1000, file), running(), "aarch64").is_err(),
                "{file:?} was accepted as a file of the release"
            );
        }
    }

    /// A size of nothing, and a size nobody would ship.
    #[test]
    fn a_size_that_is_not_a_size_is_refused() {
        for refused in [0, (LARGEST as u64) + 1] {
            assert!(
                what_it_offers(
                    &manifest("1.1.0", refused, "a.tar.gz"),
                    running(),
                    "aarch64"
                )
                .is_err(),
                "{refused} was accepted as a size"
            );
        }
    }

    /// A manifest written for a different build is not read halfway, at either level.
    #[test]
    fn a_manifest_carrying_something_this_build_does_not_understand_is_refused() {
        for extra in [
            format!(
                r#"{{"version":"1.1.0","archives":{{"aarch64":{}}},"notes":{{"en":"x"}},"requires":"something"}}"#,
                archive("a.tar.gz", 1000)
            ),
            format!(
                r#"{{"version":"1.1.0","archives":{{"aarch64":{{"file":"a.tar.gz","bytes":1000,"sha256":"{DIGEST}","mirror":"https://example.com/a.tar.gz"}}}},"notes":{{"en":"x"}}}}"#
            ),
        ] {
            assert!(
                what_it_offers(&extra, running(), "aarch64").is_err(),
                "{extra} was read"
            );
        }
    }

    /// A digest written any other way than the release step writes it is not a digest this build
    /// can compare, and the manifest naming it is refused rather than half-read.
    #[test]
    fn a_digest_that_is_not_written_as_one_is_refused() {
        let naming = |written: &str| {
            format!(
                r#"{{"version":"1.1.0","archives":{{"aarch64":{{"file":"a.tar.gz","bytes":3,"sha256":"{written}"}}}},"notes":{{}}}}"#
            )
        };
        assert!(
            what_it_offers(&naming(DIGEST), running(), "aarch64").is_ok(),
            "a digest written right was refused, so the refusals below prove nothing"
        );

        for written in ["", "00", &"A".repeat(64), &"g".repeat(64), &"0".repeat(65)] {
            let manifest = naming(written);
            assert!(
                what_it_offers(&manifest, running(), "aarch64").is_err(),
                "{written:?} was read as a digest"
            );
        }
    }

    /// The archive that came down is the one the manifest names, by its size and by what it hashes
    /// to; a download cut short is told apart from a different file.
    #[test]
    fn only_the_archive_the_manifest_names_is_taken() {
        let offered = Offered {
            version: Version::parse("1.1.0").expect("a version"),
            bytes: 3,
            notes: Notes::new(),
            file: Token::of("a.tar.gz".to_owned()),
            digest: Token::of(
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_owned(),
            ),
        };

        assert_eq!(the_archive_must_be_the_one_named(&offered, b"abc"), Ok(()));
        assert_eq!(
            the_archive_must_be_the_one_named(&offered, b"abd"),
            Err(UpdateError::NotOurs),
            "a file of the same size and other bytes was taken"
        );
        assert_eq!(
            the_archive_must_be_the_one_named(&offered, b"ab"),
            Err(UpdateError::CouldNotAsk),
            "a download cut short was called somebody else's file"
        );
    }

    #[test]
    fn a_manifest_that_is_not_a_manifest_is_refused() {
        for refused in [
            "".to_owned(),
            "not json at all".to_owned(),
            r#"{"version":"1.1.0"}"#.to_owned(),
            format!(
                r#"{{"version":"not a version","archives":{{"aarch64":{}}},"notes":{{}}}}"#,
                archive("a.tar.gz", 1)
            ),
            // The file as a list, and addresses where a name goes.
            format!(
                r#"{{"version":"1.1.0","archives":{{"aarch64":{{"file":["a.tar.gz"],"bytes":1,"sha256":"{DIGEST}"}}}},"notes":{{}}}}"#
            ),
            format!(
                r#"{{"version":"1.1.0","archives":{{"aarch64":{{"urls":["https://lyonoyl.com/releases/1.1.0/a.tar.gz"],"bytes":1,"sha256":"{DIGEST}"}}}},"notes":{{}}}}"#
            ),
            format!(
                r#"{{"version":"1.1.0","archives":{{"aarch64":{{"url":"https://lyonoyl.com/releases/1.1.0/a.tar.gz","bytes":1,"sha256":"{DIGEST}"}}}},"notes":{{}}}}"#
            ),
            // No file at all.
            format!(
                r#"{{"version":"1.1.0","archives":{{"aarch64":{{"bytes":1,"sha256":"{DIGEST}"}}}},"notes":{{}}}}"#
            ),
            r#"{"version":"1.1.0","archive":"a.tar.gz","bytes":1000,"notes":{"en":"one archive for every Mac"}}"#.to_owned(),
            // One text for every language, which is not what a person who chose another one reads.
            format!(
                r#"{{"version":"1.1.0","archives":{{"aarch64":{}}},"notes":"in one language for everybody"}}"#,
                archive("a.tar.gz", 1)
            ),
        ] {
            assert!(
                what_it_offers(&refused, running(), "aarch64").is_err(),
                "{refused:?} was read as a manifest"
            );
        }
    }
}
