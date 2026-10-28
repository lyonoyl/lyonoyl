//! Whether a newer version is out, bringing it down, and putting it in place.
//!
//! Nothing here names a protocol, an address or a path. What is offered carries a token this side
//! of the port cannot read and does not need to: how a version is fetched is the adapter's
//! business, and a port that handed a URL upward would be a port that had to be rewritten the day
//! the answer came from somewhere else.
//!
//! **An updater is the most dangerous thing in a desktop application**: it brings down code and
//! runs it as the person. Two rules follow and neither is negotiable.
//!
//! The first is that nothing is ever executed on the strength of having arrived. What comes down is
//! verified against a key compiled into this build before it is unpacked, and what is unpacked is
//! verified again against the identity of the copy it would replace. An attacker holding one of
//! those keys strands an update rather than installing their own code.
//!
//! The second is that a failure is silence rather than a dialog. This runs on its own, on a timer,
//! for somebody who did not ask — so a machine that is offline, a manifest that will not parse and a
//! host that refuses all end the same way: nothing on screen, and the question asked again later.
//! Only what a person started says anything when it fails.

use std::collections::BTreeMap;

use thiserror::Error;

use lyonoyl_domain::version::Version;

/// What changed, in each language the release wrote it in, under the name of the window's catalogue
/// for that language — `en`, `ru`.
///
/// Every word a person reads is in the language they chose, and a release note is read. Which one is
/// shown is the window's decision, because the window is what knows the language; this side carries
/// them all and chooses none.
pub type Notes = BTreeMap<String, String>;

/// A version that is out, and larger than the one running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offered {
    pub version: Version,
    /// How large the download is, as the manifest states it. What a person is shown before agreeing.
    pub bytes: u64,
    /// What changed, as the release said it in each of its languages. Travels with the manifest
    /// and is signed with it, so there is one document to verify rather than two ways for a release
    /// to go out half-signed.
    pub notes: Notes,
    /// The archive's file in the release of this version, by name. Opaque above this port, and
    /// deliberately: a use case that could read it would be a use case that could be asked to fetch
    /// something else.
    pub file: Token,
    /// What the archive hashes to, as the signed manifest names it. Opaque here like the token:
    /// the adapter that brings the archive down compares it, and nothing above that reads it.
    pub digest: Token,
}

/// Something only the adapter that made it reads: which file the offered version is, or what it
/// hashes to. Meaningless to everything above the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    pub fn of(carried: String) -> Self {
        Self(carried)
    }

    /// Read only by the adapter that made it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A version that has come down, been verified, and is waiting to be put in place.
///
/// Holding one does not mean anything is installed. It means the bytes are on this machine, their
/// signature was this build's, and the bundle inside satisfies the identity the running copy
/// carries — all three, or this does not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    pub version: Version,
    /// Where it waits. Opaque for the same reason the token is.
    pub at: Token,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UpdateError {
    /// The question could not be asked, for any reason at all: no network, a host that refused, a
    /// manifest that would not parse. Collapsed on purpose — a person waiting for their mail has
    /// nothing to do differently about any of them, and the difference belongs in the log.
    #[error("the project could not be asked")]
    CouldNotAsk,
    /// What came down was not sealed by the key this build carries, or is not signed as the copy
    /// it would replace is.
    ///
    /// Never quiet. Every other failure is a bad afternoon; this one is somebody serving this
    /// application a file that is not this application's.
    #[error("that download is not this application's")]
    NotOurs,
    /// What came down is signed, and is not the version that was offered.
    ///
    /// Raised by a check only macOS has — reading the version out of a bundle that has just been
    /// swapped in. Elsewhere there is no swapping at all, so nothing can raise it, and the lint
    /// that refuses a variant nobody builds is allowed there and only there. Named on every system
    /// all the same: what a failure is called is not a thing that should differ by platform, and a
    /// blanket allow would hide the next variant that really did go dead.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    #[error("that download is not the version it was offered as")]
    NotTheVersionOffered,
    /// The copy running is not one this can replace: it is somewhere this cannot write, it is
    /// running from a disk image or another volume than the one this application keeps its files
    /// on, the version that came down does not run on this computer, or the system it is on
    /// installs software another way.
    #[error("this copy is not one this build can replace")]
    NotOursToReplace,
    /// Somebody said stop.
    #[error("the download was stopped")]
    Stopped,
    /// The disk, the permissions, or somewhere to put it.
    #[error("there is nowhere to put it")]
    Unavailable,
}

/// Told how far a download has got, and whether to carry on.
///
/// `so_far` and `whole` are bytes. Answering `false` stops it, which is what a person pressing the
/// button looks like from here.
pub type Watching<'a> = &'a mut dyn FnMut(u64, Option<u64>) -> bool;

pub trait Updates: Send + Sync {
    /// Whether a version later than the one running is out.
    ///
    /// `None` is the ordinary answer and means nothing is newer. An error means the question could
    /// not be asked, which is a different thing and is not shown to anybody.
    fn look(&self) -> Result<Option<Offered>, UpdateError>;

    /// Brings the offered version down and verifies it, saying how far it has got as it goes.
    ///
    /// Answers only when what is on disk is this build's and is the version that was offered.
    fn bring_it_down(
        &self,
        offered: &Offered,
        watching: Watching<'_>,
    ) -> Result<Staged, UpdateError>;

    /// Puts a staged version in place of the running one.
    ///
    /// Answers when the swap is done and the application must be restarted to use it. Refuses
    /// before touching anything if the staged bundle does not carry the identity of the copy it
    /// would replace — the second lock, and the one a stolen signing key does not open.
    fn put_it_in_place(&self, staged: &Staged) -> Result<(), UpdateError>;

    /// Keeps what changed until somebody has read it.
    ///
    /// Written when the version goes in rather than fetched afterwards: by the time the application
    /// has restarted, what the project says is out may already be something newer again, and a
    /// screen asking then would show the notes for a version nobody has.
    fn remember_what_changed(&self, version: &str, notes: &Notes) -> Result<(), UpdateError>;

    /// What changed in the version now running, where it has not been read yet.
    fn what_changed(&self) -> Option<(String, Notes)>;

    /// Forgotten, because somebody read it.
    fn forget_what_changed(&self);

    /// When this copy last asked the project anything, in seconds. `None` before it ever has.
    fn when_it_last_looked(&self) -> Option<u64>;

    /// Written down that it asked, whatever the answer was.
    fn remember_it_looked(&self, now: u64);
}

impl<T: Updates + ?Sized> Updates for &T {
    fn look(&self) -> Result<Option<Offered>, UpdateError> {
        (**self).look()
    }

    fn bring_it_down(
        &self,
        offered: &Offered,
        watching: Watching<'_>,
    ) -> Result<Staged, UpdateError> {
        (**self).bring_it_down(offered, watching)
    }

    fn put_it_in_place(&self, staged: &Staged) -> Result<(), UpdateError> {
        (**self).put_it_in_place(staged)
    }

    fn remember_what_changed(&self, version: &str, notes: &Notes) -> Result<(), UpdateError> {
        (**self).remember_what_changed(version, notes)
    }

    fn what_changed(&self) -> Option<(String, Notes)> {
        (**self).what_changed()
    }

    fn forget_what_changed(&self) {
        (**self).forget_what_changed()
    }

    fn when_it_last_looked(&self) -> Option<u64> {
        (**self).when_it_last_looked()
    }

    fn remember_it_looked(&self, now: u64) {
        (**self).remember_it_looked(now)
    }
}
