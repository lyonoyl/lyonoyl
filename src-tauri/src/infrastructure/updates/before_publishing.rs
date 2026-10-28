//! A release, asked about before any copy is told of it, by the code every copy that reads it runs.
//!
//! **Ignored in an ordinary run**, because on a machine that has not just made a release there is
//! nothing to ask about. Two questions, each started by the script that needs the answer.
//!
//! [`every_copy_takes_the_release_on_this_disk`] is asked by `scripts/release-update.sh` once it has
//! made the release's files, the manifest and their seals, and before it says what to publish.
//! What it refuses is what every installed copy would refuse — a manifest whose shape or size a copy
//! does not read, a seal that does not hold, an archive that is not the one the manifest names or
//! not the version or the kind of Mac it is named for — and what somebody downloading by hand would
//! find wrong: a file the sums leave out, a sum that does not hold, a seal over other sums. Found
//! here, once, rather than by every copy at the same moment after it was published.
//!
//! [`every_place_holds_the_release`] is asked by `scripts/release-check.sh` once the files are
//! published: every file of the release brought back from each place the project publishes to,
//! through the providers every copy asks with, held to the sums this machine sealed, and whether
//! each place hands over the manifest made here yet. The manifest is what tells copies a release is
//! out, so it goes out last, once this says every place holds every file.
//!
//! **What it cannot ask is the copies already out there.** The application inside is held to the
//! signing requirement of the one it was packed from; a release signed under another identifier or
//! another team passes here, and every copy installed before it refuses it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lyonoyl_domain::version::Version;

use super::providers::{the_projects_providers, Sealed, UpdateProvider};
use super::staging;
use super::the_manifest::{self, a_plain_name, LARGEST, LONGEST};
use super::the_seal::{what_the_seal_says, TheKey};
use crate::infrastructure::net::TransportPolicy;
use crate::ports::updates::{Notes, Offered, Staged, Token, UpdateError};

/// The file every other file of a release is named in, with what it hashes to.
const SUMS: &str = "SHA256SUMS";

/// What changed, in every language, as a file of the release.
const NOTES: &str = "notes.json";

/// What the release script set, or a failure naming what it did not.
fn given(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: a release script starts this, and sets it"))
}

#[test]
#[ignore = "asks about a release this machine has just made; started by scripts/release-update.sh"]
fn every_copy_takes_the_release_on_this_disk() {
    let public = given("LYONOYL_RELEASE_PUBKEY");
    let key = TheKey::written_as(&public).expect("the update key's public half");
    let manifest_at = PathBuf::from(given("LYONOYL_RELEASE_MANIFEST"));
    let files = PathBuf::from(given("LYONOYL_RELEASE_FILES"));

    let manifest = std::fs::read_to_string(&manifest_at).expect("the manifest");
    assert!(
        manifest.len() <= LONGEST,
        "the manifest is {} bytes, past the {LONGEST} a copy reads",
        manifest.len()
    );
    let seal = std::fs::read_to_string(sealed_by(&manifest_at)).expect("the manifest's seal");
    what_the_seal_says(&key, manifest.as_bytes(), &seal).expect("the manifest's seal holds");

    // The version and the kinds of Mac it names, read as plain JSON: a copy asks for its own kind
    // and never which kinds there are, so the reader below has no way to say.
    let read: serde_json::Value = serde_json::from_str(&manifest).expect("the manifest is JSON");
    let version = read["version"]
        .as_str()
        .and_then(|written| Version::parse(written).ok())
        .expect("the manifest names its version");
    let machines: Vec<String> = read["archives"]
        .as_object()
        .expect("the manifest names its archives")
        .keys()
        .cloned()
        .collect();
    assert!(
        !machines.is_empty(),
        "the manifest offers nothing to any Mac"
    );

    // The release's files, before anything is unpacked: what somebody downloading by hand checks,
    // and the folder each archive below is read from.
    let sums = std::fs::read_to_string(files.join(SUMS)).expect("the sums");
    let over_the_sums =
        std::fs::read_to_string(sealed_by(&files.join(SUMS))).expect("the seal over the sums");
    what_the_seal_says(&key, sums.as_bytes(), &over_the_sums)
        .expect("the seal over the sums holds");
    let named = the_sums(&sums).unwrap_or_else(|why| panic!("{SUMS}: {why}"));
    let on_disk = the_files_in(&files);
    the_sums_name_every_file(&named, &on_disk).unwrap_or_else(|why| panic!("{why}"));
    for (name, digest) in &named {
        let bytes = std::fs::read(files.join(name)).expect("a file the sums name");
        assert_eq!(
            &hashed(&bytes),
            digest,
            "{name} does not hash to what {SUMS} says"
        );
    }

    // Asked as a copy of the oldest version there could be: whatever is released is newer.
    let oldest = Version::parse("0.0.0").expect("a version");
    let offers: Vec<(String, Offered, String)> = machines
        .iter()
        .map(|machine| {
            let offered = the_manifest::what_it_offers(&manifest, oldest, machine)
                .expect("every copy reads the manifest")
                .unwrap_or_else(|| panic!("nothing is offered to {machine}"));

            notes_for_every_catalogue(&offered);

            let name = offered.file.as_str().to_owned();
            an_archive_for(&name, version, machine).unwrap_or_else(|why| panic!("{why}"));

            (machine.clone(), offered, name)
        })
        .collect();

    // What the folder holds, and nothing besides it: whatever is in it is published.
    let archives: BTreeSet<String> = offers.iter().map(|(_, _, name)| name.clone()).collect();
    the_release_is(&on_disk, version, &archives).unwrap_or_else(|why| panic!("{why}"));
    let written: Notes =
        serde_json::from_slice(&std::fs::read(files.join(NOTES)).expect("the notes"))
            .expect("the notes are a text for each language");
    assert!(
        offers
            .iter()
            .all(|(_, offered, _)| offered.notes == written),
        "{NOTES} says something other than the manifest"
    );

    for (machine, offered, name) in &offers {
        let at = files.join(name);
        let bytes = std::fs::read(&at).expect("the archive");
        the_manifest::the_archive_must_be_the_one_named(offered, &bytes)
            .unwrap_or_else(|why| panic!("{name} is not the archive the manifest names: {why}"));

        what_a_copy_finds_inside(&at, offered, machine, &public);
    }
}

#[test]
#[ignore = "asks every place what it holds of a release; started by scripts/release-check.sh"]
fn every_place_holds_the_release() {
    let key =
        TheKey::written_as(&given("LYONOYL_RELEASE_PUBKEY")).expect("the update key's public half");
    let version =
        Version::parse(&given("LYONOYL_RELEASE_VERSION")).expect("the version to ask about");
    let files = PathBuf::from(given("LYONOYL_RELEASE_FILES"));
    let manifest_at = PathBuf::from(given("LYONOYL_RELEASE_MANIFEST"));
    let report_at = PathBuf::from(given("LYONOYL_RELEASE_REPORT"));

    // The places every copy asks, through the providers every copy asks them with: the same
    // addresses, the same client, the same redirects and limits. A place that answers a browser and
    // refuses a copy is a place that refuses every copy, and only this finds that out.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a runtime");
    let policy = Arc::new(TransportPolicy::new().expect("a policy"));
    let places = the_projects_providers(policy, runtime.handle().clone());
    let sealed = |file: &[u8], seal: &str| what_the_seal_says(&key, file, seal).is_ok();

    let sums = std::fs::read(files.join(SUMS)).expect("the sums made here");
    let manifest = std::fs::read_to_string(&manifest_at).expect("the manifest made here");
    let seal = std::fs::read_to_string(sealed_by(&manifest_at)).expect("the seal beside it");

    let mut report = Vec::new();
    let mut held = true;
    for place in &places {
        let name = place.name();
        match what_a_place_holds(place.as_ref(), &sealed, version, &sums) {
            Ok(count) => report.push(format!(
                "{name}: every file of {version}, {count} of them, byte for byte"
            )),
            Err(why) => {
                held = false;
                report.push(format!("{name}: {why}"));
            }
        }

        let (said, fine) =
            where_the_manifest_stands(place.the_manifest(), &manifest, &seal, version);
        held &= fine;
        report.push(format!("{name}: {said}"));
    }

    std::fs::write(&report_at, report.join("\n") + "\n").expect("the report");
    assert!(held, "a place does not hold the release made here");
}

/// Where the seal over a file is: beside it, under its whole name with `.minisig` added.
fn sealed_by(file: &Path) -> PathBuf {
    let mut named = file.as_os_str().to_owned();
    named.push(".minisig");
    PathBuf::from(named)
}

/// What a file hashes to under SHA-256, in lowercase hexadecimal, as the sums and the manifest
/// write it.
fn hashed(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// What the sums name: each file of the release by its name, with what it hashes to.
///
/// Written the way `shasum` writes and reads them — the digest in lowercase hexadecimal, two
/// spaces, a plain name — so somebody checking by hand uses the tool they already have. Refused
/// where a line is anything else, a name is there twice, or nothing is named at all.
fn the_sums(written: &str) -> Result<BTreeMap<String, String>, String> {
    let mut named = BTreeMap::new();

    for line in written.lines() {
        let Some((digest, name)) = line.split_once("  ") else {
            return Err(format!("{line:?} is not a digest and a name"));
        };
        let a_digest = digest.len() == 64
            && digest
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
        if !a_digest {
            return Err(format!("{line:?} does not start with a digest"));
        }
        if !a_plain_name(name) {
            return Err(format!("{name:?} is not a plain name"));
        }
        if named.insert(name.to_owned(), digest.to_owned()).is_some() {
            return Err(format!("{name} is named twice"));
        }
    }

    match named.is_empty() {
        true => Err("they name no file".to_owned()),
        false => Ok(named),
    }
}

/// Every file in the release's folder by name, but the sums and the seal over them.
fn the_files_in(folder: &Path) -> BTreeSet<String> {
    std::fs::read_dir(folder)
        .expect("the release's folder")
        .map(|entry| entry.expect("a file of the release"))
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name != SUMS && *name != format!("{SUMS}.minisig"))
        .collect()
}

/// Whether the sums name every file in the folder, and no file that is not there.
fn the_sums_name_every_file(
    named: &BTreeMap<String, String>,
    on_disk: &BTreeSet<String>,
) -> Result<(), String> {
    let listed: BTreeSet<String> = named.keys().cloned().collect();
    let left_out: Vec<&String> = on_disk.difference(&listed).collect();
    let not_there: Vec<&String> = listed.difference(on_disk).collect();

    match (left_out.is_empty(), not_there.is_empty()) {
        (true, true) => Ok(()),
        _ => Err(format!(
            "{SUMS} leaves out {left_out:?} and names {not_there:?}, which is not in the folder"
        )),
    }
}

/// The kind of Mac an archive's name says it is for, where it is named the way a release names
/// one: `lyonoyl-mail-<version>-macos-<kind>.app.tar.gz`.
fn the_kind_in(name: &str, version: Version) -> Option<&str> {
    name.strip_prefix(&format!("lyonoyl-mail-{version}-macos-"))?
        .strip_suffix(".app.tar.gz")
}

/// Whether `name` is an archive of `version` for `machine`: named for that kind of Mac, or for
/// both kinds in one.
fn an_archive_for(name: &str, version: Version, machine: &str) -> Result<(), String> {
    match the_kind_in(name, version) {
        Some(kind) if kind == machine || kind == "universal" => Ok(()),
        _ => Err(format!(
            "{machine} is offered {name}, which is not named as an archive of {version} for it"
        )),
    }
}

/// Whether the folder holds the release of `version` and nothing else: each archive the manifest
/// names with the disk image of the same kind beside it, the source, and the notes.
fn the_release_is(
    on_disk: &BTreeSet<String>,
    version: Version,
    archives: &BTreeSet<String>,
) -> Result<(), String> {
    let mut wanted: BTreeSet<String> = [
        format!("lyonoyl-mail-{version}-source.tar.gz"),
        NOTES.to_owned(),
    ]
    .into();
    for archive in archives {
        wanted.insert(archive.clone());
        if let Some(stem) = archive.strip_suffix(".app.tar.gz") {
            wanted.insert(format!("{stem}.dmg"));
        }
    }

    let missing: Vec<&String> = wanted.difference(on_disk).collect();
    let besides: Vec<&String> = on_disk.difference(&wanted).collect();

    match (missing.is_empty(), besides.is_empty()) {
        (true, true) => Ok(()),
        _ => Err(format!(
            "the release of {version} is missing {missing:?} and holds {besides:?} besides, \
             which would be published with it"
        )),
    }
}

/// What one place holds of the release of `version`: the sums made here, under a seal that holds,
/// and every file they name, hashing to what they say. Answers how many files that is, or
/// everything that is wrong.
fn what_a_place_holds(
    place: &dyn UpdateProvider,
    sealed: &dyn Fn(&[u8], &str) -> bool,
    version: Version,
    sums_here: &[u8],
) -> Result<usize, String> {
    let fetch = |name: &str| place.a_file(version, name, LARGEST, &mut |_, _| true).ok();

    let sums = fetch(SUMS).ok_or_else(|| format!("{SUMS} is not there"))?;
    if sums != sums_here {
        return Err(format!("{SUMS} is not the one made here"));
    }
    let seal = fetch(&format!("{SUMS}.minisig"))
        .and_then(|seal| String::from_utf8(seal).ok())
        .ok_or_else(|| format!("the seal over {SUMS} is not there"))?;
    if !sealed(&sums, &seal) {
        return Err(format!("the seal over {SUMS} does not hold"));
    }

    let named = std::str::from_utf8(&sums)
        .map_err(|_| format!("{SUMS} is not text"))
        .and_then(the_sums)?;

    let wrong: Vec<String> = named
        .iter()
        .filter_map(|(name, digest)| match fetch(name) {
            None => Some(format!("{name} is not there")),
            Some(bytes) if hashed(&bytes) != *digest => {
                Some(format!("{name} is not the file made here"))
            }
            Some(_) => None,
        })
        .collect();

    match wrong.is_empty() {
        true => Ok(named.len()),
        false => Err(wrong.join("; ")),
    }
}

/// Where a place stands with the manifest made here, from what it handed over, said as a line of
/// the report, and whether that is a state a release may be in.
///
/// Nothing handed over, or a manifest of an older version, is a release whose manifest has not gone
/// out yet: the order this happens in. The manifest made here is the release out. A different
/// manifest of this same version is a place telling copies something this machine never checked.
fn where_the_manifest_stands(
    theirs: Result<Sealed, UpdateError>,
    ours: &str,
    our_seal: &str,
    version: Version,
) -> (String, bool) {
    let Ok(theirs) = theirs else {
        return ("the manifest is not out yet".to_owned(), true);
    };

    if theirs.manifest == ours {
        return match theirs.seal == our_seal {
            true => (format!("the manifest of {version} is out"), true),
            false => (
                format!(
                    "the manifest of {version} is out, and the seal beside it is not the one \
                     made here yet"
                ),
                true,
            ),
        };
    }

    let offers = serde_json::from_str::<serde_json::Value>(&theirs.manifest)
        .ok()
        .and_then(|read| {
            read["version"]
                .as_str()
                .and_then(|v| Version::parse(v).ok())
        });
    match offers {
        Some(older) if version.is_newer_than(older) => (
            format!("the manifest is not out yet: it still offers {older}"),
            true,
        ),
        _ => (
            format!("a manifest other than the one made here is out where {version}'s goes"),
            false,
        ),
    }
}

/// A note in every language the window has a catalogue for, and none of them blank.
///
/// The catalogues are read from the tree rather than taken from the script that wrote the notes,
/// so the two are asked separately and a script that dropped one is caught here.
fn notes_for_every_catalogue(offered: &Offered) {
    let catalogues = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/shared/i18n/locales");

    for entry in std::fs::read_dir(&catalogues).expect("the catalogues") {
        let path = entry.expect("a catalogue").path();
        if path.extension().is_none_or(|what| what != "json") {
            continue;
        }
        let language = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("a catalogue's name");

        let written = offered.notes.get(language).map(|note| note.trim());
        assert!(
            written.is_some_and(|note| !note.is_empty()),
            "a person who reads {language} is told nothing about what changed"
        );
    }
}

/// What a copy finds when it unpacks the archive, asked with the code a copy asks with.
fn what_a_copy_finds_inside(archive: &Path, offered: &Offered, machine: &str, public: &str) {
    let unpacked =
        std::env::temp_dir().join(format!("lyonoyl-release-{machine}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).expect("somewhere to unpack");

    staging::unpack(archive, &unpacked, staging::UNPACKS_TO_AT_MOST).expect("the archive unpacks");
    let arrived = staging::one_bundle_inside(&unpacked).expect("one application inside");

    // The same application as the one the archive was made from, by the check a copy makes before
    // it lets anything replace it.
    let made_from = PathBuf::from(given("LYONOYL_RELEASE_APP"));
    staging::the_identity_must_match(&made_from, &arrived)
        .expect("signed as the application it was made from");

    let staged = Staged {
        version: offered.version,
        at: Token::of(archive.display().to_string()),
    };
    staging::the_version_must_be_the_one_offered(&arrived, &staged)
        .expect("the version it is offered as");

    // The system it needs, read the way a copy of that kind of Mac reads it before it puts the
    // application in its own place: a property list or a header a copy cannot read is a release
    // every copy refuses.
    if let Err(why) = staging::the_system_it_needs(&arrived, machine) {
        panic!("a copy on {machine} cannot tell which system this needs: {why}");
    }

    let information = arrived.join("Contents").join("Info");
    let executable = arrived.join("Contents").join("MacOS").join(said(
        "/usr/bin/defaults",
        &[
            "read".as_ref(),
            information.as_os_str(),
            "CFBundleExecutable".as_ref(),
        ],
    ));

    // Code for the kind of Mac it is offered to, under the name `lipo` gives it.
    let kinds = said(
        "/usr/bin/lipo",
        &["-archs".as_ref(), executable.as_os_str()],
    );
    let wanted = match machine {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("{other} is not a kind of Mac a release is built for"),
    };
    assert!(
        kinds.split_whitespace().any(|kind| kind == wanted),
        "{machine} is offered an archive holding code for {kinds} only"
    );

    // And the key, so a copy that takes this update can take the one after it.
    let line = public
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty() && !line.starts_with("untrusted comment:"))
        .expect("the key's line");
    let carried = std::fs::read(&executable).expect("the executable");
    assert!(
        carried
            .windows(line.len())
            .any(|here| here == line.as_bytes()),
        "the application inside carries no update key, so every copy that takes it is never updated again"
    );

    let _ = std::fs::remove_dir_all(&unpacked);
}

/// What a system tool prints, trimmed, or a failure saying which one.
fn said(tool: &str, arguments: &[&std::ffi::OsStr]) -> String {
    let done = std::process::Command::new(tool)
        .args(arguments)
        .output()
        .unwrap_or_else(|_| panic!("{tool} runs"));
    assert!(
        done.status.success(),
        "{tool}: {}",
        String::from_utf8_lossy(&done.stderr)
    );

    String::from_utf8_lossy(&done.stdout).trim().to_owned()
}

/// What the two questions above decide, asked of what is decided rather than of a release: they
/// run only when a release script starts them, and a rule that is wrong would otherwise be found
/// on the day of a release.
mod rules {
    use super::*;
    use crate::ports::updates::Watching;

    fn version() -> Version {
        Version::parse("1.2.0").expect("a version")
    }

    const A: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const B: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn names(listed: &[&str]) -> BTreeSet<String> {
        listed.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_sums_are_read_as_shasum_writes_them() {
        let named = the_sums(&format!(
            "{A}  notes.json\n{B}  lyonoyl-mail-1.2.0-source.tar.gz\n"
        ))
        .expect("the sums read");

        assert_eq!(named.get("notes.json").map(String::as_str), Some(A));
        assert_eq!(
            named
                .get("lyonoyl-mail-1.2.0-source.tar.gz")
                .map(String::as_str),
            Some(B)
        );
        assert_eq!(named.len(), 2);
    }

    #[test]
    fn sums_written_any_other_way_are_refused() {
        let upper = A.to_ascii_uppercase();
        for refused in [
            String::new(),
            format!("{A} notes.json"),
            format!("{A}  notes.json\n{B}  notes.json"),
            format!("{}  notes.json", &A[..63]),
            format!("{upper}  notes.json"),
            format!("{A}  ../notes.json"),
            format!("{A}  .notes.json"),
            format!("{A}  inner/notes.json"),
            format!("{A}  "),
            "notes.json".to_owned(),
        ] {
            assert!(the_sums(&refused).is_err(), "{refused:?} was read as sums");
        }
    }

    #[test]
    fn the_sums_name_every_file_in_the_folder_and_nothing_else() {
        let named = the_sums(&format!("{A}  a\n{B}  b\n")).expect("the sums read");

        assert!(the_sums_name_every_file(&named, &names(&["a", "b"])).is_ok());
        assert!(
            the_sums_name_every_file(&named, &names(&["a", "b", "c"])).is_err(),
            "a file the sums leave out would be published unchecked"
        );
        assert!(
            the_sums_name_every_file(&named, &names(&["a"])).is_err(),
            "the sums name a file nobody would find"
        );
    }

    #[test]
    fn an_archive_is_named_for_its_kind_of_mac_or_for_both() {
        let v = version();

        assert!(
            an_archive_for("lyonoyl-mail-1.2.0-macos-aarch64.app.tar.gz", v, "aarch64").is_ok()
        );
        assert!(an_archive_for("lyonoyl-mail-1.2.0-macos-x86_64.app.tar.gz", v, "x86_64").is_ok());
        for machine in ["aarch64", "x86_64"] {
            assert!(
                an_archive_for("lyonoyl-mail-1.2.0-macos-universal.app.tar.gz", v, machine).is_ok(),
                "{machine}"
            );
        }

        for (name, machine) in [
            ("lyonoyl-mail-1.2.0-macos-x86_64.app.tar.gz", "aarch64"),
            ("lyonoyl-mail-1.1.0-macos-aarch64.app.tar.gz", "aarch64"),
            ("lyonoyl-mail-1.2.0-linux-aarch64.app.tar.gz", "aarch64"),
            ("Lyonoyl-Mail-1.2.0-aarch64.app.tar.gz", "aarch64"),
            ("lyonoyl-mail-1.2.0-macos-aarch64.dmg", "aarch64"),
            ("lyonoyl-mail-1.2.0-macos-.app.tar.gz", "aarch64"),
        ] {
            assert!(
                an_archive_for(name, v, machine).is_err(),
                "{name} was taken as an archive for {machine}"
            );
        }
    }

    #[test]
    fn the_folder_holds_the_release_and_nothing_besides() {
        let v = version();
        let archives = names(&["lyonoyl-mail-1.2.0-macos-universal.app.tar.gz"]);
        let whole = [
            "lyonoyl-mail-1.2.0-macos-universal.app.tar.gz",
            "lyonoyl-mail-1.2.0-macos-universal.dmg",
            "lyonoyl-mail-1.2.0-source.tar.gz",
            "notes.json",
        ];

        assert!(the_release_is(&names(&whole), v, &archives).is_ok());

        for left_out in whole {
            let short: Vec<&str> = whole.into_iter().filter(|name| *name != left_out).collect();
            assert!(
                the_release_is(&names(&short), v, &archives).is_err(),
                "a release without {left_out} was taken"
            );
        }

        // An archive and an image from an earlier run of another kind, which the manifest no
        // longer names and the folder would still publish.
        let mut stale = whole.to_vec();
        stale.push("lyonoyl-mail-1.2.0-macos-aarch64.app.tar.gz");
        assert!(the_release_is(&names(&stale), v, &archives).is_err());
        let mut stale = whole.to_vec();
        stale.push("lyonoyl-mail-1.2.0-macos-aarch64.dmg");
        assert!(the_release_is(&names(&stale), v, &archives).is_err());
    }

    /// Files by their names.
    type Files<'a> = Vec<(&'a str, &'a [u8])>;

    /// A place, as the files of the release of 1.2.0 it holds, and no manifest.
    struct Holding(BTreeMap<String, Vec<u8>>);

    fn a_place(files: &[(&str, &[u8])]) -> Holding {
        Holding(
            files
                .iter()
                .map(|(name, bytes)| ((*name).to_owned(), bytes.to_vec()))
                .collect(),
        )
    }

    impl UpdateProvider for Holding {
        fn name(&self) -> &str {
            "a place"
        }

        fn the_manifest(&self) -> Result<Sealed, UpdateError> {
            Err(UpdateError::CouldNotAsk)
        }

        fn a_file(
            &self,
            asked: Version,
            name: &str,
            _at_most: usize,
            _watching: Watching<'_>,
        ) -> Result<Vec<u8>, UpdateError> {
            match asked == version() {
                true => self.0.get(name).cloned().ok_or(UpdateError::CouldNotAsk),
                false => Err(UpdateError::CouldNotAsk),
            }
        }
    }

    /// A seal holds where it says it does: the seal is not what these tests are about.
    fn sealed(_: &[u8], seal: &str) -> bool {
        seal == "holds"
    }

    fn sums_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        files
            .iter()
            .map(|(name, bytes)| format!("{}  {name}\n", hashed(bytes)))
            .collect::<String>()
            .into_bytes()
    }

    #[test]
    fn a_place_holding_every_file_byte_for_byte_holds_the_release() {
        let sums = sums_of(&[("a", b"first"), ("b", b"second")]);
        let place = a_place(&[
            (SUMS, &sums),
            ("SHA256SUMS.minisig", b"holds"),
            ("a", b"first"),
            ("b", b"second"),
        ]);

        assert_eq!(what_a_place_holds(&place, &sealed, version(), &sums), Ok(2));
        assert!(
            what_a_place_holds(
                &place,
                &sealed,
                Version::parse("1.3.0").expect("a version"),
                &sums
            )
            .is_err(),
            "the release of another version was said to be held"
        );
    }

    #[test]
    fn a_place_missing_anything_or_holding_anything_else_does_not() {
        let sums = sums_of(&[("a", b"first"), ("b", b"second")]);
        let m = "SHA256SUMS.minisig";

        let places: [(&str, Files); 6] = [
            (
                "no sums",
                vec![(m, b"holds"), ("a", b"first"), ("b", b"second")],
            ),
            (
                "other sums",
                vec![
                    (SUMS, b"other"),
                    (m, b"holds"),
                    ("a", b"first"),
                    ("b", b"second"),
                ],
            ),
            (
                "no seal",
                vec![(SUMS, &sums), ("a", b"first"), ("b", b"second")],
            ),
            (
                "a seal that does not hold",
                vec![
                    (SUMS, &sums),
                    (m, b"broken"),
                    ("a", b"first"),
                    ("b", b"second"),
                ],
            ),
            (
                "a file missing",
                vec![(SUMS, &sums), (m, b"holds"), ("a", b"first")],
            ),
            (
                "a file changed",
                vec![
                    (SUMS, &sums),
                    (m, b"holds"),
                    ("a", b"first"),
                    ("b", b"secnod"),
                ],
            ),
        ];

        for (what, files) in places {
            assert!(
                what_a_place_holds(&a_place(&files), &sealed, version(), &sums).is_err(),
                "a place with {what} was said to hold the release"
            );
        }
    }

    #[test]
    fn where_a_place_stands_with_the_manifest() {
        let v = version();
        let ours = r#"{"version":"1.2.0"}"#;
        let handing = |manifest: &str, seal: &str| {
            Ok(Sealed {
                manifest: manifest.to_owned(),
                seal: seal.to_owned(),
            })
        };

        let (said, fine) =
            where_the_manifest_stands(Err(UpdateError::CouldNotAsk), ours, "seal", v);
        assert!(fine && said.contains("not out yet"), "{said}");

        let (said, fine) =
            where_the_manifest_stands(handing(r#"{"version":"1.1.0"}"#, "old"), ours, "seal", v);
        assert!(fine && said.contains("still offers 1.1.0"), "{said}");

        let (said, fine) = where_the_manifest_stands(handing(ours, "seal"), ours, "seal", v);
        assert!(fine && said.ends_with("is out"), "{said}");

        let (said, fine) = where_the_manifest_stands(handing(ours, "old"), ours, "seal", v);
        assert!(fine && said.contains("not the one made here yet"), "{said}");

        for theirs in [
            r#"{"version":"1.2.0","other":true}"#,
            r#"{"version":"1.3.0"}"#,
            "not a manifest",
        ] {
            let (said, fine) = where_the_manifest_stands(handing(theirs, "seal"), ours, "seal", v);
            assert!(
                !fine,
                "{theirs:?} where this version's manifest goes was taken: {said}"
            );
        }
    }
}
