//! What the mail looks like once everything a person asked for is laid over what a server said.
//!
//! Two records answer "where is this message and has it been read". The index holds what the last
//! pass read from the server; the operation log holds what a person has asked for since, including
//! what the server has carried out and no pass has read back. Neither is the answer alone. A list,
//! a count beside a folder and the number on the application's icon are all this fold of the
//! second over the first, and they are the same fold so that they cannot disagree.
//!
//! Nothing here is written anywhere. The index keeps saying what the server said, and the log keeps
//! saying what was asked; the fold is recomputed from both whenever either changes. That is what
//! lets a way back be the removal of an instruction, and what lets a rebuilt index come back right:
//! the log survives it, and is laid over the fresh index the same way.

use std::collections::{BTreeMap, BTreeSet};

use crate::message::{Flags, FolderId, MessageId};
use crate::operation::{DeleteIntent, OperationKind, OperationState, Stopped};

/// What one instruction does to the copy of a message it names, in the terms the fold needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Read, or unread.
    Seen(bool),
    /// Out of the folder, and into another one or into nothing.
    Leaves { to: Option<FolderId> },
    /// It stopped, and a person has to look. It changes nothing about where the message is or
    /// whether it has been read: nothing happened.
    Stopped(Stopped),
}

impl Change {
    /// What an instruction in the log does to its copy, or nothing when it does nothing a view
    /// shows.
    ///
    /// `trash` is where the account keeps deleted mail, which is where a delete that can be taken
    /// back sends a message; an account with nowhere to keep it sends it nowhere until the server
    /// is asked and the instruction stops.
    pub fn of(
        kind: OperationKind,
        state: OperationState,
        stopped: Option<Stopped>,
        trash: Option<FolderId>,
    ) -> Option<Self> {
        if state == OperationState::Conflicted {
            return Some(Self::Stopped(stopped.unwrap_or(Stopped::TheServerRefused)));
        }

        match kind {
            OperationKind::SetFlags { add, remove } => {
                if add.contains(Flags::SEEN) {
                    Some(Self::Seen(true))
                } else if remove.contains(Flags::SEEN) {
                    Some(Self::Seen(false))
                } else {
                    None
                }
            }
            OperationKind::MoveTo { folder } => Some(Self::Leaves { to: Some(folder) }),
            OperationKind::Delete {
                intent: DeleteIntent::ToTrash,
            } => Some(Self::Leaves { to: trash }),
            OperationKind::Delete {
                intent: DeleteIntent::Permanently,
            } => Some(Self::Leaves { to: None }),
        }
    }
}

/// One instruction, as the fold reads it: which copy, and what it does to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asked {
    pub message: MessageId,
    pub folder: FolderId,
    pub change: Change,
}

/// A message on its way into a folder the index does not have it in yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arriving {
    /// The folder the index still has it in, which is where what it says is read from.
    pub from: FolderId,
    /// Read or unread as it left, where somebody said so on the way.
    pub seen: Option<bool>,
}

/// One copy of a message, as what was asked makes it look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shown {
    /// Read or unread as a person asked, on this copy. Nothing where nobody asked.
    pub seen: Option<bool>,
    /// On its way out of this folder.
    pub leaving: bool,
    /// On its way in: sent here by an instruction, and not read into this folder by any pass.
    pub arriving: Option<Arriving>,
    /// Why an instruction about this copy stopped, where one has.
    pub stopped: Option<Stopped>,
}

impl Shown {
    /// Read or unread, for a copy on its way in: what was asked of it here, then what it carried
    /// in, then what the copy it came from says.
    ///
    /// A folder that already held a copy of the message and is sent a second one is answered by
    /// the one arriving. The two are one message here and two on the server, and a server that
    /// keeps both says so on the next pass.
    pub fn arrived_seen(&self, came_from_unread: bool) -> bool {
        self.seen
            .or(self.arriving.and_then(|arriving| arriving.seen))
            .unwrap_or(!came_from_unread)
    }
}

/// Every copy something was asked about, folded in the order it was asked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AsAsked {
    copies: BTreeMap<(MessageId, FolderId), Shown>,
}

/// A copy the index holds, as a count reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held {
    pub message: MessageId,
    pub folder: FolderId,
    pub unread: bool,
}

/// How far what was asked moves the counts the index gives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counted {
    /// For each folder touched: how many more messages it holds, and how many more are unread.
    /// Either may be negative.
    pub folders: BTreeMap<FolderId, (i64, i64)>,
    /// How many more messages are unread in an inbox, which is what an account's own row counts.
    pub inbox_unread: i64,
}

impl AsAsked {
    /// The fold. Later instructions win over earlier ones, and a move carries what it moved.
    ///
    /// The order is the order they were asked in, which is the order the log keeps them in and the
    /// order the server is told them in.
    pub fn of(asked: impl IntoIterator<Item = Asked>) -> Self {
        let mut folded = Self::default();

        for Asked {
            message,
            folder,
            change,
        } in asked
        {
            match change {
                Change::Seen(seen) => folded.at(message, folder).seen = Some(seen),
                Change::Stopped(why) => folded.at(message, folder).stopped = Some(why),
                // A message cannot leave for where it already is.
                Change::Leaves { to } if to == Some(folder) => {}
                Change::Leaves { to } => {
                    let leaving = folded.at(message, folder);
                    leaving.leaving = true;

                    // What it carries is what it was here: read as asked, or as it came in, and
                    // from wherever the index still has it.
                    let carried = Arriving {
                        from: leaving.arriving.map_or(folder, |arriving| arriving.from),
                        seen: leaving
                            .seen
                            .or(leaving.arriving.and_then(|arriving| arriving.seen)),
                    };

                    if let Some(to) = to {
                        // Whatever was said about a copy there before is about a copy that has
                        // since gone; what arrives speaks for itself. Sent back to where it was, it
                        // is not leaving any more.
                        *folded.at(message, to) = Shown {
                            arriving: Some(carried),
                            ..Shown::default()
                        };
                    }
                }
            }
        }

        folded
    }

    fn at(&mut self, message: MessageId, folder: FolderId) -> &mut Shown {
        self.copies.entry((message, folder)).or_default()
    }

    /// Whether nothing was asked about anything, which is the ordinary case and costs nothing.
    pub fn is_empty(&self) -> bool {
        self.copies.is_empty()
    }

    /// What was asked about one copy, if anything was.
    pub fn copy(&self, message: MessageId, folder: FolderId) -> Option<&Shown> {
        self.copies.get(&(message, folder))
    }

    /// The messages on their way into a folder, whether or not the index already has them there.
    ///
    /// The caller leaves out the ones it has: those are the copy itself rather than a message on
    /// its way.
    pub fn arriving_into(&self, folder: FolderId) -> Vec<MessageId> {
        self.copies
            .iter()
            .filter(|((_, into), shown)| {
                *into == folder && shown.arriving.is_some() && !shown.leaving
            })
            .map(|((message, _), _)| *message)
            .collect()
    }

    /// Every message something was asked about.
    pub fn messages(&self) -> BTreeSet<MessageId> {
        self.copies.keys().map(|(message, _)| *message).collect()
    }

    /// How far the counts the index gives move once this is laid over them.
    ///
    /// `held` is every copy the index holds of the messages in [`AsAsked::messages`], and
    /// `is_inbox` says which folders an account's own count reads. A folder counts a message once
    /// however many copies of it it holds, and it is unread there when any of them is — which is
    /// how the index counts, so the two can be added.
    pub fn counted(&self, held: &[Held], is_inbox: impl Fn(FolderId) -> bool) -> Counted {
        let mut counted = Counted::default();

        // Grouped by message once, rather than searched for each one: emptying a large trash
        // asks about every message in it, and the sidebar is read after every click.
        let mut held_by_message: BTreeMap<MessageId, BTreeMap<FolderId, bool>> = BTreeMap::new();
        for copy in held {
            *held_by_message
                .entry(copy.message)
                .or_default()
                .entry(copy.folder)
                .or_insert(false) |= copy.unread;
        }
        let mut asked_by_message: BTreeMap<MessageId, Vec<(FolderId, &Shown)>> = BTreeMap::new();
        for ((message, folder), shown) in &self.copies {
            asked_by_message
                .entry(*message)
                .or_default()
                .push((*folder, shown));
        }

        for (message, asked) in asked_by_message {
            let before = held_by_message.remove(&message).unwrap_or_default();

            let mut after = before.clone();
            for (folder, shown) in asked {
                if shown.leaving {
                    after.remove(&folder);
                } else if let Some(arriving) = shown.arriving {
                    // In, from wherever the index still has it -- or back where it was, carrying
                    // whatever was said about it on the way. Where a pass has already read it out
                    // of there and into here, the copy it read in is the one that speaks, as it
                    // does on the list: counted from a copy the index no longer holds, a message
                    // moved unread would be counted read until the move was settled.
                    let came_from_unread = before
                        .get(&arriving.from)
                        .or_else(|| before.get(&folder))
                        .copied()
                        .unwrap_or(false);
                    after.insert(folder, !shown.arrived_seen(came_from_unread));
                } else if let (true, Some(seen)) = (before.contains_key(&folder), shown.seen) {
                    after.insert(folder, !seen);
                }
            }

            let folders: BTreeSet<FolderId> = before.keys().chain(after.keys()).copied().collect();
            for folder in folders {
                let was = before.get(&folder);
                let is = after.get(&folder);
                let moved = counted.folders.entry(folder).or_insert((0, 0));
                moved.0 += i64::from(is.is_some()) - i64::from(was.is_some());
                moved.1 += i64::from(is.copied().unwrap_or(false))
                    - i64::from(was.copied().unwrap_or(false));
            }

            let unread_in_an_inbox = |copies: &BTreeMap<FolderId, bool>| {
                copies
                    .iter()
                    .any(|(folder, unread)| *unread && is_inbox(*folder))
            };
            counted.inbox_unread +=
                i64::from(unread_in_an_inbox(&after)) - i64::from(unread_in_an_inbox(&before));
        }

        counted
            .folders
            .retain(|_, (total, unread)| *total != 0 || *unread != 0);
        counted
    }
}

/// A count moved by what was asked, never below nothing.
///
/// A count cannot be less than nothing, and a fold over an index that has just been rebuilt can
/// say it should be for a moment: the log remembers a message leaving a folder the fresh index has
/// not read into yet.
pub fn moved_by(count: u32, by: i64) -> u32 {
    u32::try_from((i64::from(count) + by).max(0)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;
