use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::ports::progress::Progress;

/// How far the reading happening right now has got, for the line at the foot of the window.
///
/// Two units, because a round is nine mailboxes read at the same moment: they start together and
/// finish within a second of each other, so counted by mailbox the answer is nought for most of a
/// round and then everything. What finishes steadily is the folder — each account reads its own
/// one after another — so the bar moves by folders while the words count mailboxes.
///
/// Held in memory and nowhere else. It describes something that is true for a few seconds, and a
/// copy in a database is a copy that outlives what it describes.
#[derive(Clone, Default)]
pub struct HowFarAlong {
    reading: Arc<Mutex<HashMap<String, Reading>>>,
}

/// What one whole mailbox is worth, so that mailboxes of different sizes are still one bar.
///
/// Integers rather than a fraction: the arithmetic has one answer on every machine. A thousand
/// because a bar a thousand steps wide moves smoothly at any mailbox count, and because the total
/// is a multiple of it — three mailboxes are three thousand — so a mailbox finishing lands on a
/// number exactly rather than a third of the way between two.
const PARTS: u32 = 1000;

#[derive(Clone, Copy)]
struct Reading {
    folders: u32,
    done: u32,
    /// Whether the pass is over, however it ended.
    ///
    /// Kept rather than taken out. What a finished pass read is part of how far the burst has got,
    /// and a mailbox that vanishes from the sum the moment it finishes takes its folders with it —
    /// which is a bar that slides backwards at the one moment something went right.
    over: bool,
}

impl Reading {
    /// This mailbox's share of the burst, in parts of a thousand.
    ///
    /// Its own folders and nobody else's: a mailbox with three folders and a mailbox with thirty
    /// are each one mailbox of the round, and each is half done at half its own folders.
    fn share(&self) -> u32 {
        if self.over {
            return PARTS;
        }

        match self.folders {
            // Claimed, and has not said how many folders it holds yet. Nothing of it is done.
            0 => 0,
            folders => self.done * PARTS / folders,
        }
    }
}

/// What the window is told: mailboxes finished out of mailboxes asked for, and how far through
/// the whole burst the reading has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Along {
    pub mailboxes_done: u32,
    pub mailboxes: u32,
    /// How far through, and out of what. `of` is a whole mailbox's worth for every mailbox asked
    /// for, so nothing here is divided and nothing is rounded away: three mailboxes done of nine
    /// is exactly three ninths of the bar, not two and a nearly.
    pub through: u32,
    pub of: u32,
}

impl HowFarAlong {
    /// A pass over this account has finished, however it ended.
    pub fn finished(&self, account_id: &str) {
        if let Some(one) = self
            .reading
            .lock()
            .expect("the lock is never poisoned")
            .get_mut(account_id)
        {
            one.over = true;
        }
    }

    /// The burst is over: nothing at all is being read.
    ///
    /// What ends a burst is the last claim being dropped, and this is called from there. Without
    /// it a round would begin against the finished passes of the one before it, and the line would
    /// say a round was half done before anything in it had been read.
    pub fn nothing_is_being_read(&self) {
        self.reading
            .lock()
            .expect("the lock is never poisoned")
            .clear();
    }

    /// How far the burst has got, given how many mailboxes it was asked for.
    ///
    /// The total is every mailbox asked for rather than the ones that have reported, so a pass
    /// that has not yet listed its folders is nought of its own share rather than absent from the
    /// sum. That is what keeps the bar from sliding backwards in the first second of a round,
    /// while the passes are still finding out how much there is to read.
    pub fn along(&self, mailboxes_done: u32, mailboxes: u32) -> Along {
        let reading = self.reading.lock().expect("the lock is never poisoned");

        Along {
            mailboxes_done,
            mailboxes,
            through: reading.values().map(Reading::share).sum(),
            of: mailboxes * PARTS,
        }
    }
}

impl Progress for HowFarAlong {
    fn reading(&self, account_id: &str, folders: u32) {
        self.reading
            .lock()
            .expect("the lock is never poisoned")
            .insert(
                account_id.to_owned(),
                Reading {
                    folders,
                    done: 0,
                    over: false,
                },
            );
    }

    fn folder_read(&self, account_id: &str) {
        if let Some(one) = self
            .reading
            .lock()
            .expect("the lock is never poisoned")
            .get_mut(account_id)
        {
            // Capped at what the pass said it would read. A report past that would draw a bar
            // past full, and one mailbox cannot be more than its own share of the round.
            one.done = (one.done + 1).min(one.folders);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_move_while_mailboxes_stand_still() {
        // The whole reason folders are counted at all. Nine mailboxes read at once start and
        // finish together, so counted by mailbox a round is nought for five seconds and then
        // nine — a bar that stands still and then fills in one frame. Folders finish one after
        // another inside each account, all round long.
        let along = HowFarAlong::default();
        for account in ["acc-1", "acc-2", "acc-3"] {
            along.reading(account, 6);
        }

        assert_eq!(
            along.along(0, 3),
            Along {
                mailboxes_done: 0,
                mailboxes: 3,
                through: 0,
                of: 3 * PARTS
            }
        );

        along.folder_read("acc-1");
        along.folder_read("acc-1");
        along.folder_read("acc-2");

        let moving = along.along(0, 3);
        assert_eq!(moving.mailboxes_done, 0, "not one mailbox has finished");
        assert!(moving.through > 0, "and yet the bar has moved");
    }

    #[test]
    fn a_burst_that_ended_leaves_nothing_behind() {
        // A round that stopped halfway must not count into the next one, and a mailbox still
        // listed as reading would hold the bar short of full for ever.
        let along = HowFarAlong::default();
        along.reading("acc-1", 6);
        along.folder_read("acc-1");
        along.finished("acc-1");
        along.nothing_is_being_read();

        assert_eq!(along.along(0, 0), Along::default());
    }

    #[test]
    fn a_folder_counted_for_a_pass_nobody_started_is_ignored() {
        // The claim is what starts a pass and what ends it, so a report arriving after the end is
        // a report about nothing. Counted anyway it would take the bar past full.
        let along = HowFarAlong::default();
        along.folder_read("acc-1");

        assert_eq!(along.along(0, 0), Along::default());
    }

    #[test]
    fn a_big_mailbox_finishing_does_not_take_the_bar_backwards() {
        // The fault this arithmetic exists to prevent, in the shape that produced it. Two
        // mailboxes of very different sizes: counted by folders pooled together, the big one
        // finishing and leaving the sum takes the bar from most of the way along to half of it.
        // Counted by share, a mailbox that finishes can only ever add.
        let along = HowFarAlong::default();
        along.reading("acc-1", 10);
        along.reading("acc-2", 2);
        for _ in 0..9 {
            along.folder_read("acc-1");
        }

        let nearly = along.along(0, 2).through;
        along.finished("acc-1");
        let finished = along.along(1, 2).through;

        assert!(
            finished >= nearly,
            "the bar went backwards when a mailbox finished: {nearly} then {finished}"
        );
        assert_eq!(finished, PARTS, "one whole mailbox of the two asked for");
    }

    #[test]
    fn a_mailbox_that_has_not_said_its_size_yet_does_not_shrink_the_bar() {
        // A pass lists its mailboxes before it can say how many folders it has, and the passes in
        // a round do not finish listing at the same moment. Counted over the mailboxes that have
        // reported, the second one reporting would divide the whole by a bigger number and slide
        // the bar back; counted over the mailboxes asked for, it changes nothing.
        let along = HowFarAlong::default();
        along.reading("acc-1", 4);
        along.folder_read("acc-1");
        along.folder_read("acc-1");

        let before = along.along(0, 2).through;
        along.reading("acc-2", 20);
        let after = along.along(0, 2).through;

        assert_eq!(
            before, after,
            "a mailbox joining the count moved the bar without any work being done"
        );
        assert_eq!(before, PARTS / 2, "half of one mailbox");
    }

    #[test]
    fn every_mailbox_finished_is_a_full_bar() {
        // What the line's last moment shows. Short of full here and the round ends on a bar that
        // never arrived.
        let along = HowFarAlong::default();
        along.reading("acc-1", 3);
        along.reading("acc-2", 7);
        along.finished("acc-1");
        along.finished("acc-2");

        let full = along.along(2, 2);
        assert_eq!(full.through, full.of, "every mailbox done is a full bar");
    }

    #[test]
    fn a_folder_reported_twice_cannot_take_the_bar_past_full() {
        let along = HowFarAlong::default();
        along.reading("acc-1", 2);
        for _ in 0..5 {
            along.folder_read("acc-1");
        }

        assert_eq!(along.along(0, 1).through, PARTS);
    }
}
