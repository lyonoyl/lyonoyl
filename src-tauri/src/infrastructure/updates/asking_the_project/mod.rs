//! The updater: asking the project whether a newer version is out, bringing it down, and putting it
//! in place.
//!
//! The one implementation of the `Updates` port, and a composition of parts that know nothing of
//! each other. The places it asks are [`UpdateProvider`]s, handed in, and it never learns their
//! addresses or how they are reached; what a manifest says is `the_manifest`; whether a file is the
//! project's own is `the_seal`; putting a version in place is `staging`; and what it keeps between
//! runs is `what_it_keeps`.
//!
//! **No place is trusted with more than being there.** A copy asks the places in order and takes the
//! first manifest the seal holds over, so on an ordinary day one place sees a copy look; the archive
//! comes from the first place that hands over the file the manifest names, held to its size and
//! digest. What installs is decided by the seal, the digest and the Developer ID, so a place lost to
//! somebody else can hold copies back from a newer version and see who asks, and can install
//! nothing.
//!
//! **Where the key comes from decides what this can install.** The public half is compiled in at
//! build time. A build made without it cannot verify anything and therefore does not look: saying
//! *no update* to somebody whose build could not check one is the only honest answer, and it is
//! quieter than a message about a key nobody but the person who built it can do anything about.

use std::path::PathBuf;

use lyonoyl_domain::version::Version;

use crate::infrastructure::journal::journal;
use crate::infrastructure::updates::providers::UpdateProvider;
use crate::infrastructure::updates::the_manifest::{self, a_plain_name, LARGEST};
use crate::infrastructure::updates::the_seal::{what_the_seal_says, TheKey};
use crate::infrastructure::updates::{staging, what_it_keeps};
use crate::ports::journal::{Journal, Note, Value};
use crate::ports::updates::{Notes, Offered, Staged, UpdateError, Updates, Watching};

/// The key this build will accept an update from, put here when it was built.
///
/// `option_env!` rather than `env!`: a checkout anybody clones has no key and must still build.
#[cfg(not(feature = "screenshots"))]
const KEY: Option<&str> = option_env!("LYONOYL_UPDATE_PUBKEY");

/// None in the build made for screenshots, whatever the machine it is built on holds: an update
/// would replace the scene that build plays with the application itself.
#[cfg(feature = "screenshots")]
const KEY: Option<&str> = None;

pub struct AskingTheProject {
    /// The places the project publishes to, in the order a copy asks them.
    providers: Vec<Box<dyn UpdateProvider>>,
    /// This build's own version, as the manifest will have to beat.
    running: Version,
    /// Where this application keeps its files, handed in from the one place that answers that.
    beside: PathBuf,
}

impl AskingTheProject {
    pub fn new(providers: Vec<Box<dyn UpdateProvider>>, beside: PathBuf) -> Self {
        Self {
            providers,
            beside,
            // Parsed from what the manifest of this crate says, so the two can never disagree.
            running: Version::parse(env!("CARGO_PKG_VERSION"))
                .expect("this application's own version is three numbers"),
        }
    }

    /// The manifest from the first place that hands over one the seal holds over.
    ///
    /// A place that does not answer, or answers with a manifest the seal does not hold over, is
    /// passed over for the next. A manifest the seal holds over is the answer whatever it offers:
    /// every place holds the same files, and asking the next would tell a second host who runs this
    /// for the same answer.
    fn a_sealed_manifest(&self, key: &TheKey) -> Result<String, UpdateError> {
        for (at, provider) in self.providers.iter().enumerate() {
            // Verified before it is read. A manifest is a small document and the temptation is to
            // parse it first and check it after, which is a parser fed by a stranger.
            //
            // A seal that does not hold over it is a question that could not be asked here rather
            // than an alarm. The two files can be cached apart for minutes after a release, so a
            // copy asking then can be handed the new manifest and the old seal.
            let sealed = provider.the_manifest().ok().filter(|sealed| {
                what_the_seal_says(key, sealed.manifest.as_bytes(), &sealed.seal).is_ok()
            });

            if let Some(sealed) = sealed {
                // Written down only once a later place answered: every place failing is a machine
                // that is offline, which is an ordinary day rather than something to report.
                for earlier in 0..at {
                    passed_over("update.manifestElsewhere", earlier, None);
                }
                return Ok(sealed.manifest);
            }
        }

        Err(UpdateError::CouldNotAsk)
    }

    /// The archive the signed manifest names, from the first place that hands it over: as large as
    /// the manifest says, and hashing to what it says.
    ///
    /// A place that does not answer, stops short, or hands over a different file is passed over for
    /// the next, because every place holds the same file and the digest decides which file is the
    /// one. A person saying stop stops it at once, and no other place is asked. Where none hands it
    /// over, a different file outranks a failure to fetch: somebody serving this application a file
    /// that is not its own is never reported as a bad connection.
    fn the_archive(
        &self,
        offered: &Offered,
        watching: Watching<'_>,
    ) -> Result<Vec<u8>, UpdateError> {
        // Held to the manifest's rule again here, where it is used: what was offered came back from
        // above the port, and a name that leaves the release's folder is asked of no place at all.
        let name = offered.file.as_str();
        if !a_plain_name(name) {
            return Err(UpdateError::CouldNotAsk);
        }
        // No more than the manifest says it weighs, which the manifest already holds to `LARGEST`.
        let at_most = usize::try_from(offered.bytes).unwrap_or(LARGEST);
        let mut failed = UpdateError::CouldNotAsk;

        for (at, provider) in self.providers.iter().enumerate() {
            // The size first, so a download cut short is said to be that, then the digest the
            // signed manifest names.
            let refused = match provider.a_file(offered.version, name, at_most, &mut *watching) {
                Ok(archive) => {
                    match the_manifest::the_archive_must_be_the_one_named(offered, &archive) {
                        Ok(()) => return Ok(archive),
                        Err(refused) => refused,
                    }
                }
                Err(UpdateError::Stopped) => return Err(UpdateError::Stopped),
                Err(_) => UpdateError::CouldNotAsk,
            };

            if refused == UpdateError::NotOurs {
                failed = UpdateError::NotOurs;
            }
            if at + 1 < self.providers.len() {
                passed_over(
                    "update.archiveElsewhere",
                    at,
                    Some(refused == UpdateError::NotOurs),
                );
            }
        }

        Err(failed)
    }
}

/// Written down that a place was passed over for the next, so a place that stopped answering is in
/// the log somebody sends with a problem rather than nowhere. The place by its position in the list,
/// counted from one, since nothing here writes an address into the journal; `notOurs` says whether
/// what came down was a different file rather than nothing.
fn passed_over(event: &'static str, at: usize, not_ours: Option<bool>) {
    let journal = journal();
    if !journal.keeps(Note::Trouble) {
        return;
    }

    let place = Value::Count(at as u64 + 1);
    match not_ours {
        Some(not_ours) => journal.write_down(
            Note::Trouble,
            event,
            &[("place", place), ("notOurs", Value::Yes(not_ours))],
        ),
        None => journal.write_down(Note::Trouble, event, &[("place", place)]),
    }
}

impl Updates for AskingTheProject {
    fn look(&self) -> Result<Option<Offered>, UpdateError> {
        let Some(key) = KEY else {
            return Ok(None);
        };
        let key = TheKey::written_as(key)?;

        let manifest = self.a_sealed_manifest(&key)?;

        let offered =
            the_manifest::what_it_offers(&manifest, self.running, std::env::consts::ARCH)?;

        if let Some(offered) = &offered {
            let journal = journal();
            if journal.keeps(Note::Worth) {
                journal.write_down(
                    Note::Worth,
                    "update.offered",
                    &[("bytes", Value::Bytes(offered.bytes))],
                );
            }
        }

        Ok(offered)
    }

    fn bring_it_down(
        &self,
        offered: &Offered,
        watching: Watching<'_>,
    ) -> Result<Staged, UpdateError> {
        // A build with no key never looked, so nothing it holds was offered by the project.
        if KEY.is_none() {
            return Err(UpdateError::NotOurs);
        }

        let archive = self.the_archive(offered, watching)?;

        staging::put_it_somewhere(&self.beside, offered, &archive)
    }

    fn put_it_in_place(&self, staged: &Staged) -> Result<(), UpdateError> {
        staging::swap_it_in(&self.beside, staged)
    }

    fn remember_what_changed(&self, version: &str, notes: &Notes) -> Result<(), UpdateError> {
        what_it_keeps::remember_what_changed(&self.beside, version, notes)
    }

    fn what_changed(&self) -> Option<(String, Notes)> {
        what_it_keeps::what_changed(&self.beside, env!("CARGO_PKG_VERSION"))
    }

    fn forget_what_changed(&self) {
        what_it_keeps::forget_what_changed(&self.beside)
    }

    fn when_it_last_looked(&self) -> Option<u64> {
        what_it_keeps::when_it_last_looked(&self.beside)
    }

    fn remember_it_looked(&self, now: u64) {
        what_it_keeps::remember_it_looked(&self.beside, now)
    }
}

/// What a caller has to hold to be told about an update at all.
///
/// Its own function so the one place that decides is one place: a build with no key does not look,
/// and a screen that asked anyway would be a screen showing *no update* for a reason it cannot
/// explain.
pub fn this_build_can_be_updated() -> bool {
    KEY.is_some()
}

#[cfg(test)]
mod tests;
