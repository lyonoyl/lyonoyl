//! Finding where a person's mail lives, from their address alone.
//!
//! An address says nothing about which machine holds the mail, so somebody has to be asked — and
//! who that somebody is decides who learns that this person uses this domain. That is why the
//! order is here, in a use case anybody can read, rather than inside whatever does the asking.
//!
//! This device first, ours second, and the domain's own third. An account already here at the same
//! domain tells nobody anything and is the one answer somebody has already seen work: its servers
//! answered on this machine. The table compiled into this build tells nobody anything either,
//! because there is nobody to tell. The domain itself is next and is nearly as private: what it
//! learns is that somebody is setting up mail there, and it is already the one holding that mail.
//! Anything further out — a directory somebody else keeps, which learns the domain of a stranger
//! who has consented to nothing — is not asked here, and when it is, it will say so on screen
//! first.

use crate::application::error::UseCaseError;
use crate::ports::account_repository::AccountRepository;
use crate::ports::provider_directory::{ProviderDirectory, Published};
use lyonoyl_domain::account::MailAccount;
use lyonoyl_domain::provider::{domain_of, known_for, ServerSettings, Submission};

/// Where the answer came from, which the person is told.
///
/// Not decoration: "we already knew" and "your provider says so" are different degrees of
/// confidence, and one of them means a machine somewhere was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// From an account already on this device at the same domain. Nothing left this machine.
    OnThisDevice,
    /// From the table this build carries. Nothing left this machine.
    Carried,
    /// From the domain itself, which published them.
    PublishedByTheDomain,
}

/// What an account signs in as, said in terms of its own address.
///
/// A shape rather than a user name, and that is the point of it: a user name another account signs
/// in with has no way to travel in this, so nothing one account signs in with can reach the form of
/// another. What reaches the form is built from the address being added and from nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInAs {
    /// The whole address. What most servers expect, and what the form falls back to by itself.
    Address,
    /// The part of the address before the `@`.
    LocalPart,
}

impl SignInAs {
    /// The user name this shape gives the address being added.
    pub fn user_name_for(self, address: &str) -> String {
        let address = address.trim();

        match self {
            Self::Address => address.to_owned(),
            Self::LocalPart => address
                .rsplit_once('@')
                .map_or(address, |(local, _)| local)
                .to_owned(),
        }
    }

    /// The shape of the user name an account signs in with, read against that account's address.
    ///
    /// Compared without letter case, the way the store compares addresses. A user name that is
    /// neither the address nor its local part is something only that account's owner knows, so it
    /// is not a shape at all: the answer is the address, which is what the form would have filled
    /// in had nobody been asked.
    fn of(user_name: &str, address: &str) -> Self {
        let user_name = user_name.trim();
        let address = address.trim();

        if user_name.eq_ignore_ascii_case(address) {
            return Self::Address;
        }

        match address.rsplit_once('@') {
            Some((local, _)) if user_name.eq_ignore_ascii_case(local) => Self::LocalPart,
            _ => Self::Address,
        }
    }
}

/// What to sign in to each server as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignIn {
    pub imap: SignInAs,
    /// Only meaningful where there is a sending server to sign in to.
    pub submission: SignInAs,
}

impl SignIn {
    /// The address for both, which is what every answer but an account on this device gives.
    pub const ADDRESS: Self = Self {
        imap: SignInAs::Address,
        submission: SignInAs::Address,
    };
}

/// What came of looking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settings {
    /// Where the mail is, what to sign in to it as, and who said so.
    Known {
        servers: Box<ServerSettings>,
        sign_in: SignIn,
        found: Found,
    },
    /// Nobody who was asked knows. The person types the settings in, which is what they would have
    /// done anyway — so this is an ordinary answer rather than a failure.
    Unknown,
}

/// Looks for the servers behind one address.
pub struct FindServerSettings<A: AccountRepository, D: ProviderDirectory> {
    accounts: A,
    directory: D,
}

impl<A: AccountRepository, D: ProviderDirectory> FindServerSettings<A, D> {
    pub fn new(accounts: A, directory: D) -> Self {
        Self {
            accounts,
            directory,
        }
    }

    /// Asks, in order, until something answers.
    ///
    /// Run when a person asks for it and never while they type: a cascade on every keystroke would
    /// send half-finished domains to whoever owns them, and the half-finished ones are not even
    /// this person's.
    ///
    /// `correcting` names the account a screen is correcting, if it is correcting one. Its own
    /// servers are what is being corrected, so they are no answer about where its mail lives.
    pub fn execute(
        &self,
        address: &str,
        correcting: Option<&str>,
    ) -> Result<Settings, UseCaseError> {
        // Not one address, so there is nothing to ask about and nobody to ask. Answered rather
        // than refused: a person halfway through typing has not made a mistake.
        let Some(domain) = domain_of(address) else {
            return Ok(Settings::Unknown);
        };

        if let Some(beside) = self.beside(address, &domain, correcting) {
            return Ok(Settings::Known {
                servers: Box::new(servers_of(&beside)),
                sign_in: SignIn {
                    imap: SignInAs::of(beside.imap_username(), beside.summary().email()),
                    submission: beside.submission().map_or(SignInAs::Address, |sending| {
                        SignInAs::of(sending.username(), beside.summary().email())
                    }),
                },
                found: Found::OnThisDevice,
            });
        }

        if let Some(settings) = known_for(&domain) {
            return Ok(Settings::Known {
                servers: Box::new(settings),
                sign_in: SignIn::ADDRESS,
                found: Found::Carried,
            });
        }

        // A domain that cannot be reached is not a domain that publishes nothing, and neither is
        // an answer to give up on: the person can still type the settings in. So both come back
        // as "nobody knows" here, and the difference lives where it can be acted on — in the
        // adapter, which decides whether asking again is worth it.
        match self.directory.published_by(&domain) {
            Ok(Published::Settings(settings)) => Ok(Settings::Known {
                servers: settings,
                sign_in: SignIn::ADDRESS,
                found: Found::PublishedByTheDomain,
            }),
            Ok(Published::Nothing) | Err(_) => Ok(Settings::Unknown),
        }
    }

    /// An account already on this device at the same domain, if there is one to take servers from.
    ///
    /// The same domain means the part after the `@`, without letter case and exactly: a subdomain
    /// is somebody else's servers as often as it is the same ones. The account being corrected and
    /// an account with this very address are not asked, because neither is another mailbox.
    ///
    /// One that can send is preferred, since it answers for both servers; among equals, the first
    /// the store lists, which is the one added first — the one somebody most likely typed the
    /// servers in for. A store that cannot be read answers nothing rather than failing: the rest of
    /// the cascade still works without it.
    fn beside(&self, address: &str, domain: &str, correcting: Option<&str>) -> Option<MailAccount> {
        let address = address.trim();

        let at_this_domain: Vec<MailAccount> = self
            .accounts
            .list()
            .ok()?
            .into_iter()
            .filter(|listed| Some(listed.id()) != correcting)
            .filter(|listed| !listed.email().trim().eq_ignore_ascii_case(address))
            .filter(|listed| {
                listed
                    .email()
                    .trim()
                    .rsplit_once('@')
                    .is_some_and(|(_, theirs)| theirs.eq_ignore_ascii_case(domain))
            })
            // Read whole only once it is known to be at this domain. One that went between the two
            // questions, or will not read, is passed over rather than allowed to stop the others.
            .filter_map(|listed| self.accounts.find(listed.id()).ok().flatten())
            .collect();

        let able_to_send = at_this_domain
            .iter()
            .position(|account| account.submission().is_some());

        at_this_domain.into_iter().nth(able_to_send.unwrap_or(0))
    }
}

/// The servers of an account, without anything it signs in with.
fn servers_of(account: &MailAccount) -> ServerSettings {
    ServerSettings {
        imap_host: account.imap().host().to_owned(),
        imap_port: account.imap().port(),
        imap_encryption: account.imap().encryption(),
        smtp: account.submission().map(|sending| Submission {
            host: sending.server().host().to_owned(),
            port: sending.server().port(),
            encryption: sending.server().encryption(),
        }),
    }
}

#[cfg(test)]
mod tests;
