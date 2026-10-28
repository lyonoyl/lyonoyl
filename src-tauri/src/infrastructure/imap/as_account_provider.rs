//! The one implementation of [`AccountProvider`] there is.
//!
//! An account reached over IMAP holds a password, and a password is the whole of its story: it is
//! sent as it was stored, it cannot be renewed without somebody typing a new one, and there is
//! nobody to tell when it is finished with. Three of the four methods below are therefore very
//! short, and that is the seam working rather than the seam being pointless — the shape is what
//! the next implementation fits into, and this one shows what the shape costs when there is
//! nothing to do.

use secrecy::SecretString;

use crate::ports::account_provider::{AccountCapabilities, AccountProvider, AccountProviderError};
use crate::ports::credential::Credential;
use lyonoyl_domain::account::MailAccount;

/// Accounts that sign in with a password, over IMAP and SMTP.
///
/// Which is every account this application supports — Gmail with an app password, Outlook,
/// Fastmail, iCloud, Proton Bridge and anything else that speaks the standards, without a line
/// of code that knows which of them it is talking to.
pub struct ImapProviderAccount;

impl AccountProvider for ImapProviderAccount {
    fn authenticate(
        &self,
        _account: &MailAccount,
        stored: &SecretString,
    ) -> Result<Credential, AccountProviderError> {
        Ok(Credential::password(stored.clone()))
    }

    fn refresh_credentials(
        &self,
        _account: &MailAccount,
        _stored: &SecretString,
    ) -> Result<Credential, AccountProviderError> {
        // A password is as good as it will ever be. A server refusing one is a fact about the
        // password, and the only thing that changes it is a person typing another — so this says
        // so rather than pretending to try.
        Err(AccountProviderError::NothingToRefresh)
    }

    fn revoke_credentials(
        &self,
        _account: &MailAccount,
        _stored: &SecretString,
    ) -> Result<(), AccountProviderError> {
        // Nobody to tell: a password is chosen, not issued. Removing the account still removes it
        // from this device, which is the half that matters here.
        Ok(())
    }

    fn capabilities(&self) -> AccountCapabilities {
        AccountCapabilities::OF_A_PASSWORD
    }
}
