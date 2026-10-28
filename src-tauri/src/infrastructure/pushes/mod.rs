//! Telling a server what a person just did, a moment after they did it.
//!
//! What a person does is written down and shown at once; this is the other half, and it runs by
//! itself. A push is asked for the instant an instruction is written, gathers whatever else is
//! asked about the same account in the next quarter of a second, and then tells that account's
//! server — outside the rounds, so it never waits for one, never starts one, and never moves the
//! timer they run on. Nor does the watch on that account's inbox read it again when the server
//! reports the push back to it: see `A_PUSH_ECHOES_FOR` in [`crate::infrastructure::schedule`].
//!
//! Silent. Nothing at the foot of the window moves for it: the change is already on the screen,
//! and a person does not need telling that it is on its way. What does get said is a change that
//! is late, which the line works out from the queue itself.
//!
//! One push per account at a time, and at most one more waiting behind it: a burst of clicking is
//! one push, and anything asked while it runs is carried by the next.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Semaphore;

use crate::infrastructure::schedule::{Announce, InFlight, Why};
use crate::ports::account_sync::{AccountPush, Pushed, ReadBack};
use crate::ports::outbox::A_COPY_SETTLES;

/// How long a push waits for more to be asked about the same account before it goes.
///
/// Long enough that somebody walking a folder with the arrow keys, marking a message read at
/// every step, is one push rather than one per keystroke; short enough that the server has it
/// before they could have looked anywhere else.
pub const GATHERED_FOR: Duration = Duration::from_millis(250);

/// Asks for an account's changes to be told to its server.
#[derive(Clone)]
pub struct Pushes(Arc<Pushing>);

struct Pushing {
    push: Arc<dyn AccountPush>,
    in_flight: InFlight,
    runtime: tokio::runtime::Handle,
    at_once: Arc<Semaphore>,
    announce: Announce,
    gathered_for: Duration,
    /// How long after a message went out its copy is filed.
    copy_settles: Duration,
    /// The accounts with a push underway or about to be, and whether more was asked for since it
    /// took what it carries.
    underway: Mutex<HashMap<String, bool>>,
}

impl Pushes {
    pub fn start(
        push: Arc<dyn AccountPush>,
        in_flight: InFlight,
        runtime: tokio::runtime::Handle,
        // The same limit every way of talking to a server shares: a push is a conversation like
        // any other, and a limit that left it out would describe some of them.
        at_once: Arc<Semaphore>,
        announce: Announce,
    ) -> Self {
        Self::gathering_for(
            push,
            in_flight,
            runtime,
            at_once,
            announce,
            GATHERED_FOR,
            A_COPY_SETTLES,
        )
    }

    /// The same, told how long to gather for and how long a copy settles. Arguments only so a test
    /// can drive the real thing with a real clock: a push is blocking work, and a paused clock
    /// never moves past one.
    fn gathering_for(
        push: Arc<dyn AccountPush>,
        in_flight: InFlight,
        runtime: tokio::runtime::Handle,
        at_once: Arc<Semaphore>,
        announce: Announce,
        gathered_for: Duration,
        copy_settles: Duration,
    ) -> Self {
        Self(Arc::new(Pushing {
            push,
            in_flight,
            runtime,
            at_once,
            announce,
            gathered_for,
            copy_settles,
            underway: Mutex::default(),
        }))
    }

    /// Something was written down for this account: tell its server.
    ///
    /// Answers at once. A push already underway takes note that there is more, and one gathering
    /// takes this with it.
    pub fn please_push(&self, account_id: &str) {
        let mut underway = self.0.underway.lock().expect("the lock is never poisoned");

        if let Some(more) = underway.get_mut(account_id) {
            *more = true;
            return;
        }

        underway.insert(account_id.to_owned(), false);
        drop(underway);

        let pushes = self.clone();
        let account_id = account_id.to_owned();
        self.0
            .runtime
            .spawn(async move { pushes.pushing(account_id).await });
    }

    /// Pushes until nothing more has been asked for about this account, then reads back once
    /// whatever the whole burst moved.
    async fn pushing(self, account_id: String) {
        let mut moved = false;
        let mut went_out = false;

        loop {
            // What is asked for while this waits is carried by this push, so it is not more.
            tokio::time::sleep(self.0.gathered_for).await;
            self.asked_again(&account_id);

            let pushed = self.once(&account_id).await;
            moved |= pushed.moved;
            went_out |= pushed.sent;

            // Round again, after the same short wait: a round, a watcher or a person waiting for
            // this account gets its turn in between, rather than behind a queue of any length.
            if pushed.more || self.asked_again(&account_id) {
                continue;
            }

            // What went out is filed once its server has had its moment, so a person looking in
            // Sent finds it seconds after it went rather than a round later. Anything asked for
            // meanwhile is pushed first, and a message written then does not wait for the copy.
            if went_out {
                if !self.copy_settled(&account_id).await {
                    continue;
                }
                went_out = false;
                moved |= self.file_copies(&account_id).await;
            }

            // The queue is as empty as this burst will make it. What it moved is read back now,
            // once, and anything that was waiting for exactly that read goes straight after it.
            if std::mem::take(&mut moved) && self.read_back(&account_id).await {
                continue;
            }

            let mut underway = self.0.underway.lock().expect("the lock is never poisoned");
            if underway.get(&account_id).copied().unwrap_or(false) {
                continue;
            }
            underway.remove(&account_id);
            return;
        }
    }

    /// Waits out the moment a copy's server is given, and answers whether it ran its course: more
    /// asked for about this account ends it early, to be pushed first.
    async fn copy_settled(&self, account_id: &str) -> bool {
        let until = tokio::time::Instant::now() + self.0.copy_settles;
        loop {
            let left = until.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return true;
            }
            tokio::time::sleep(left.min(self.0.gathered_for)).await;
            if self.asked_again(account_id) {
                return false;
            }
        }
    }

    /// Whether more was asked for about this account since the last look, and forgets that it was.
    fn asked_again(&self, account_id: &str) -> bool {
        self.0
            .underway
            .lock()
            .expect("the lock is never poisoned")
            .get_mut(account_id)
            .map(std::mem::take)
            .unwrap_or(false)
    }

    /// One push, answering with what it did — nothing, when it could not run at all.
    async fn once(&self, account_id: &str) -> Pushed {
        let push = Arc::clone(&self.0.push);
        self.holding(account_id, move |account_id| push.push(account_id))
            .await
            .and_then(Result::ok)
            .unwrap_or_default()
    }

    /// Files the copies of what went out, answering whether any is now on the server to be read
    /// back.
    async fn file_copies(&self, account_id: &str) -> bool {
        let push = Arc::clone(&self.0.push);
        self.holding(account_id, move |account_id| push.file_copies(account_id))
            .await
            .and_then(Result::ok)
            .unwrap_or(false)
    }

    /// Reads the account back after a burst that moved mail, announcing what that changed, and
    /// answers whether it let go of anything waiting for it.
    async fn read_back(&self, account_id: &str) -> bool {
        let push = Arc::clone(&self.0.push);
        let Some(Ok(ReadBack { changed, let_go })) = self
            .holding(account_id, move |account_id| push.read_back(account_id))
            .await
        else {
            // A read that failed leaves what it would have settled owed, laid over the index as
            // before; the round after it reads the account again.
            return false;
        };

        if changed.is_anything() {
            (self.0.announce)(account_id, changed);
        }
        let_go > 0
    }

    /// Runs one piece of blocking work on the account, holding a place and the account for it.
    async fn holding<T: Send + 'static>(
        &self,
        account_id: &str,
        work: impl FnOnce(&str) -> T + Send + 'static,
    ) -> Option<T> {
        let Ok(_permit) = Arc::clone(&self.0.at_once).acquire_owned().await else {
            return None;
        };
        // A push waits for whatever holds the account and is never turned away.
        let claim = self
            .0
            .in_flight
            .claim_waiting(account_id, Why::Push)
            .await?;

        let working = account_id.to_owned();
        self.0
            .runtime
            .spawn_blocking(move || {
                let done = work(&working);
                drop(claim);
                done
            })
            .await
            .ok()
    }
}

#[cfg(test)]
mod tests;
