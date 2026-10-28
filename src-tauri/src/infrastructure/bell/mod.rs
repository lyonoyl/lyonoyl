//! One bell for a hundred mailboxes.
//!
//! Rounds finish four accounts at a time and a hundred accounts finish all afternoon. Ringing on
//! each is a machine that pings until somebody turns it off — and turning it off is the outcome
//! this whole feature is trying to avoid.
//!
//! So arrivals are gathered rather than announced. A first arrival opens a short wave; everything
//! that lands during it joins in; when the wave closes the bell asks once, about the whole of it.
//! And after it has rung it stays quiet for a while, however much arrives — because the second
//! interruption in a minute tells a person nothing the first did not.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a wave gathers before the bell asks.
///
/// Ten seconds because a round reads four accounts at a time and a hundred accounts take several
/// rounds: a wave shorter than the gap between them would ring once per round rather than once per
/// wave, which is the thing being prevented.
pub const A_WAVE: Duration = Duration::from_secs(10);

/// The quiet after a ring.
pub const AND_THEN_QUIET: Duration = Duration::from_secs(60);

/// What the bell was told, and when it may next speak.
struct Waiting {
    /// One entry per account, so the same account reporting twice in a wave is counted once for
    /// each pass rather than added to itself for ever.
    unseen: HashMap<String, u32>,
    /// When the wave opened, or nothing while none is open.
    since: Option<Instant>,
    /// When the bell last rang.
    last: Option<Instant>,
}

/// Gathers arrivals and decides when a wave is finished.
///
/// Holds no notifier and no settings: it answers *when*, and the use case answers *whether*. Two
/// questions in one object would be a clock that knew about somebody's preferences.
#[derive(Clone)]
pub struct Bell {
    waiting: Arc<Mutex<Waiting>>,
    wave: Duration,
    quiet: Duration,
}

impl Bell {
    /// The durations are arguments so a test drives the real object at twenty milliseconds rather
    /// than a copy of it at ten seconds.
    pub fn ringing_after(wave: Duration, quiet: Duration) -> Self {
        Self {
            waiting: Arc::new(Mutex::new(Waiting {
                unseen: HashMap::new(),
                since: None,
                last: None,
            })),
            wave,
            quiet,
        }
    }

    /// What one account's pass found. Opens a wave if none is open.
    pub fn reports(&self, account_id: &str, unseen: u32, now: Instant) {
        if unseen == 0 {
            return;
        }

        let mut waiting = self.waiting.lock().expect("not poisoned");
        waiting.unseen.insert(account_id.to_owned(), unseen);
        waiting.since.get_or_insert(now);
    }

    /// How much to announce, or nothing.
    ///
    /// Called on a timer. Answers `Some` exactly once per wave, and only when the wave has had its
    /// time and the quiet after the last ring has passed.
    ///
    /// A wave that is refused because the bell is still quiet is **emptied**, not deferred: mail
    /// that arrived a minute ago is not news a minute later, and holding it would ring about it
    /// the moment the quiet ends.
    pub fn what_to_say(&self, now: Instant) -> Option<u32> {
        let mut waiting = self.waiting.lock().expect("not poisoned");

        let since = waiting.since?;
        if now.duration_since(since) < self.wave {
            return None;
        }

        let unseen: u32 = waiting.unseen.values().copied().sum();
        waiting.unseen.clear();
        waiting.since = None;

        let too_soon = waiting
            .last
            .is_some_and(|last| now.duration_since(last) < self.quiet);

        if too_soon || unseen == 0 {
            return None;
        }

        waiting.last = Some(now);
        Some(unseen)
    }
}

impl Default for Bell {
    fn default() -> Self {
        Self::ringing_after(A_WAVE, AND_THEN_QUIET)
    }
}

#[cfg(test)]
mod tests;
