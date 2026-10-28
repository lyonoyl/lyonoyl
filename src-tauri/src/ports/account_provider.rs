// Only `ImapProviderAccount` implements this, and the two methods below that a password account has no
// work for are unreached while nothing other than IMAP does.
#![allow(dead_code)]

use secrecy::SecretString;
use thiserror::Error;

use crate::ports::credential::Credential;
use lyonoyl_domain::account::MailAccount;

/// How an account proves who it is, whatever kind of account it is.
///
/// One implementation exists, `ImapProviderAccount`: this application supports generic IMAP and SMTP,
/// which reaches Gmail with an app password, Outlook, Fastmail, iCloud and Proton Bridge alike, and
/// it carries no logic particular to any of them.
///
/// The trait is the seam. A provider that signs in through a browser and holds a token that
/// expires is a **new implementation of this**, not a change to the four layers above it. Nothing
/// here names a provider, carries an identifier, or knows what OAuth is.
pub trait AccountProvider: Send + Sync {
    /// The credential to send, for an account and the secret stored against it.
    ///
    /// For a password account this is the password, handed straight through. For an account that
    /// signs in some other way it is where a token would be minted or read from a cache — which is
    /// why this returns a credential rather than taking one: what to send is the provider's answer,
    /// not the caller's.
    fn authenticate(
        &self,
        account: &MailAccount,
        stored: &SecretString,
    ) -> Result<Credential, AccountProviderError>;

    /// Buys a fresh credential when the one in hand has stopped being accepted.
    ///
    /// A password account has nothing to refresh: the password is as good as it will ever be, and
    /// a server refusing it means the person has to supply another. So this answers
    /// [`AccountProviderError::NothingToRefresh`], and the caller's answer to that is to ask.
    fn refresh_credentials(
        &self,
        account: &MailAccount,
        stored: &SecretString,
    ) -> Result<Credential, AccountProviderError>;

    /// Tells whoever issued the credential that it is no longer wanted.
    ///
    /// Removing an account removes its secrets from this device either way; this is the half that
    /// reaches outward. A password account has nobody to tell — a password is not issued, it is
    /// chosen — so this does nothing and says so.
    fn revoke_credentials(
        &self,
        account: &MailAccount,
        stored: &SecretString,
    ) -> Result<(), AccountProviderError>;

    /// What this kind of account can be asked to do.
    ///
    /// Read by the interface, so that a screen offers what is possible rather than what is
    /// imagined. Every account answers the same, and nothing branches on the kind of account;
    /// whatever does, asks here.
    fn capabilities(&self) -> AccountCapabilities;
}

/// What an account supports, as facts rather than as a provider's name.
///
/// Never "is this Gmail". A screen that asks which provider an account belongs to is a screen that
/// will be wrong about the next one; a screen that asks what the account can do keeps working.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountCapabilities {
    /// Whether the credential can be renewed without asking the person for anything.
    ///
    /// False for a password: when a server stops accepting one, somebody has to type another.
    pub can_refresh: bool,
    /// Whether whoever issued the credential can be told it is finished with.
    pub can_revoke: bool,
}

impl AccountCapabilities {
    /// What an account holding a password can do, which is neither of those things.
    pub const OF_A_PASSWORD: Self = Self {
        can_refresh: false,
        can_revoke: false,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AccountProviderError {
    /// This kind of account has nothing to refresh. Not a failure: the answer is to ask the person.
    #[error("this account's credential cannot be renewed by itself")]
    NothingToRefresh,
    /// Whoever issues the credential could not be reached.
    #[error("the provider cannot be reached")]
    Offline,
    /// The stored secret is not one this provider can use.
    #[error("the stored credential cannot be used")]
    Unusable,
}
