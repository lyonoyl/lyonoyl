//! Conversations: which messages belong together, and what happens when two turn out to be one.
//!
//! A conversation is not something a server keeps. It is a conclusion drawn from headers, and the
//! conclusion changes as older mail arrives: two conversations that looked separate become one the
//! moment a message naming both is stored. Everything here is about surviving that.

use crate::message::{MessageId, Rfc822MessageId};

/// The name a conversation is addressed by: the identifier of its oldest message.
///
/// A row on screen has to keep its name while the list is scrolled, selected from and restored,
/// and a conversation has no name of its own to give it. So the name is borrowed from the message
/// that was there first, and merging never renames the one that survives — see [`joining`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadKey(MessageId);

impl ThreadKey {
    /// The conversation whose oldest message is this one.
    pub fn of(root: MessageId) -> Self {
        Self(root)
    }

    /// The message the name was taken from.
    pub fn root(self) -> MessageId {
        self.0
    }
}

/// What a message arriving does to the conversations already known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Joining {
    /// Nothing it names is here yet, so it begins a conversation of its own.
    Begins,
    /// One conversation is named, and the message belongs to it.
    Joins(ThreadKey),
    /// Several are named, so they were one conversation all along. The oldest survives under the
    /// name it already had; the rest are absorbed into it.
    Ties {
        into: ThreadKey,
        absorbing: Vec<ThreadKey>,
    },
}

/// Which conversation a message belongs to, given the ones its headers name.
///
/// `named` is what the message's own identifier and everything in its reference chain already
/// belong to. Repeats and order do not matter, and neither does how the caller found them.
///
/// The survivor is always the lowest, which is what makes merging monotonic: a name that survived
/// one merge survives every later one, so a row already on screen keeps the name it was drawn
/// with. Choosing by anything else — the largest, the newest, the one with the most messages —
/// renames rows under a reader for no gain.
pub fn joining(named: &[ThreadKey]) -> Joining {
    let mut distinct: Vec<ThreadKey> = named.to_vec();
    distinct.sort_unstable();
    distinct.dedup();

    let mut names = distinct.into_iter();
    match names.next() {
        None => Joining::Begins,
        Some(oldest) => {
            let absorbing: Vec<ThreadKey> = names.collect();
            if absorbing.is_empty() {
                Joining::Joins(oldest)
            } else {
                Joining::Ties {
                    into: oldest,
                    absorbing,
                }
            }
        }
    }
}

/// How many names of one message are ever consulted.
///
/// A `References` header has no length limit, and a message is hostile input. Without a bound, one
/// message carrying a hundred thousand identifiers costs a hundred thousand lookups at the moment
/// it is stored, on a connection holding a write transaction.
pub const NAMES_CONSULTED: usize = 128;

/// The names a message's threading is decided by, in the order they are looked up.
///
/// Its own identifier first, because a reply may already have arrived and named it. Then the chain
/// it carries, trimmed to [`NAMES_CONSULTED`] by keeping both ends: the oldest name is the one the
/// conversation is likely to be called after, and the newest are its immediate ancestors. The
/// middle of a long chain adds nothing that either end does not already reach.
pub fn names_that_decide<'a>(
    own: Option<&'a Rfc822MessageId>,
    chain: &'a [Rfc822MessageId],
) -> Vec<&'a Rfc822MessageId> {
    let mut names: Vec<&Rfc822MessageId> = own.into_iter().collect();

    let room = NAMES_CONSULTED.saturating_sub(names.len());
    names.extend(both_ends(chain, room));
    names
}

/// A chain cut to at most `room` names: the oldest, and the newest that fill the rest.
///
/// The same cut wherever a chain is cut: threading reads a message's chain through it, and a reply
/// carries the names it keeps, so what a reply carries is what its conversation is threaded by. A
/// chain that fits is kept whole, in its order.
pub fn both_ends<T>(chain: &[T], room: usize) -> impl Iterator<Item = &T> {
    let (from_the_front, from_the_back) = match chain.len() <= room {
        true => (chain.len(), 0),
        false => (1.min(room), room - 1.min(room)),
    };

    chain[..from_the_front]
        .iter()
        .chain(&chain[chain.len() - from_the_back..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn key(id: u64) -> ThreadKey {
        ThreadKey::of(MessageId::new(id).expect("non-zero"))
    }

    fn name(at: usize) -> Rfc822MessageId {
        Rfc822MessageId::parse(&format!("<{at}@example.invalid>")).expect("well formed")
    }

    #[test]
    fn a_message_naming_nothing_known_begins_a_conversation() {
        assert_eq!(joining(&[]), Joining::Begins);
    }

    #[test]
    fn a_message_naming_one_conversation_joins_it() {
        assert_eq!(joining(&[key(7)]), Joining::Joins(key(7)));
    }

    #[test]
    fn naming_the_same_conversation_twice_is_naming_it_once() {
        // A reference chain repeats identifiers, and so does a message that answers itself.
        assert_eq!(joining(&[key(7), key(7), key(7)]), Joining::Joins(key(7)));
    }

    #[test]
    fn a_message_naming_several_ties_them_into_the_oldest() {
        assert_eq!(
            joining(&[key(9), key(3), key(5)]),
            Joining::Ties {
                into: key(3),
                absorbing: vec![key(5), key(9)],
            }
        );
    }

    #[test]
    fn the_order_the_names_arrive_in_does_not_change_the_answer() {
        let one = joining(&[key(9), key(3), key(5)]);
        let other = joining(&[key(5), key(9), key(3)]);
        assert_eq!(one, other);
    }

    #[test]
    fn a_conversation_absorbs_nothing_it_already_is() {
        match joining(&[key(3), key(5)]) {
            Joining::Ties { into, absorbing } => {
                assert!(!absorbing.contains(&into), "it would absorb itself");
            }
            other => panic!("two names are a tie, not {other:?}"),
        }
    }

    proptest! {
        /// The invariant a row on screen depends on: whatever a conversation is tied to, its name
        /// only ever moves to a lower one. So a name that survived a merge survives every later
        /// merge, and a reader looking at a thread never has it renamed under them.
        #[test]
        fn a_surviving_name_never_moves_upward(ids in prop::collection::vec(1u64..1000, 1..40)) {
            let named: Vec<ThreadKey> = ids.iter().map(|id| key(*id)).collect();
            let lowest = named.iter().min().copied().expect("at least one");

            let survivor = match joining(&named) {
                Joining::Begins => unreachable!("at least one name was given"),
                Joining::Joins(one) => one,
                Joining::Ties { into, .. } => into,
            };

            prop_assert_eq!(survivor, lowest);
        }

        /// Nothing is lost and nothing is invented: every name given is either the survivor or
        /// absorbed into it, exactly once.
        #[test]
        fn every_name_given_is_accounted_for(ids in prop::collection::vec(1u64..1000, 1..40)) {
            let named: Vec<ThreadKey> = ids.iter().map(|id| key(*id)).collect();

            let mut accounted = match joining(&named) {
                Joining::Begins => Vec::new(),
                Joining::Joins(one) => vec![one],
                Joining::Ties { into, absorbing } => {
                    let mut all = vec![into];
                    all.extend(absorbing);
                    all
                }
            };
            accounted.sort_unstable();

            let mut expected = named;
            expected.sort_unstable();
            expected.dedup();

            prop_assert_eq!(accounted, expected);
        }
    }

    #[test]
    fn a_short_chain_is_consulted_whole() {
        let own = name(0);
        let chain: Vec<Rfc822MessageId> = (1..=5).map(name).collect();

        let consulted = names_that_decide(Some(&own), &chain);

        assert_eq!(consulted.len(), 6);
        assert_eq!(consulted[0], &own);
        assert_eq!(consulted[5], &chain[4]);
    }

    #[test]
    fn a_chain_longer_than_the_bound_keeps_both_ends() {
        // The header is written by whoever sent the message, so its length is not ours to trust.
        let own = name(0);
        let chain: Vec<Rfc822MessageId> = (1..=50_000).map(name).collect();

        let consulted = names_that_decide(Some(&own), &chain);

        assert_eq!(consulted.len(), NAMES_CONSULTED);
        assert_eq!(consulted[0], &own, "its own name is always consulted");
        assert_eq!(
            consulted[1], &chain[0],
            "and the one the thread is named after"
        );
        assert_eq!(
            consulted[NAMES_CONSULTED - 1],
            &chain[chain.len() - 1],
            "and its immediate parent"
        );
    }

    #[test]
    fn a_message_without_a_name_of_its_own_is_still_threaded_by_its_chain() {
        // A message whose `Message-ID` is missing or unusable is never merged with another, but it
        // can still answer one that exists.
        let chain: Vec<Rfc822MessageId> = (1..=3).map(name).collect();

        let consulted = names_that_decide(None, &chain);

        assert_eq!(consulted.len(), 3);
        assert_eq!(consulted[0], &chain[0]);
    }
}
