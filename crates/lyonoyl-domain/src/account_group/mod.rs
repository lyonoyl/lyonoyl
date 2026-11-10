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
    /// How the accounts in it are ordered.
    pub sorting: Sorting,
    /// The accounts in it, in the order they are drawn.
    pub accounts: Vec<String>,
}

/// How the mailboxes in a group are ordered.
///
/// By hand is the order a person dragged them into. The others are worked out each time the group
/// is read, so a mailbox that joins a sorted group takes its place by the same rule as the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sorting {
    #[default]
    Manual,
    /// The mailbox added to Lyonoyl Mail first stands first.
    DateAdded,
    /// By the name the sidebar draws it under, whatever its case or its accents.
    Name,
}

/// What a sorting reads about a mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub id: String,
    /// The name the sidebar draws it under.
    pub label: String,
    /// The name as it compares: `label` with its case and accents taken off, so that "école"
    /// stands among the names in e rather than after every one in z.
    pub folded: String,
    /// When it was added, written so that text order is time order.
    pub added: String,
}

impl Member {
    /// A mailbox is drawn under the name its owner gave it, or under its address where that name
    /// is blank, and a sort by name has to read the same word the person reads.
    ///
    /// `fold` takes the case and the accents off a name. It is handed in because the tables that
    /// say what an accent is, in every alphabet, are not this crate's to carry.
    pub fn new(
        id: String,
        display_name: &str,
        email: &str,
        added: String,
        fold: impl Fn(&str) -> String,
    ) -> Self {
        let name = display_name.trim();
        let label = if name.is_empty() { email } else { name };

        Self {
            id,
            label: label.to_owned(),
            folded: fold(label),
            added,
        }
    }
}

/// One way of sorting: the key it reads off a mailbox, whose own order is how two compare.
///
/// A new way of sorting is a key and one more entry in [`sorted`]; nothing else changes.
trait SortKey {
    type Key: Ord;

    fn key(member: &Member) -> Self::Key;
}

struct DateAdded;

impl SortKey for DateAdded {
    type Key = String;

    fn key(member: &Member) -> String {
        member.added.clone()
    }
}

struct Name;

impl SortKey for Name {
    type Key = String;

    fn key(member: &Member) -> String {
        member.folded.clone()
    }
}

/// The members' identifiers in the order `sorting` draws them.
///
/// The members arrive in the order they were put in by hand, and a sort that finds two equal keeps
/// them in it: two mailboxes under one name stand as somebody left them.
pub fn sorted(members: Vec<Member>, sorting: Sorting) -> Vec<String> {
    match sorting {
        Sorting::Manual => identifiers(members),
        Sorting::DateAdded => sorted_by::<DateAdded>(members),
        Sorting::Name => sorted_by::<Name>(members),
    }
}

fn sorted_by<K: SortKey>(mut members: Vec<Member>) -> Vec<String> {
    // Stable: the key is read once for each member and ties keep the order they came in.
    members.sort_by_cached_key(K::key);
    identifiers(members)
}

fn identifiers(members: Vec<Member>) -> Vec<String> {
    members.into_iter().map(|member| member.id).collect()
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
