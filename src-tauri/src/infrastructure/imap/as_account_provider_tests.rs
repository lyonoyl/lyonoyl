use secrecy::{ExposeSecret, SecretString};

use super::as_account_provider::ImapProviderAccount;
use crate::ports::account_provider::{AccountProvider, AccountProviderError};
use crate::ports::credential::Credential;
use lyonoyl_domain::account::{Encryption, MailAccount, MailAccountSummary, ServerSettings};

fn account() -> MailAccount {
    MailAccount::new(
        MailAccountSummary::new("acc-1", "a@example.com", "A", "imap").expect("well-formed"),
        ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid"),
        "a@example.com",
    )
    .expect("well-formed")
}

#[test]
fn a_password_is_sent_as_it_was_stored() {
    let sent = ImapProviderAccount
        .authenticate(&account(), &SecretString::from("correct horse"))
        .expect("a password is a credential");

    match sent {
        Credential::Password(password) => {
            assert_eq!(password.expose_secret(), "correct horse")
        }
        Credential::AccessToken(_) => panic!("a password account does not mint a token"),
    }
}

#[test]
fn there_is_nothing_to_refresh_and_it_says_so() {
    // Not a failure to report to somebody. The caller's answer to this is to ask for another
    // password, and a variant that said "offline" or "unusable" would send them looking for a
    // problem that is not there.
    assert!(matches!(
        ImapProviderAccount.refresh_credentials(&account(), &SecretString::from("x")),
        Err(AccountProviderError::NothingToRefresh)
    ));
}

#[test]
fn revoking_succeeds_because_there_is_nobody_to_tell() {
    // A password is chosen, not issued. Answering with an error here would make removing an
    // account look like it half failed.
    assert_eq!(
        ImapProviderAccount.revoke_credentials(&account(), &SecretString::from("x")),
        Ok(())
    );
}

#[test]
fn a_password_account_says_it_can_neither_renew_nor_revoke() {
    // Read by the interface, so a screen offers what is possible rather than what is imagined.
    let can = ImapProviderAccount.capabilities();

    assert!(!can.can_refresh);
    assert!(!can.can_revoke);
}
