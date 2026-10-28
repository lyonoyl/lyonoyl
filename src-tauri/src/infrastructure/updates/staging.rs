//! Putting a version that came down where the running one is.
//!
//! The irreversible half. Everything before it can be abandoned by deleting a file; this replaces
//! the application somebody is using, and a mistake here is a person left with no mail client and
//! no obvious way back.
//!
//! **Two locks, and the second is the one a stolen key does not open.** The first is the seal,
//! checked before anything is written. The second is asked here: the bundle that came down must
//! satisfy the same designated requirement as the copy it would replace — the same bundle
//! identifier, signed by the same team, through the same certificate authority. Somebody holding
//! this project's signing key alone gets an archive that is refused at the door; they would need
//! Apple's half as well.
//!
//! That second lock is macOS and nowhere else, and this file says so rather than letting *two
//! locks* quietly become one lock on the platforms that have no such notion.
//!
//! **What is replaced is the running application, at the path it is running from.** Not a path from
//! a manifest, not a path from a setting — `current_exe`, walked up to the bundle it sits in. A
//! path that came from outside is a path an attacker chooses.

use std::path::{Path, PathBuf};

use super::what_it_keeps;
use crate::ports::updates::{Offered, Staged, Token, UpdateError};

/// Writes what came down where it can wait, and answers a handle to it.
///
/// Called only after the seal has been checked. The bytes are already known to be this build's;
/// what this adds is somewhere for them that is not shared with everybody on the machine.
pub(super) fn put_it_somewhere(
    beside: &Path,
    offered: &Offered,
    bytes: &[u8],
) -> Result<Staged, UpdateError> {
    let waiting = what_it_keeps::the_folder(beside)?;
    let at = waiting.join(format!("{}.tar.gz", offered.version));

    std::fs::write(&at, bytes).map_err(|_| UpdateError::Unavailable)?;

    Ok(Staged {
        version: offered.version,
        at: Token::of(at.to_string_lossy().into_owned()),
    })
}

/// Puts a staged version in place of the running one.
///
/// `beside` is where this application keeps its files, which is where the archive is opened unless
/// the running copy is on another volume.
#[cfg(target_os = "macos")]
pub(super) fn swap_it_in(beside: &Path, staged: &Staged) -> Result<(), UpdateError> {
    let running = the_bundle_this_is_running_from()?;

    put_in_place_of(&running, beside, staged, on_one_volume, |arrived| {
        // The second lock, asked of the tree nothing else could have written and before a single
        // file moves.
        the_identity_must_match(&running, arrived)?;

        // And that it is the version somebody was offered. The manifest is signed and names both
        // the version and where the archive is, so the two agree unless the release process
        // uploaded the wrong file — which is a mistake rather than an attack, and is exactly the
        // mistake that would otherwise install a version nobody was shown.
        the_version_must_be_the_one_offered(arrived, staged)?;

        // And that this Mac runs it at all. The manifest says which kind of Mac an archive is for
        // and nothing about which system, and a bundle this Mac will not open, put where the
        // application was, is a person left with no mail client.
        it_runs_on_this_mac(arrived)
    })
}

/// The swap, with what the bundle that came down has to pass and whether two folders are on one
/// volume handed in, so the order it keeps can be held by a test that has no certificate to sign
/// with and one disk to write on.
///
/// **The archive is opened where this application keeps its own files**, when that folder is on the
/// volume the running copy is on. That folder is this person's alone, and whatever an archive left
/// in it — refused, or cut short by the application quitting halfway — is cleared on the way out or
/// the next time an archive is opened, and is never in the folder the applications are in. The
/// bundle goes beside the running copy only once it has passed, and only by renaming it there:
/// across volumes that would be a copy, and a copy of an application bundle is a moment where half
/// of it is in place. **A copy on another volume** — a home folder moved to another disk, an
/// application kept on one — **has its archive opened beside it instead**, in a hidden folder
/// cleared the same way, so the bundle that passes is renamed into place there as well.
#[cfg(target_os = "macos")]
fn put_in_place_of(
    running: &Path,
    beside: &Path,
    staged: &Staged,
    one_volume: impl FnOnce(&Path, &Path) -> bool,
    admitted: impl FnOnce(&Path) -> Result<(), UpdateError>,
) -> Result<(), UpdateError> {
    let next_to_it = running.parent().ok_or(UpdateError::NotOursToReplace)?;
    let waiting = what_it_keeps::the_folder(beside)?;

    let (unpacked, cannot_write) = match one_volume(&waiting, next_to_it) {
        true => (waiting.join("unpacked"), UpdateError::Unavailable),
        // A folder beside the running copy that cannot be written is a copy this one cannot
        // replace, as the swap below would find.
        false => (
            next_to_it.join(".lyonoyl-unpacked"),
            UpdateError::NotOursToReplace,
        ),
    };
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).map_err(|_| cannot_write)?;
    // Gone on every way out, refused or not.
    let _cleared = ClearedOnLeaving(&unpacked);

    // The archive has done its work once it is unpacked, whatever comes after.
    let opened = unpack(Path::new(staged.at.as_str()), &unpacked, UNPACKS_TO_AT_MOST);
    let _ = std::fs::remove_file(staged.at.as_str());
    opened?;

    let arrived = one_bundle_inside(&unpacked)?;
    admitted(&arrived)?;

    // Beside the running copy now, under a name nobody opens, so the swap below is two renames in
    // one folder.
    let brought = next_to_it.join(format!(".lyonoyl-{}", staged.version));
    let _ = std::fs::remove_dir_all(&brought);
    std::fs::rename(&arrived, &brought).map_err(|why| match why.kind() {
        std::io::ErrorKind::CrossesDevices | std::io::ErrorKind::PermissionDenied => {
            UpdateError::NotOursToReplace
        }
        _ => UpdateError::Unavailable,
    })?;
    // Gone if the swap below does not take it, and nothing to clear once it has.
    let _brought_cleared = ClearedOnLeaving(&brought);

    // The old one is kept until the new one is in place, and only then thrown away. A swap that
    // deletes first is a swap that leaves nothing at all when the second half fails.
    let stepped_aside = next_to_it.join(format!(".lyonoyl-previous-{}", staged.version));
    let _ = std::fs::remove_dir_all(&stepped_aside);
    std::fs::rename(running, &stepped_aside).map_err(|_| UpdateError::NotOursToReplace)?;

    if let Err(why) = std::fs::rename(&brought, running) {
        let _ = std::fs::rename(&stepped_aside, running);
        return Err(match why.kind() {
            std::io::ErrorKind::PermissionDenied => UpdateError::NotOursToReplace,
            _ => UpdateError::Unavailable,
        });
    }

    let _ = std::fs::remove_dir_all(&stepped_aside);

    Ok(())
}

/// A directory removed when this goes out of scope, however that happens.
#[cfg(target_os = "macos")]
struct ClearedOnLeaving<'a>(&'a Path);

#[cfg(target_os = "macos")]
impl Drop for ClearedOnLeaving<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0);
    }
}

/// Whether two folders are on one volume, so a rename from one to the other moves a name rather
/// than failing. Not known is not one: the folder beside the running copy is the one a rename into
/// its place always reaches.
#[cfg(target_os = "macos")]
fn on_one_volume(one: &Path, other: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    match (std::fs::metadata(one), std::fs::metadata(other)) {
        (Ok(one), Ok(other)) => one.dev() == other.dev(),
        _ => false,
    }
}

/// Nowhere else installs software this way.
///
/// Said out loud rather than left as a missing branch: on Linux an application is a package the
/// system owns, and replacing it from inside itself is a thing to be refused rather than a thing
/// nobody got round to.
#[cfg(not(target_os = "macos"))]
pub(super) fn swap_it_in(_beside: &Path, _staged: &Staged) -> Result<(), UpdateError> {
    Err(UpdateError::NotOursToReplace)
}

/// The application bundle the running process is inside.
#[cfg(target_os = "macos")]
fn the_bundle_this_is_running_from() -> Result<PathBuf, UpdateError> {
    let running = std::env::current_exe().map_err(|_| UpdateError::NotOursToReplace)?;

    // `…/Lyonoyl Mail.app/Contents/MacOS/lyonoyl`, so the bundle is three levels up. A binary that
    // is not inside one is a development build run straight out of `target`, and there is nothing
    // there to replace.
    let bundle = running
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or(UpdateError::NotOursToReplace)?;

    match bundle.extension().is_some_and(|what| what == "app") {
        true => Ok(bundle.to_path_buf()),
        false => Err(UpdateError::NotOursToReplace),
    }
}

/// The one bundle an archive unpacked to: the only thing at its top named like one, and a folder.
///
/// A folder and not a link to one. `file_type` does not follow a link, and a link named like a
/// bundle is how an archive would point every check at one place and the swap at another: the
/// checks follow it to whatever it names, and the rename puts the link itself where the
/// application was.
#[cfg(target_os = "macos")]
pub(super) fn one_bundle_inside(unpacked: &Path) -> Result<PathBuf, UpdateError> {
    let mut found = std::fs::read_dir(unpacked)
        .map_err(|_| UpdateError::Unavailable)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|what| what == "app"));

    let one = found.next().ok_or(UpdateError::NotOurs)?;

    // An archive holding two is an archive nobody here made.
    if found.next().is_some() {
        return Err(UpdateError::NotOurs);
    }

    match one.file_type() {
        Ok(kind) if kind.is_dir() => Ok(one.path()),
        _ => Err(UpdateError::NotOurs),
    }
}

/// Whether the bundle that came down is the version it was offered as.
#[cfg(target_os = "macos")]
pub(super) fn the_version_must_be_the_one_offered(
    arrived: &Path,
    staged: &Staged,
) -> Result<(), UpdateError> {
    let carried = what_the_bundle_says(arrived, "CFBundleShortVersionString")
        .map_err(|_| UpdateError::NotTheVersionOffered)?
        .unwrap_or_default();

    match carried == staged.version.to_string() {
        true => Ok(()),
        false => Err(UpdateError::NotTheVersionOffered),
    }
}

/// What a bundle's property list says under `key`, or nothing where it says nothing.
///
/// Asked of the system rather than read out of the file by hand: the answer is exact, and a reader
/// of this project's own writing that guessed at the shape of a property list would be one more
/// thing to get right for no gain. Refused only where the system could not be asked at all.
#[cfg(target_os = "macos")]
fn what_the_bundle_says(bundle: &Path, key: &str) -> Result<Option<String>, UpdateError> {
    let said = std::process::Command::new("/usr/bin/defaults")
        .arg("read")
        .arg(bundle.join("Contents").join("Info"))
        .arg(key)
        .output()
        .map_err(|_| UpdateError::NotOursToReplace)?;

    Ok(said
        .status
        .success()
        .then(|| String::from_utf8_lossy(&said.stdout).trim().to_owned()))
}

/// The oldest system a bundle runs on, as a copy running as `machine` reads it out of the bundle:
/// what its property list asks for, or what the code for that kind of processor was built for,
/// whichever is newer. Nothing where neither names one.
///
/// Refused where the bundle holds no code for `machine`, or says something about either that
/// cannot be read: a bundle this copy cannot tell runs here is not one it puts in its own place.
/// The release step asks the same of every archive before it is published.
#[cfg(target_os = "macos")]
pub(super) fn the_system_it_needs(
    arrived: &Path,
    machine: &str,
) -> Result<Option<SystemVersion>, UpdateError> {
    let asks_for = match what_the_bundle_says(arrived, "LSMinimumSystemVersion")? {
        Some(written) => Some(SystemVersion::read(&written).ok_or(UpdateError::NotOursToReplace)?),
        None => None,
    };

    // A name inside the bundle's own folder for code, and nothing that leaves it.
    let executable = what_the_bundle_says(arrived, "CFBundleExecutable")?
        .filter(|name| !name.is_empty() && name != "." && name != ".." && !name.contains('/'))
        .ok_or(UpdateError::NotOursToReplace)?;

    let mut code = std::fs::File::open(arrived.join("Contents").join("MacOS").join(executable))
        .map_err(|_| UpdateError::NotOursToReplace)?;
    let code = the_code_for(&mut code, machine).ok_or(UpdateError::NotOursToReplace)?;

    Ok(asks_for.max(code.built_for))
}

/// Whether the bundle that came down runs on this Mac: a system at least as new as the one it
/// needs, and code for the kind of processor this copy runs as.
///
/// The kind is this copy's own rather than the machine's, because it is the one the manifest was
/// asked for: a copy running translated asks for, and is given, the archive of the kind it runs as.
#[cfg(target_os = "macos")]
fn it_runs_on_this_mac(arrived: &Path) -> Result<(), UpdateError> {
    let Some(needs) = the_system_it_needs(arrived, std::env::consts::ARCH)? else {
        return Ok(());
    };

    match this_macs_version().is_some_and(|this_mac| needs <= this_mac) {
        true => Ok(()),
        false => Err(UpdateError::NotOursToReplace),
    }
}

/// The version of macOS this Mac runs, as the system says it.
#[cfg(target_os = "macos")]
fn this_macs_version() -> Option<SystemVersion> {
    let said = std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        // Set, it makes the system answer with the version software too old to read a newer one
        // expects, rather than with the one it is.
        .env_remove("SYSTEM_VERSION_COMPAT")
        .output()
        .ok()?;

    match said.status.success() {
        true => SystemVersion::read(&String::from_utf8_lossy(&said.stdout)),
        false => None,
    }
}

/// A version of macOS, as a property list, the system or an executable's header names one. Ordered
/// part by part, so comparing two is comparing the systems.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct SystemVersion(u32, u32, u32);

#[cfg(target_os = "macos")]
impl SystemVersion {
    /// One to three numbers between dots, the way `LSMinimumSystemVersion` and `sw_vers` write
    /// one: `11`, `10.15`, `15.6.1`. Anything else is not read as a version.
    fn read(written: &str) -> Option<Self> {
        let parts = written
            .trim()
            .split('.')
            .map(
                |part| match !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) {
                    true => part.parse::<u32>().ok(),
                    false => None,
                },
            )
            .collect::<Option<Vec<u32>>>()?;

        match parts[..] {
            [major] => Some(Self(major, 0, 0)),
            [major, minor] => Some(Self(major, minor, 0)),
            [major, minor, patch] => Some(Self(major, minor, patch)),
            _ => None,
        }
    }

    /// As a Mach-O load command packs one: the major version in the top sixteen bits, then a byte
    /// each for the minor version and the patch.
    fn packed(packed: u32) -> Self {
        Self(packed >> 16, (packed >> 8) & 0xff, packed & 0xff)
    }
}

/// What an executable's header says about the code in it for one kind of processor.
#[cfg(target_os = "macos")]
#[derive(Debug, PartialEq, Eq)]
struct Code {
    /// The oldest system it was built for, where its load commands say.
    built_for: Option<SystemVersion>,
}

/// The code in an executable that a copy running as `machine` — `aarch64` or `x86_64`, as
/// `std::env::consts::ARCH` names them — would run, read from the executable's own header.
///
/// Nothing where it holds no such code, or where what is there does not read as a Mach-O header. A
/// thin file holds one kind; a universal one holds a table of slices, one per kind, each with a
/// header of its own. Nothing past the headers is read.
#[cfg(target_os = "macos")]
fn the_code_for(
    executable: &mut (impl std::io::Read + std::io::Seek),
    machine: &str,
) -> Option<Code> {
    use mach_o::{be_u32, be_u64, Kind, FAT_MAGIC, FAT_MAGIC_64, MOST_SLICES};
    use std::io::SeekFrom;

    let wanted = Kind::for_machine(machine)?;

    let magic = be_u32(executable)?;
    if magic != FAT_MAGIC && magic != FAT_MAGIC_64 {
        executable.seek(SeekFrom::Start(0)).ok()?;
        return mach_o::the_slice_here(executable, wanted);
    }

    let slices = be_u32(executable)?;
    if slices > MOST_SLICES {
        return None;
    }

    let mut found = None;
    for _ in 0..slices {
        let (cputype, cpusubtype) = (be_u32(executable)?, be_u32(executable)?);
        let offset = match magic == FAT_MAGIC_64 {
            true => be_u64(executable)?,
            false => u64::from(be_u32(executable)?),
        };
        // What is left of the entry: its size and alignment, and on the wide table a reserved word.
        let rest = match magic == FAT_MAGIC_64 {
            true => 16,
            false => 8,
        };
        executable.seek(SeekFrom::Current(rest)).ok()?;

        if found.is_none() && wanted.is(cputype, cpusubtype) {
            found = Some(offset);
        }
    }

    executable.seek(SeekFrom::Start(found?)).ok()?;
    mach_o::the_slice_here(executable, wanted)
}

/// The few numbers a Mach-O header is read by, as `<mach-o/loader.h>`, `<mach-o/fat.h>` and
/// `<mach/machine.h>` name them, and the reading of one slice's header.
#[cfg(target_os = "macos")]
mod mach_o {
    use std::io::Read;

    use super::{Code, SystemVersion};

    /// A universal file's table, with 32-bit and with 64-bit offsets. Written big-endian.
    pub(super) const FAT_MAGIC: u32 = 0xcafe_babe;
    pub(super) const FAT_MAGIC_64: u32 = 0xcafe_babf;
    /// A 64-bit slice, written in the order of the processor it is for, which on a Mac is
    /// little-endian.
    const MH_MAGIC_64: u32 = 0xfeed_facf;

    const CPU_TYPE_X86_64: u32 = 0x0100_0007;
    const CPU_TYPE_ARM64: u32 = 0x0100_000c;
    /// The top byte of a subtype holds capabilities rather than a kind.
    const CPU_SUBTYPE_MASK: u32 = 0xff00_0000;
    const CPU_SUBTYPE_X86_64_ALL: u32 = 3;
    const CPU_SUBTYPE_ARM64_ALL: u32 = 0;
    const CPU_SUBTYPE_ARM64_V8: u32 = 1;

    const LC_VERSION_MIN_MACOSX: u32 = 0x24;
    const LC_BUILD_VERSION: u32 = 0x32;
    const PLATFORM_MACOS: u32 = 1;

    /// More slices than any universal file holds, and more load-command bytes than any executable
    /// carries: a header claiming either is not read on.
    pub(super) const MOST_SLICES: u32 = 16;
    const MOST_COMMAND_BYTES: u32 = 4 * 1024 * 1024;

    /// A kind of code a copy can run as.
    #[derive(Debug, Clone, Copy)]
    pub(super) enum Kind {
        Arm64,
        X86_64,
    }

    impl Kind {
        pub(super) fn for_machine(machine: &str) -> Option<Self> {
            match machine {
                "aarch64" => Some(Self::Arm64),
                "x86_64" => Some(Self::X86_64),
                _ => None,
            }
        }

        /// Whether a header's processor type and subtype are this kind. `arm64e` is not `arm64`,
        /// nor `x86_64h` `x86_64`: a release is built as the plain kind, and the release step asks
        /// for it by that name.
        pub(super) fn is(self, cputype: u32, cpusubtype: u32) -> bool {
            let subtype = cpusubtype & !CPU_SUBTYPE_MASK;
            match self {
                Self::Arm64 => {
                    cputype == CPU_TYPE_ARM64
                        && matches!(subtype, CPU_SUBTYPE_ARM64_ALL | CPU_SUBTYPE_ARM64_V8)
                }
                Self::X86_64 => cputype == CPU_TYPE_X86_64 && subtype == CPU_SUBTYPE_X86_64_ALL,
            }
        }
    }

    /// The slice whose header starts where `executable` stands: its own header has to name the
    /// kind wanted, and its load commands say which system it was built for, where they say.
    pub(super) fn the_slice_here(executable: &mut impl Read, wanted: Kind) -> Option<Code> {
        let mut header = [0u8; 32];
        executable.read_exact(&mut header).ok()?;
        let word = |at: usize| le_u32(&header, at);

        if word(0)? != MH_MAGIC_64 || !wanted.is(word(4)?, word(8)?) {
            return None;
        }

        let (commands, size) = (word(16)?, word(20)?);
        if size > MOST_COMMAND_BYTES {
            return None;
        }
        let mut table = vec![0u8; size as usize];
        executable.read_exact(&mut table).ok()?;

        let mut at = 0usize;
        let mut built_for = None;
        for _ in 0..commands {
            // One command, read within its own length and no further.
            let length = le_u32(&table, at + 4)? as usize;
            let command = table.get(at..at + length).filter(|_| length >= 8)?;

            match le_u32(command, 0)? {
                LC_BUILD_VERSION if le_u32(command, 8)? == PLATFORM_MACOS => {
                    built_for = Some(SystemVersion::packed(le_u32(command, 12)?));
                }
                LC_VERSION_MIN_MACOSX => {
                    built_for = Some(SystemVersion::packed(le_u32(command, 8)?));
                }
                _ => {}
            }
            at += length;
        }

        Some(Code { built_for })
    }

    fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    }

    pub(super) fn be_u32(from: &mut impl Read) -> Option<u32> {
        let mut word = [0u8; 4];
        from.read_exact(&mut word).ok()?;
        Some(u32::from_be_bytes(word))
    }

    pub(super) fn be_u64(from: &mut impl Read) -> Option<u64> {
        let mut word = [0u8; 8];
        from.read_exact(&mut word).ok()?;
        Some(u64::from_be_bytes(word))
    }
}

/// Whether what came down is the same application as the one it would replace.
///
/// **Asked of the signature, not of what the signature says about itself.** The running copy's
/// designated requirement — its bundle identifier, its team, and Apple's certificate authority above
/// both — is read off the running copy, and the bundle that came down has to meet it: `codesign`
/// checks every file the arrived bundle's signature seals, and the certificates it was made with,
/// against that requirement. Two requirements compared as text prove nothing, because the text is
/// something a signer writes. A bundle signed ad hoc carries this application's requirement word for
/// word when it is told to, and so does a genuine bundle whose executable was changed after it was
/// signed; both pass a comparison, and both are refused here.
#[cfg(target_os = "macos")]
pub(super) fn the_identity_must_match(running: &Path, arrived: &Path) -> Result<(), UpdateError> {
    let requirement = the_requirement_of(running)?;

    if !an_update_can_be_held_to(&requirement) {
        return Err(UpdateError::NotOursToReplace);
    }

    it_meets(arrived, &requirement)
}

/// Whether the running copy's requirement is one an update can be held to.
///
/// Apple's certificate authority at its root and, where any developer's certificate would do, one
/// team's in particular. A copy signed ad hoc carries a requirement naming the hash of its own
/// build, which names no authority and which nothing else ever meets. One naming only an identifier,
/// or Apple without a team, is met by somebody else's signature and leaves the seal standing alone.
/// Either way the copy is not one an update replaces. Read off the text as `codesign` prints it,
/// which is enough for what it guards: a release signed by mistake under something weaker.
#[cfg(target_os = "macos")]
fn an_update_can_be_held_to(requirement: &str) -> bool {
    let apples = requirement.contains("anchor apple");
    let any_developer = requirement.contains("anchor apple generic");
    let one_team = requirement.contains("certificate leaf[subject.OU]");

    apples && (!any_developer || one_team)
}

/// Whether a bundle's signature holds over every file it seals, and meets `requirement`.
#[cfg(target_os = "macos")]
fn it_meets(bundle: &Path, requirement: &str) -> Result<(), UpdateError> {
    // The leading `=` makes the argument the requirement itself rather than a file holding one.
    // `--strict` refuses what a lenient reading lets past — a link inside the bundle that points out
    // of it among them — and `--deep` reaches the code nested inside the bundle.
    let verified = std::process::Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict", "-R"])
        .arg(format!("={requirement}"))
        .arg("--")
        .arg(bundle)
        .output()
        .map_err(|_| UpdateError::NotOursToReplace)?;

    match verified.status.success() {
        true => Ok(()),
        false => Err(UpdateError::NotOurs),
    }
}

/// The designated requirement a bundle's signature carries, as `codesign` prints it.
#[cfg(target_os = "macos")]
fn the_requirement_of(bundle: &Path) -> Result<String, UpdateError> {
    let said = std::process::Command::new("/usr/bin/codesign")
        .args(["-d", "-r-", "--"])
        .arg(bundle)
        .output()
        .map_err(|_| UpdateError::NotOursToReplace)?;

    let printed =
        String::from_utf8_lossy(&said.stdout).to_string() + &String::from_utf8_lossy(&said.stderr);

    printed
        .lines()
        .find_map(|line| line.split_once("designated =>"))
        .map(|(_, rule)| rule.trim().to_owned())
        .ok_or(UpdateError::NotOursToReplace)
}

/// The most an archive may unpack to: the bytes of the tar stream inside its gzip.
///
/// The application is tens of megabytes and an archive is held to `LARGEST` before it comes down;
/// this is room for one several times larger, and a stop for a stream that has decided to expand
/// for ever. The release step unpacks every archive under the same ceiling before it is published,
/// so a release that passed there is one that fits here.
#[cfg(target_os = "macos")]
pub(super) const UNPACKS_TO_AT_MOST: u64 = 1024 * 1024 * 1024;

/// Unpacks a gzipped tar into `into`, refusing anything that would write outside it and an archive
/// that unpacks to more than `at_most` bytes.
///
/// The gzip is opened here and `tar` is handed the stream through a pipe this process holds, so
/// what it unpacks is counted as it goes — and an application that quits, or is stopped any other
/// way, takes the pipe with it: the `tar` it started reads the end of the stream and stops, rather
/// than carrying on writing into a folder nobody will look at again.
#[cfg(target_os = "macos")]
pub(super) fn unpack(archive: &Path, into: &Path, at_most: u64) -> Result<(), UpdateError> {
    use std::process::{Command, Stdio};

    let compressed = std::fs::File::open(archive).map_err(|_| UpdateError::Unavailable)?;
    let mut stream = flate2::read::GzDecoder::new(std::io::BufReader::new(compressed));

    // `--strip-components` is deliberately absent: the archive holds one bundle at its top and
    // nothing above it, and stripping would let an archive with a different shape put its contents
    // somewhere this did not mean. `-P` is absent too, so an absolute path inside is refused rather
    // than honoured.
    let mut tar = Command::new("/usr/bin/tar")
        .arg("-xf")
        .arg("-")
        .arg("-C")
        .arg(into)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| UpdateError::Unavailable)?;

    let mut pipe = tar.stdin.take();
    let fed = match pipe.as_mut() {
        Some(pipe) => fed_at_most(&mut stream, pipe, at_most),
        None => Err(UpdateError::Unavailable),
    };
    // A stream refused halfway is not left for `tar` to finish on its own: it is stopped while the
    // pipe is still open, before it can read the end of the stream and close off the file it was
    // writing as though it were whole.
    if fed.is_err() {
        let _ = tar.kill();
    }
    // Closing the pipe is what tells `tar` the stream is over.
    drop(pipe);
    let unpacked = tar.wait().map_err(|_| UpdateError::Unavailable)?;
    fed?;

    // The archive's seal and its digest held before it got here, so one that will not unpack is
    // this machine's trouble — a full disk, a folder it cannot write — and not somebody else's file.
    match unpacked.success() {
        true => Ok(()),
        false => Err(UpdateError::Unavailable),
    }
}

/// Copies the stream into the pipe, and refuses it once it runs past `at_most` bytes.
///
/// Running past the ceiling is the archive's doing rather than this machine's: no release the
/// release step let through does it, so the stream is said to be not this application's.
#[cfg(target_os = "macos")]
fn fed_at_most(
    stream: &mut impl std::io::Read,
    pipe: &mut impl std::io::Write,
    at_most: u64,
) -> Result<(), UpdateError> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut so_far = 0u64;

    loop {
        let read = match stream.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => read,
            Err(why) if why.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(UpdateError::Unavailable),
        };

        so_far += read as u64;
        if so_far > at_most {
            return Err(UpdateError::NotOurs);
        }

        match pipe.write_all(&buffer[..read]) {
            Ok(()) => {}
            // `tar` stopped reading: at the end of the archive, or over a failure of its own. Which
            // of the two it was is its answer to give, and it gives it as it exits.
            Err(why) if why.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
            Err(_) => return Err(UpdateError::Unavailable),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing is replaced on a system that installs software another way.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn nothing_is_replaced_where_the_system_owns_the_application() {
        let staged = Staged {
            version: lyonoyl_domain::version::Version::parse("9.9.9").expect("a version"),
            at: Token::of("/nowhere".to_owned()),
        };

        assert_eq!(
            swap_it_in(Path::new("/nowhere"), &staged),
            Err(UpdateError::NotOursToReplace)
        );
    }

    /// A binary that is not inside a bundle has nothing to replace.
    ///
    /// Which is every development build: `cargo` leaves it in `target`, and an updater that swapped
    /// something there would be replacing a file the next build overwrites anyway.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_build_running_outside_a_bundle_is_not_one_to_replace() {
        // The test binary is in `target/debug/deps`, which is exactly that case.
        assert_eq!(
            the_bundle_this_is_running_from(),
            Err(UpdateError::NotOursToReplace)
        );
    }

    /// Somewhere of its own for one test, empty.
    fn a_place(name: &str) -> PathBuf {
        let at =
            std::env::temp_dir().join(format!("lyonoyl-staging-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).expect("a place");
        at
    }

    /// Calculator, which every Mac has and Apple signed: a copy whose requirement nobody but Apple
    /// can meet.
    #[cfg(target_os = "macos")]
    const APPLES: &str = "/System/Applications/Calculator.app";

    /// A requirement a test can meet without a certificate: an identifier and nothing else, which a
    /// bundle signed ad hoc satisfies.
    #[cfg(target_os = "macos")]
    const NAMED_ONLY: &str = r#"identifier "com.example.lyonoyl""#;

    /// A bundle holding somebody else's code — a tool the system ships, under this application's
    /// executable name — signed ad hoc, with `requirement` written into it where one is given.
    #[cfg(target_os = "macos")]
    fn a_bundle(at: &Path, identifier: &str, requirement: Option<&str>) -> PathBuf {
        let bundle = at.join("Lyonoyl Mail.app");
        let contents = bundle.join("Contents");
        std::fs::create_dir_all(contents.join("MacOS")).expect("a bundle");
        std::fs::copy("/bin/echo", contents.join("MacOS").join("lyonoyl")).expect("an executable");
        std::fs::write(
            contents.join("Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleExecutable</key><string>lyonoyl</string>\
                 <key>CFBundleIdentifier</key><string>{identifier}</string>\
                 </dict></plist>\n"
            ),
        )
        .expect("its property list");

        signed(&bundle, requirement);

        bundle
    }

    /// Signs a bundle ad hoc over whatever is in it now, with `requirement` written into it where
    /// one is given.
    #[cfg(target_os = "macos")]
    fn signed(bundle: &Path, requirement: Option<&str>) {
        let mut signing = std::process::Command::new("/usr/bin/codesign");
        signing.args(["--force", "--sign", "-"]);
        if let Some(requirement) = requirement {
            signing
                .arg("--requirements")
                .arg(format!("=designated => {requirement}"));
        }
        let done = signing
            .arg("--")
            .arg(bundle)
            .output()
            .expect("codesign runs");
        assert!(
            done.status.success(),
            "{}",
            String::from_utf8_lossy(&done.stderr)
        );
    }

    /// The requirement a signature carries is text its signer wrote, and carrying it proves nothing.
    ///
    /// Somebody else's code, signed ad hoc and told to carry Calculator's own requirement. The two
    /// print the same, word for word — which is all a comparison of the text could ever see — and
    /// the bundle is refused, because it does not meet that requirement: nothing Apple signed is in
    /// it.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_carrying_the_running_copys_requirement_word_for_word_is_refused() {
        let running = Path::new(APPLES);
        let requirement = the_requirement_of(running).expect("Calculator's requirement");
        let at = a_place("forged");
        let forged = a_bundle(&at, "com.apple.calculator", Some(&requirement));

        assert_eq!(
            the_requirement_of(&forged).as_deref(),
            Ok(requirement.as_str()),
            "the forged bundle carries a different requirement, so this tests something else"
        );
        assert_eq!(
            the_identity_must_match(running, &forged),
            Err(UpdateError::NotOurs)
        );

        std::fs::remove_dir_all(&at).ok();
    }

    /// The running copy meets its own requirement, so what is refused here is what was forged
    /// rather than everything.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_same_application_is_accepted() {
        assert_eq!(
            the_identity_must_match(Path::new(APPLES), Path::new(APPLES)),
            Ok(())
        );
    }

    /// A bundle that met the requirement when it was signed, and was changed afterwards, is refused —
    /// though it carries the same requirement and the same signature it always did.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_changed_after_it_was_signed_is_refused() {
        use std::io::Write;

        let at = a_place("changed");
        let arrived = a_bundle(&at, "com.example.lyonoyl", Some(NAMED_ONLY));
        assert_eq!(
            it_meets(&arrived, NAMED_ONLY),
            Ok(()),
            "refused before anything was changed, so the refusal below proves nothing"
        );

        // One byte added to the end of the executable.
        std::fs::OpenOptions::new()
            .append(true)
            .open(arrived.join("Contents").join("MacOS").join("lyonoyl"))
            .and_then(|mut executable| executable.write_all(b"x"))
            .expect("changed");

        assert_eq!(it_meets(&arrived, NAMED_ONLY), Err(UpdateError::NotOurs));

        std::fs::remove_dir_all(&at).ok();
    }

    /// A file put into a signed bundle afterwards is refused the same way: everything the signature
    /// seals is checked, not only the executable.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_file_added_to_a_signed_bundle_is_refused() {
        let at = a_place("added");
        let arrived = a_bundle(&at, "com.example.lyonoyl", Some(NAMED_ONLY));
        assert_eq!(
            it_meets(&arrived, NAMED_ONLY),
            Ok(()),
            "refused before anything was added, so the refusal below proves nothing"
        );

        let resources = arrived.join("Contents").join("Resources");
        std::fs::create_dir_all(&resources).expect("a folder");
        std::fs::write(resources.join("extra"), b"x").expect("added");

        assert_eq!(it_meets(&arrived, NAMED_ONLY), Err(UpdateError::NotOurs));

        std::fs::remove_dir_all(&at).ok();
    }

    /// A bundle signed with a link in it that points out of the bundle is refused, though every byte
    /// of it is as it was signed: where the link lands is outside anything the signature vouches for,
    /// and only a strict reading looks.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_holding_a_link_out_of_itself_is_refused() {
        let at = a_place("link");
        let arrived = a_bundle(&at, "com.example.lyonoyl", Some(NAMED_ONLY));
        assert_eq!(
            it_meets(&arrived, NAMED_ONLY),
            Ok(()),
            "refused before the link was there, so the refusal below proves nothing"
        );

        let resources = arrived.join("Contents").join("Resources");
        std::fs::create_dir_all(&resources).expect("a folder");
        std::os::unix::fs::symlink("/etc/hosts", resources.join("away")).expect("a link out");
        signed(&arrived, Some(NAMED_ONLY));

        assert_eq!(it_meets(&arrived, NAMED_ONLY), Err(UpdateError::NotOurs));

        std::fs::remove_dir_all(&at).ok();
    }

    /// A copy signed ad hoc is named by the hash of its own build, which nothing else will ever
    /// match. It is not a copy an update replaces, and that is said before anything moves.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_running_copy_signed_ad_hoc_is_not_one_to_replace() {
        let at = a_place("ad-hoc");
        let running = a_bundle(&at, "com.example.lyonoyl", None);

        assert_eq!(
            the_identity_must_match(&running, &running),
            Err(UpdateError::NotOursToReplace)
        );

        std::fs::remove_dir_all(&at).ok();
    }

    /// A copy whose requirement names only its identifier is met by anybody's signature under that
    /// name, so an update held to it would be held by the seal alone. It is not one to replace.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_running_copy_named_only_by_its_identifier_is_not_one_to_replace() {
        let at = a_place("named-only");
        let running = a_bundle(&at, "com.example.lyonoyl", Some(NAMED_ONLY));

        assert_eq!(
            the_identity_must_match(&running, &running),
            Err(UpdateError::NotOursToReplace)
        );

        std::fs::remove_dir_all(&at).ok();
    }

    /// Apple at the root, and one team where any developer's certificate would do — the shape a
    /// Developer ID signature carries, with a team identifier that is nobody's.
    #[cfg(target_os = "macos")]
    #[test]
    fn only_a_requirement_naming_apple_and_a_team_holds_an_update() {
        let developer = concat!(
            r#"identifier "com.example.lyonoyl" and anchor apple generic and "#,
            r#"certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and "#,
            r#"certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and "#,
            r#"certificate leaf[subject.OU] = "0000000000""#,
        );
        assert!(an_update_can_be_held_to(developer));
        assert!(an_update_can_be_held_to(
            r#"identifier "com.apple.calculator" and anchor apple"#
        ));

        for weak in [
            r#"cdhash H"0000000000000000000000000000000000000000""#,
            NAMED_ONLY,
            r#"identifier "com.example.lyonoyl" and anchor apple generic"#,
        ] {
            assert!(
                !an_update_can_be_held_to(weak),
                "{weak} was taken as a requirement an update can be held to"
            );
        }
    }

    /// One bundle at the top of an archive, and nothing else named like one.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_archive_holds_one_bundle_and_no_other() {
        let at = a_place("one");
        assert_eq!(
            one_bundle_inside(&at),
            Err(UpdateError::NotOurs),
            "an archive holding nothing was taken"
        );

        std::fs::create_dir_all(at.join("Lyonoyl Mail.app")).expect("a bundle");
        assert_eq!(one_bundle_inside(&at), Ok(at.join("Lyonoyl Mail.app")));

        std::fs::create_dir_all(at.join("Another.app")).expect("a second");
        assert_eq!(
            one_bundle_inside(&at),
            Err(UpdateError::NotOurs),
            "an archive holding two was taken"
        );

        std::fs::remove_dir_all(&at).ok();
    }

    /// A link named like a bundle is not one, wherever it points: the checks would follow it, and
    /// the swap would put the link itself where the application was.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_link_named_like_a_bundle_is_not_a_bundle() {
        let at = a_place("linked");
        let elsewhere = at.join("payload").join("Lyonoyl Mail.app");
        std::fs::create_dir_all(&elsewhere).expect("a bundle somewhere else");
        std::os::unix::fs::symlink(&elsewhere, at.join("Lyonoyl Mail.app")).expect("a link");

        assert_eq!(one_bundle_inside(&at), Err(UpdateError::NotOurs));

        std::fs::remove_dir_all(&at).ok();
    }

    /// A bundle of another version than the one offered is refused, whatever else holds.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_of_another_version_than_the_one_offered_is_refused() {
        let at = a_place("version");
        let bundle = at.join("Lyonoyl Mail.app");
        std::fs::create_dir_all(bundle.join("Contents")).expect("a bundle");
        std::fs::write(
            bundle.join("Contents").join("Info.plist"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
             <key>CFBundleShortVersionString</key><string>1.1.0</string>\
             </dict></plist>\n",
        )
        .expect("its property list");

        let offered = |version: &str| Staged {
            version: lyonoyl_domain::version::Version::parse(version).expect("a version"),
            at: Token::of("/nowhere".to_owned()),
        };

        assert_eq!(
            the_version_must_be_the_one_offered(&bundle, &offered("1.1.0")),
            Ok(())
        );
        assert_eq!(
            the_version_must_be_the_one_offered(&bundle, &offered("1.2.0")),
            Err(UpdateError::NotTheVersionOffered)
        );

        std::fs::remove_dir_all(&at).ok();
    }

    /// An archive that will not unpack is this machine's trouble — its seal and digest held before
    /// it got here — and is said as that rather than as somebody else's file.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_archive_that_will_not_unpack_is_this_machines_trouble() {
        let at = a_place("unpack");
        let archive = at.join("archive.tar.gz");
        std::fs::write(&archive, b"not an archive").expect("written");
        std::fs::create_dir_all(at.join("into")).expect("somewhere to unpack");

        assert_eq!(
            unpack(&archive, &at.join("into"), UNPACKS_TO_AT_MOST),
            Err(UpdateError::Unavailable)
        );

        std::fs::remove_dir_all(&at).ok();
    }

    /// Bytes no compressor can shrink, the same on every run.
    #[cfg(target_os = "macos")]
    fn noise(length: usize) -> Vec<u8> {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        (0..length)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }

    /// A gzipped tar of `files` under `at`, made by the system's own `tar`, entries in the order
    /// given and nothing of this machine in it.
    #[cfg(target_os = "macos")]
    fn an_archive_of(at: &Path, files: &[(&str, &[u8])]) -> PathBuf {
        let source = at.join("source");
        for (name, content) in files {
            let path = source.join(name);
            std::fs::create_dir_all(path.parent().expect("inside the source")).expect("a folder");
            std::fs::write(&path, content).expect("a file");
        }

        let archive = at.join("archive.tar.gz");
        let made = std::process::Command::new("/usr/bin/tar")
            .env("COPYFILE_DISABLE", "1")
            .args(["--no-mac-metadata", "--no-xattrs", "-czf"])
            .arg(&archive)
            .arg("-C")
            .arg(&source)
            .args(files.iter().map(|(name, _)| name))
            .output()
            .expect("tar runs");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );

        archive
    }

    /// How many bytes an archive's tar stream is, which is what the ceiling counts.
    #[cfg(target_os = "macos")]
    fn unpacks_to(archive: &Path) -> u64 {
        use std::io::Read;

        let mut stream = Vec::new();
        flate2::read::GzDecoder::new(std::fs::File::open(archive).expect("the archive"))
            .read_to_end(&mut stream)
            .expect("a gzip");
        stream.len() as u64
    }

    /// An archive unpacks to as much as it may and not a byte more: at the ceiling it is taken,
    /// one byte under the ceiling it is refused.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_archive_is_taken_up_to_the_ceiling_and_refused_past_it() {
        let at = a_place("ceiling");
        let archive = an_archive_of(&at, &[("inside", b"what the archive holds")]);
        let whole = unpacks_to(&archive);

        std::fs::create_dir_all(at.join("at")).expect("somewhere to unpack");
        assert_eq!(unpack(&archive, &at.join("at"), whole), Ok(()));
        assert_eq!(
            std::fs::read(at.join("at").join("inside")).expect("unpacked"),
            b"what the archive holds"
        );

        std::fs::create_dir_all(at.join("under")).expect("somewhere to unpack");
        assert_eq!(
            unpack(&archive, &at.join("under"), whole - 1),
            Err(UpdateError::NotOurs)
        );

        std::fs::remove_dir_all(&at).ok();
    }

    /// A small archive that expands to far more than it may is stopped near the ceiling, rather
    /// than unpacked whole and judged afterwards.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_archive_that_expands_past_the_ceiling_is_stopped_on_the_way() {
        let at = a_place("expands");
        // Not zeros, which `tar` writes as a hole rather than as bytes on the disk.
        let expands = vec![7u8; 4 * 1024 * 1024];
        let archive = an_archive_of(&at, &[("expands", &expands)]);
        assert!(
            std::fs::metadata(&archive).expect("the archive").len() < 256 * 1024,
            "the archive does not compress, so this tests something else"
        );
        let into = at.join("into");
        std::fs::create_dir_all(&into).expect("somewhere to unpack");

        assert_eq!(
            unpack(&archive, &into, 256 * 1024),
            Err(UpdateError::NotOurs)
        );
        let written = std::fs::metadata(into.join("expands")).map_or(0, |file| file.len());
        assert!(
            written <= 256 * 1024,
            "{written} bytes were unpacked under a ceiling of a quarter of a megabyte"
        );

        std::fs::remove_dir_all(&at).ok();
    }

    /// The variable that turns the test below into the application it stops.
    #[cfg(target_os = "macos")]
    const UNPACKING_IN: &str = "LYONOYL_TEST_UNPACKING_IN";

    /// The application in the test below: unpacks the archive arriving where it is pointed, which
    /// that test never finishes sending, until that test stops it. Run on its own it is pointed
    /// nowhere and returns.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "the application the test below starts and stops; run on its own it does nothing"]
    fn an_application_unpacking_an_archive() {
        let Some(at) = std::env::var_os(UNPACKING_IN).map(PathBuf::from) else {
            return;
        };

        let _ = unpack(&at.join("arriving"), &at.join("into"), UNPACKS_TO_AT_MOST);
    }

    /// Waits up to twenty seconds for `done`, and says whether it came.
    #[cfg(target_os = "macos")]
    fn until(mut done: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < deadline {
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        done()
    }

    /// The processes whose command line names `into`, which is what is unpacking there.
    #[cfg(target_os = "macos")]
    fn unpacking_into(into: &Path) -> Vec<u32> {
        let listed = std::process::Command::new("/bin/ps")
            .args(["-axww", "-o", "pid=,command="])
            .output()
            .expect("ps runs");
        let into = into.to_string_lossy();

        String::from_utf8_lossy(&listed.stdout)
            .lines()
            .filter(|line| line.contains(into.as_ref()))
            .filter_map(|line| line.split_whitespace().next()?.parse().ok())
            .collect()
    }

    /// An application that quits while an archive is unpacking takes the unpacking with it.
    ///
    /// Half the archive arrives and the rest never does, which is what a slow disk or a large
    /// archive looks like from where `tar` stands. The application is then stopped the way quitting
    /// stops it — nothing of it runs afterwards, no destructor and no exit hook — and no `tar` it
    /// started may be left writing into a folder nobody will look at again.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_application_that_quits_while_unpacking_leaves_no_unpacking_behind() {
        use std::io::Write;

        let at = a_place("quitting");
        let archive = an_archive_of(&at, &[("first", b"x"), ("rest", &noise(1024 * 1024))]);
        let bytes = std::fs::read(&archive).expect("the archive");
        let (arriving, into) = (at.join("arriving"), at.join("into"));
        std::fs::create_dir_all(&into).expect("somewhere to unpack");
        let made = std::process::Command::new("/usr/bin/mkfifo")
            .arg(&arriving)
            .status()
            .expect("mkfifo runs");
        assert!(made.success(), "no pipe to send the archive through");

        let mut application = std::process::Command::new(std::env::current_exe().expect("this"))
            .args([
                "--exact",
                "infrastructure::updates::staging::tests::an_application_unpacking_an_archive",
                "--ignored",
                "--test-threads=1",
            ])
            .env(UNPACKING_IN, &at)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the application starts");

        // Opens once something opens the other end: the application, or a `tar` it handed the
        // archive's name to.
        let mut sending = std::fs::OpenOptions::new()
            .write(true)
            .open(&arriving)
            .expect("the pipe opens");
        sending
            .write_all(&bytes[..bytes.len() / 2])
            .expect("half the archive is sent");

        let started = until(|| into.join("first").exists());
        let _ = application.kill();
        let _ = application.wait();
        let stopped = started && until(|| unpacking_into(&into).is_empty());

        // Whatever is left is stopped here, so a failure does not outlive the test.
        for left in unpacking_into(&into) {
            let _ = std::process::Command::new("/bin/kill")
                .args(["-9", &left.to_string()])
                .status();
        }
        drop(sending);
        std::fs::remove_dir_all(&at).ok();

        assert!(started, "nothing was unpacked, so this proves nothing");
        assert!(
            stopped,
            "a tar the application started went on unpacking after the application quit"
        );
    }

    /// A version waiting to be swapped in, laid out as a swap finds one: the running copy among
    /// the applications, and the archive of the next one where this application keeps its files.
    #[cfg(target_os = "macos")]
    struct ASwap {
        at: PathBuf,
        applications: PathBuf,
        running: PathBuf,
        data: PathBuf,
        staged: Staged,
    }

    #[cfg(target_os = "macos")]
    impl ASwap {
        fn laid_out(name: &str) -> Self {
            let at = a_place(name);
            let applications = at.join("Applications");
            let running = applications.join("Lyonoyl Mail.app");
            std::fs::create_dir_all(running.join("Contents")).expect("the running copy");
            std::fs::write(running.join("Contents").join("which"), "running").expect("marked");

            let next = an_archive_of(&at, &[("Lyonoyl Mail.app/Contents/which", b"arrived")]);
            let data = at.join("data");
            let bytes = std::fs::read(&next).expect("the archive");
            let staged = put_it_somewhere(
                &data,
                &Offered {
                    version: lyonoyl_domain::version::Version::parse("1.2.0").expect("a version"),
                    bytes: bytes.len() as u64,
                    notes: crate::ports::updates::Notes::new(),
                    file: Token::of("a.tar.gz".to_owned()),
                    digest: Token::of(String::new()),
                },
                &bytes,
            )
            .expect("waiting");

            Self {
                at,
                applications,
                running,
                data,
                staged,
            }
        }

        /// Which copy is where the application is.
        fn running_now(&self) -> String {
            std::fs::read_to_string(self.running.join("Contents").join("which")).expect("a copy")
        }

        /// What is in a folder, by name.
        fn in_folder(folder: &Path) -> Vec<String> {
            let mut named: Vec<String> = std::fs::read_dir(folder)
                .expect("the folder")
                .map(|entry| {
                    entry
                        .expect("an entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            named.sort();
            named
        }
    }

    #[cfg(target_os = "macos")]
    impl Drop for ASwap {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.at);
        }
    }

    /// A bundle that passes takes the running copy's place, and nothing else is left anywhere:
    /// not beside the application, and not where the archive was opened.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_that_passes_takes_the_running_copys_place_and_leaves_nothing_behind() {
        let swap = ASwap::laid_out("passes");

        assert_eq!(
            put_in_place_of(
                &swap.running,
                &swap.data,
                &swap.staged,
                on_one_volume,
                |_| Ok(())
            ),
            Ok(())
        );

        assert_eq!(swap.running_now(), "arrived");
        assert_eq!(ASwap::in_folder(&swap.applications), ["Lyonoyl Mail.app"]);
        assert_eq!(
            ASwap::in_folder(&swap.data.join("updates")),
            Vec::<String>::new()
        );
    }

    /// A bundle is opened and checked where this application keeps its own files, and one that is
    /// refused is gone from there without anything of it having been beside the running copy.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_that_is_refused_is_opened_and_cleared_away_outside_the_applications() {
        let swap = ASwap::laid_out("refused");
        let mut checked = None;

        let refused = put_in_place_of(
            &swap.running,
            &swap.data,
            &swap.staged,
            on_one_volume,
            |arrived| {
                checked = Some(arrived.to_path_buf());
                Err(UpdateError::NotOurs)
            },
        );

        assert_eq!(refused, Err(UpdateError::NotOurs));
        let checked = checked.expect("the bundle was checked");
        assert!(
            checked.starts_with(&swap.data),
            "checked at {checked:?}, outside the folder this application keeps its files in"
        );
        assert_eq!(swap.running_now(), "running");
        assert_eq!(ASwap::in_folder(&swap.applications), ["Lyonoyl Mail.app"]);
        assert_eq!(
            ASwap::in_folder(&swap.data.join("updates")),
            Vec::<String>::new()
        );
    }

    /// A copy on another volume than the folder this application keeps its files in — a home
    /// folder moved to another disk — has its archive opened beside it, where the bundle that
    /// passes is renamed into its place rather than copied across, and nothing is left in either
    /// folder.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_copy_on_another_volume_has_its_archive_opened_beside_it_and_is_replaced() {
        let swap = ASwap::laid_out("another-volume");
        let mut checked = None;

        let swapped = put_in_place_of(
            &swap.running,
            &swap.data,
            &swap.staged,
            |_, _| false,
            |arrived| {
                checked = Some(arrived.to_path_buf());
                Ok(())
            },
        );

        assert_eq!(swapped, Ok(()));
        let checked = checked.expect("the bundle was checked");
        assert!(
            checked.starts_with(&swap.applications),
            "opened at {checked:?}, on a volume a rename into the running copy's place cannot leave"
        );
        assert_eq!(swap.running_now(), "arrived");
        assert_eq!(ASwap::in_folder(&swap.applications), ["Lyonoyl Mail.app"]);
        assert_eq!(
            ASwap::in_folder(&swap.data.join("updates")),
            Vec::<String>::new()
        );
    }

    /// And one refused there leaves nothing beside the running copy.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_refused_beside_a_copy_on_another_volume_is_cleared_away() {
        let swap = ASwap::laid_out("another-volume-refused");

        let refused = put_in_place_of(
            &swap.running,
            &swap.data,
            &swap.staged,
            |_, _| false,
            |_| Err(UpdateError::NotOurs),
        );

        assert_eq!(refused, Err(UpdateError::NotOurs));
        assert_eq!(swap.running_now(), "running");
        assert_eq!(ASwap::in_folder(&swap.applications), ["Lyonoyl Mail.app"]);
        assert_eq!(
            ASwap::in_folder(&swap.data.join("updates")),
            Vec::<String>::new()
        );
    }

    /// Two folders are on one volume when the system says they are, and not when it says otherwise
    /// or cannot say.
    #[cfg(target_os = "macos")]
    #[test]
    fn folders_are_on_one_volume_only_when_the_system_says_so() {
        let at = a_place("volumes");
        let inside = at.join("inside");
        std::fs::create_dir_all(&inside).expect("a folder");

        assert!(on_one_volume(&at, &inside));
        // The device files are a file system of their own on every Mac.
        assert!(
            !on_one_volume(&at, Path::new("/dev")),
            "two volumes were taken for one"
        );
        assert!(!on_one_volume(&at, &at.join("not there")));

        std::fs::remove_dir_all(&at).ok();
    }

    /// What a test bundle's executable is for: the kind this copy runs as, or the other one.
    #[cfg(target_os = "macos")]
    fn this_kind() -> (u32, u32) {
        match std::env::consts::ARCH {
            "aarch64" => (0x0100_000c, 0),
            _ => (0x0100_0007, 3),
        }
    }

    #[cfg(target_os = "macos")]
    fn the_other_kind() -> (u32, u32) {
        match std::env::consts::ARCH {
            "aarch64" => (0x0100_0007, 3),
            _ => (0x0100_000c, 0),
        }
    }

    /// The load command a header names its system with.
    #[cfg(target_os = "macos")]
    #[derive(Clone, Copy)]
    enum Names {
        Nothing,
        /// `LC_BUILD_VERSION` for macOS, the major and minor version.
        Build(u32, u32),
        /// The older `LC_VERSION_MIN_MACOSX`.
        Minimum(u32, u32),
    }

    /// A thin 64-bit Mach-O header of one kind of code, with an unrelated load command first so
    /// the reading of the table is a walk, and the system it was built for named as `names` says.
    #[cfg(target_os = "macos")]
    fn a_thin_header((cputype, cpusubtype): (u32, u32), names: Names) -> Vec<u8> {
        let word = |value: u32| value.to_le_bytes();
        let mut commands = Vec::new();
        // LC_UUID: the command, its length, and sixteen bytes.
        commands.extend(word(0x1b));
        commands.extend(word(24));
        commands.extend([0u8; 16]);
        match names {
            Names::Nothing => {}
            Names::Build(major, minor) => {
                for value in [0x32, 24, 1, major << 16 | minor << 8, 0, 0] {
                    commands.extend(word(value));
                }
            }
            Names::Minimum(major, minor) => {
                for value in [0x24, 16, major << 16 | minor << 8, 0] {
                    commands.extend(word(value));
                }
            }
        }
        let count = match names {
            Names::Nothing => 1,
            _ => 2,
        };

        let mut header = Vec::new();
        for value in [
            0xfeed_facf,
            cputype,
            cpusubtype,
            2,
            count,
            commands.len() as u32,
            0,
            0,
        ] {
            header.extend(word(value));
        }
        header.extend(commands);
        header
    }

    /// A universal file holding `slices`, each at its own offset after the table; with 64-bit
    /// offsets where `wide`.
    #[cfg(target_os = "macos")]
    fn a_universal_file(slices: &[((u32, u32), Vec<u8>)], wide: bool) -> Vec<u8> {
        let mut file = Vec::new();
        file.extend(
            match wide {
                true => 0xcafe_babf_u32,
                false => 0xcafe_babe_u32,
            }
            .to_be_bytes(),
        );
        file.extend((slices.len() as u32).to_be_bytes());

        let entry = if wide { 32 } else { 20 };
        let mut offset = 4096_u64;
        let mut placed = Vec::new();
        for ((cputype, cpusubtype), slice) in slices {
            file.extend(cputype.to_be_bytes());
            file.extend(cpusubtype.to_be_bytes());
            match wide {
                true => {
                    file.extend(offset.to_be_bytes());
                    file.extend((slice.len() as u64).to_be_bytes());
                    file.extend(12_u32.to_be_bytes());
                    file.extend(0_u32.to_be_bytes());
                }
                false => {
                    file.extend((offset as u32).to_be_bytes());
                    file.extend((slice.len() as u32).to_be_bytes());
                    file.extend(12_u32.to_be_bytes());
                }
            }
            placed.push((offset, slice));
            offset += 4096 * (1 + slice.len() as u64 / 4096);
        }
        assert_eq!(
            file.len(),
            8 + entry * slices.len(),
            "the table is the size it says"
        );

        for (at, slice) in placed {
            file.resize(at as usize, 0);
            file.extend(slice);
        }
        file
    }

    #[cfg(target_os = "macos")]
    fn read_for(file: Vec<u8>, machine: &str) -> Option<Code> {
        the_code_for(&mut std::io::Cursor::new(file), machine)
    }

    #[cfg(target_os = "macos")]
    fn built_for(major: u32, minor: u32) -> Option<Code> {
        Some(Code {
            built_for: Some(SystemVersion(major, minor, 0)),
        })
    }

    /// A version of macOS is read the way a property list and the system write one, and nothing
    /// else is read as one.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_system_version_is_read_as_a_property_list_and_the_system_write_it() {
        assert_eq!(SystemVersion::read("10.15"), Some(SystemVersion(10, 15, 0)));
        assert_eq!(SystemVersion::read("15.6.1"), Some(SystemVersion(15, 6, 1)));
        assert_eq!(SystemVersion::read("26"), Some(SystemVersion(26, 0, 0)));
        assert_eq!(
            SystemVersion::read(" 11.0\n"),
            Some(SystemVersion(11, 0, 0))
        );
        for unread in [
            "", ".", "10.", "10..15", "1.2.3.4", "ten", "10.15b", "-1", "+1",
        ] {
            assert_eq!(SystemVersion::read(unread), None, "{unread:?} was read");
        }

        assert!(SystemVersion(10, 15, 0) < SystemVersion(11, 0, 0));
        assert!(SystemVersion(15, 6, 1) > SystemVersion(15, 6, 0));
        assert_eq!(SystemVersion::packed(0x000b_0000), SystemVersion(11, 0, 0));
        assert_eq!(SystemVersion::packed(0x000a_0f01), SystemVersion(10, 15, 1));
    }

    /// A thin executable holds one kind of code, and the system it was built for is read from
    /// either command a header names it with.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_thin_executable_holds_its_own_kind_of_code_and_no_other() {
        let arm = (0x0100_000c, 0);
        let intel = (0x0100_0007, 0x8000_0003);

        assert_eq!(
            read_for(a_thin_header(arm, Names::Build(11, 0)), "aarch64"),
            built_for(11, 0)
        );
        assert_eq!(
            read_for(a_thin_header(arm, Names::Build(11, 0)), "x86_64"),
            None
        );
        assert_eq!(
            read_for(a_thin_header(intel, Names::Minimum(10, 13)), "x86_64"),
            built_for(10, 13)
        );
        assert_eq!(
            read_for(a_thin_header(arm, Names::Nothing), "aarch64"),
            Some(Code { built_for: None }),
            "a header naming no system is code built for any"
        );
        assert_eq!(
            read_for(a_thin_header(arm, Names::Build(11, 0)), "riscv64"),
            None,
            "a kind of processor nobody builds for was taken"
        );
    }

    /// Each kind of Mac finds its own slice of a universal executable, with the system that slice
    /// was built for — and not the other slice's, whichever table the file uses.
    #[cfg(target_os = "macos")]
    #[test]
    fn each_kind_of_mac_finds_its_own_slice_of_a_universal_executable() {
        let arm = (0x0100_000c, 0);
        let intel = (0x0100_0007, 3);

        for wide in [false, true] {
            let both = || {
                a_universal_file(
                    &[
                        (intel, a_thin_header(intel, Names::Minimum(10, 13))),
                        (arm, a_thin_header(arm, Names::Build(11, 0))),
                    ],
                    wide,
                )
            };

            assert_eq!(
                read_for(both(), "aarch64"),
                built_for(11, 0),
                "wide: {wide}"
            );
            assert_eq!(
                read_for(both(), "x86_64"),
                built_for(10, 13),
                "wide: {wide}"
            );
        }
    }

    /// `arm64e` is not `arm64` and `x86_64h` is not `x86_64`: a release is the plain kind, and a
    /// universal file holding only the other one — as the system's own tools do — holds nothing
    /// for this copy.
    #[cfg(target_os = "macos")]
    #[test]
    fn code_of_a_neighbouring_kind_is_not_taken_for_this_one() {
        let arm64e = (0x0100_000c, 0x8000_0002);
        let x86_64h = (0x0100_0007, 8);
        let intel = (0x0100_0007, 3);

        assert_eq!(
            read_for(a_thin_header(arm64e, Names::Build(11, 0)), "aarch64"),
            None
        );
        assert_eq!(
            read_for(a_thin_header(x86_64h, Names::Build(11, 0)), "x86_64"),
            None
        );
        assert_eq!(
            read_for(
                a_universal_file(
                    &[
                        (intel, a_thin_header(intel, Names::Build(10, 13))),
                        (arm64e, a_thin_header(arm64e, Names::Build(11, 0))),
                    ],
                    false
                ),
                "aarch64"
            ),
            None
        );
    }

    /// What does not read as a Mach-O header holds no code a copy will run.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_executable_whose_header_does_not_read_holds_nothing() {
        let arm = (0x0100_000c, 0);
        let whole = a_thin_header(arm, Names::Build(11, 0));

        let mut cut_short = whole.clone();
        cut_short.truncate(whole.len() - 4);

        let mut too_large = whole.clone();
        too_large[20..24].copy_from_slice(&(64 * 1024 * 1024_u32).to_le_bytes());

        // The second command claims no length at all, and then too little to hold what it names.
        let mut no_length = whole.clone();
        no_length[32 + 24 + 4..32 + 24 + 8].copy_from_slice(&0_u32.to_le_bytes());
        let mut too_short = whole.clone();
        too_short[32 + 24 + 4..32 + 24 + 8].copy_from_slice(&8_u32.to_le_bytes());

        // A table pointing past the end of the file.
        let mut astray = a_universal_file(&[(arm, whole.clone())], false);
        astray[16..20].copy_from_slice(&0x7fff_0000_u32.to_be_bytes());

        let mut too_many = a_universal_file(&[(arm, whole.clone())], false);
        too_many[4..8].copy_from_slice(&1000_u32.to_be_bytes());

        for (what, file) in [
            ("nothing", Vec::new()),
            ("three bytes", vec![0xcf, 0xfa, 0xed]),
            ("text", b"not an executable at all".to_vec()),
            ("cut short", cut_short),
            ("a table too large", too_large),
            ("a command of no length", no_length),
            ("a command too short for what it names", too_short),
            ("a slice past the end", astray),
            ("too many slices", too_many),
        ] {
            assert_eq!(read_for(file, "aarch64"), None, "{what} was read as code");
        }
    }

    /// The executable these tests run in was built for this Mac, and reads as code this Mac runs.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_executable_running_this_test_reads_as_code_this_mac_runs() {
        let mut running =
            std::fs::File::open(std::env::current_exe().expect("this")).expect("readable");
        let code = the_code_for(&mut running, std::env::consts::ARCH).expect("code for this Mac");
        let this_mac = this_macs_version().expect("the system says its version");

        assert!(
            code.built_for.is_none_or(|built_for| built_for <= this_mac),
            "built for {:?}, on {this_mac:?}",
            code.built_for
        );
    }

    /// A bundle laid out as a release lays one out, with `asks_for` as its property list's minimum
    /// system and `code` as its executable.
    #[cfg(target_os = "macos")]
    fn a_bundle_needing(
        at: &Path,
        asks_for: Option<&str>,
        executable: &str,
        code: &[u8],
    ) -> PathBuf {
        let bundle = at.join("Lyonoyl Mail.app");
        let contents = bundle.join("Contents");
        std::fs::create_dir_all(contents.join("MacOS")).expect("a bundle");
        std::fs::write(contents.join("MacOS").join("lyonoyl"), code).expect("an executable");

        let minimum = asks_for.map_or(String::new(), |version| {
            format!("<key>LSMinimumSystemVersion</key><string>{version}</string>")
        });
        std::fs::write(
            contents.join("Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleExecutable</key><string>{executable}</string>{minimum}\
                 </dict></plist>\n"
            ),
        )
        .expect("its property list");

        bundle
    }

    /// This Mac's own version, written as a property list would ask for it, with `patch` added.
    #[cfg(target_os = "macos")]
    fn this_mac_and(patch: u32) -> String {
        let SystemVersion(major, minor, at) = this_macs_version().expect("the system's version");
        format!("{major}.{minor}.{}", at + patch)
    }

    /// A bundle runs here when this Mac is at least as new as the system it asks for — the same
    /// system included — and is refused before anything moves when it asks for a newer one.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_asking_for_a_newer_system_than_this_mac_is_refused() {
        let code = a_thin_header(this_kind(), Names::Build(11, 0));

        for (case, (asks_for, runs)) in [
            (Some("10.13".to_owned()), true),
            (Some(this_mac_and(0)), true),
            (None, true),
            (Some(this_mac_and(1)), false),
            (Some("999.0".to_owned()), false),
        ]
        .into_iter()
        .enumerate()
        {
            // A folder of its own for each, so no answer about one bundle is read for another.
            let at = a_place(&format!("asks-for-{case}"));
            let bundle = a_bundle_needing(&at, asks_for.as_deref(), "lyonoyl", &code);

            assert_eq!(
                it_runs_on_this_mac(&bundle),
                match runs {
                    true => Ok(()),
                    false => Err(UpdateError::NotOursToReplace),
                },
                "asking for {asks_for:?}"
            );
            std::fs::remove_dir_all(&at).ok();
        }
    }

    /// The code's own header is held to this Mac as well as the property list is, and the newer
    /// of the two is what a bundle needs.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_whose_code_was_built_for_a_newer_system_is_refused() {
        let at = a_place("built-for");
        let newer = a_thin_header(this_kind(), Names::Build(999, 0));
        let bundle = a_bundle_needing(&at, Some("10.13"), "lyonoyl", &newer);

        assert_eq!(
            the_system_it_needs(&bundle, std::env::consts::ARCH),
            Ok(Some(SystemVersion(999, 0, 0)))
        );
        assert_eq!(
            it_runs_on_this_mac(&bundle),
            Err(UpdateError::NotOursToReplace)
        );

        std::fs::remove_dir_all(&at).ok();
    }

    /// A bundle with no code for the kind of Mac this copy runs as, one naming its executable
    /// outside itself, and one asking for a system written any other way are each refused.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_this_copy_cannot_tell_runs_here_is_refused() {
        let ours = a_thin_header(this_kind(), Names::Build(11, 0));
        let theirs = a_thin_header(the_other_kind(), Names::Build(11, 0));

        for (what, asks_for, executable, code) in [
            (
                "no code for this kind of Mac",
                Some("10.13"),
                "lyonoyl",
                &theirs,
            ),
            (
                "an executable outside the bundle",
                Some("10.13"),
                "../MacOS/lyonoyl",
                &ours,
            ),
            ("no executable named", Some("10.13"), "", &ours),
            (
                "a system written another way",
                Some("eleven"),
                "lyonoyl",
                &ours,
            ),
        ] {
            let at = a_place(&format!("cannot-tell-{}", what.replace(' ', "-")));
            let bundle = a_bundle_needing(&at, asks_for, executable, code);

            assert_eq!(
                it_runs_on_this_mac(&bundle),
                Err(UpdateError::NotOursToReplace),
                "{what} was taken"
            );
            std::fs::remove_dir_all(&at).ok();
        }
    }
}
