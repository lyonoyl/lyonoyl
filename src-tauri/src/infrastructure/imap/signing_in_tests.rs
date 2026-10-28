//! What actually goes on the wire when a session is opened, asserted byte for byte.
//!
//! The XOAUTH2 payload is invisible twice over: it is base64, and inside it the fields are
//! separated by a control character. A test that checked the builder alone would prove the string
//! is right and say nothing about whether it reaches the server, in the right command, encoded the
//! right way. So this drives a scripted server and asserts the exact line.

use secrecy::SecretString;
use tokio_test::io::Builder;

use super::read_only::ReadOnly;
use super::{open_session, ImapAccount};
use crate::infrastructure::net::{Deadline, Encryption, Endpoint, Silence};
use crate::ports::credential::Credential;

fn endpoint() -> Endpoint {
    Endpoint {
        host: "imap.example.com".to_owned(),
        port: 993,
        encryption: Encryption::Tls,
    }
}

fn account(credential: Credential) -> ImapAccount {
    ImapAccount {
        endpoint: endpoint(),
        username: "someuser@example.com".to_owned(),
        credential,
    }
}

/// Google's own published example, on the wire, in the command the provider expects.
///
/// The base64 is Google's, copied from their documentation rather than computed here — an
/// expectation this build produced would pass whatever this build did.
#[tokio::test]
async fn a_token_is_sent_as_the_xoauth2_payload_google_publishes() {
    let mut script = Builder::new();
    script
        // A real server greets. Taking this out of the script is how the token path passed while
        // it was broken.
        .read(b"* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=XOAUTH2] ready\r\n")
        .write(b"A0001 AUTHENTICATE XOAUTH2\r\n")
        .read(b"+ \r\n")
        .write(b"dXNlcj1zb21ldXNlckBleGFtcGxlLmNvbQFhdXRoPUJlYXJlciB5YTI5LnZGOWRmdDRxbVRjMk52YjNSbGNrQmhkSFJoZG1semRHRXVZMjl0Q2cBAQ==\r\n")
        .read(b"A0001 OK Success\r\n");

    let account = account(Credential::AccessToken(SecretString::from(
        "ya29.vF9dft4qmTc2Nvb3RlckBhdHRhdmlzdGEuY29tCg",
    )));

    let opened = open_session(client_over(script), &account).await;

    assert!(opened.is_ok(), "the scripted server accepted the payload");
}

/// A password still goes in LOGIN. The seam must not have changed what already worked.
#[tokio::test]
async fn a_password_still_goes_in_login() {
    let mut script = Builder::new();
    script
        .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
        .write(b"A0001 LOGIN \"someuser@example.com\" \"secret\"\r\n")
        .read(b"A0001 OK LOGIN completed\r\n");

    let account = account(Credential::Password(SecretString::from("secret")));

    let opened = open_session(client_over(script), &account).await;

    assert!(opened.is_ok(), "the scripted server accepted the login");
}

/// A refused token comes back as a refusal, not as a protocol fault.
///
/// The provider answers a bad token with a challenge carrying a JSON explanation and then a NO.
/// What the person has to do about it is sign in again, which is the same thing a wrong password
/// means — so the two arrive here as the same kind of failure and are told apart above.
#[tokio::test]
async fn a_refused_token_is_a_refusal() {
    let mut script = Builder::new();
    script
        // A real server greets. Taking this out of the script is how the token path passed while
        // it was broken.
        .read(b"* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=XOAUTH2] ready\r\n")
        .write(b"A0001 AUTHENTICATE XOAUTH2\r\n")
        .read(b"+ \r\n")
        .write(b"dXNlcj1zb21ldXNlckBleGFtcGxlLmNvbQFhdXRoPUJlYXJlciBleHBpcmVkAQE=\r\n")
        // What Google actually sends: a challenge holding the reason, base64-encoded.
        .read(b"+ eyJzdGF0dXMiOiI0MDEiLCJzY2hlbWVzIjoiQmVhcmVyIiwic2NvcGUiOiJodHRwczovL21haWwuZ29vZ2xlLmNvbS8ifQ==\r\n")
        // The mechanism's answer to that is an empty line.
        .write(b"\r\n")
        .read(b"A0001 NO Invalid credentials (Failure)\r\n");

    let account = account(Credential::AccessToken(SecretString::from("expired")));

    let opened = open_session(client_over(script), &account).await;

    assert!(opened.is_err(), "an expired token is refused");
}

/// A server that turns the connection away before anything is asked is not a wrong password.
///
/// `* BYE` as the greeting is what a server says when it will not serve this connection at all —
/// too many of them from one account, a maintenance window, an address not serving today. Told
/// apart because the answers differ: one is waited out, the other is somebody being asked to type
/// their password again for no reason.
#[tokio::test]
async fn a_server_that_says_goodbye_first_is_offline_rather_than_a_refusal() {
    let mut script = Builder::new();
    script.read(b"* BYE Too many simultaneous connections\r\n");

    let account = account(Credential::Password(SecretString::from("secret")));

    let opened = open_session(client_over(script), &account).await;

    assert!(matches!(
        opened.err(),
        Some(crate::ports::mail_provider::MailProviderError::Offline)
    ));
}

/// A client built the way the production one is: wrapped so every command has a deadline.
///
/// Not a bare mock. The wrapper is what stops a session waiting for ever, and a test that skipped
/// it would be exercising a different object from the one that ships — and would need the gate
/// that forbids exactly that to be waved through for this file.
fn client_over(script: Builder) -> async_imap::Client<Deadline<ReadOnly<tokio_test::io::Mock>>> {
    let mut script = script;

    async_imap::Client::new(Deadline::over(
        ReadOnly::guarding(script.build()),
        Silence::of(std::time::Duration::from_secs(5)),
    ))
}
