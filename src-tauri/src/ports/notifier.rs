// Reached from the composition root once the bell is wired.
#![allow(dead_code)]

use thiserror::Error;

/// Telling the operating system that something happened.
///
/// One method and two strings, because that is the whole of what a notification has room for. What
/// goes in them is decided one level up, by [`AnnounceMail`], against what the person asked a
/// notice to say — so this trait neither knows nor can be told which parts of a message reached
/// it, and an adapter cannot add one.
///
/// [`AnnounceMail`]: crate::application::announce_mail::AnnounceMail
pub trait Notifier: Send + Sync {
    fn show(&self, notice: &Notice) -> Result<(), NotifyError>;
}

/// What is shown.
///
/// Either half may hold a message's own words, where the person asked for them — so both are
/// already through [`lyonoyl_domain::notice::for_a_notice`] by the time they arrive here. A
/// notification is drawn by the system, often on a locked screen, on a surface this application
/// does not own and cannot withdraw: what reaches it uncleaned stays there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The product's name. A brand, not prose.
    pub title: String,
    /// One sentence, handed over from the interface catalogue in the language the person chose.
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NotifyError {
    /// The system would not show it. Nothing follows: a notification that failed is not worth
    /// telling somebody about in another notification.
    #[error("the system did not show the notification")]
    Refused,
}
