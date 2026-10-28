use std::fmt;

use crate::error::DomainError;

/// How the connection to a mail server is protected.
///
/// The numbers are stored in the profile database and must not be renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encryption {
    /// Encrypted from the first byte. What almost every provider expects, on port 993.
    #[default]
    Tls = 0,
    /// A plain connection upgraded by a protocol command, on port 143.
    StartTls = 1,
    /// No encryption. Accepted only when the server is on this machine.
    None = 2,
}

impl Encryption {
    pub fn from_stored(value: i64) -> Self {
        match value {
            1 => Self::StartTls,
            2 => Self::None,
            _ => Self::Tls,
        }
    }

    pub fn stored(self) -> i64 {
        self as i64
    }
}

/// Where a mail server is and how to reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSettings {
    host: String,
    port: u16,
    encryption: Encryption,
}

impl ServerSettings {
    /// Validates what the user typed. A host with spaces or a scheme in it is the usual mistake:
    /// people paste `imaps://mail.example.com` out of a help page.
    pub fn new(
        host: impl Into<String>,
        port: u16,
        encryption: Encryption,
    ) -> Result<Self, DomainError> {
        let host = host.into();
        let host = host.trim().to_owned();

        if host.is_empty() {
            return Err(DomainError::EmptyHost);
        }
        if host.contains(char::is_whitespace) || host.contains("://") || host.contains('/') {
            return Err(DomainError::MalformedHost);
        }
        if port == 0 {
            return Err(DomainError::InvalidPort);
        }

        Ok(Self {
            host,
            port,
            encryption,
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn encryption(&self) -> Encryption {
        self.encryption
    }
}

/// The port a provider is expected on, offered as a default when the user picks an encryption.
pub fn default_imap_port(encryption: Encryption) -> u16 {
    match encryption {
        Encryption::Tls => 993,
        Encryption::StartTls | Encryption::None => 143,
    }
}

/// Non-secret account metadata.
///
/// Passwords and refresh tokens are not stored here; they live in the store this device keeps its
/// secrets in.
/// Fields are private so an invalid account cannot be constructed.
#[derive(Clone, PartialEq, Eq)]
pub struct MailAccountSummary {
    id: String,
    email: String,
    display_name: String,
    provider: String,
}

impl MailAccountSummary {
    /// Validates the invariants of an account. Returns [`DomainError`] rather than panicking:
    /// the input may come from a database written by an older version of the app.
    pub fn new(
        id: impl Into<String>,
        email: impl Into<String>,
        display_name: impl Into<String>,
        provider: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let id = id.into();
        if id.trim().is_empty() {
            return Err(DomainError::EmptyAccountId);
        }

        let email = email.into();
        let trimmed = email.trim();
        if trimmed.is_empty() {
            return Err(DomainError::EmptyEmail);
        }
        // A deliberately minimal check: full RFC 5322 validation belongs to the address parser
        // that arrives with the composer. Here we only reject what is certainly not an address.
        match trimmed.split_once('@') {
            Some((local, domain)) if !local.is_empty() && domain.contains('.') => {}
            _ => return Err(DomainError::MalformedEmail),
        }

        let display_name = display_name.into();
        let provider = provider.into();
        // An empty display name is legitimate: the UI falls back to the address.
        Ok(Self {
            id,
            email: trimmed.to_owned(),
            display_name,
            provider,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }
}

/// Where a written message is handed over, and who hands it over.
///
/// Separate from the reading settings because they are separate servers, and separately optional:
/// an account can be read without being able to send, and that is an ordinary state — mail arrives
/// either way, and a message written from it waits with a reason rather than disappearing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    server: ServerSettings,
    username: String,
}

impl Submission {
    /// The user name is what the server expects. Usually the address, and not always: a provider
    /// that wants something else for reading usually wants the same something else for sending.
    pub fn new(server: ServerSettings, username: impl Into<String>) -> Result<Self, DomainError> {
        let username = username.into().trim().to_owned();
        if username.is_empty() {
            return Err(DomainError::EmptyUsername);
        }

        Ok(Self { server, username })
    }

    pub fn server(&self) -> &ServerSettings {
        &self.server
    }

    pub fn username(&self) -> &str {
        &self.username
    }
}

/// An account as it is configured: who it is, and how to reach its mail.
///
/// The password is not here. It lives in the store this device keeps its secrets in, under a name
/// derived from the account identifier, and this type has no way to carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailAccount {
    summary: MailAccountSummary,
    imap: ServerSettings,
    imap_username: String,
    submission: Option<Submission>,
}

impl MailAccount {
    /// The user name is what the server expects, which is usually but not always the address.
    pub fn new(
        summary: MailAccountSummary,
        imap: ServerSettings,
        imap_username: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let imap_username = imap_username.into().trim().to_owned();
        if imap_username.is_empty() {
            return Err(DomainError::EmptyUsername);
        }

        Ok(Self {
            summary,
            imap,
            imap_username,
            submission: None,
        })
    }

    /// The same account, able to send.
    ///
    /// A separate step rather than another argument: an account is worth having before anybody has
    /// said how its mail goes out, and the reading half is what has to work first.
    pub fn sending_through(self, submission: Submission) -> Self {
        Self {
            submission: Some(submission),
            ..self
        }
    }

    /// Where its messages are handed over, if anybody has said.
    pub fn submission(&self) -> Option<&Submission> {
        self.submission.as_ref()
    }

    /// The same account under an identifier it already has here.
    ///
    /// Used when a password is supplied again for an account already stored: the identifier names
    /// its folders, its mail and its place in the operation log, and a new one would orphan them.
    pub fn renamed_to(self, id: impl Into<String>) -> Result<Self, DomainError> {
        let summary = MailAccountSummary::new(
            id,
            self.summary.email,
            self.summary.display_name,
            self.summary.provider,
        )?;

        Ok(Self {
            summary,
            imap: self.imap,
            imap_username: self.imap_username,
            submission: self.submission,
        })
    }

    pub fn summary(&self) -> &MailAccountSummary {
        &self.summary
    }

    pub fn imap(&self) -> &ServerSettings {
        &self.imap
    }

    pub fn imap_username(&self) -> &str {
        &self.imap_username
    }
}

/// `Debug` is implemented by hand rather than derived.
///
/// The address and the display name are personal data. The domain part is kept for diagnostics.
impl fmt::Debug for MailAccountSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MailAccountSummary")
            .field("id", &self.id)
            .field("email", &RedactedAddress(&self.email))
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

/// Prints an address as `u***@example.com`: enough to correlate, not enough to leak.
struct RedactedAddress<'a>(&'a str);

impl fmt::Debug for RedactedAddress<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.split_once('@') {
            Some((local, domain)) => {
                let head = local.chars().next().unwrap_or('*');
                write!(formatter, "\"{head}***@{domain}\"")
            }
            None => formatter.write_str("\"[redacted]\""),
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn an_account_that_can_send_keeps_that_through_a_rename() {
        // The identifier names its folders, its mail and its place in the operation log. Losing
        // where its messages go while gaining the right identifier would leave an account that
        // reads mail and silently cannot answer any of it.
        let sending = Submission::new(
            ServerSettings::new("smtp.example.com", 587, Encryption::StartTls).expect("valid"),
            "zeta@example.com",
        )
        .expect("valid");

        let account = MailAccount::new(
            MailAccountSummary::new("temporary", "zeta@example.com", "Zeta", "imap")
                .expect("valid"),
            ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid"),
            "zeta@example.com",
        )
        .expect("valid")
        .sending_through(sending.clone());

        let renamed = account.renamed_to("account-1").expect("valid");

        assert_eq!(renamed.submission(), Some(&sending));
    }

    #[test]
    fn a_server_to_send_through_with_nobody_to_sign_in_as_is_refused() {
        assert!(Submission::new(
            ServerSettings::new("smtp.example.com", 587, Encryption::StartTls).expect("valid"),
            "   ",
        )
        .is_err());
    }

    #[test]
    fn an_account_nobody_has_given_a_sending_server_is_still_an_account() {
        // Mail arrives either way, and a client that refused to add the account until somebody
        // knew their SMTP port would be a client nobody gets past the first screen of.
        let account = MailAccount::new(
            MailAccountSummary::new("account-1", "zeta@example.com", "Zeta", "imap")
                .expect("valid"),
            ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid"),
            "zeta@example.com",
        )
        .expect("valid");

        assert_eq!(account.submission(), None);
    }
    use super::*;

    fn valid() -> Result<MailAccountSummary, DomainError> {
        MailAccountSummary::new("acc-1", "user@example.com", "User", "imap")
    }

    fn settings() -> ServerSettings {
        ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid")
    }

    #[test]
    fn an_account_keeps_the_user_name_the_server_expects() {
        let account = MailAccount::new(
            valid().expect("valid summary"),
            settings(),
            "  user@example.com  ",
        )
        .expect("valid account");

        assert_eq!(account.imap_username(), "user@example.com");
        assert_eq!(account.imap().port(), 993);
    }

    #[test]
    fn an_account_without_a_user_name_is_rejected() {
        assert_eq!(
            MailAccount::new(valid().expect("valid summary"), settings(), "  ")
                .expect_err("no user name"),
            DomainError::EmptyUsername
        );
    }

    #[test]
    fn accepts_the_settings_a_provider_publishes() {
        let settings = ServerSettings::new("imap.example.com", 993, Encryption::Tls)
            .expect("a well-formed server");
        assert_eq!(settings.host(), "imap.example.com");
        assert_eq!(settings.port(), 993);
    }

    #[test]
    fn trims_what_was_pasted() {
        let settings = ServerSettings::new("  imap.example.com  ", 993, Encryption::Tls)
            .expect("surrounding whitespace is tolerated");
        assert_eq!(settings.host(), "imap.example.com");
    }

    #[test]
    fn rejects_a_host_that_is_really_a_url() {
        // What people paste out of a help page.
        for host in [
            "imaps://imap.example.com",
            "imap.example.com/inbox",
            "imap example com",
        ] {
            assert_eq!(
                ServerSettings::new(host, 993, Encryption::Tls).expect_err("not a host"),
                DomainError::MalformedHost,
                "input: {host}"
            );
        }
    }

    #[test]
    fn rejects_an_empty_host_and_a_zero_port() {
        assert_eq!(
            ServerSettings::new("   ", 993, Encryption::Tls).expect_err("no host"),
            DomainError::EmptyHost
        );
        assert_eq!(
            ServerSettings::new("imap.example.com", 0, Encryption::Tls).expect_err("no port"),
            DomainError::InvalidPort
        );
    }

    #[test]
    fn each_encryption_has_the_port_providers_publish() {
        assert_eq!(default_imap_port(Encryption::Tls), 993);
        assert_eq!(default_imap_port(Encryption::StartTls), 143);
        assert_eq!(default_imap_port(Encryption::None), 143);
    }

    #[test]
    fn encryption_survives_storage_as_a_number() {
        for encryption in [Encryption::Tls, Encryption::StartTls, Encryption::None] {
            assert_eq!(Encryption::from_stored(encryption.stored()), encryption);
        }
        assert_eq!(Encryption::from_stored(99), Encryption::Tls);
    }

    #[test]
    fn accepts_a_well_formed_account() {
        let account = valid().expect("a well-formed account is accepted");
        assert_eq!(account.email(), "user@example.com");
        assert_eq!(account.id(), "acc-1");
    }

    #[test]
    fn trims_surrounding_whitespace_in_the_address() {
        let account = MailAccountSummary::new("acc-1", "  user@example.com  ", "User", "imap")
            .expect("surrounding whitespace is tolerated");
        assert_eq!(account.email(), "user@example.com");
    }

    #[test]
    fn rejects_an_empty_identifier() {
        let error = MailAccountSummary::new("   ", "user@example.com", "User", "imap")
            .expect_err("an empty identifier is rejected");
        assert_eq!(error, DomainError::EmptyAccountId);
    }

    #[test]
    fn rejects_an_empty_address() {
        let error = MailAccountSummary::new("acc-1", "", "User", "imap")
            .expect_err("an empty address is rejected");
        assert_eq!(error, DomainError::EmptyEmail);
    }

    #[test]
    fn rejects_addresses_without_a_separator_or_domain_dot() {
        for candidate in ["user", "@example.com", "user@", "user@localhost"] {
            let error = MailAccountSummary::new("acc-1", candidate, "User", "imap")
                .expect_err("a malformed address is rejected");
            assert_eq!(error, DomainError::MalformedEmail, "input: {candidate}");
        }
    }

    #[test]
    fn allows_an_empty_display_name() {
        let account = MailAccountSummary::new("acc-1", "user@example.com", "", "imap")
            .expect("an empty display name falls back to the address in the UI");
        assert_eq!(account.display_name(), "");
    }
}
