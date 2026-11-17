// Reached from the composition root once the bell is wired.
#![allow(dead_code)]

use lyonoyl_domain::operation::OperationTarget;
use thiserror::Error;

/// Telling the operating system that something happened.
///
/// One method, two strings, and what may be done from the notice itself, because that is the whole
/// of what a notification has room for. What goes in them is decided one level up, by
/// [`AnnounceMail`], against what the person asked a notice to say — so this trait neither knows
/// nor can be told which parts of a message reached it, and an adapter cannot add one.
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
/// does not own: what reaches it uncleaned has been shown, however soon it is taken away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Who the message is from, or the product's name where that may not be said. A name, not
    /// prose.
    pub title: String,
    /// One sentence, handed over from the interface catalogue in the language the person chose.
    pub body: String,
    /// What may be done from the notice to the one message it names, or nothing where it names
    /// none: a notice standing for several messages names the newest, and its buttons act on that
    /// one and no other.
    pub offers: Option<Offers>,
}

/// The message a notice names, and where its buttons may put it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offers {
    pub about: OperationTarget,
    /// Never empty: a notice with nothing to offer carries no `Offers` at all.
    pub each: Vec<Offer>,
}

/// One button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub into: PutInto,
    /// What the button says, handed over from the interface catalogue.
    pub said: String,
}

/// Where a notice's button puts the message it names: the two things somebody does with mail they
/// have read enough of on a notice, done as the window's own buttons do them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutInto {
    Trash,
    Junk,
}

/// The interface's words a notice may carry, handed over from the catalogue in the language the
/// person chose, all at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeWords {
    /// What the notice says where it may say nothing about the message itself.
    pub new_mail: String,
    /// The button that throws the message away.
    pub into_trash: String,
    /// The button that puts it with the junk.
    pub into_junk: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum NotifyError {
    /// The system would not show it. Nothing follows: a notification that failed is not worth
    /// telling somebody about in another notification.
    #[error("the system did not show the notification")]
    Refused,
}
