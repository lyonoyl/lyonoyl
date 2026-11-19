use std::sync::Mutex;

use super::*;
use crate::ports::account_repository::AccountRepositoryError;
use lyonoyl_domain::signature::LONGEST_SIGNATURE;

/// Mailboxes by id, and what each keeps; or a store that cannot be reached.
struct Kept {
    signatures: Mutex<Vec<(String, String)>>,
    reachable: bool,
}

impl Kept {
    fn with(accounts: &[&str]) -> Self {
        Self {
            signatures: Mutex::new(
                accounts
                    .iter()
                    .map(|id| ((*id).to_owned(), String::new()))
                    .collect(),
            ),
            reachable: true,
        }
    }

    fn of(&self, account_id: &str) -> String {
        self.signatures
            .lock()
            .expect("test lock")
            .iter()
            .find(|(id, _)| id == account_id)
            .map(|(_, signature)| signature.clone())
            .expect("a mailbox of the test's")
    }
}

impl Signatures for &Kept {
    fn set_signature(
        &self,
        account_id: &str,
        signature: &Signature,
    ) -> Result<bool, AccountRepositoryError> {
        if !self.reachable {
            return Err(AccountRepositoryError::Unavailable);
        }
        let mut signatures = self.signatures.lock().expect("test lock");
        let Some((_, kept)) = signatures.iter_mut().find(|(id, _)| id == account_id) else {
            return Ok(false);
        };
        *kept = signature.as_str().to_owned();
        Ok(true)
    }
}

#[test]
fn the_signature_is_kept_for_its_mailbox_as_the_domain_writes_it() {
    let kept = Kept::with(&["acc-1", "acc-2"]);

    let answered = SetSignature::new(&kept)
        .execute("acc-2", "\r\nKappa Vandel\r\nkappa@example.net  \r\n")
        .expect("kept");

    assert_eq!(answered.as_str(), "Kappa Vandel\nkappa@example.net");
    assert_eq!(kept.of("acc-2"), "Kappa Vandel\nkappa@example.net");
    assert_eq!(kept.of("acc-1"), "", "another mailbox's was touched");
}

#[test]
fn nothing_typed_takes_the_signature_away() {
    let kept = Kept::with(&["acc-1"]);
    SetSignature::new(&kept)
        .execute("acc-1", "Kappa Vandel")
        .expect("kept");

    let answered = SetSignature::new(&kept)
        .execute("acc-1", " \n ")
        .expect("kept");

    assert!(answered.is_empty());
    assert_eq!(kept.of("acc-1"), "");
}

#[test]
fn a_signature_longer_than_the_domain_keeps_is_refused_and_the_old_one_stays() {
    let kept = Kept::with(&["acc-1"]);
    SetSignature::new(&kept)
        .execute("acc-1", "Kappa Vandel")
        .expect("kept");

    let refused = SetSignature::new(&kept).execute("acc-1", &"k".repeat(LONGEST_SIGNATURE + 1));

    assert!(matches!(refused, Err(UseCaseError::SignatureTooLong)));
    assert_eq!(kept.of("acc-1"), "Kappa Vandel");
}

#[test]
fn a_mailbox_that_is_not_here_is_said_to_be_not_here() {
    let kept = Kept::with(&["acc-1"]);

    let refused = SetSignature::new(&kept).execute("acc-9", "Kappa Vandel");

    assert!(matches!(refused, Err(UseCaseError::UnknownAccount)));
}

#[test]
fn a_store_that_cannot_be_reached_says_so() {
    let kept = Kept {
        reachable: false,
        ..Kept::with(&["acc-1"])
    };

    let refused = SetSignature::new(&kept).execute("acc-1", "Kappa Vandel");

    assert!(matches!(refused, Err(UseCaseError::StorageUnavailable)));
}
