//! Keeping what is being written when the application ends.
//!
//! The composer keeps what is on screen a moment after the typing stops, and every way of leaving
//! it waits for that keep. Ending the application takes none of those ways. Cmd+Q, the Dock's Quit
//! and logging out end it from the system's side and cannot be held, and once they have begun the
//! window cannot be asked anything: what it sends arrives on the thread the end runs on. So the
//! window notes what is on screen to the core as it changes, the core holds the newest note here,
//! and the end writes it down as a keep would.
//!
//! Four things decide what becomes of a message being written, and one lock puts them in order: a
//! keep, throwing the draft away, handing the message to the outbox, and the end. The end comes
//! once. A keep it catches halfway finishes first, and the end writes after it, under the number
//! that keep got. A keep of the same message that had not begun when the end came writes nothing:
//! what the end wrote is newer. A message being handed over as the end comes is in the outbox, the
//! draft it was kept as thrown away in the same step, or, when the end came first, a draft: never
//! neither, and never both unless that draft cannot be thrown away.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::application::error::UseCaseError;
use crate::application::keep_draft::{bounded, KeepDraft};
use crate::ports::drafts::{Draft, DraftError, DraftFile, Drafts};

/// How long a name in a note may be: the account's, or the composer's own.
///
/// Both are made by this build and are far shorter. A note naming a longer one is refused rather
/// than held, because what a note holds stays in memory until the next note replaces it.
const LONGEST_NAME: usize = 64;

/// What one composer has on screen, as it noted it.
#[derive(Debug, Clone)]
pub struct Note {
    /// Which composer noted it: made at random by the window as the composer opens, and the same in
    /// every keep that composer makes. A number the window counted would start again with the window,
    /// and could be one the core still holds from before.
    pub writing: String,
    pub account_id: String,
    /// The draft it is kept under, when one has been kept.
    pub under: Option<i64>,
    pub draft: Draft,
    /// The files on screen, held rather than named: a file the window lets go of once it has been
    /// noted is still here for the end to keep.
    pub files: Vec<DraftFile>,
}

/// The newest state of the message being written, for the end of the application to keep.
///
/// In memory and nowhere else, and one at a time: the window has one composer open, and a note
/// from another replaces this one.
#[derive(Clone, Default)]
pub struct BeingWritten {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    noted: Mutex<Noted>,
    /// Taken by a keep, by throwing a draft away and by handing a message to the outbox for as long
    /// as they write, and by the end for as long as it does. Holds what the end did, once it has
    /// come.
    ///
    /// Never taken by a note: a note arrives on the thread that draws the window, and nothing that
    /// waits for a disk may stand in its way.
    writing_down: Mutex<Option<Ended>>,
}

#[derive(Default)]
struct Noted {
    note: Option<Note>,
    /// The composer that noted last, whether its note is held or was let go of: the one on screen,
    /// and the only one that notes again.
    last: Option<String>,
    /// The number the first keep of that composer landed as.
    ///
    /// The window learns that number when the keep answers, and a note it sends before then still
    /// says it has none. Read as written, that note is a message never kept, and the end would
    /// write it down beside the draft the keep made.
    first_kept: Option<i64>,
}

/// What the end did.
enum Ended {
    /// Nothing written: nothing was noted, the note was refused, the profile could not be had, or
    /// the files were being set aside.
    Nothing,
    /// The note of the composer `writing`, written down under `id`.
    Wrote { writing: String, id: i64 },
}

impl BeingWritten {
    /// Holds `note` in place of whatever was held.
    ///
    /// Refused, and what was held stays as it was, when a keep would refuse it — more text or
    /// heavier files than a message may carry — or when it names its account or its composer by a
    /// name longer than this build makes. A refused keep leaves its draft as it was, and this is
    /// the same: what the end writes is what the last keep that could be written would have.
    pub fn note(&self, note: Note) -> Result<(), UseCaseError> {
        if note.writing.len() > LONGEST_NAME || note.account_id.len() > LONGEST_NAME {
            return Err(UseCaseError::MessageIsTooLarge);
        }
        let draft = bounded(note.draft, &note.files)?;

        let mut noted = lock(&self.inner.noted);
        // Another composer in the place of the one before: that one's first keep is a message of
        // its own.
        if noted.last.as_ref() != Some(&note.writing) {
            noted.last = Some(note.writing.clone());
            noted.first_kept = None;
        }
        let under = note.under.or(noted.first_kept);
        noted.note = Some(Note {
            under,
            draft,
            ..note
        });

        Ok(())
    }

    /// Lets go of what the composer `writing` noted: its message was handed to the outbox, thrown
    /// away or kept on the way out, the composer has gone, or it holds nothing a keep would write.
    ///
    /// Another composer's note stays. A composer taken off the screen can still be finishing what
    /// it started, and what it lets go of then is its own, never the note of the one in its place.
    /// Nor does a composer that lets go stop being the one that noted last: noting again once there
    /// is something to keep, it is given the number its first keep landed as.
    pub fn forget(&self, writing: &str) {
        let mut noted = lock(&self.inner.noted);
        if noted
            .note
            .as_ref()
            .is_some_and(|note| note.writing == writing)
        {
            noted.note = None;
        }
    }

    /// Keeps a draft for the composer `writing`, in turn with the end, and answers its number.
    ///
    /// After the end has written this composer's note, a keep under that number or under none
    /// writes nothing and answers the number the end wrote: it was asked for before the end came,
    /// so what it holds is older. Every other keep is written as it would have been.
    ///
    /// A first keep, under no number, of the composer that noted last gives its notes that number:
    /// the one held, when it says it has none, and every one sent before the window hears of the
    /// keep, whether or not a note was held as it landed. That is what the end writes under, so the
    /// message is one draft however the keep and the end fall. The first keep of a composer taken
    /// off the screen gives nothing: it notes nothing again, and the one in its place is writing a
    /// message of its own.
    pub fn keep<D: Drafts>(
        &self,
        drafts: D,
        writing: &str,
        account_id: &str,
        under: Option<i64>,
        draft: Draft,
        files: &[DraftFile],
    ) -> Result<i64, UseCaseError> {
        let ended = lock(&self.inner.writing_down);
        if let Some(Ended::Wrote { writing: wrote, id }) = &*ended {
            if wrote == writing && under.is_none_or(|under| under == *id) {
                return Ok(*id);
            }
        }

        let id = KeepDraft::new(drafts).execute(account_id, under, draft, files)?;

        if under.is_none() {
            let mut noted = lock(&self.inner.noted);
            if noted.last.as_deref() == Some(writing) {
                noted.first_kept = Some(id);
                if let Some(note) = noted.note.as_mut().filter(|note| note.writing == writing) {
                    note.under.get_or_insert(id);
                }
            }
        }

        Ok(id)
    }

    /// Hands the message of the composer `writing` to the outbox with `send`, in turn with the end,
    /// and in the same step lets go of that composer's note and throws away `kept`, the draft the
    /// message was kept as.
    ///
    /// Until the outbox holds the message, the note is the only place the last of the typing is.
    /// Let go of before, an end coming in between would find nothing to write, and the message
    /// would be in neither place once the process had gone; let go of after, an end coming in
    /// between would write down a draft of a message on its way. The draft goes in the same step
    /// for the same reason: thrown away once the window heard the message had gone, it would
    /// outlive an end coming in between, an older copy of the message in the drafts for somebody
    /// to send again. A message refused on its way is still being written, and keeps its note and
    /// its draft.
    ///
    /// A draft the profile will not let go of stays, and so does one whose throwing away breaks
    /// halfway: the message is handed over all the same. Answered with either, the window would say
    /// the message was not sent, and sending it again would put it in the outbox twice.
    ///
    /// Refused as cancelled once the end has written this composer's note: the message is a draft,
    /// and in the outbox as well it would be a second copy. Nobody is told, because the
    /// application is ending.
    pub fn hand_over<D: Drafts, T>(
        &self,
        drafts: D,
        writing: &str,
        kept: Option<i64>,
        send: impl FnOnce() -> Result<T, UseCaseError>,
    ) -> Result<T, UseCaseError> {
        let ended = lock(&self.inner.writing_down);
        if matches!(&*ended, Some(Ended::Wrote { writing: wrote, .. }) if wrote == writing) {
            return Err(UseCaseError::Cancelled);
        }

        let sent = send()?;
        self.forget(writing);
        if let Some(id) = kept {
            // Refused or broken, it stays: the message is in the outbox all the same.
            let _ = catch_unwind(AssertUnwindSafe(|| self.thrown_away(drafts, id)));
        }

        Ok(sent)
    }

    /// Throws away the draft kept under `id`, and lets go of a note kept under it, in turn with the
    /// end.
    ///
    /// A message thrown away never comes back as a draft. Written by the end after the throwing
    /// away, a note under that number would find the draft gone and be written down as one of its
    /// own; let go of here, there is nothing for the end to write.
    pub fn discard<D: Drafts>(&self, drafts: D, id: i64) -> Result<(), DraftError> {
        let _in_turn = lock(&self.inner.writing_down);
        self.thrown_away(drafts, id)
    }

    /// Throws away the draft kept under `id`, and lets go of a note and a first keep under that
    /// number, for a caller that has taken its turn with the end.
    fn thrown_away<D: Drafts>(&self, drafts: D, id: i64) -> Result<(), DraftError> {
        {
            let mut noted = lock(&self.inner.noted);
            if noted
                .note
                .as_ref()
                .is_some_and(|note| note.under == Some(id))
            {
                noted.note = None;
            }
            if noted.first_kept == Some(id) {
                noted.first_kept = None;
            }
        }

        drafts.discard(id)
    }

    /// Writes down the newest note as a keep, once, as the application ends.
    ///
    /// `drafts` is asked for only when there is a note to write. Called again, it does nothing: the
    /// application is told it is ending more than once on the way out.
    ///
    /// Never panics, and never says anything. A keep that is refused — no room left in the
    /// account's drafts, a profile that cannot be written — writes nothing and leaves everything
    /// else as it was, and nothing on the way out is in a position to tell anybody.
    pub fn at_the_end<D: Drafts>(&self, drafts: impl FnOnce() -> Option<D>) {
        let mut ended = lock(&self.inner.writing_down);
        if ended.is_some() {
            return;
        }

        let note = lock(&self.inner.noted).note.take();
        let wrote = note.and_then(|note| {
            catch_unwind(AssertUnwindSafe(|| {
                KeepDraft::new(drafts()?)
                    .execute(&note.account_id, note.under, note.draft, &note.files)
                    .ok()
            }))
            .ok()
            .flatten()
            .map(|id| Ended::Wrote {
                writing: note.writing,
                id,
            })
        });

        *ended = Some(wrote.unwrap_or(Ended::Nothing));
    }

    /// Leaves the end nothing to write, for good: the files it would write into are being set
    /// aside.
    ///
    /// Written after they were moved, a note would go into a profile kept for somebody to look at,
    /// which is the one file on this device this application must leave exactly as it was.
    pub fn set_aside(&self) {
        let mut ended = lock(&self.inner.writing_down);
        ended.get_or_insert(Ended::Nothing);
        lock(&self.inner.noted).note = None;
    }
}

/// The lock, whatever happened while somebody else held it: a note or a keep that panicked leaves
/// nothing here half changed, and the end of the application must not stop at it.
fn lock<T>(held: &Mutex<T>) -> MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
