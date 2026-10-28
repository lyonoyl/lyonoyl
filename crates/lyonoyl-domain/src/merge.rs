//! One window of the list, taken from several files that each hold part of it.

use std::collections::BinaryHeap;

/// Where a row sits in the order the list is kept in: newest first.
///
/// Two fields rather than one because two messages can arrive in the same second, and an order
/// that cannot settle a tie is not an order: the same window read twice would answer differently,
/// and the row a person is looking at would move under them.
///
/// The identifier settles it, which works only while an identifier is unique across every file on
/// the device. That is what the account carried in its top bits is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place {
    /// When the message arrived, as the list orders by.
    pub arrived_at: i64,
    /// The message, to settle a second two of them share.
    pub id: u64,
}

/// Takes one window out of runs that are each already in order.
///
/// Each run is one file answering for the accounts it holds, already sorted — the index does that,
/// and doing it again here would be sorting what is sorted. What is left is choosing, at each
/// step, which run holds the next row.
///
/// It reads `offset + limit` rows and no more, plus the one it must look at in every run to know
/// where to start. A window deep in a long list therefore costs what the depth costs and not what
/// the mailbox costs — the same bargain a single file makes, kept across several.
pub fn window_across<I, T>(runs: &mut [I], offset: usize, limit: usize) -> Vec<T>
where
    I: Iterator<Item = (Place, T)>,
{
    // One row from each run to begin: the next row of the whole list is the newest of these, and
    // it cannot be anywhere else while every run is in order.
    let mut next: BinaryHeap<(Place, usize)> = BinaryHeap::new();
    let mut held: Vec<Option<T>> = runs.iter().map(|_| None).collect();

    for (which, run) in runs.iter_mut().enumerate() {
        if let Some((place, row)) = run.next() {
            next.push((place, which));
            held[which] = Some(row);
        }
    }

    let mut taken = Vec::new();
    let mut passed = 0usize;

    while let Some((_, which)) = next.pop() {
        let row = held[which]
            .take()
            .expect("the run that was chosen held a row");

        if passed < offset {
            passed += 1;
        } else {
            taken.push(row);
            if taken.len() == limit {
                break;
            }
        }

        if let Some((place, row)) = runs[which].next() {
            next.push((place, which));
            held[which] = Some(row);
        }
    }

    taken
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// What the window is, worked out without the merge: every row in one heap, in order.
    ///
    /// Deliberately not the algorithm above. An oracle that merged would be blind wherever the
    /// merge is.
    fn all_of_them(runs: &[Vec<(Place, u32)>], offset: usize, limit: usize) -> Vec<u32> {
        let mut every: Vec<(Place, u32)> = runs.iter().flatten().copied().collect();
        every.sort_by_key(|(place, _)| std::cmp::Reverse(*place));
        every
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|(_, row)| row)
            .collect()
    }

    /// Runs as the index hands them over: each already newest-first, and no two rows alike.
    fn runs_in_order() -> impl Strategy<Value = Vec<Vec<(Place, u32)>>> {
        proptest::collection::vec(
            proptest::collection::vec((0i64..40, 0u64..400), 0..12),
            1..6,
        )
        .prop_map(|runs| {
            let mut given = 0u32;
            let mut seen = std::collections::HashSet::new();
            runs.into_iter()
                .map(|run| {
                    let mut rows: Vec<(Place, u32)> = run
                        .into_iter()
                        .filter(|(_, id)| seen.insert(*id))
                        .map(|(arrived_at, id)| {
                            given += 1;
                            (Place { arrived_at, id }, given)
                        })
                        .collect();
                    rows.sort_by_key(|(place, _)| std::cmp::Reverse(*place));
                    rows
                })
                .collect()
        })
    }

    proptest! {
        #[test]
        fn a_window_across_files_holds_what_one_list_of_them_all_would(
            runs in runs_in_order(),
            offset in 0usize..20,
            limit in 1usize..12,
        ) {
            let mut cursors: Vec<_> = runs.iter().map(|run| run.iter().copied()).collect();
            let merged = window_across(&mut cursors, offset, limit);

            prop_assert_eq!(merged, all_of_them(&runs, offset, limit));
        }

        /// Reading two halves of a window one after the other reads the same rows as reading it
        /// whole. A merge that lost its place between windows would show a row twice or skip one.
        #[test]
        fn two_windows_side_by_side_are_the_window_that_spans_them(
            runs in runs_in_order(),
            offset in 0usize..10,
            first in 1usize..6,
            second in 1usize..6,
        ) {
            let taken = |at, how_many| {
                let mut cursors: Vec<_> = runs.iter().map(|run| run.iter().copied()).collect();
                window_across(&mut cursors, at, how_many)
            };

            let mut apart = taken(offset, first);
            apart.extend(taken(offset + first, second));

            prop_assert_eq!(apart, taken(offset, first + second));
        }
    }

    #[test]
    fn nothing_at_all_is_an_empty_window_rather_than_a_failure() {
        let mut cursors: Vec<std::vec::IntoIter<(Place, u32)>> = Vec::new();
        assert!(window_across(&mut cursors, 0, 50).is_empty());

        let mut empty = vec![Vec::<(Place, u32)>::new().into_iter()];
        assert!(window_across(&mut empty, 0, 50).is_empty());
    }

    #[test]
    fn a_window_past_the_end_is_empty_rather_than_the_last_rows_again() {
        let run = vec![
            (
                Place {
                    arrived_at: 2,
                    id: 2,
                },
                20u32,
            ),
            (
                Place {
                    arrived_at: 1,
                    id: 1,
                },
                10u32,
            ),
        ];
        let mut cursors = vec![run.into_iter()];
        assert!(window_across(&mut cursors, 5, 50).is_empty());
    }
}
