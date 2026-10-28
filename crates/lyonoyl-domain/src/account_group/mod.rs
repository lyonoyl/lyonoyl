//! Groups of accounts, which is what a person with fifty of them arranges the sidebar by.
//!
//! A group is a name and an ordered set of accounts. It is an arrangement and nothing more: mail
//! belongs to an account, never to a group, so removing a group is a change to the sidebar and not
//! to anybody's mail.
//!
//! **An account is in at most one group.** That is the same rule the mail follows one level down —
//! a message belongs to one account so that "where is this" has one answer — and it is what makes
//! this folders rather than labels. An account in two groups would be drawn twice in a sidebar
//! whose whole purpose is to say where things are.

use thiserror::Error;

/// The longest name a group may carry.
///
/// The sidebar is a column a few words wide, and a name is read there and nowhere else. The limit
/// is counted in characters rather than bytes: a name in Russian or Greek would otherwise be worth
/// half of one in English, which is not a rule anybody could have guessed from looking at it.
pub const LONGEST_NAME: usize = 64;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GroupNameError {
    #[error("a group name is required")]
    Empty,
    #[error("a group name may hold at most {LONGEST_NAME} characters")]
    TooLong,
}

/// A name that can be drawn in a sidebar and found again.
///
/// Surrounding whitespace is removed rather than refused: somebody who pastes a name with a space
/// on the end meant the name, and a group called `"Work "` sitting beside one called `"Work"` is a
/// pair nobody can tell apart.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GroupName(String);

impl GroupName {
    pub fn new(typed: &str) -> Result<Self, GroupNameError> {
        let trimmed = typed.trim();

        if trimmed.is_empty() {
            return Err(GroupNameError::Empty);
        }

        if trimmed.chars().count() > LONGEST_NAME {
            return Err(GroupNameError::TooLong);
        }

        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<GroupName> for String {
    fn from(name: GroupName) -> Self {
        name.0
    }
}

/// A group as it is kept: a name, and the accounts in it in the order they are drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountGroup {
    pub id: String,
    pub name: GroupName,
    /// Where the group sits at the top of the sidebar, among the other groups and the accounts in
    /// none. Lower is higher up.
    pub position: i64,
    /// Folded away or open. Kept beside the group rather than on this machine: somebody who folds
    /// away the twelve mailboxes they read once a month has said so about the group, and being
    /// asked again on the next device is the application forgetting on their behalf.
    pub collapsed: bool,
    /// The accounts in it, in the order they are drawn.
    pub accounts: Vec<String>,
}

/// One thing at the top of the sidebar: a group, or an account in no group.
///
/// The two share one order there. Groups are not drawn above every account outside them, nor
/// below: where each one sits is written down, as the order inside a group is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placed {
    Group(String),
    Account(String),
}

/// Whether a set of accounts is worth reading as one list.
///
/// A group holding nothing has no mail to merge, and asking every file for a window that must come
/// back empty is work with a known answer. The caller shows the group and its emptiness instead.
pub fn worth_reading(accounts: &[String]) -> bool {
    !accounts.is_empty()
}

#[cfg(test)]
mod tests;
