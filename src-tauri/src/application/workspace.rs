use crate::application::error::UseCaseError;
use crate::ports::account_repository::AccountRepository;
use lyonoyl_domain::account::MailAccountSummary;

/// What the shell needs to render the workspace before any mailbox is opened.
///
/// Deriving `Debug` is safe here: the accounts inside redact their own addresses.
#[derive(Debug)]
pub struct WorkspaceOverview {
    pub accounts: Vec<MailAccountSummary>,
    /// Which of them somebody has told where to send from. A message written from one that is not
    /// here would wait in the outbox for ever with nothing said.
    pub able_to_send: Vec<String>,
    pub is_local_first: bool,
}

/// Reads the workspace overview.
///
/// Dependencies arrive through a port and are injected by the constructor; tests substitute an
/// in-memory fake.
pub struct GetWorkspaceOverview<R: AccountRepository> {
    accounts: R,
}

impl<R: AccountRepository> GetWorkspaceOverview<R> {
    pub fn new(accounts: R) -> Self {
        Self { accounts }
    }

    pub fn execute(&self) -> Result<WorkspaceOverview, UseCaseError> {
        Ok(WorkspaceOverview {
            accounts: self.accounts.list()?,
            able_to_send: self.accounts.able_to_send()?,
            is_local_first: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::ports::account_repository::AccountRepositoryError;

    /// Hand-written fake rather than a mocking framework; validated against the real adapter by
    /// a contract test.
    #[derive(Default)]
    struct FakeAccounts {
        stored: Mutex<Vec<MailAccountSummary>>,
        sending: Vec<String>,
        failure: Option<AccountRepositoryError>,
    }

    impl FakeAccounts {
        fn with(accounts: Vec<MailAccountSummary>) -> Self {
            Self {
                stored: Mutex::new(accounts),
                sending: Vec::new(),
                failure: None,
            }
        }

        fn failing(error: AccountRepositoryError) -> Self {
            Self {
                stored: Mutex::new(Vec::new()),
                sending: Vec::new(),
                failure: Some(error),
            }
        }
    }

    impl AccountRepository for FakeAccounts {
        fn find(
            &self,
            _account_id: &str,
        ) -> Result<Option<lyonoyl_domain::account::MailAccount>, AccountRepositoryError> {
            unreachable!("the overview reads no single account")
        }

        fn find_by_email(&self, _email: &str) -> Result<Option<String>, AccountRepositoryError> {
            unreachable!("the overview looks nothing up by address")
        }

        fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
            match self.failure {
                Some(AccountRepositoryError::Unavailable) => {
                    Err(AccountRepositoryError::Unavailable)
                }
                Some(AccountRepositoryError::Corrupted) => Err(AccountRepositoryError::Corrupted),
                Some(AccountRepositoryError::NoNumbersLeft) => {
                    Err(AccountRepositoryError::NoNumbersLeft)
                }
                None => Ok(self.sending.clone()),
            }
        }

        fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
            match self.failure {
                Some(AccountRepositoryError::Unavailable) => {
                    Err(AccountRepositoryError::Unavailable)
                }
                Some(AccountRepositoryError::Corrupted) => Err(AccountRepositoryError::Corrupted),
                Some(AccountRepositoryError::NoNumbersLeft) => {
                    Err(AccountRepositoryError::NoNumbersLeft)
                }
                None => Ok(self
                    .stored
                    .lock()
                    .expect("fake lock is never poisoned")
                    .clone()),
            }
        }

        fn save(
            &self,
            account: &lyonoyl_domain::account::MailAccount,
        ) -> Result<(), AccountRepositoryError> {
            self.stored
                .lock()
                .expect("fake lock is never poisoned")
                .push(account.summary().clone());
            Ok(())
        }

        fn delete(&self, account_id: &str) -> Result<(), AccountRepositoryError> {
            self.stored
                .lock()
                .expect("fake lock is never poisoned")
                .retain(|stored| stored.id() != account_id);
            Ok(())
        }

        fn set_collapsed(
            &self,
            _account_id: &str,
            _collapsed: bool,
        ) -> Result<(), AccountRepositoryError> {
            Ok(())
        }

        fn number_of(&self, _: &str) -> Result<Option<i64>, AccountRepositoryError> {
            unreachable!("this fake is not asked which file the mail is in")
        }
    }

    fn account(id: &str) -> MailAccountSummary {
        MailAccountSummary::new(id, "user@example.com", "User", "imap")
            .expect("test fixture is well-formed")
    }

    #[test]
    fn reports_an_empty_workspace_when_no_account_is_configured() {
        let overview = GetWorkspaceOverview::new(FakeAccounts::default())
            .execute()
            .expect("an empty store is not an error");

        assert!(overview.accounts.is_empty());
        assert!(overview.is_local_first);
    }

    #[test]
    fn returns_stored_accounts() {
        let overview =
            GetWorkspaceOverview::new(FakeAccounts::with(vec![account("a"), account("b")]))
                .execute()
                .expect("stored accounts are returned");

        assert_eq!(overview.accounts.len(), 2);
        assert_eq!(overview.accounts[0].id(), "a");
    }

    #[test]
    fn maps_an_unavailable_store_to_a_distinguishable_error() {
        let error =
            GetWorkspaceOverview::new(FakeAccounts::failing(AccountRepositoryError::Unavailable))
                .execute()
                .expect_err("an unavailable store surfaces as an error");

        assert!(matches!(error, UseCaseError::StorageUnavailable));
    }

    #[test]
    fn maps_corrupted_rows_to_a_separate_error() {
        let error =
            GetWorkspaceOverview::new(FakeAccounts::failing(AccountRepositoryError::Corrupted))
                .execute()
                .expect_err("corrupted data surfaces as an error");

        assert!(matches!(error, UseCaseError::StorageCorrupted));
    }

    #[test]
    fn says_which_accounts_have_somewhere_to_send_from() {
        // Reading and sending are separate servers. A message written from an account nobody has
        // given a sending server would wait in the outbox for ever with nothing said.
        let accounts = FakeAccounts {
            stored: Mutex::new(vec![account("acc-1"), account("acc-2")]),
            sending: vec!["acc-1".to_owned()],
            failure: None,
        };

        let overview = GetWorkspaceOverview::new(accounts)
            .execute()
            .expect("the workspace is readable");

        assert_eq!(overview.able_to_send, vec!["acc-1".to_owned()]);
    }
}
