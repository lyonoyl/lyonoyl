use std::collections::HashMap;

use super::*;
use crate::ports::account_repository::AccountRepositoryError;
use lyonoyl_domain::account::{MailAccount, MailAccountSummary};

/// Accounts and their numbers, in memory.
struct FakeAccounts {
    rows: Vec<(&'static str, &'static str, i64)>,
}

impl AccountRepository for FakeAccounts {
    fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
        Ok(self
            .rows
            .iter()
            .map(|(id, email, _)| {
                MailAccountSummary::new(*id, *email, "", "imap").expect("well-formed")
            })
            .collect())
    }

    fn number_of(&self, account_id: &str) -> Result<Option<i64>, AccountRepositoryError> {
        Ok(self
            .rows
            .iter()
            .find(|(id, _, _)| *id == account_id)
            .map(|(_, _, number)| *number))
    }

    fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
        Ok(Vec::new())
    }

    fn find(&self, _account_id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
        Ok(None)
    }

    fn find_by_email(&self, _email: &str) -> Result<Option<String>, AccountRepositoryError> {
        Ok(None)
    }

    fn save(&self, _account: &MailAccount) -> Result<(), AccountRepositoryError> {
        Ok(())
    }

    fn delete(&self, _account_id: &str) -> Result<(), AccountRepositoryError> {
        Ok(())
    }

    fn set_collapsed(
        &self,
        _account_id: &str,
        _collapsed: bool,
    ) -> Result<(), AccountRepositoryError> {
        Ok(())
    }
}

/// Files whose sizes are given, and which may refuse one of them.
struct FakeFiles {
    bytes: HashMap<i64, u64>,
    refuses: Option<i64>,
}

impl MailFiles for FakeFiles {
    fn remove(&self, _account: i64) -> Result<(), crate::ports::mail_files::MailFilesError> {
        Ok(())
    }

    fn held_by(&self, account: i64) -> Result<u64, crate::ports::mail_files::MailFilesError> {
        if self.refuses == Some(account) {
            return Err(crate::ports::mail_files::MailFilesError::Unavailable);
        }

        Ok(self.bytes.get(&account).copied().unwrap_or(0))
    }
}

fn held(
    rows: Vec<(&'static str, &'static str, i64)>,
    bytes: &[(i64, u64)],
    refuses: Option<i64>,
) -> Vec<Held> {
    WhatIsHeld::new(
        FakeAccounts { rows },
        FakeFiles {
            bytes: bytes.iter().copied().collect(),
            refuses,
        },
    )
    .execute()
    .expect("readable")
}

#[test]
fn the_largest_is_first_because_that_is_the_question_being_asked() {
    // The screen exists to answer "what is taking the room". Ordered by name, somebody with fifty
    // mailboxes reads fifty rows to find out.
    let answer = held(
        vec![
            ("acc-a", "a@example.com", 1),
            ("acc-b", "b@example.com", 2),
            ("acc-c", "c@example.com", 3),
        ],
        &[(1, 10), (2, 900), (3, 400)],
        None,
    );

    assert_eq!(
        answer.iter().map(|one| one.bytes).collect::<Vec<_>>(),
        vec![900, 400, 10]
    );
}

#[test]
fn an_account_that_never_synchronised_holds_nothing_rather_than_failing() {
    // No file at all is the ordinary state of an account added a moment ago. Zero is what is on
    // the disk, and it is an answer.
    let answer = held(vec![("acc-a", "a@example.com", 1)], &[], None);

    assert_eq!(answer[0].bytes, 0);
    assert_eq!(answer[0].account_id, "acc-a");
}

#[test]
fn one_file_that_cannot_be_measured_does_not_hide_the_others() {
    // The whole point of a file per account is that one failing is one account's trouble. A list
    // that refused to draw would make it everybody's.
    let answer = held(
        vec![("acc-a", "a@example.com", 1), ("acc-b", "b@example.com", 2)],
        &[(1, 10), (2, 900)],
        Some(2),
    );

    assert_eq!(answer.len(), 2);
    assert_eq!(
        answer
            .iter()
            .find(|one| one.account_id == "acc-a")
            .map(|one| one.bytes),
        Some(10)
    );
    assert_eq!(
        answer
            .iter()
            .find(|one| one.account_id == "acc-b")
            .map(|one| one.bytes),
        Some(0),
        "unmeasurable reads as nothing, and the row is still drawn"
    );
}

#[test]
fn two_accounts_holding_the_same_are_ordered_by_address() {
    // Otherwise the order is whatever the store happened to return, and a screen that reorders
    // itself between two visits is one nobody trusts.
    let answer = held(
        vec![("acc-z", "z@example.com", 1), ("acc-a", "a@example.com", 2)],
        &[(1, 500), (2, 500)],
        None,
    );

    assert_eq!(
        answer
            .iter()
            .map(|one| one.email.as_str())
            .collect::<Vec<_>>(),
        vec!["a@example.com", "z@example.com"]
    );
}
