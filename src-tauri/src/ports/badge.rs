use thiserror::Error;

/// The number on the application's own icon.
///
/// One method and one number, for the same reason the notifier has one method and two strings: an
/// icon badge is drawn by the system, on a surface this application does not own, and often while
/// the machine is locked. A count is the most it may ever carry — no sender, no subject, no
/// mailbox — and what cannot be passed cannot be leaked.
pub trait Badge: Send + Sync {
    /// Zero takes the badge off rather than drawing a nought.
    fn show(&self, unread: u32) -> Result<(), BadgeError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BadgeError {
    /// The system would not draw it, or does not draw one at all. Nothing follows: an icon without
    /// a number is what this application looked like before there was one.
    #[error("the system did not draw the badge")]
    Refused,
}
