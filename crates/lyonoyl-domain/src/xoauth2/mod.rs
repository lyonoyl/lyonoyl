//! XOAUTH2: how a bearer token is handed to an IMAP or SMTP server.
//!
//! Not a standard. It is a SASL mechanism Google defined and Microsoft copied, and the two
//! publish byte-identical examples — one `0x01` between the user and the token, the literal text
//! `auth=Bearer ` with a single space, and two `0x01` at the end. Both were decoded rather than
//! read, because a mechanism whose separators are invisible characters is one where "looks right"
//! and "is right" are different things.
//!
//! What is here is the string and nothing else. Encoding it is base64, and base64 is a dependency
//! this crate does not have and will not add for one call — so the adapter that sends it encodes
//! it. That split is not a compromise: what the bytes are is a rule, and how they are wrapped for
//! a particular protocol is transport.

use thiserror::Error;

/// The byte that separates the fields. Google writes it `^A`; it is Control-A, `0x01`.
const SEPARATOR: u8 = 0x01;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum Xoauth2Error {
    /// A user name or a token carrying the separator could rewrite the structure around it — a
    /// token holding `\x01auth=Bearer …` would append a second, attacker-chosen field. Refused
    /// here rather than escaped, because the mechanism has no escape.
    #[error("a user name or token may not contain the separator byte")]
    HoldsTheSeparator,
    /// An empty half is not a credential. Sent, it authenticates as nobody with nothing and the
    /// server answers with something that reads like a wrong password.
    #[error("both the user name and the token are required")]
    Empty,
}

/// The initial client response, before it is encoded.
///
/// `user=<user>\x01auth=Bearer <token>\x01\x01`, which is what both providers' published examples
/// decode to exactly.
pub fn initial_response(user: &str, access_token: &str) -> Result<Vec<u8>, Xoauth2Error> {
    if user.is_empty() || access_token.is_empty() {
        return Err(Xoauth2Error::Empty);
    }

    if user.as_bytes().contains(&SEPARATOR) || access_token.as_bytes().contains(&SEPARATOR) {
        return Err(Xoauth2Error::HoldsTheSeparator);
    }

    let mut response = Vec::with_capacity(user.len() + access_token.len() + 22);
    response.extend_from_slice(b"user=");
    response.extend_from_slice(user.as_bytes());
    response.push(SEPARATOR);
    response.extend_from_slice(b"auth=Bearer ");
    response.extend_from_slice(access_token.as_bytes());
    response.push(SEPARATOR);
    response.push(SEPARATOR);

    Ok(response)
}

#[cfg(test)]
mod tests;
