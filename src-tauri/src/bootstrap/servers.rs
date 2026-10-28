//! Which servers the mail comes from and goes to, and which message the window opens on.
//!
//! The one place each of them is chosen, for every pass, every message opened, every account
//! checked, every change a person makes and every message sent: a use case is handed whatever this
//! answers, and knows only the port. The build made for screenshots answers with its scene.

/// Where every account's mail is read from: each account's own server.
///
/// The one place that says so, for every pass, every message opened and every account checked or
/// added: a use case is handed whatever this answers, and knows only the port.
#[cfg(not(feature = "screenshots"))]
pub fn mail_servers(
    runtime: tokio::runtime::Handle,
) -> crate::infrastructure::imap::ImapProviderFactory {
    crate::infrastructure::imap::ImapProviderFactory::new(runtime)
}

/// Where every account's mail is read from in the build made for screenshots: the scene it plays,
/// and nowhere else. An account the scene does not have is a server with no connection to it.
#[cfg(feature = "screenshots")]
pub fn mail_servers(_runtime: tokio::runtime::Handle) -> crate::infrastructure::scene::Servers {
    super::scene::played()
}

/// The message the window opens on, with the folder it is read in: none. A person opens what they
/// want to read.
#[cfg(not(feature = "screenshots"))]
pub fn opening() -> Option<(
    lyonoyl_domain::message::MessageId,
    lyonoyl_domain::message::FolderId,
)> {
    None
}

/// The same in the build made for screenshots: the message its scene names, if it names one.
#[cfg(feature = "screenshots")]
pub fn opening() -> Option<(
    lyonoyl_domain::message::MessageId,
    lyonoyl_domain::message::FolderId,
)> {
    super::scene::opened()
}

/// Where what a person did reaches an account's server, and where a copy of what they sent is
/// kept: that server, over a connection opened to act rather than to read.
#[cfg(not(feature = "screenshots"))]
pub fn mail_actions(
    account: &lyonoyl_domain::account::MailAccount,
    credential: &crate::ports::credential::Credential,
    runtime: tokio::runtime::Handle,
) -> Result<
    impl crate::ports::mail_actions::MailActions + crate::ports::mail_actions::SentMail,
    crate::ports::mail_actions::MailActionError,
> {
    crate::infrastructure::imap::actions::ImapMailActions::new(
        crate::infrastructure::imap::imap_account(account, credential),
        runtime,
    )
}

/// The same in the build made for screenshots: the scene, which takes everything and passes none
/// of it on. A server named in a scene is somebody's, and a change made in a scene is not.
#[cfg(feature = "screenshots")]
pub fn mail_actions(
    _account: &lyonoyl_domain::account::MailAccount,
    _credential: &crate::ports::credential::Credential,
    _runtime: tokio::runtime::Handle,
) -> Result<
    impl crate::ports::mail_actions::MailActions + crate::ports::mail_actions::SentMail,
    crate::ports::mail_actions::MailActionError,
> {
    Ok(crate::infrastructure::scene::Agrees)
}

/// What carries a written message out: the account's sending server.
#[cfg(not(feature = "screenshots"))]
pub fn mail_submission(
    submission: &lyonoyl_domain::account::Submission,
    credential: crate::ports::credential::Credential,
    runtime: tokio::runtime::Handle,
) -> impl crate::ports::mail_submission::MailSubmission {
    crate::infrastructure::smtp::SmtpSubmission::new(submission, credential, runtime)
}

/// The same in the build made for screenshots: the scene, which takes the message and sends it
/// nowhere.
#[cfg(feature = "screenshots")]
pub fn mail_submission(
    _submission: &lyonoyl_domain::account::Submission,
    _credential: crate::ports::credential::Credential,
    _runtime: tokio::runtime::Handle,
) -> impl crate::ports::mail_submission::MailSubmission {
    crate::infrastructure::scene::Agrees
}
