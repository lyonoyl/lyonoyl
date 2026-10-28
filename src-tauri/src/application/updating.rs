//! Whether to look for an update, and what to do with one that is found.
//!
//! The decisions live here rather than in the adapter, and there are three of them: whether this
//! copy asks at all, how often, and what a version being offered means. The adapter knows how to
//! fetch and verify; it does not know that a person turned this off, or that it was asked ten
//! minutes ago.
//!
//! **A failed look says nothing to anybody.** This runs on a timer for somebody who did not ask, so
//! a machine that is offline and a host that will not answer both end as *nothing is out*. Only
//! what a person started — pressing *Install* — reports what went wrong.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::application::error::UseCaseError;
use crate::ports::updates::{Notes, Offered, Staged, UpdateError, Updates, Watching};

/// How often a copy asks.
///
/// Once a day. Often enough that somebody hears about a fix within a day of it existing, seldom
/// enough that a thousand copies are a thousand reads of one small file rather than a load.
pub const HOW_OFTEN: Duration = Duration::from_secs(24 * 60 * 60);

pub struct LookForAnUpdate<U: Updates> {
    updates: U,
}

impl<U: Updates> LookForAnUpdate<U> {
    pub fn new(updates: U) -> Self {
        Self { updates }
    }

    /// Whether a newer version is out, if it is time to ask and this copy asks at all.
    ///
    /// `None` covers every reason there is nothing to show — turned off, asked recently, nothing
    /// newer, could not ask — because a screen has the same thing to do about all four. Which of
    /// them it was belongs in the log.
    pub fn execute(&self, allowed: bool, now: u64) -> Option<Offered> {
        if !allowed || !it_is_time(self.updates.when_it_last_looked(), now) {
            return None;
        }

        // Written down before the answer, so a project that could not be reached still counts as
        // having been asked. Without it, a machine that is offline asks again on every start for as
        // long as it stays offline — which is a check nobody switched on, running as often as the
        // window opens.
        self.updates.remember_it_looked(now);

        self.updates.look().ok().flatten()
    }

    /// The same question a person asked out loud, which is answered out loud.
    ///
    /// Pressing *check now* and being told nothing at all is a dead control, and this project has a
    /// rule about that: what somebody started reports what happened to it.
    pub fn because_somebody_asked(&self) -> Result<Option<Offered>, UseCaseError> {
        self.updates.look().map_err(into_use_case)
    }
}

/// Bringing down what was offered, and putting it in place.
///
/// Its own use case because it is the half a person waits on: it is started by pressing a button,
/// it reports what it is doing, and it can be stopped. Nothing here happens on a timer.
pub struct InstallTheUpdate<U: Updates> {
    updates: U,
}

impl<U: Updates> InstallTheUpdate<U> {
    pub fn new(updates: U) -> Self {
        Self { updates }
    }

    /// Brings it down, verifies it, and stages it. Says how far it has got as it goes.
    pub fn bring_it_down(
        &self,
        offered: &Offered,
        watching: Watching<'_>,
    ) -> Result<Staged, UseCaseError> {
        self.updates
            .bring_it_down(offered, watching)
            .map_err(into_use_case)
    }

    /// Puts a staged version in place. The application has to be restarted afterwards.
    ///
    /// What changed is written down first. A swap that succeeded and left no note is a person who
    /// restarts into a version that says nothing about itself; a note left by a swap that then
    /// failed is about a version that is not there, and the reader below refuses it for naming the
    /// wrong one.
    pub fn put_it_in_place(&self, staged: &Staged, notes: &Notes) -> Result<(), UseCaseError> {
        let _ = self
            .updates
            .remember_what_changed(&staged.version.to_string(), notes);

        self.updates.put_it_in_place(staged).map_err(into_use_case)
    }

    /// What changed in the version now running, where nobody has read it yet.
    pub fn what_changed(&self) -> Option<(String, Notes)> {
        self.updates.what_changed()
    }

    /// Read, and therefore not shown again.
    pub fn changes_seen(&self) {
        self.updates.forget_what_changed()
    }
}

/// Whether enough time has passed.
///
/// A copy that has never asked asks now. A clock that went backwards — a machine whose time was
/// corrected, or one that sleeps across a change — asks now rather than waiting out a day that has
/// already passed by its own arithmetic.
fn it_is_time(last_asked: Option<u64>, now: u64) -> bool {
    match last_asked {
        None => true,
        Some(then) if then > now => true,
        Some(then) => now - then >= HOW_OFTEN.as_secs(),
    }
}

/// Seconds, as everything this application stores counts time.
pub fn seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

fn into_use_case(why: UpdateError) -> UseCaseError {
    match why {
        UpdateError::CouldNotAsk => UseCaseError::Offline,
        UpdateError::Unavailable => UseCaseError::StorageUnavailable,
        UpdateError::Stopped => UseCaseError::Cancelled,
        // The one a copy on Windows or Linux meets every time, and it is not about the download.
        // Swept in with the rest it reached the window as "that download carries a different
        // signature" — a sentence about somebody serving a file that is not this application's,
        // shown to somebody whose only fault is the system they are on.
        UpdateError::NotOursToReplace => UseCaseError::NotOursToReplace,
        // What is left is about the download itself: signed by the wrong key, or not the version
        // it was offered as.
        UpdateError::NotOurs | UpdateError::NotTheVersionOffered => UseCaseError::ProviderProtocol,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use lyonoyl_domain::version::Version;

    use super::*;
    use crate::ports::updates::Token;

    /// A project that answers whatever the test told it to, and remembers being asked.
    #[derive(Default)]
    struct Project {
        answer: Option<Offered>,
        refuse: bool,
        asked: Mutex<u32>,
        last_looked: Mutex<Option<u64>>,
    }

    impl Updates for &Project {
        fn look(&self) -> Result<Option<Offered>, UpdateError> {
            *self.asked.lock().expect("test lock") += 1;

            match self.refuse {
                true => Err(UpdateError::CouldNotAsk),
                false => Ok(self.answer.clone()),
            }
        }

        fn bring_it_down(
            &self,
            _offered: &Offered,
            _watching: Watching<'_>,
        ) -> Result<Staged, UpdateError> {
            unreachable!("looking does not bring anything down")
        }

        fn put_it_in_place(&self, _staged: &Staged) -> Result<(), UpdateError> {
            unreachable!("looking does not install anything")
        }

        fn remember_what_changed(&self, _version: &str, _notes: &Notes) -> Result<(), UpdateError> {
            unreachable!("looking remembers nothing")
        }

        fn what_changed(&self) -> Option<(String, Notes)> {
            None
        }

        fn forget_what_changed(&self) {}

        fn when_it_last_looked(&self) -> Option<u64> {
            *self.last_looked.lock().expect("test lock")
        }

        fn remember_it_looked(&self, now: u64) {
            *self.last_looked.lock().expect("test lock") = Some(now);
        }
    }

    fn offered() -> Offered {
        Offered {
            version: Version::parse("1.1.0").expect("a version"),
            bytes: 1000,
            notes: Notes::from([("en".to_owned(), "what changed".to_owned())]),
            file: Token::of("a.tar.gz".to_owned()),
            digest: Token::of("0".repeat(64)),
        }
    }

    const DAY: u64 = 24 * 60 * 60;

    /// Somebody who said no is not asked again, and the project is not asked either.
    ///
    /// The second half is the one worth a test: a check that returned nothing while still sending
    /// the request would be a setting that changes what is shown and not what leaves the machine.
    #[test]
    fn a_copy_told_not_to_ask_does_not_ask() {
        let project = Project {
            answer: Some(offered()),
            ..Project::default()
        };

        let found = LookForAnUpdate::new(&project).execute(false, DAY * 100);

        assert!(found.is_none());
        assert_eq!(
            *project.asked.lock().expect("test lock"),
            0,
            "a copy told not to ask asked anyway"
        );
    }

    #[test]
    fn a_copy_that_asked_recently_waits() {
        let now = DAY * 100;
        let project = Project {
            answer: Some(offered()),
            last_looked: Mutex::new(Some(now - 60)),
            ..Project::default()
        };

        let found = LookForAnUpdate::new(&project).execute(true, now);

        assert!(found.is_none());
        assert_eq!(*project.asked.lock().expect("test lock"), 0);
    }

    #[test]
    fn a_copy_that_has_never_asked_asks_now() {
        let project = Project {
            answer: Some(offered()),
            ..Project::default()
        };

        let found = LookForAnUpdate::new(&project).execute(true, DAY * 100);

        assert_eq!(found, Some(offered()));
    }

    #[test]
    fn a_day_later_it_asks_again() {
        let now = DAY * 100;
        let project = Project {
            answer: Some(offered()),
            last_looked: Mutex::new(Some(now - DAY)),
            ..Project::default()
        };

        let found = LookForAnUpdate::new(&project).execute(true, now);

        assert_eq!(found, Some(offered()));
    }

    /// A clock corrected backwards does not lock a copy out of asking for a day.
    #[test]
    fn a_clock_that_went_backwards_does_not_stop_it_asking() {
        let now = DAY * 100;
        let project = Project {
            answer: Some(offered()),
            // A machine whose time was corrected, or one that slept across a change.
            last_looked: Mutex::new(Some(now + DAY * 30)),
            ..Project::default()
        };

        let found = LookForAnUpdate::new(&project).execute(true, now);

        assert_eq!(found, Some(offered()));
    }

    /// A look nobody started fails silently.
    #[test]
    fn a_look_that_could_not_be_made_says_nothing() {
        let project = Project {
            refuse: true,
            ..Project::default()
        };

        let found = LookForAnUpdate::new(&project).execute(true, DAY * 100);

        assert!(found.is_none(), "a failed look was reported as an offer");
    }

    /// A copy that could not reach the project still counts as having asked.
    ///
    /// Without this a machine that is offline asks again every time the window opens, which is a
    /// check nobody switched on running as often as somebody opens their mail.
    #[test]
    fn a_look_that_failed_still_counts_as_having_looked() {
        let project = Project {
            refuse: true,
            ..Project::default()
        };
        let now = DAY * 100;

        LookForAnUpdate::new(&project).execute(true, now);

        assert_eq!(
            *project.last_looked.lock().expect("test lock"),
            Some(now),
            "a look that could not be made was not written down"
        );
    }

    /// A look a person started reports what happened to it.
    #[test]
    fn a_look_somebody_asked_for_answers_out_loud() {
        let project = Project {
            refuse: true,
            ..Project::default()
        };

        let outcome = LookForAnUpdate::new(&project).because_somebody_asked();

        assert!(
            outcome.is_err(),
            "a person pressed a button and was told nothing"
        );
    }
}
