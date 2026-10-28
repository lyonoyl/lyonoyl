//! Whether an account's mail is current, and what stopped it being so.
//!
//! One answer per account, replaced each pass. Not a history: a person looking at their mailbox
//! wants one sentence — this is up to date, or it is not and here is why — and a log of every pass
//! is a different thing that answers a different question.

use thiserror::Error;

/// Why the last pass did not finish, in the terms a person can act on.
///
/// Four, because each leads somewhere different: wait for the network, sign in again, deal with a
/// database that will not open, or report something nobody expected. The failures underneath are
/// more various than this, and collapsing them here is deliberate: "the server sent a malformed
/// FETCH response" is true and useless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhyItStopped {
    Offline,
    SignIn,
    Storage,
    Other,
}

impl WhyItStopped {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::SignIn => "signIn",
            Self::Storage => "storage",
            Self::Other => "other",
        }
    }

    /// Reads a stored value. Unknown means a newer build wrote a reason this one has no words for,
    /// and "something else" is the honest answer rather than a guess.
    pub fn parse(value: &str) -> Self {
        match value {
            "offline" => Self::Offline,
            "signIn" => Self::SignIn,
            "storage" => Self::Storage,
            _ => Self::Other,
        }
    }
}

/// How an account is doing.
///
/// The shape the store keeps per account. What the interface is shown is a sentence about the
/// whole workspace, built in `read_model::activity` — so this is what a test reads back to check
/// what was written.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "read back by the tests that check what a pass wrote"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Activity {
    /// Unix seconds of the last pass that finished. `None` before the first one ever does.
    pub last_success_at: Option<i64>,
    /// Why the most recent pass did not finish. `None` when the last one did.
    pub stopped_by: Option<WhyItStopped>,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum ActivityError {
    #[error("the profile database is unavailable")]
    Unavailable,
}

pub trait AccountActivity: Send + Sync {
    /// Records that a pass finished, at that moment.
    ///
    /// Clears whatever stopped the last one: a pass that worked is the answer to the question,
    /// and an old reason left beside a fresh success would be a sentence that contradicts itself.
    fn finished(&self, account_id: &str, at: i64) -> Result<(), ActivityError>;

    /// Records why a pass did not finish, keeping when one last did.
    ///
    /// The two together are the whole of what a person needs: it is not working *and* this is how
    /// stale what they are looking at has become.
    fn stopped(&self, account_id: &str, why: WhyItStopped) -> Result<(), ActivityError>;
}

impl<A: AccountActivity + ?Sized> AccountActivity for &A {
    fn finished(&self, account_id: &str, at: i64) -> Result<(), ActivityError> {
        (**self).finished(account_id, at)
    }

    fn stopped(&self, account_id: &str, why: WhyItStopped) -> Result<(), ActivityError> {
        (**self).stopped(account_id, why)
    }
}
