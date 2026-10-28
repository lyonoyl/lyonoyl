/// The newest piece of mail nobody has seen, for the one thing that has to describe it.
///
/// Its own port and not a corner of the list read model, because what it answers is a different
/// question: the list is asked what to draw in a window somebody is looking at, and this is asked
/// what to say to somebody who is not. The one caller is [`AnnounceMail`], and the shape here is
/// the shape a notification has — three fields, all of them optional, none of them an identifier.
///
/// [`AnnounceMail`]: crate::application::announce_mail::AnnounceMail
pub trait Arrivals: Send + Sync {
    /// The newest unseen message in any inbox, or nothing.
    ///
    /// No failure to report, because there is nothing anybody could do about one. Mail read in the
    /// second between the pass and this question, an inbox that is empty, and a file that would not
    /// open are three different things and one answer: the notice says that mail arrived, which is
    /// what it knew before it asked.
    fn newest_unseen(&self) -> Option<Arrived>;
}

/// What arrived, in the words the message itself used.
///
/// Every field is what a stranger wrote, so nothing here reaches a screen without going through
/// [`lyonoyl_domain::notice::for_a_notice`] first.
///
/// No identifier and no address. A notification cannot be pressed to open the message it is about
/// — nothing in this application listens for that — so a message id here would be a thing carried
/// out of the core for no reader, and an address would be somebody's correspondent on a lock
/// screen for no reader either.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Arrived {
    /// Who it is from, as they wrote their own name. Absent where the message gave none.
    pub sender: Option<String>,
    pub subject: Option<String>,
    /// The opening of the message, as the arrival itself carried it: a pass reads a bounded prefix
    /// of the body alongside the headers, so mail that has just arrived has one. Absent where that
    /// prefix held nothing a reader could use — a message with no text part at all.
    pub preview: Option<String>,
}
