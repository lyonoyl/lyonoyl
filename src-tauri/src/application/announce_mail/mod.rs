//! Whether to interrupt somebody, and with what.
//!
//! The only place that decides. Everything above it counts mail and everything below it draws
//! rectangles; this answers one question, in one function, with no I/O of its own — which is why
//! it can be tested completely without a desktop.

use crate::ports::arrivals::{Arrivals, Arrived};
use crate::ports::attention::Attention;
use crate::ports::notifier::{Notice, Notifier, NotifyError};
use crate::ports::settings_repository::PreferencesRepository;
use lyonoyl_domain::notice::{for_a_notice, BETWEEN_PARTS};
use lyonoyl_domain::preferences::{NewMailNotice, NoticeShows};

/// How much of each part of a message a notification may carry.
///
/// A notification is two lines on somebody's screen and the system cuts what does not fit, so
/// these are about what is worth reading rather than what fits. A name and a subject are short
/// because they are one line each; the opening is longer because it is the part a person reads to
/// decide whether to stop what they are doing.
const NAME_AT_MOST: usize = 60;
const SUBJECT_AT_MOST: usize = 120;
const PREVIEW_AT_MOST: usize = 200;

/// Why nothing was shown. Not an error — three of the four are the ordinary case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Silent {
    /// Nothing arrived that nobody had seen.
    NothingNew,
    /// The person is looking at the window. Telling them mail arrived, while they are reading that
    /// mail, is the application talking over itself.
    TheyAreLooking,
    /// They asked not to be told.
    TheyAskedNotToBe,
    /// The settings could not be read. Silence is the safe answer: a store that will not open is
    /// not a reason to start interrupting somebody who may have turned this off.
    SettingsUnreadable,
}

/// Decides, and tells the system if the answer is yes.
pub struct AnnounceMail<S: PreferencesRepository, A: Attention, N: Notifier, R: Arrivals> {
    settings: S,
    attention: A,
    notifier: N,
    arrivals: R,
    /// The product's name, read from the build rather than written here: it is a brand and this
    /// crate is not where brands live.
    product: String,
}

impl<S: PreferencesRepository, A: Attention, N: Notifier, R: Arrivals> AnnounceMail<S, A, N, R> {
    pub fn new(
        settings: S,
        attention: A,
        notifier: N,
        arrivals: R,
        product: impl Into<String>,
    ) -> Self {
        Self {
            settings,
            attention,
            notifier,
            arrivals,
            product: product.into(),
        }
    }

    /// `unseen` is mail that arrived in an inbox and that nobody has seen. `sentence` is the one
    /// line the interface handed over, already in the person's language.
    ///
    /// The count decides **whether** to speak and never **what** to say. It is not in the
    /// sentence: a number needs plural rules, plural rules live in the interface, and a count of
    /// somebody's unread mail is a thing to put on a locked screen only if it is worth the risk —
    /// which it is not, because what a person does about it is open the application either way.
    pub fn execute(&self, unseen: u32, sentence: &str) -> Result<Result<(), Silent>, NotifyError> {
        if unseen == 0 {
            return Ok(Err(Silent::NothingNew));
        }

        // Asked before the settings, because it is the cheaper question and the commoner answer.
        if self.attention.has_it() {
            return Ok(Err(Silent::TheyAreLooking));
        }

        let Ok(preferences) = self.settings.load() else {
            return Ok(Err(Silent::SettingsUnreadable));
        };

        if preferences.new_mail_notice() == NewMailNotice::Never {
            return Ok(Err(Silent::TheyAskedNotToBe));
        }

        let shows = preferences.notice_shows();

        // Asked for only where something may be said about it. With every part turned off there is
        // nothing a message could add, and reading one to find that out is a question put to the
        // database every time mail arrives, for an answer nobody sees.
        let arrived = shows
            .anything()
            .then(|| self.arrivals.newest_unseen())
            .flatten()
            .unwrap_or_default();

        self.notifier
            .show(&self.what_to_say(&arrived, shows, sentence))?;

        Ok(Ok(()))
    }

    /// The two lines the system is given, and the whole of how they are chosen.
    ///
    /// The title names who wrote, because that is what a person looks at first and what decides
    /// whether they read the rest. Where it cannot — the part is off, or the message gave no name
    /// — the product's own name goes there instead, so the notice is never headed by a blank.
    ///
    /// The body is what the message is about and how it opens, in that order, one to a line. Where
    /// neither may be said — both parts off, or a message that gave neither — the sentence the
    /// window handed over stands in its place, so a notice is never shown with an empty body.
    fn what_to_say(&self, arrived: &Arrived, shows: NoticeShows, sentence: &str) -> Notice {
        let part = |raw: &Option<String>, on: bool, at_most: usize| -> Option<String> {
            on.then(|| raw.as_deref().and_then(|raw| for_a_notice(raw, at_most)))
                .flatten()
        };

        let title = part(&arrived.sender, shows.sender, NAME_AT_MOST)
            .unwrap_or_else(|| self.product.clone());

        let body: Vec<String> = [
            part(&arrived.subject, shows.subject, SUBJECT_AT_MOST),
            part(&arrived.preview, shows.preview, PREVIEW_AT_MOST),
        ]
        .into_iter()
        .flatten()
        .collect();

        let body = match body.is_empty() {
            true => sentence.to_owned(),
            false => body.join(&BETWEEN_PARTS.to_string()),
        };

        Notice { title, body }
    }
}

#[cfg(test)]
mod tests;
