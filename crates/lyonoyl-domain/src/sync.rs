//! Sync state and the rule for removing local placements.
//!
//! A resync strategy reports what it observed; this module decides what may be deleted from that
//! report. An incomplete pass produces no deletions.

use std::collections::HashSet;
use std::ops::RangeInclusive;

use crate::message::{Flags, FolderId, ModSeq, Rfc822MessageId, Uid, UidValidity};

/// Where a provider keeps its sync state.
///
/// IMAP CONDSTORE keeps a cursor per folder; Gmail `historyId` and JMAP `state` keep one per
/// account covering every mailbox. The port declares which of the two applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncScope {
    Account,
    Mailbox,
}

/// A resume point, opaque to everything above the adapter.
///
/// Kept as a string rather than a number: a cursor may be an IMAP modseq, a Gmail `historyId` or
/// an opaque JMAP state token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCursor(String);

impl SyncCursor {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a resync strategy observed about disappearances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VanishedReport {
    /// QRESYNC `VANISHED (EARLIER)`: the server listed removals authoritatively.
    Exact(Vec<Uid>),
    /// A complete listing of what exists inside a known window. Local placements inside that
    /// window and absent from the listing are gone.
    AuthoritativeSet {
        window: RangeInclusive<Uid>,
        present: Vec<Uid>,
    },
    /// The pass was incomplete. Nothing is known about deletions.
    Unknown,
}

/// Message metadata as a provider reports it.
///
/// `references` is collected in the first fetch: the headers that place a message in a conversation
/// cannot be obtained later without walking the whole mailbox again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageEnvelope {
    pub rfc822_message_id: Option<Rfc822MessageId>,
    pub subject: Option<String>,
    /// Who it is from, as the list shows it: the name when the message gave one.
    pub sender: Option<String>,
    /// Who else the message was addressed to, as addresses and never as names.
    ///
    /// What an answer to everybody is built from. Kept as two lists because the two mean different
    /// things to the people in them: `To` is who the message is for, `Cc` is who is being kept
    /// informed, and an answer that promotes the second group to the first changes what was said
    /// about them.
    pub to: Vec<String>,
    pub cc: Vec<String>,
    /// Who it is from, as a reply has to address it.
    ///
    /// A separate field because the two are different things and only one of them can be sent to:
    /// `From: Zeta Quorin <zeta@example.com>` shows as `Zeta Quorin` and is answered at
    /// `zeta@example.com`. Absent when the message gave no address at all.
    pub sender_address: Option<String>,
    /// Unix seconds. The server's internal date when there is one; the `Date` header is a fallback.
    pub received_at: i64,
    pub rfc822_size: Option<u32>,
    /// The opening of the message as plain text, for the list to show under the subject.
    ///
    /// Text, never markup: it is rendered as a text node beside a hundred others, and a message
    /// that could put markup there would be putting it into the application's own window.
    pub preview: Option<String>,
    pub references: Vec<Rfc822MessageId>,
    pub flags: Flags,
}

/// A message that appeared in a folder since the previous pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrivedMessage {
    pub uid: Uid,
    pub modseq: Option<ModSeq>,
    pub envelope: MessageEnvelope,
}

/// Flags that changed on a message already known in this folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagUpdate {
    pub uid: Uid,
    pub flags: Flags,
    pub modseq: Option<ModSeq>,
}

/// Everything one pass learned about one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderDelta {
    pub folder_id: FolderId,
    /// The generation the UIDs below belong to. A mismatch with the stored value invalidates them.
    pub uid_validity: UidValidity,
    /// The lowest UID this pass looked at, whether or not anything was there.
    ///
    /// How far back the folder has now been read. Without it a folder whose mail all sits below the
    /// first window looks empty and is never read again: nothing arrived, so nothing recorded how
    /// far the looking went.
    pub covered_from: Option<Uid>,
    /// Whether this pass stopped before the newest message the folder held when it was read.
    ///
    /// New mail is read a window at a time, and what lies past the window's end has not been seen:
    /// a message moved into the folder a moment ago is there, and so is everything that arrived
    /// while this device was away. Such a folder has not been read to its end, and what it does
    /// not hold yet says nothing about where a message went.
    pub stopped_short: bool,
    pub arrived: Vec<ArrivedMessage>,
    pub flag_updates: Vec<FlagUpdate>,
    pub vanished: VanishedReport,
}

/// One pass over a provider: the folder deltas, and the cursor to store once they are applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncBatch {
    pub scope: SyncScope,
    /// The cursor the pass started from, kept so a stale batch can be recognised.
    pub cursor_before: Option<SyncCursor>,
    pub cursor_after: SyncCursor,
    pub per_mailbox: Vec<FolderDelta>,
}

/// What to do with a delta, given the UID generation stored for that folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaVerdict {
    Apply,
    /// The folder's UID space was reset: every stored `(folder, uid)` pair now addresses nothing.
    /// The messages are still on the server, so the folder is re-read rather than deleted.
    ResyncFolder,
}

/// Compares the generation a delta was produced in against the stored one.
pub fn verify_uid_validity(stored: Option<UidValidity>, reported: UidValidity) -> DeltaVerdict {
    match stored {
        None => DeltaVerdict::Apply,
        Some(stored) if stored == reported => DeltaVerdict::Apply,
        Some(_) => DeltaVerdict::ResyncFolder,
    }
}

/// Below this many placements a percentage is meaningless, so the floor applies instead.
pub const REMOVAL_FLOOR: usize = 50;

/// The share of a folder that may vanish in one pass before a human is asked.
pub const REMOVAL_SHARE_PERCENT: usize = 20;

/// The outcome of evaluating one report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deletions {
    /// Nothing may be removed: either the report knows nothing, or no local placement is affected.
    None,
    /// These placements are gone and may be removed now.
    Remove(Vec<Uid>),
    /// Authoritative, but above the safety cap. Removal waits for an explicit confirmation.
    NeedsConfirmation {
        proposed: Vec<Uid>,
        /// How many placements the folder holds in total.
        known: usize,
    },
}

/// How many placements may disappear in one pass without asking.
pub fn removal_cap(known: usize) -> usize {
    REMOVAL_FLOOR.max(known * REMOVAL_SHARE_PERCENT / 100)
}

/// Decides what a report permits to delete from the placements held locally for one folder.
///
/// `known` holds the local UIDs inside the part of the folder the report talks about, and the
/// result is produced by filtering it, so a deletion is always a subset of what is stored.
///
/// `folder_size` is the number of placements in the whole folder. The safety cap is a share of the
/// folder, not of the report: a report naming ten thousand UIDs may legitimately clear a mailbox
/// of ten thousand, and may not clear a mailbox of a million.
pub fn deletions_for(report: &VanishedReport, known: &[Uid], folder_size: usize) -> Deletions {
    let proposed: Vec<Uid> = match report {
        // An incomplete answer carries no information about deletions.
        VanishedReport::Unknown => return Deletions::None,

        VanishedReport::Exact(vanished) => {
            let vanished: HashSet<Uid> = vanished.iter().copied().collect();
            known
                .iter()
                .copied()
                .filter(|uid| vanished.contains(uid))
                .collect()
        }

        VanishedReport::AuthoritativeSet { window, present } => {
            let present: HashSet<Uid> = present.iter().copied().collect();
            known
                .iter()
                .copied()
                // The listing makes no claim outside its window, so history below it is untouched.
                .filter(|uid| window.contains(uid) && !present.contains(uid))
                .collect()
        }
    };

    if proposed.is_empty() {
        return Deletions::None;
    }

    if proposed.len() > removal_cap(folder_size) {
        return Deletions::NeedsConfirmation {
            proposed,
            known: folder_size,
        };
    }

    Deletions::Remove(proposed)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn uid(value: u32) -> Uid {
        Uid::new(value).expect("test UIDs are non-zero")
    }

    fn uids(values: impl IntoIterator<Item = u32>) -> Vec<Uid> {
        values.into_iter().map(uid).collect()
    }

    fn generation(value: u32) -> UidValidity {
        UidValidity::new(value).expect("test generations are non-zero")
    }

    #[test]
    fn a_folder_never_synced_before_accepts_any_generation() {
        assert_eq!(
            verify_uid_validity(None, generation(42)),
            DeltaVerdict::Apply
        );
    }

    #[test]
    fn a_matching_generation_is_applied() {
        assert_eq!(
            verify_uid_validity(Some(generation(42)), generation(42)),
            DeltaVerdict::Apply
        );
    }

    #[test]
    fn a_changed_generation_forces_a_resync_instead_of_deletions() {
        // The UIDs did not disappear, they stopped meaning anything. Re-reading the folder is the
        // only correct answer; removing the placements would look like the mail was deleted.
        assert_eq!(
            verify_uid_validity(Some(generation(42)), generation(43)),
            DeltaVerdict::ResyncFolder
        );
    }

    #[test]
    fn an_incomplete_pass_deletes_nothing_however_much_is_missing() {
        // Every message is unaccounted for, and none of them may be removed.
        let known = uids(1..=500);
        assert_eq!(
            deletions_for(&VanishedReport::Unknown, &known, known.len()),
            Deletions::None
        );
    }

    #[test]
    fn an_exact_report_removes_what_it_names() {
        let known = uids([1, 2, 3, 4]);
        let report = VanishedReport::Exact(uids([2, 4]));

        assert_eq!(
            deletions_for(&report, &known, known.len()),
            Deletions::Remove(uids([2, 4]))
        );
    }

    #[test]
    fn a_report_naming_uids_we_never_had_cannot_steer_the_result() {
        let known = uids([1, 2]);
        let report = VanishedReport::Exact(uids([2, 900, 901]));

        assert_eq!(
            deletions_for(&report, &known, known.len()),
            Deletions::Remove(uids([2]))
        );
    }

    #[test]
    fn an_authoritative_set_removes_the_gaps_inside_its_window() {
        let known = uids([10, 11, 12, 13]);
        let report = VanishedReport::AuthoritativeSet {
            window: uid(10)..=uid(13),
            present: uids([10, 13]),
        };

        assert_eq!(
            deletions_for(&report, &known, known.len()),
            Deletions::Remove(uids([11, 12]))
        );
    }

    #[test]
    fn an_authoritative_set_never_touches_history_outside_its_window() {
        // Windowed backfill depends on this: a pass over recent UIDs says nothing about old ones.
        let known = uids([1, 2, 3, 100, 101]);
        let report = VanishedReport::AuthoritativeSet {
            window: uid(100)..=uid(101),
            present: uids([100]),
        };

        assert_eq!(
            deletions_for(&report, &known, known.len()),
            Deletions::Remove(uids([101]))
        );
    }

    #[test]
    fn a_report_that_affects_nothing_of_ours_is_not_a_deletion() {
        let known = uids([1, 2, 3]);

        assert_eq!(
            deletions_for(&VanishedReport::Exact(uids([7, 8])), &known, known.len()),
            Deletions::None
        );
        assert_eq!(
            deletions_for(&VanishedReport::Exact(Vec::new()), &known, known.len()),
            Deletions::None
        );
        assert_eq!(
            deletions_for(&VanishedReport::Exact(uids([1])), &[], 0),
            Deletions::None
        );
    }

    #[test]
    fn a_small_mailbox_may_empty_itself_without_confirmation() {
        // Below the floor the percentage does not apply: a ten-message mailbox may empty fully.
        let known = uids(1..=10);
        let report = VanishedReport::Exact(uids(1..=10));

        assert_eq!(
            deletions_for(&report, &known, known.len()),
            Deletions::Remove(uids(1..=10))
        );
    }

    #[test]
    fn a_mass_deletion_in_a_large_folder_is_referred_to_the_user() {
        let known = uids(1..=1000);
        let cap = removal_cap(known.len());
        assert_eq!(cap, 200, "20% of a thousand, well above the floor");

        let within = VanishedReport::Exact(uids(1..=200));
        assert!(matches!(
            deletions_for(&within, &known, known.len()),
            Deletions::Remove(_)
        ));

        let beyond = VanishedReport::Exact(uids(1..=201));
        match deletions_for(&beyond, &known, known.len()) {
            Deletions::NeedsConfirmation { proposed, known } => {
                assert_eq!(proposed.len(), 201);
                assert_eq!(known, 1000);
            }
            other => panic!("a fifth of the folder vanishing must be confirmed, got {other:?}"),
        }
    }

    #[test]
    fn the_cap_is_a_share_of_the_folder_not_of_the_report() {
        // A report that names twenty thousand UIDs is ordinary in a folder of a hundred thousand
        // and alarming in a folder of thirty thousand. Only the folder size can tell them apart.
        let named = uids(1..=20_000);
        let report = VanishedReport::Exact(named.clone());

        assert!(matches!(
            deletions_for(&report, &named, 100_000),
            Deletions::Remove(_)
        ));
        assert!(matches!(
            deletions_for(&report, &named, 30_000),
            Deletions::NeedsConfirmation { .. }
        ));
    }

    // Properties checked over generated inputs.

    prop_compose! {
        fn any_uid()(value in 1u32..300) -> Uid {
            uid(value)
        }
    }

    prop_compose! {
        fn any_uids()(values in prop::collection::vec(1u32..300, 0..80)) -> Vec<Uid> {
            let mut values: Vec<u32> = values;
            values.sort_unstable();
            values.dedup();
            uids(values)
        }
    }

    fn any_report() -> impl Strategy<Value = VanishedReport> {
        prop_oneof![
            any_uids().prop_map(VanishedReport::Exact),
            (any_uid(), any_uid(), any_uids()).prop_map(|(a, b, present)| {
                let (start, end) = if a <= b { (a, b) } else { (b, a) };
                VanishedReport::AuthoritativeSet {
                    window: start..=end,
                    present,
                }
            }),
            Just(VanishedReport::Unknown),
        ]
    }

    fn removed(verdict: &Deletions) -> Vec<Uid> {
        match verdict {
            Deletions::Remove(uids) => uids.clone(),
            // Nothing is removed until the confirmation is given.
            Deletions::None | Deletions::NeedsConfirmation { .. } => Vec::new(),
        }
    }

    proptest! {
        /// An incomplete pass never deletes, for any local set.
        #[test]
        fn unknown_never_deletes(known in any_uids()) {
            prop_assert_eq!(deletions_for(&VanishedReport::Unknown, &known, known.len()), Deletions::None);
        }

        /// Only locally held UIDs can be deleted, whatever the report names.
        #[test]
        fn deletions_are_always_a_subset_of_what_we_hold(report in any_report(), known in any_uids()) {
            let verdict = deletions_for(&report, &known, known.len());
            let held: HashSet<Uid> = known.iter().copied().collect();
            for uid in removed(&verdict) {
                prop_assert!(held.contains(&uid), "{uid:?} was never ours");
            }
            if let Deletions::NeedsConfirmation { proposed, .. } = &verdict {
                for uid in proposed {
                    prop_assert!(held.contains(uid), "{uid:?} was never ours");
                }
            }
        }

        /// A windowed listing deletes nothing outside its window.
        #[test]
        fn a_window_bounds_what_a_listing_can_delete(
            start in any_uid(),
            end in any_uid(),
            present in any_uids(),
            known in any_uids(),
        ) {
            let (start, end) = if start <= end { (start, end) } else { (end, start) };
            let report = VanishedReport::AuthoritativeSet { window: start..=end, present };
            for uid in removed(&deletions_for(&report, &known, known.len())) {
                prop_assert!(uid >= start && uid <= end, "{uid:?} is outside {start:?}..={end:?}");
            }
        }

        /// Re-evaluating an applied pass proposes nothing, so a retry is idempotent.
        #[test]
        fn applying_a_pass_twice_changes_nothing_the_second_time(
            report in any_report(),
            known in any_uids(),
        ) {
            let first = removed(&deletions_for(&report, &known, known.len()));
            let gone: HashSet<Uid> = first.iter().copied().collect();
            let remaining: Vec<Uid> = known.into_iter().filter(|uid| !gone.contains(uid)).collect();

            prop_assert!(removed(&deletions_for(&report, &remaining, remaining.len())).is_empty());
        }
    }
}
