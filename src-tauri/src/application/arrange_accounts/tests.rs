use std::sync::Mutex;

use super::*;

/// The arrangement, in memory. Hand-written rather than generated: what it has to be is a store
/// that behaves, so that a failing test here is about the use case and not about a mock.
#[derive(Default)]
struct InMemoryGroups {
    groups: Mutex<Vec<AccountGroup>>,
    accounts: Mutex<Vec<String>>,
    /// The last order written for the top of the sidebar.
    arranged: Mutex<Option<Vec<Placed>>>,
    /// What the store answers with, when it is asked to fail.
    refuses: Option<AccountGroupError>,
}

impl InMemoryGroups {
    fn holding(accounts: &[&str]) -> Self {
        Self {
            accounts: Mutex::new(accounts.iter().map(|id| (*id).to_owned()).collect()),
            ..Self::default()
        }
    }

    fn refusing(error: AccountGroupError) -> Self {
        Self {
            refuses: Some(error),
            ..Self::default()
        }
    }

    fn named(&self) -> Vec<String> {
        self.groups
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|group| group.name.as_str().to_owned())
            .collect()
    }
}

impl AccountGroups for InMemoryGroups {
    fn list(&self) -> Result<Vec<AccountGroup>, AccountGroupError> {
        match &self.refuses {
            Some(AccountGroupError::Unavailable) => Err(AccountGroupError::Unavailable),
            Some(AccountGroupError::Corrupted) => Err(AccountGroupError::Corrupted),
            _ => Ok(self.groups.lock().expect("not poisoned").clone()),
        }
    }

    fn create(&self, name: &GroupName) -> Result<String, AccountGroupError> {
        if let Some(AccountGroupError::Unavailable) = &self.refuses {
            return Err(AccountGroupError::Unavailable);
        }

        let mut groups = self.groups.lock().expect("not poisoned");
        let position = groups.len() as i64;
        let id = format!("group-{}", position + 1);

        groups.push(AccountGroup {
            id: id.clone(),
            name: name.clone(),
            position,
            collapsed: false,
            accounts: Vec::new(),
        });

        Ok(id)
    }

    fn rename(&self, group_id: &str, name: &GroupName) -> Result<(), AccountGroupError> {
        let mut groups = self.groups.lock().expect("not poisoned");
        let Some(group) = groups.iter_mut().find(|group| group.id == group_id) else {
            return Err(AccountGroupError::NoSuchGroup);
        };

        group.name = name.clone();
        Ok(())
    }

    fn remove(&self, group_id: &str) -> Result<(), AccountGroupError> {
        let mut groups = self.groups.lock().expect("not poisoned");
        let before = groups.len();
        groups.retain(|group| group.id != group_id);

        if groups.len() == before {
            return Err(AccountGroupError::NoSuchGroup);
        }

        Ok(())
    }

    fn place(&self, account_id: &str, group_id: Option<&str>) -> Result<(), AccountGroupError> {
        if !self
            .accounts
            .lock()
            .expect("not poisoned")
            .iter()
            .any(|held| held == account_id)
        {
            return Err(AccountGroupError::NoSuchAccount);
        }

        let mut groups = self.groups.lock().expect("not poisoned");

        if let Some(group_id) = group_id {
            if !groups.iter().any(|group| group.id == group_id) {
                return Err(AccountGroupError::NoSuchGroup);
            }
        }

        for group in groups.iter_mut() {
            group.accounts.retain(|held| held != account_id);
        }

        if let Some(group_id) = group_id {
            if let Some(group) = groups.iter_mut().find(|group| group.id == group_id) {
                group.accounts.push(account_id.to_owned());
            }
        }

        Ok(())
    }

    fn reorder(&self, account_ids: &[String]) -> Result<(), AccountGroupError> {
        if let Some(AccountGroupError::Unavailable) = &self.refuses {
            return Err(AccountGroupError::Unavailable);
        }

        let mut accounts = self.accounts.lock().expect("not poisoned");
        if let Some(stranger) = account_ids.iter().find(|id| !accounts.contains(id)) {
            let _ = stranger;
            return Err(AccountGroupError::NoSuchAccount);
        }

        // The named ones in the order given, then everything the caller said nothing about, in
        // the order it already had. A store that dropped the rest would lose an account because
        // somebody dragged a row in a different group.
        let mut ordered = account_ids.to_vec();
        ordered.extend(
            accounts
                .iter()
                .filter(|id| !account_ids.contains(id))
                .cloned(),
        );
        *accounts = ordered;

        Ok(())
    }

    fn arrange(&self, order: &[Placed]) -> Result<(), AccountGroupError> {
        if let Some(AccountGroupError::Unavailable) = &self.refuses {
            return Err(AccountGroupError::Unavailable);
        }

        *self.arranged.lock().expect("not poisoned") = Some(order.to_vec());
        Ok(())
    }

    fn set_collapsed(&self, group_id: &str, collapsed: bool) -> Result<(), AccountGroupError> {
        let mut groups = self.groups.lock().expect("not poisoned");
        let Some(group) = groups.iter_mut().find(|group| group.id == group_id) else {
            return Err(AccountGroupError::NoSuchGroup);
        };

        group.collapsed = collapsed;
        Ok(())
    }
}

#[test]
fn a_group_needs_a_name_and_nothing_is_written_without_one() {
    let store = InMemoryGroups::default();
    let arrange = ArrangeAccounts::new(store);

    assert!(matches!(
        arrange.create("   "),
        Err(UseCaseError::GroupNameRequired)
    ));
    assert_eq!(
        arrange.list().expect("readable").len(),
        0,
        "a refusal writes nothing"
    );
}

#[test]
fn a_name_too_long_is_refused_by_its_own_name() {
    let arrange = ArrangeAccounts::new(InMemoryGroups::default());

    // Its own variant rather than one refusal for both: the screen says what to change, and
    // "a group needs a name" is the wrong sentence in front of somebody who typed too much of one.
    assert!(matches!(
        arrange.create(&"a".repeat(65)),
        Err(UseCaseError::GroupNameTooLong)
    ));
}

#[test]
fn the_name_that_is_kept_is_the_one_without_the_space_around_it() {
    let store = InMemoryGroups::default();
    let arrange = ArrangeAccounts::new(store);

    arrange
        .create("  Work  ")
        .expect("a name with space is a name");

    let groups = arrange.list().expect("readable");
    assert_eq!(groups[0].name.as_str(), "Work");
}

#[test]
fn renaming_a_group_that_is_gone_says_which_thing_was_missing() {
    let arrange = ArrangeAccounts::new(InMemoryGroups::default());

    // A group deleted in another window. The screen has something to say about that, and it is
    // not the sentence about a mailbox that is gone.
    assert!(matches!(
        arrange.rename("group-1", "Work"),
        Err(UseCaseError::UnknownGroup)
    ));
}

#[test]
fn a_missing_group_and_a_missing_account_are_two_different_answers() {
    let arrange = ArrangeAccounts::new(InMemoryGroups::holding(&["acc-a"]));
    let group = arrange.create("Work").expect("a group");

    assert!(matches!(
        arrange.place("acc-a", Some("group-does-not-exist")),
        Err(UseCaseError::UnknownGroup)
    ));
    assert!(matches!(
        arrange.place("acc-gone", Some(&group)),
        Err(UseCaseError::UnknownAccount)
    ));
}

#[test]
fn moving_an_account_leaves_the_group_it_came_from() {
    let arrange = ArrangeAccounts::new(InMemoryGroups::holding(&["acc-a"]));
    let work = arrange.create("Work").expect("a group");
    let personal = arrange.create("Personal").expect("another group");

    arrange.place("acc-a", Some(&work)).expect("joins");
    arrange.place("acc-a", Some(&personal)).expect("moves");

    let holding: Vec<String> = arrange
        .list()
        .expect("readable")
        .iter()
        .filter(|group| !group.accounts.is_empty())
        .map(|group| group.name.as_str().to_owned())
        .collect();

    assert_eq!(holding, vec!["Personal".to_owned()]);
}

#[test]
fn a_store_that_will_not_open_is_reported_as_such_rather_than_as_an_empty_arrangement() {
    let arrange = ArrangeAccounts::new(InMemoryGroups::refusing(AccountGroupError::Unavailable));

    // An empty list would be a sidebar that quietly forgot everything a person arranged.
    assert!(matches!(
        arrange.list(),
        Err(UseCaseError::StorageUnavailable)
    ));
}

#[test]
fn a_row_this_build_cannot_read_is_reported_as_corrupted() {
    let arrange = ArrangeAccounts::new(InMemoryGroups::refusing(AccountGroupError::Corrupted));

    assert!(matches!(
        arrange.list(),
        Err(UseCaseError::StorageCorrupted)
    ));
}

#[test]
fn the_store_is_only_asked_for_names_that_passed_the_rule() {
    let store = std::sync::Arc::new(InMemoryGroups::default());

    // Checked against the store rather than against the answer: a rule enforced after the write
    // is not a rule, and the answer alone cannot tell the two apart.
    let arrange = ArrangeAccounts::new(std::sync::Arc::clone(&store));
    arrange.create("").ok();
    arrange.create("  ").ok();
    arrange.create("Work").expect("a real name");

    assert_eq!(store.named(), vec!["Work".to_owned()]);
}

/// Shared so a test can hold the store and hand a handle to the use case at the same time.
///
/// Delegating through the `Arc` rather than through a reference: an implementation on `&T` would
/// resolve to itself and recurse for ever, and it would compile.
impl AccountGroups for std::sync::Arc<InMemoryGroups> {
    fn list(&self) -> Result<Vec<AccountGroup>, AccountGroupError> {
        InMemoryGroups::list(self)
    }

    fn create(&self, name: &GroupName) -> Result<String, AccountGroupError> {
        InMemoryGroups::create(self, name)
    }

    fn rename(&self, group_id: &str, name: &GroupName) -> Result<(), AccountGroupError> {
        InMemoryGroups::rename(self, group_id, name)
    }

    fn remove(&self, group_id: &str) -> Result<(), AccountGroupError> {
        InMemoryGroups::remove(self, group_id)
    }

    fn place(&self, account_id: &str, group_id: Option<&str>) -> Result<(), AccountGroupError> {
        InMemoryGroups::place(self, account_id, group_id)
    }

    fn reorder(&self, account_ids: &[String]) -> Result<(), AccountGroupError> {
        InMemoryGroups::reorder(self, account_ids)
    }

    fn arrange(&self, order: &[Placed]) -> Result<(), AccountGroupError> {
        InMemoryGroups::arrange(self, order)
    }

    fn set_collapsed(&self, group_id: &str, collapsed: bool) -> Result<(), AccountGroupError> {
        InMemoryGroups::set_collapsed(self, group_id, collapsed)
    }
}

#[test]
fn the_order_somebody_dragged_into_is_written_down() {
    let store = std::sync::Arc::new(InMemoryGroups::holding(&["acc-1", "acc-2", "acc-3"]));
    let arrange = ArrangeAccounts::new(std::sync::Arc::clone(&store));

    arrange
        .reorder(&["acc-3".to_owned(), "acc-1".to_owned(), "acc-2".to_owned()])
        .expect("written");

    assert_eq!(
        *store.accounts.lock().expect("not poisoned"),
        vec!["acc-3".to_owned(), "acc-1".to_owned(), "acc-2".to_owned()]
    );
}

#[test]
fn a_drag_that_ended_where_it_started_is_nothing_to_do() {
    // The window should not have to work out whether a gesture changed anything before it is
    // allowed to report it, and a store asked to order nothing is a store asked a question with
    // no answer.
    let store = std::sync::Arc::new(InMemoryGroups::refusing(AccountGroupError::Unavailable));
    let arrange = ArrangeAccounts::new(std::sync::Arc::clone(&store));

    arrange.reorder(&[]).expect("nothing to do, and no failure");
}

#[test]
fn an_order_naming_a_mailbox_this_device_does_not_hold_is_refused() {
    // Not skipped. The list is the order, so an entry that cannot be placed leaves every account
    // after it at a number that means something else.
    let store = InMemoryGroups::holding(&["acc-1"]);
    let arrange = ArrangeAccounts::new(store);

    assert!(matches!(
        arrange.reorder(&["acc-1".to_owned(), "gone".to_owned()]),
        Err(UseCaseError::UnknownAccount)
    ));
}

#[test]
fn ordering_one_group_leaves_the_accounts_it_did_not_name_alone() {
    // A person drags a row inside one group. Every mailbox in every other group stays where it
    // was, and none of them disappears because it was not mentioned.
    let store = std::sync::Arc::new(InMemoryGroups::holding(&["acc-1", "acc-2", "acc-3"]));
    let arrange = ArrangeAccounts::new(std::sync::Arc::clone(&store));

    arrange
        .reorder(&["acc-2".to_owned(), "acc-1".to_owned()])
        .expect("written");

    let held = store.accounts.lock().expect("not poisoned");
    assert_eq!(held.len(), 3, "nothing was lost");
    assert!(held.contains(&"acc-3".to_owned()));
}

#[test]
fn the_order_of_the_top_of_the_sidebar_reaches_the_store_as_it_was_given() {
    let store = std::sync::Arc::new(InMemoryGroups::holding(&["acc-1"]));
    let arrange = ArrangeAccounts::new(std::sync::Arc::clone(&store));
    let order = vec![
        Placed::Account("acc-1".to_owned()),
        Placed::Group("group-1".to_owned()),
    ];

    arrange.arrange(&order).expect("written");

    assert_eq!(*store.arranged.lock().expect("not poisoned"), Some(order));
}

#[test]
fn an_empty_order_for_the_top_asks_the_store_nothing() {
    let store = std::sync::Arc::new(InMemoryGroups {
        refuses: Some(AccountGroupError::Unavailable),
        ..InMemoryGroups::default()
    });
    let arrange = ArrangeAccounts::new(std::sync::Arc::clone(&store));

    arrange
        .arrange(&[])
        .expect("a drag that ended where it started is not a failure");
    assert_eq!(*store.arranged.lock().expect("not poisoned"), None);
}
