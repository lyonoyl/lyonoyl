//! Arranging accounts into groups, which is how a sidebar of fifty mailboxes stays readable.
//!
//! A group is a name over a set of accounts and nothing else. No message moves, no mail file is
//! opened, and emptying every group leaves a person with exactly the mailboxes they had.
//!
//! What the use case adds over the store beneath it is the naming rule and the two refusals a
//! screen has to tell apart: a group somebody deleted in another window, and a mailbox that is
//! gone. Both would otherwise arrive as one error from the database.

use crate::application::error::UseCaseError;
use crate::ports::account_groups::{AccountGroupError, AccountGroups};
use lyonoyl_domain::account_group::{AccountGroup, GroupName, GroupNameError, Placed};

/// Makes, names and empties the groups in the sidebar.
pub struct ArrangeAccounts<G: AccountGroups> {
    groups: G,
}

impl<G: AccountGroups> ArrangeAccounts<G> {
    pub fn new(groups: G) -> Self {
        Self { groups }
    }

    /// Every group, in the order they are drawn, each with its accounts in theirs.
    pub fn list(&self) -> Result<Vec<AccountGroup>, UseCaseError> {
        Ok(self.groups.list()?)
    }

    /// Makes a group and answers with the identifier it was given.
    pub fn create(&self, typed: &str) -> Result<String, UseCaseError> {
        Ok(self.groups.create(&named(typed)?)?)
    }

    pub fn rename(&self, group_id: &str, typed: &str) -> Result<(), UseCaseError> {
        Ok(self.groups.rename(group_id, &named(typed)?)?)
    }

    /// Removes the group. The accounts in it take its place at the top of the sidebar.
    pub fn remove(&self, group_id: &str) -> Result<(), UseCaseError> {
        Ok(self.groups.remove(group_id)?)
    }

    /// Puts an account in a group, or — with `None` — at the top of the sidebar, below the group
    /// it was in.
    pub fn place(&self, account_id: &str, group_id: Option<&str>) -> Result<(), UseCaseError> {
        Ok(self.groups.place(account_id, group_id)?)
    }

    /// Writes down the order a person dragged accounts into.
    ///
    /// An empty list is nothing to do rather than a refusal: a drag that ended where it started
    /// changes no order, and the window should not have to work out whether it is worth saying so.
    pub fn reorder(&self, account_ids: &[String]) -> Result<(), UseCaseError> {
        if account_ids.is_empty() {
            return Ok(());
        }

        Ok(self.groups.reorder(account_ids)?)
    }

    /// Writes down the order of everything at the top of the sidebar: groups and the accounts in
    /// no group, as one run. Empty is nothing to do, for the reason it is in [`Self::reorder`].
    pub fn arrange(&self, order: &[Placed]) -> Result<(), UseCaseError> {
        if order.is_empty() {
            return Ok(());
        }

        Ok(self.groups.arrange(order)?)
    }

    /// Folds a group away, or opens it again.
    pub fn set_collapsed(&self, group_id: &str, collapsed: bool) -> Result<(), UseCaseError> {
        Ok(self.groups.set_collapsed(group_id, collapsed)?)
    }
}

/// The typed name, or the reason it is not one.
fn named(typed: &str) -> Result<GroupName, UseCaseError> {
    GroupName::new(typed).map_err(|error| match error {
        GroupNameError::Empty => UseCaseError::GroupNameRequired,
        GroupNameError::TooLong => UseCaseError::GroupNameTooLong,
    })
}

impl From<AccountGroupError> for UseCaseError {
    fn from(error: AccountGroupError) -> Self {
        match error {
            AccountGroupError::Unavailable => Self::StorageUnavailable,
            AccountGroupError::Corrupted => Self::StorageCorrupted,
            AccountGroupError::NoSuchGroup => Self::UnknownGroup,
            AccountGroupError::NoSuchAccount => Self::UnknownAccount,
        }
    }
}

#[cfg(test)]
mod tests;
