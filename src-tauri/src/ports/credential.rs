// The token half is built by whatever obtains one, and nothing does. What is here is the seam and
// the protocol work.
#![allow(dead_code)]

use secrecy::SecretString;

/// What is sent to a mail server to prove who is asking.
///
/// Two shapes, because there are two, and a single `SecretString` could not tell them apart: a
/// password goes in `LOGIN` and lasts until somebody changes it; an access token goes in
/// `AUTHENTICATE XOAUTH2` and is measured in minutes. Everything above this point hands one over
/// and does not care which it is; the adapter that speaks the protocol is where the difference
/// becomes two different commands.
///
/// A port type rather than a domain one: it names `SecretString`, and the domain crate depends on
/// `thiserror` and nothing else.
#[derive(Clone)]
pub enum Credential {
    /// A password, or a provider's app-specific password, which is a password by another name.
    Password(SecretString),
    /// A bearer token bought with OAuth. Short-lived by nature: whoever hands one over has already
    /// decided it is fresh enough, and the server is the one that says otherwise.
    AccessToken(SecretString),
}

impl Credential {
    /// A password, which is what everything did before there was anything else.
    pub fn password(secret: SecretString) -> Self {
        Self::Password(secret)
    }
}

/// Never the secret itself, in any form. This type exists to be handled, not to be read: a
/// `Debug` that showed either half would put it in every log line that prints a struct holding it.
impl std::fmt::Debug for Credential {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(match self {
            Self::Password(_) => "Credential::Password(…)",
            Self::AccessToken(_) => "Credential::AccessToken(…)",
        })
    }
}
