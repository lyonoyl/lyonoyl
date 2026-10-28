use thiserror::Error;

use lyonoyl_domain::account_group::{AccountGroup, GroupName, Placed};

/// Failures the store of account groups can report.
///
/// Free of adapter types: the adapter maps its own errors into these variants.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AccountGroupError {
    /// The store cannot be reached: the file is locked, missing or corrupted.
    #[error("the local account store is unavailable")]
    Unavailable,
    /// A stored row violates a domain invariant — usually written by another version of the app.
    #[error("a stored group is invalid")]
    Corrupted,
    /// Named a group this device does not hold.
    ///
    /// Its own variant rather than a silent no-op: a rename that quietly did nothing is a screen
    /// that shows the old name back and no reason why.
    #[error("no such group")]
    NoSuchGroup,
    /// Named an account this device does not hold.
    #[error("no such account")]
    NoSuchAccount,
}

/// Read and write access to the arrangement of accounts in the sidebar.
///
/// The mail index never hears about any of this: a group is a name over a set of accounts, and
/// moving an account between groups moves no message at all.
pub trait AccountGroups: Send + Sync {
    /// Every group, in the order they are drawn, each with its accounts in theirs.
    fn list(&self) -> Result<Vec<AccountGroup>, AccountGroupError>;

    /// Makes a group and returns the identifier it was given. It starts empty, and last at the
    /// top of the sidebar.
    fn create(&self, name: &GroupName) -> Result<String, AccountGroupError>;

    fn rename(&self, group_id: &str, name: &GroupName) -> Result<(), AccountGroupError>;

    /// Removes the group. The accounts in it take its place at the top of the sidebar, in the
    /// order they had in it.
    fn remove(&self, group_id: &str) -> Result<(), AccountGroupError>;

    /// Puts an account in a group, last in it, or — with `None` — at the top of the sidebar,
    /// directly below the group it was in. An account already in no group stays where it is.
    ///
    /// One call rather than add-and-remove, because an account is in at most one group: there is
    /// no state between the two where it is in both, and none where it is in neither by accident.
    fn place(&self, account_id: &str, group_id: Option<&str>) -> Result<(), AccountGroupError>;

    /// Writes down the order a person dragged accounts into.
    ///
    /// The whole run in the order it should be drawn, not "move this one up". An instruction that
    /// names a place has to agree with what is already stored about every other account, and two
    /// windows dragging at once would each be right about a list neither of them has. A list is
    /// the answer itself: whatever was there before, this is the order now.
    ///
    /// It moves nothing between groups. Where an account belongs is [`AccountGroups::place`], and
    /// an order that also re-homed accounts would be two decisions behind one gesture.
    fn reorder(&self, account_ids: &[String]) -> Result<(), AccountGroupError>;

    /// Writes down the order of everything at the top of the sidebar: the groups and the accounts
    /// in none, as one run.
    ///
    /// The whole run, for the reason [`AccountGroups::reorder`] takes one. Anything at the top that
    /// the run does not name — an account added in the meantime — goes after it, in the order it
    /// had. An account named here that is in a group is refused as not being there: inside a group
    /// is not the top of the sidebar, and moving it out is [`AccountGroups::place`].
    fn arrange(&self, order: &[Placed]) -> Result<(), AccountGroupError>;

    /// Folds a group away, or opens it again.
    ///
    /// Kept beside the group rather than on this machine: somebody who folds away the twelve
    /// mailboxes they read once a month has said so about the group, not about this desk.
    fn set_collapsed(&self, group_id: &str, collapsed: bool) -> Result<(), AccountGroupError>;
}
