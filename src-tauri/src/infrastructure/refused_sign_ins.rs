//! The accounts whose server refused their password, and when the timer may sign in to them again.
//!
//! A sign-in refused every thirty seconds is what a provider answers by shutting the whole address
//! out, every other mailbox on it included. So once an account's password is refused, nothing on a
//! timer signs in with it until its wait is over: [`FIRST_WAIT`] the first time, doubling up to
//! [`LONGEST_WAIT`]. Then one attempt goes, and its answer either lifts the refusal or doubles the
//! wait. A person asking for the account, new settings, or any sign-in that works lifts it at once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::time::Instant;

use crate::ports::account_sync::AccountSyncError;

/// How long the timer leaves an account whose password was refused, the first time.
pub const FIRST_WAIT: Duration = Duration::from_secs(15 * 60);

/// The longest the timer leaves one, however often its password was refused.
pub const LONGEST_WAIT: Duration = Duration::from_secs(2 * 3600);

/// The accounts whose server refused their password. Shared by every round, watch, push and
/// refresh, as the claims on accounts are.
#[derive(Clone, Default)]
pub struct RefusedSignIns(Arc<Mutex<Held>>);

#[derive(Default)]
struct Held {
    refused: HashMap<String, Refusal>,
    /// How many times each account's refusal was lifted by a person, so the answer to a sign-in
    /// that set out before the last lift does not put the refusal back.
    lifted: HashMap<String, u64>,
}

#[derive(Debug, Clone, Copy)]
struct Refusal {
    /// When the timer may sign in again.
    until: Instant,
    /// How long the current wait is.
    wait: Duration,
    /// Whether the one attempt after a wait is on its way.
    asking_again: bool,
}

/// Which lift of an account's refusal a sign-in set out after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ticket(u64);

impl RefusedSignIns {
    /// The ticket of a sign-in about to set out.
    pub fn ticket(&self, account_id: &str) -> Ticket {
        Ticket(locked(&self.0).lifted.get(account_id).copied().unwrap_or(0))
    }

    /// Whether the timer may sign in to this account now.
    ///
    /// Once a wait is over, one attempt is let through and the rest are held until it answers.
    pub fn may_sign_in(&self, account_id: &str) -> bool {
        let now = Instant::now();
        let mut held = locked(&self.0);
        match held.refused.get_mut(account_id) {
            None => true,
            Some(refusal) if refusal.asking_again || now < refusal.until => false,
            Some(refusal) => {
                refusal.asking_again = true;
                true
            }
        }
    }

    /// Whether this account's password stands refused, letting nothing through.
    pub fn holds_off(&self, account_id: &str) -> bool {
        locked(&self.0).refused.contains_key(account_id)
    }

    /// How a sign-in that set out with `ticket` ended: a sign-in that worked lifts the refusal, a
    /// password the server refused is kept refused, and anything else — no network, a server that
    /// did not answer — leaves the refusal as it stood, to be asked again after another wait.
    pub fn ended(&self, account_id: &str, ticket: Ticket, how: Result<(), AccountSyncError>) {
        let mut held = locked(&self.0);
        match how {
            Ok(()) => {
                held.refused.remove(account_id);
            }
            Err(AccountSyncError::SignInRefused) => {
                if held.lifted.get(account_id).copied().unwrap_or(0) != ticket.0 {
                    return;
                }
                let now = Instant::now();
                let wait = match held.refused.get(account_id) {
                    None => FIRST_WAIT,
                    // The attempt after a wait, refused too.
                    Some(refusal) if refusal.asking_again => (refusal.wait * 2).min(LONGEST_WAIT),
                    // The same refusal heard twice, by a pass and a watch that set out together.
                    Some(_) => return,
                };
                held.refused.insert(
                    account_id.to_owned(),
                    Refusal {
                        until: now + wait,
                        wait,
                        asking_again: false,
                    },
                );
            }
            Err(AccountSyncError::Failed) => {
                if let Some(refusal) = held.refused.get_mut(account_id) {
                    if refusal.asking_again {
                        refusal.until = Instant::now() + refusal.wait;
                        refusal.asking_again = false;
                    }
                }
            }
        }
    }

    /// The attempt let through after a wait did not set out after all: the next one may go.
    pub fn did_not_ask(&self, account_id: &str) {
        if let Some(refusal) = locked(&self.0).refused.get_mut(account_id) {
            refusal.asking_again = false;
        }
    }

    /// A person asked for the account, or gave it new settings: the refusal is lifted, and the
    /// answer to a sign-in that set out before now does not put it back.
    pub fn lift(&self, account_id: &str) {
        let mut held = locked(&self.0);
        held.refused.remove(account_id);
        *held.lifted.entry(account_id.to_owned()).or_default() += 1;
    }
}

/// The accounts, whatever a thread that panicked holding them left behind: a moment and a count
/// each, where one read from half an update is a sign-in a moment early or late.
fn locked(held: &Mutex<Held>) -> MutexGuard<'_, Held> {
    held.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT: &str = "acc-1";

    fn refused_once() -> RefusedSignIns {
        let refused = RefusedSignIns::default();
        let ticket = refused.ticket(ACCOUNT);
        refused.ended(ACCOUNT, ticket, Err(AccountSyncError::SignInRefused));
        refused
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_password_is_asked_again_once_after_each_wait_and_the_wait_doubles() {
        let refused = refused_once();
        let mut wait = FIRST_WAIT;
        for _ in 0..5 {
            assert!(!refused.may_sign_in(ACCOUNT));
            tokio::time::advance(wait - Duration::from_secs(1)).await;
            assert!(!refused.may_sign_in(ACCOUNT), "asked before {wait:?}");
            tokio::time::advance(Duration::from_secs(1)).await;

            // One attempt, and nothing else while it is on its way.
            let ticket = refused.ticket(ACCOUNT);
            assert!(refused.may_sign_in(ACCOUNT));
            assert!(!refused.may_sign_in(ACCOUNT));
            refused.ended(ACCOUNT, ticket, Err(AccountSyncError::SignInRefused));

            wait = (wait * 2).min(LONGEST_WAIT);
        }

        // Fifteen minutes, half an hour, an hour, then two hours for as long as it lasts.
        assert_eq!(wait, LONGEST_WAIT);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refusal_heard_twice_is_one_refusal() {
        // A pass and a watch set out together and are both refused: the wait does not double.
        let refused = RefusedSignIns::default();
        let pass = refused.ticket(ACCOUNT);
        let watch = refused.ticket(ACCOUNT);
        refused.ended(ACCOUNT, pass, Err(AccountSyncError::SignInRefused));
        refused.ended(ACCOUNT, watch, Err(AccountSyncError::SignInRefused));

        tokio::time::advance(FIRST_WAIT).await;
        assert!(refused.may_sign_in(ACCOUNT));
    }

    #[tokio::test(start_paused = true)]
    async fn a_sign_in_that_works_lifts_the_refusal() {
        let refused = refused_once();
        tokio::time::advance(FIRST_WAIT).await;
        let ticket = refused.ticket(ACCOUNT);
        assert!(refused.may_sign_in(ACCOUNT));

        refused.ended(ACCOUNT, ticket, Ok(()));

        assert!(!refused.holds_off(ACCOUNT));
        assert!(refused.may_sign_in(ACCOUNT));
        assert!(refused.may_sign_in(ACCOUNT));
    }

    #[test]
    fn a_person_lifts_the_refusal_and_an_answer_from_before_does_not_put_it_back() {
        // New settings saved while a pass with the old password is on its way.
        let refused = RefusedSignIns::default();
        let before = refused.ticket(ACCOUNT);

        refused.lift(ACCOUNT);
        refused.ended(ACCOUNT, before, Err(AccountSyncError::SignInRefused));

        assert!(!refused.holds_off(ACCOUNT));
        assert!(refused.may_sign_in(ACCOUNT));

        // One that set out after the lift and is refused is kept.
        let after = refused.ticket(ACCOUNT);
        refused.ended(ACCOUNT, after, Err(AccountSyncError::SignInRefused));
        assert!(refused.holds_off(ACCOUNT));
    }

    #[tokio::test(start_paused = true)]
    async fn an_attempt_that_heard_nothing_waits_again_and_one_that_never_went_lets_the_next_go() {
        let refused = refused_once();
        tokio::time::advance(FIRST_WAIT).await;

        // No network for the attempt: the same wait again, not a doubled one.
        let ticket = refused.ticket(ACCOUNT);
        assert!(refused.may_sign_in(ACCOUNT));
        refused.ended(ACCOUNT, ticket, Err(AccountSyncError::Failed));
        assert!(!refused.may_sign_in(ACCOUNT));
        tokio::time::advance(FIRST_WAIT).await;
        assert!(refused.may_sign_in(ACCOUNT));

        // Let through, then not sent at all: the next may go at once.
        refused.did_not_ask(ACCOUNT);
        assert!(refused.may_sign_in(ACCOUNT));
        assert!(refused.holds_off(ACCOUNT));
    }

    #[test]
    fn other_accounts_are_not_held_back_by_one_refusal() {
        let refused = refused_once();

        assert!(!refused.may_sign_in(ACCOUNT));
        assert!(refused.may_sign_in("acc-2"));
        assert!(!refused.holds_off("acc-2"));
    }
}
